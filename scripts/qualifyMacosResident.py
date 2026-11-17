"""Hosted Apple Silicon process observations; no GUI/release acceptance claim."""
import datetime
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time

STARTUP_CHECKPOINT_PATTERN = re.compile(r'^native_qa startup_checkpoint=([a-z_]+)$', re.MULTILINE)
RUNTIME_PACKAGES = ('tauri', 'tao', 'wry', 'objc2', 'objc2-exception-helper')


def command(*args):
    return subprocess.check_output(args, text=True, timeout=15).strip()


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def cpu_seconds(value):
    parts = value.replace('-', ':').split(':')
    if len(parts) == 4:
        return int(parts[0]) * 86400 + int(parts[1]) * 3600 + int(parts[2]) * 60 + float(parts[3])
    return sum(float(part) * 60 ** index for index, part in enumerate(reversed(parts)))


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


def main():
    candidate = pathlib.Path(sys.argv[1]).resolve(strict=True)
    output = pathlib.Path(sys.argv[2]).resolve()
    evidence = dict(sourceHead=command('git', 'rev-parse', 'HEAD'),
                    sourceTree=command('git', 'rev-parse', 'HEAD^{tree}'),
                    candidate=str(candidate), candidateSha256=hashlib.sha256(candidate.read_bytes()).hexdigest(),
                    runner=platform.platform(), architecture=platform.machine(),
                    runtimeDependencies=locked_runtime_versions(),
                    startupCheckpoints=[], lastStartupCheckpoint=None,
                    startupAttribution='startup trace unavailable',
                    classification='UNVERIFIED', samples=[], cleanup={}, failure=None)
    info = candidate.parent.parent / 'Info.plist'
    if info.exists():
        evidence['bundleInfoPlistSha256'] = hashlib.sha256(info.read_bytes()).hexdigest()
        evidence['applicationBundle'] = str(candidate.parent.parent.parent)
    task = None
    process = None
    try:
        if platform.system() != 'Darwin' or platform.machine() != 'arm64':
            raise RuntimeError('requires supported native Apple Silicon macOS runner')
        if evidence['sourceHead'] != os.environ['EXPECTED_SOURCE_SHA']:
            raise RuntimeError('exact source SHA mismatch')
        task = pathlib.Path(tempfile.mkdtemp(prefix='resident-qualification-', dir=os.environ['RUNNER_TEMP'])).resolve()
        evidence['profile'] = str(task / 'profile')
        trace = task / 'trace.log'
        env = dict(os.environ, ZC_NATIVE_QA_PROFILE_ROOT=evidence['profile'],
                   ZC_GLOBAL_INDEX_QA_TRACE=str(trace), HOME=str(task / 'home'),
                   TMPDIR=str(task / 'temp'))
        for name in ('profile', 'home', 'temp'):
            (task / name).mkdir()
        started = time.monotonic()
        with (task / 'stderr.log').open('w') as stderr, (task / 'stdout.log').open('w') as stdout:
            process = subprocess.Popen([str(candidate), '--background'], env=env, stdout=stdout, stderr=stderr)
            deadline = started + 180
            previous = None
            stable_since = None
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(f'background application exited: code={process.returncode}')
                lines = trace.read_text().splitlines() if trace.exists() else []
                coordinator_events = [line.rsplit(' ', 1)[-1] for line in lines]
                events = [event for event in coordinator_events if event in ('coordinator_cycle', 'coordinator_wait')]
                counts = (coordinator_events.count('coordinator_cycle'), coordinator_events.count('coordinator_wait'))
                if events and events[-1] == 'coordinator_wait' and counts == previous:
                    stable_since = stable_since or time.monotonic()
                    if time.monotonic() - stable_since >= 5:
                        break
                else:
                    stable_since = None
                previous = counts
                time.sleep(0.5)
            else:
                raise RuntimeError('coordinator did not settle to blocking wait within 180s')
            log = (task / 'stderr.log').read_text()
            evidence['startupLog'] = log
            checkpoints, attribution = startup_trace(log)
            evidence['startupCheckpoints'] = checkpoints
            evidence['lastStartupCheckpoint'] = checkpoints[-1] if checkpoints else None
            evidence['startupAttribution'] = attribution
            if 'ui_runtime startup_mode=background webview_count=0 labels=' not in log:
                raise RuntimeError('missing background / zero Main Search WebView diagnostic')
            before = trace.read_text().splitlines()
            for index in range(31):
                if process.poll() is not None:
                    raise RuntimeError(f'resident exited: code={process.returncode}')
                observation = command('ps', '-p', str(process.pid), '-o', 'rss=', '-o', 'time=', '-o', 'etime=')
                rss, cpu, lifetime = observation.split()
                fds = command('lsof', '-a', '-p', str(process.pid), '-Ff')
                children = subprocess.run(['pgrep', '-P', str(process.pid)], capture_output=True, text=True, timeout=15)
                if children.returncode not in (0, 1):
                    raise RuntimeError(f'child process inspection failed: {children.stderr}')
                child_rows = [command('ps', '-p', child, '-o', 'comm=') for child in children.stdout.split()]
                webviews = [row for row in child_rows if 'WebKit' in row]
                if webviews:
                    raise RuntimeError(f'unexpected WebKit children: {webviews}')
                total = cpu_seconds(cpu)
                evidence['samples'].append(dict(timestamp=now(), pid=process.pid,
                    rssBytes=int(rss) * 1024, fdCount=sum(row[1:].isdigit() for row in fds.splitlines() if row.startswith('f')),
                    cpuTotalSeconds=total, cpuDeltaSeconds=0 if index == 0 else total - evidence['samples'][-1]['cpuTotalSeconds'],
                    lifetime=lifetime, elapsedSeconds=time.monotonic() - started, webviewChildren=webviews))
                if index < 30:
                    time.sleep(1)
            after = trace.read_text().splitlines()
            before_events = [line.rsplit(' ', 1)[-1] for line in before]
            after_events = [line.rsplit(' ', 1)[-1] for line in after]
            evidence['coordinatorCycleDelta'] = after_events.count('coordinator_cycle') - before_events.count('coordinator_cycle')
            evidence['coordinatorWaitDelta'] = after_events.count('coordinator_wait') - before_events.count('coordinator_wait')
            evidence['trace'] = after
            if evidence['coordinatorCycleDelta'] or evidence['coordinatorWaitDelta']:
                raise RuntimeError('coordinator work continued during settled sampling')
            log = (task / 'stderr.log').read_text()
            if 'main_window_created' in log or 'search_window_ready' in log:
                raise RuntimeError('unexpected window creation')
            for field in ('rssBytes', 'fdCount'):
                if all(b[field] > a[field] for a, b in zip(evidence['samples'], evidence['samples'][1:])):
                    raise RuntimeError(f'monotonic resource growth: {field}')
            evidence['classification'] = 'HARD PASS / OBSERVATIONAL MEMORY'
    except Exception as error:
        evidence['failure'] = repr(error)
        evidence['classification'] = 'BLOCKED'
    finally:
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
            evidence['stderr'] = (task / 'stderr.log').read_text() if (task / 'stderr.log').exists() else ''
            checkpoints, attribution = startup_trace(evidence['stderr'])
            evidence['startupCheckpoints'] = checkpoints
            evidence['lastStartupCheckpoint'] = checkpoints[-1] if checkpoints else None
            evidence['startupAttribution'] = attribution
            try:
                if task.parent != pathlib.Path(os.environ['RUNNER_TEMP']).resolve() or not task.name.startswith('resident-qualification-'):
                    raise RuntimeError('task cleanup boundary mismatch')
                shutil.rmtree(task)
                evidence['cleanup']['taskRemoved'] = not task.exists()
            except Exception as error:
                evidence['cleanup']['failure'] = repr(error)
                evidence['classification'] = 'BLOCKED'
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(evidence, indent=2) + '\n')
        output.with_suffix('.md').write_text(f"{evidence['classification']}\n\n{evidence['failure']}\n\nSee JSON for all raw samples and cleanup.\n")
    return 0 if evidence['classification'].startswith('HARD PASS') else 1


if __name__ == '__main__':
    sys.exit(main())
