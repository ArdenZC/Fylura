"""Hosted Apple Silicon process observations; no GUI/release acceptance claim."""
import datetime
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.parse

STARTUP_CHECKPOINT_PATTERN = re.compile(r'^native_qa startup_checkpoint=([a-z_]+)$', re.MULTILINE)
RUNTIME_PACKAGES = ('tauri', 'tao', 'wry', 'objc2', 'objc2-exception-helper')
PRE_IDLE_SAMPLE_INTERVAL_SECONDS = 10
TRACE_EVENT_LIMIT = 256
ERROR_LINE_LIMIT = 80
PATH_TOKEN_PATTERN = re.compile(r'(?<![\w:])/(?:[^\s\t\r\n\"\'<>]+)')
ERROR_PATTERN = re.compile(r'\b(error|failed|panic|panicked|exception|abort|fatal)\b', re.IGNORECASE)


def command(*args, timeout=15):
    return subprocess.check_output(args, text=True, timeout=timeout).strip()


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def cpu_seconds(value):
    parts = value.replace('-', ':').split(':')
    if len(parts) == 4:
        return int(parts[0]) * 86400 + int(parts[1]) * 3600 + int(parts[2]) * 60 + float(parts[3])
    return sum(float(part) * 60 ** index for index, part in enumerate(reversed(parts)))


def safe_text(value, private_roots=()):
    text = str(value)
    for root in sorted((str(root) for root in private_roots if root), key=len, reverse=True):
        text = text.replace(root, '[TASK_PATH]')
    return PATH_TOKEN_PATTERN.sub('[PATH]', text)


def locked_runtime_versions():
    lockfile = pathlib.Path('src-tauri/Cargo.lock')
    if not lockfile.exists():
        return {}
    versions = {name: [] for name in RUNTIME_PACKAGES}
    for package in re.split(r'(?=^\[\[package\]\]$)', lockfile.read_text(), flags=re.MULTILINE):
        name = re.search(r'^name = "([^"]+)"$', package, flags=re.MULTILINE)
        version = re.search(r'^version = "([^"]+)"$', package, flags=re.MULTILINE)
        if name and version and name.group(1) in versions:
            versions[name.group(1)].append(version.group(1))
    return {name: sorted(set(found)) for name, found in versions.items() if found}


def startup_trace(log):
    checkpoints = STARTUP_CHECKPOINT_PATTERN.findall(log)
    if 'process_main_entered' in checkpoints and 'tauri_setup_entered' not in checkpoints:
        attribution = 'abort before Zen user setup'
    elif 'tauri_setup_entered' in checkpoints:
        attribution = 'Zen setup entered'
    elif checkpoints:
        attribution = 'partial startup trace; setup boundary unverified'
    else:
        attribution = 'startup trace unavailable'
    return checkpoints, attribution


class CoordinatorTrace:
    """Incrementally read the append-only native-QA trace and retain a bounded tail."""

    def __init__(self):
        self.offset = 0
        self.partial = b''
        self.present = False
        self.bytes_read = 0
        self.event_count = 0
        self.dropped_events = 0
        self.events = []
        self.errors = []
        self.dropped_errors = 0
        self.counts = {}
        self.read_error = None
        self.truncated = False

    def refresh(self, path):
        if path is None or not path.exists():
            return
        try:
            size = path.stat().st_size
            if size < self.offset:
                self.truncated = True
                self.offset = 0
                self.partial = b''
            with path.open('rb') as trace_file:
                trace_file.seek(self.offset)
                data = trace_file.read()
                self.offset += len(data)
            if not data:
                self.present = True
                return
            self.present = True
            self.bytes_read += len(data)
            chunks = (self.partial + data).split(b'\n')
            self.partial = chunks.pop()
            for chunk in chunks:
                self._record(chunk.decode('utf-8', errors='replace').rstrip('\r'))
        except OSError as error:
            self.read_error = f'{type(error).__name__}: {error}'

    def _record(self, line):
        if not line:
            return
        event = line.partition('\t')[0]
        self.event_count += 1
        self.counts[event] = self.counts.get(event, 0) + 1
        if len(self.events) == TRACE_EVENT_LIMIT:
            self.events.pop(0)
            self.dropped_events += 1
        self.events.append(line)
        if event.endswith('_error'):
            if len(self.errors) == 32:
                self.errors.pop(0)
                self.dropped_errors += 1
            self.errors.append(line)

    def snapshot(self):
        return {
            'present': self.present,
            'bytesRead': self.bytes_read,
            'eventCount': self.event_count,
            'eventCounts': dict(sorted(self.counts.items())),
            'lastEvent': self.events[-1].partition('\t')[0] if self.events else None,
            'eventsRetained': len(self.events),
            'eventsDropped': self.dropped_events,
            'errorsDropped': self.dropped_errors,
            'truncated': self.truncated,
            'readError': self.read_error,
            'events': self.events[:],
            'errors': self.errors[:],
        }


def process_observation(pid, started, phase, previous_cpu=None):
    ps = subprocess.run(
        ['ps', '-p', str(pid), '-o', 'state=', '-o', 'rss=', '-o', 'time=', '-o', 'etime='],
        capture_output=True, text=True, timeout=5)
    if ps.returncode != 0 or not ps.stdout.strip():
        raise RuntimeError(f'process observation failed: {ps.stderr.strip() or "process missing"}')
    fields = ps.stdout.split()
    if len(fields) != 4:
        raise RuntimeError(f'unexpected process observation fields: {len(fields)}')
    state, rss, cpu, lifetime = fields
    fd_result = subprocess.run(
        ['lsof', '-a', '-p', str(pid), '-Ff'],
        capture_output=True, text=True, timeout=8)
    if fd_result.returncode != 0:
        raise RuntimeError(f'FD observation failed: {fd_result.stderr.strip() or fd_result.returncode}')
    total_cpu = cpu_seconds(cpu)
    return {
        'phase': phase,
        'timestamp': now(),
        'pid': pid,
        'alive': True,
        'processState': state,
        'rssBytes': int(rss) * 1024,
        'fdCount': sum(row[1:].isdigit() for row in fd_result.stdout.splitlines() if row.startswith('f')),
        'cpuTotalSeconds': total_cpu,
        'cpuDeltaSeconds': 0 if previous_cpu is None else total_cpu - previous_cpu,
        'lifetime': lifetime,
        'elapsedSeconds': time.monotonic() - started,
    }


def source_status_snapshot(database_path, private_roots):
    if database_path is None or not database_path.exists():
        return {'available': False, 'sources': [], 'readError': None}
    try:
        uri = 'file:' + urllib.parse.quote(database_path.as_posix(), safe='/') + '?mode=ro'
        connection = sqlite3.connect(uri, uri=True, timeout=1)
        try:
            rows = connection.execute(
                'SELECT enabled, provider, index_status, last_error '
                'FROM global_volumes ORDER BY provider, index_status').fetchall()
        finally:
            connection.close()
        return {
            'available': True,
            'sources': [
                {'enabled': bool(enabled), 'provider': safe_text(provider, private_roots),
                 'indexStatus': safe_text(status, private_roots),
                 'lastError': safe_text(error, private_roots) if error else None}
                for enabled, provider, status, error in rows
            ],
            'readError': None,
        }
    except (sqlite3.Error, OSError) as error:
        return {'available': False, 'sources': [],
                'readError': safe_text(f'{type(error).__name__}: {error}', private_roots)}


def error_diagnostics(trace_snapshot, log_text, source_status, private_roots):
    trace_errors = []
    for line in trace_snapshot.get('errors', []):
        event, separator, detail = line.partition('\t')
        if separator and event.endswith('_error'):
            trace_errors.append({'source': event[:-6], 'message': safe_text(detail, private_roots)})
    stderr_lines = [line for line in log_text.splitlines() if ERROR_PATTERN.search(line)]
    stderr_lines = [safe_text(line, private_roots) for line in stderr_lines[-ERROR_LINE_LIMIT:]]
    source_errors = [
        {'provider': item['provider'], 'indexStatus': item['indexStatus'], 'message': item['lastError']}
        for item in source_status.get('sources', []) if item.get('lastError')
    ]
    if source_status.get('readError'):
        stderr_lines.append('native_qa source_status_read_error: ' + source_status['readError'])
    return trace_errors, stderr_lines, source_errors


def coordinator_diagnosis(trace_snapshot, errors, stderr_lines, process_alive):
    counts = trace_snapshot.get('eventCounts', {})
    cycles = counts.get('coordinator_cycle', 0)
    waits = counts.get('coordinator_wait', 0)
    wakes = sum(count for name, count in counts.items() if name.startswith('coordinator_wake_'))
    worker_errors = [item for item in errors if item['source'] == 'coordinator_worker']
    source_errors = [item for item in errors if item['source'] in ('global_index_source', 'spotlight_initial_query')]
    source_errors.extend(item for item in errors if item['source'] == 'database_source_status')
    trace_write_failures = [line for line in stderr_lines if 'coordinator_trace_open_error' in line
                            or 'coordinator_trace_write_error' in line]
    coordinator_panicked = any('zen-canvas-global-index' in line and 'panicked' in line
                               for line in stderr_lines)
    if worker_errors:
        diagnosis = 'coordinator_worker_failed'
    elif coordinator_panicked:
        diagnosis = 'coordinator_worker_panicked'
    elif process_alive is None:
        diagnosis = 'probe_not_started'
    elif not process_alive:
        diagnosis = 'application_process_exited'
    elif trace_write_failures:
        diagnosis = 'qa_trace_unwritable'
    elif trace_snapshot.get('readError'):
        diagnosis = 'qa_trace_read_failed'
    elif not trace_snapshot.get('present') or trace_snapshot.get('eventCount', 0) == 0:
        diagnosis = 'qa_trace_missing'
    elif (counts.get('spotlight_initial_query_started', 0)
          and not counts.get('spotlight_initial_query_completed', 0)):
        diagnosis = 'initial_spotlight_query_incomplete'
    elif wakes >= 2 or cycles >= 4:
        diagnosis = 'repeated_coordinator_wakes'
    elif source_errors:
        diagnosis = 'global_index_source_failed'
    elif cycles == 0:
        diagnosis = 'coordinator_cycle_not_started'
    elif waits == 0:
        diagnosis = 'coordinator_cycle_active_without_wait'
    else:
        diagnosis = 'coordinator_wait_observed'
    return {
        'classification': diagnosis,
        'coordinatorCycles': cycles,
        'coordinatorWaits': waits,
        'coordinatorDeliveredWakes': wakes,
        'coordinatorWorkerErrors': len(worker_errors),
        'coordinatorWorkerPanicked': coordinator_panicked,
        'sourceErrors': len(source_errors),
        'initialSpotlightQueryStarted': bool(counts.get('spotlight_initial_query_started', 0)),
        'initialSpotlightQueryCompleted': bool(counts.get('spotlight_initial_query_completed', 0)),
        'repeatedWakeObserved': wakes >= 2 or cycles >= 4,
        'applicationProcessAlive': process_alive,
    }


def write_evidence_artifacts(output, trace_snapshot, error_messages):
    trace_path = output.with_name(output.stem + '-coordinator-trace.log')
    error_path = output.with_name(output.stem + '-errors.txt')
    trace_header = {
        key: value for key, value in trace_snapshot.items()
        if key != 'events'
    }
    trace_path.write_text(
        '# Coordinator native-QA trace; bounded to the most recent '
        f'{TRACE_EVENT_LIMIT} events. Summary: {json.dumps(trace_header, sort_keys=True)}\n'
        + ''.join(line + '\n' for line in trace_snapshot.get('events', [])))
    error_path.write_text('\n'.join(error_messages) + ('\n' if error_messages else ''))
    return trace_path.name, error_path.name


def main():
    candidate = pathlib.Path(sys.argv[1]).resolve(strict=True)
    output = pathlib.Path(sys.argv[2]).resolve()
    evidence = dict(sourceHead=command('git', 'rev-parse', 'HEAD'),
                    sourceTree=command('git', 'rev-parse', 'HEAD^{tree}'),
                    candidateName=candidate.name,
                    candidateSha256=hashlib.sha256(candidate.read_bytes()).hexdigest(),
                    runner=platform.platform(), architecture=platform.machine(),
                    runtimeDependencies=locked_runtime_versions(),
                    idleGateSeconds=180, preIdleSamplingIntervalSeconds=PRE_IDLE_SAMPLE_INTERVAL_SECONDS,
                    startupCheckpoints=[], lastStartupCheckpoint=None,
                    startupAttribution='startup trace unavailable',
                    classification='UNVERIFIED', preIdleSamples=[], samples=[],
                    cleanup={}, failure=None, coordinatorDiagnosis=None)
    info = candidate.parent.parent / 'Info.plist'
    if info.exists():
        evidence['bundleInfoPlistSha256'] = hashlib.sha256(info.read_bytes()).hexdigest()
    task = None
    process = None
    trace = None
    trace_observer = CoordinatorTrace()
    private_roots = []
    source_status = {'available': False, 'sources': [], 'readError': None}
    stderr_text = ''
    stdout_text = ''
    try:
        if platform.system() != 'Darwin' or platform.machine() != 'arm64':
            raise RuntimeError('requires supported native Apple Silicon macOS runner')
        if evidence['sourceHead'] != os.environ['EXPECTED_SOURCE_SHA']:
            raise RuntimeError('exact source SHA mismatch')
        task = pathlib.Path(tempfile.mkdtemp(prefix='resident-qualification-', dir=os.environ['RUNNER_TEMP'])).resolve()
        profile = task / 'profile'
        trace = task / 'trace.log'
        evidence['profileIsolated'] = True
        env = dict(os.environ, ZC_NATIVE_QA_PROFILE_ROOT=str(profile),
                   ZC_GLOBAL_INDEX_QA_TRACE=str(trace), HOME=str(task / 'home'),
                   TMPDIR=str(task / 'temp'))
        for name in ('profile', 'home', 'temp'):
            (task / name).mkdir()
        private_roots = [task, pathlib.Path(env['HOME']), pathlib.Path(env['TMPDIR']), profile]
        started = time.monotonic()
        with (task / 'stderr.log').open('w') as stderr, (task / 'stdout.log').open('w') as stdout:
            process = subprocess.Popen([str(candidate), '--background'], env=env, stdout=stdout, stderr=stderr)
            deadline = started + 180
            previous = None
            stable_since = None
            previous_pre_idle_cpu = None
            next_pre_idle_sample = started
            idle_gate_elapsed = None
            while time.monotonic() < deadline:
                trace_observer.refresh(trace)
                if process.poll() is not None:
                    raise RuntimeError(f'background application exited: code={process.returncode}')
                current_time = time.monotonic()
                events = trace_observer.events
                counts = (trace_observer.counts.get('coordinator_cycle', 0),
                          trace_observer.counts.get('coordinator_wait', 0))
                if events and events[-1].partition('\t')[0] == 'coordinator_wait' and counts == previous:
                    stable_since = stable_since or current_time
                    if current_time - stable_since >= 5:
                        idle_gate_elapsed = current_time - started
                        break
                else:
                    stable_since = None
                previous = counts
                if current_time >= deadline:
                    raise RuntimeError('coordinator did not settle to blocking wait within 180s')
                if current_time >= next_pre_idle_sample:
                    try:
                        sample = process_observation(process.pid, started, 'PRE_IDLE', previous_pre_idle_cpu)
                        previous_pre_idle_cpu = sample['cpuTotalSeconds']
                    except Exception as error:
                        sample = {'phase': 'PRE_IDLE', 'timestamp': now(), 'pid': process.pid,
                                  'alive': process.poll() is None,
                                  'elapsedSeconds': current_time - started,
                                  'observationError': safe_text(f'{type(error).__name__}: {error}', private_roots)}
                    sample['coordinatorCycleCount'] = trace_observer.counts.get('coordinator_cycle', 0)
                    sample['coordinatorWaitCount'] = trace_observer.counts.get('coordinator_wait', 0)
                    sample['coordinatorWakeCount'] = sum(
                        count for name, count in trace_observer.counts.items()
                        if name.startswith('coordinator_wake_'))
                    evidence['preIdleSamples'].append(sample)
                    current_time = time.monotonic()
                    if current_time >= deadline:
                        raise RuntimeError('coordinator did not settle to blocking wait within 180s')
                    next_pre_idle_sample = current_time + PRE_IDLE_SAMPLE_INTERVAL_SECONDS
                time.sleep(0.5)
            else:
                raise RuntimeError('coordinator did not settle to blocking wait within 180s')

            trace_observer.refresh(trace)
            startup_log = (task / 'stderr.log').read_text(errors='replace')
            checkpoints, attribution = startup_trace(startup_log)
            evidence['startupCheckpoints'] = checkpoints
            evidence['lastStartupCheckpoint'] = checkpoints[-1] if checkpoints else None
            evidence['startupAttribution'] = attribution
            evidence['idleGateElapsedSeconds'] = idle_gate_elapsed
            if 'ui_runtime startup_mode=background webview_count=0 labels=' not in startup_log:
                raise RuntimeError('missing background / zero Main Search WebView diagnostic')
            before_counts = dict(trace_observer.counts)
            previous_idle_cpu = None
            for index in range(31):
                if process.poll() is not None:
                    raise RuntimeError(f'resident exited: code={process.returncode}')
                sample = process_observation(process.pid, started, 'IDLE_QUALIFIED', previous_idle_cpu)
                previous_idle_cpu = sample['cpuTotalSeconds']
                children = subprocess.run(['pgrep', '-P', str(process.pid)], capture_output=True,
                                          text=True, timeout=8)
                if children.returncode not in (0, 1):
                    raise RuntimeError(f'child process inspection failed: {children.stderr.strip()}')
                child_rows = []
                for child in children.stdout.split():
                    child_rows.append(command('ps', '-p', child, '-o', 'comm=', timeout=5))
                webviews = [row for row in child_rows if 'WebKit' in row]
                sample['webviewChildren'] = webviews
                sample['elapsedSeconds'] = time.monotonic() - started
                evidence['samples'].append(sample)
                if webviews:
                    raise RuntimeError(f'unexpected WebKit children: {webviews}')
                if index < 30:
                    time.sleep(1)
            trace_observer.refresh(trace)
            evidence['coordinatorCycleDelta'] = trace_observer.counts.get('coordinator_cycle', 0) - before_counts.get('coordinator_cycle', 0)
            evidence['coordinatorWaitDelta'] = trace_observer.counts.get('coordinator_wait', 0) - before_counts.get('coordinator_wait', 0)
            log = (task / 'stderr.log').read_text(errors='replace')
            if 'main_window_created' in log or 'search_window_ready' in log:
                raise RuntimeError('unexpected window creation')
            for field in ('rssBytes', 'fdCount'):
                idle_values = [sample[field] for sample in evidence['samples']]
                if all(b > a for a, b in zip(idle_values, idle_values[1:])):
                    raise RuntimeError(f'monotonic resource growth: {field}')
            if evidence['coordinatorCycleDelta'] or evidence['coordinatorWaitDelta']:
                raise RuntimeError('coordinator work continued during settled sampling')
            evidence['classification'] = 'HARD PASS / OBSERVATIONAL MEMORY'
    except Exception as error:
        evidence['failure'] = safe_text(f'{type(error).__name__}: {error}', private_roots)
        evidence['classification'] = 'BLOCKED'
    finally:
        output.parent.mkdir(parents=True, exist_ok=True)
        if task is not None:
            trace_observer.refresh(trace)
            source_status = source_status_snapshot(task / 'profile' / 'zen-canvas.sqlite3', private_roots)
        process_alive_before_cleanup = None if process is None else process.poll() is None
        evidence['processAliveBeforeCleanup'] = process_alive_before_cleanup
        if process is not None:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
            evidence['cleanup']['processStopped'] = process.poll() is not None
        if task is not None:
            stderr_path = task / 'stderr.log'
            stdout_path = task / 'stdout.log'
            stderr_text = stderr_path.read_text(errors='replace') if stderr_path.exists() else ''
            stdout_text = stdout_path.read_text(errors='replace') if stdout_path.exists() else ''
            trace_observer.refresh(trace)
            checkpoints, attribution = startup_trace(stderr_text + '\n' + stdout_text)
            evidence['startupCheckpoints'] = checkpoints
            evidence['lastStartupCheckpoint'] = checkpoints[-1] if checkpoints else None
            evidence['startupAttribution'] = attribution
            trace_snapshot = trace_observer.snapshot()
            trace_errors, stderr_lines, source_errors = error_diagnostics(
                trace_snapshot, stderr_text + '\n' + stdout_text, source_status, private_roots)
            evidence['coordinatorTrace'] = {
                key: safe_text(value, private_roots) if isinstance(value, str) else value
                for key, value in trace_snapshot.items() if key not in ('events', 'errors')
            }
            evidence['coordinatorTrace']['events'] = [safe_text(line, private_roots)
                                                       for line in trace_snapshot['events']]
            evidence['coordinatorTrace']['errors'] = [safe_text(line, private_roots)
                                                       for line in trace_snapshot['errors']]
            evidence['coordinatorErrors'] = trace_errors
            evidence['sourceStatus'] = source_status
            evidence['sourceErrors'] = source_errors
            evidence['stderrDiagnostics'] = stderr_lines
            evidence['stderr'] = '\n'.join(stderr_lines)
            evidence['stdoutDiagnostics'] = [safe_text(line, private_roots) for line in stdout_text.splitlines()
                                              if ERROR_PATTERN.search(line)][-ERROR_LINE_LIMIT:]
            evidence['coordinatorDiagnosis'] = coordinator_diagnosis(
                trace_snapshot,
                trace_errors + [{'source': 'database_source_status', 'message': item['message']}
                                for item in source_errors],
                stderr_lines, process_alive_before_cleanup)
            if evidence.get('classification', '').startswith('HARD PASS') and source_errors:
                evidence['classification'] = 'BLOCKED'
                evidence['failure'] = 'global index source error observed during resident qualification'
            error_messages = [f"{item['source']}: {item['message']}" for item in trace_errors]
            error_messages.extend(stderr_lines)
            error_messages.extend(f"{item['provider']} ({item['indexStatus']}): {item['message']}"
                                  for item in source_errors)
            try:
                trace_name, error_name = write_evidence_artifacts(output, evidence['coordinatorTrace'], error_messages)
                evidence['coordinatorTraceArtifact'] = trace_name
                evidence['errorArtifact'] = error_name
            except OSError as error:
                evidence['artifactWriteError'] = safe_text(f'{type(error).__name__}: {error}', private_roots)
                evidence['classification'] = 'BLOCKED'
            try:
                if task.parent != pathlib.Path(os.environ['RUNNER_TEMP']).resolve() or not task.name.startswith('resident-qualification-'):
                    raise RuntimeError('task cleanup boundary mismatch')
                shutil.rmtree(task)
                evidence['cleanup']['taskRemoved'] = not task.exists()
            except Exception as error:
                evidence['cleanup']['failure'] = safe_text(f'{type(error).__name__}: {error}', private_roots)
                evidence['classification'] = 'BLOCKED'
        else:
            trace_snapshot = trace_observer.snapshot()
            evidence['coordinatorTrace'] = trace_snapshot
            evidence['coordinatorDiagnosis'] = coordinator_diagnosis(trace_snapshot, [], [], None)
            try:
                trace_name, error_name = write_evidence_artifacts(output, trace_snapshot, [])
                evidence['coordinatorTraceArtifact'] = trace_name
                evidence['errorArtifact'] = error_name
            except OSError as error:
                evidence['artifactWriteError'] = safe_text(f'{type(error).__name__}: {error}', private_roots)
                evidence['classification'] = 'BLOCKED'
        output.write_text(json.dumps(evidence, indent=2) + '\n')
        output.with_suffix('.md').write_text(
            f"{evidence['classification']}\n\n{evidence.get('failure') or ''}\n\n"
            f"Coordinator diagnosis: {evidence.get('coordinatorDiagnosis')}\n"
            f"PRE_IDLE samples: {len(evidence.get('preIdleSamples', []))}; "
            f"IDLE_QUALIFIED samples: {len(evidence.get('samples', []))}.\n")
    return 0 if evidence['classification'].startswith('HARD PASS') else 1


if __name__ == '__main__':
    sys.exit(main())
