"""Optional bounded LLDB backtrace for the isolated native-QA macOS candidate."""
import datetime
import hashlib
import json
import os
import pathlib
import platform
import shutil
import signal
import subprocess
import sys
import tempfile


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def unavailable(reason, output, identity=None):
    evidence = {
        'classification': 'DIAGNOSTIC UNAVAILABLE',
        'reason': reason,
        'runner': platform.platform(),
        'architecture': platform.machine(),
        'startedAt': now(),
        'finishedAt': now(),
        'cleanup': {'debuggerStopped': True, 'taskRemoved': True},
    }
    if identity:
        evidence.update(identity)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(evidence, indent=2) + '\n')
    output.with_suffix('.md').write_text(f"{evidence['classification']}\n\n{reason}\n")
    return 0


def main():
    candidate = pathlib.Path(sys.argv[1]).resolve()
    output = pathlib.Path(sys.argv[2]).resolve()
    source_head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True, timeout=15).strip()
    source_tree = subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], text=True, timeout=15).strip()
    identity = {'sourceHead': source_head, 'sourceTree': source_tree, 'candidate': str(candidate)}
    if source_head != os.environ.get('EXPECTED_SOURCE_SHA'):
        return unavailable('exact source SHA mismatch', output, identity)
    lldb = shutil.which('lldb')
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':
        return unavailable('requires a hosted Apple Silicon macOS runner', output, identity)
    if not lldb:
        return unavailable('LLDB is not installed on this runner', output, identity)
    if not candidate.is_file():
        return unavailable('native-QA candidate executable is missing', output, identity)

    runner_temp = pathlib.Path(os.environ.get('RUNNER_TEMP', tempfile.gettempdir())).resolve()
    task = pathlib.Path(tempfile.mkdtemp(prefix='resident-startup-lldb-', dir=runner_temp)).resolve()
    for name in ('profile', 'home', 'temp'):
        (task / name).mkdir()
    for name in ('.config', '.local/share', '.cache'):
        (task / 'home' / name).mkdir(parents=True)
    env = dict(os.environ,
               ZC_NATIVE_QA_PROFILE_ROOT=str(task / 'profile'),
               ZC_GLOBAL_INDEX_QA_TRACE=str(task / 'global-index-trace.log'),
               HOME=str(task / 'home'),
               CFFIXED_USER_HOME=str(task / 'home'),
               TMPDIR=str(task / 'temp'),
               XDG_CONFIG_HOME=str(task / 'home' / '.config'),
               XDG_DATA_HOME=str(task / 'home' / '.local' / 'share'),
               XDG_CACHE_HOME=str(task / 'home' / '.cache'))
    evidence = {
        **identity,
        'classification': 'DIAGNOSTIC UNAVAILABLE',
        'runner': platform.platform(),
        'architecture': platform.machine(),
        'candidateSha256': hashlib.sha256(candidate.read_bytes()).hexdigest(),
        'lldbPath': lldb,
        'startedAt': now(),
        'timeoutSeconds': 45,
        'breakpoints': ['objc_exception_throw', '__rust_foreign_exception'],
        'cleanup': {},
    }
    debugger = None
    raw = ''
    try:
        command = [
            lldb, '--batch', '--no-lldbinit',
            '-o', f'target create "{candidate}"',
            '-o', 'breakpoint set --name objc_exception_throw',
            '-o', 'breakpoint set --name __rust_foreign_exception',
            '-o', 'process handle SIGABRT --notify true --stop true --pass false',
            '-o', 'settings set target.run-args -- --background',
            '-o', 'run',
            '-o', 'thread backtrace all',
            '-o', 'process status',
            '-o', 'process kill',
            '-o', 'quit',
        ]
        debugger = subprocess.Popen(command, env=env, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, text=True,
                                    start_new_session=True)
        try:
            raw, _ = debugger.communicate(timeout=evidence['timeoutSeconds'])
        except subprocess.TimeoutExpired:
            evidence['timedOut'] = True
            os.killpg(debugger.pid, signal.SIGTERM)
            try:
                raw, _ = debugger.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(debugger.pid, signal.SIGKILL)
                raw, _ = debugger.communicate(timeout=5)
        evidence['lldbExitCode'] = debugger.returncode
        evidence['rawOutput'] = raw
        evidence['startupCheckpoints'] = [
            line.split('=', 1)[1]
            for line in raw.splitlines()
            if line.startswith('native_qa startup_checkpoint=')
        ]
        captured_stop_reason = any(
            reason in raw for reason in (
                'stop reason = breakpoint',
                'stop reason = signal SIGABRT',
                'stop reason = hit Objective-C exception',
            )
        )
        backtrace_observed = 'frame #0:' in raw and captured_stop_reason
        evidence['backtraceCaptured'] = backtrace_observed
        if backtrace_observed:
            evidence['classification'] = 'DIAGNOSTIC AVAILABLE'
        else:
            evidence['reason'] = 'LLDB completed without capturing a breakpoint or SIGABRT backtrace'
    except Exception as error:
        evidence['reason'] = repr(error)
        evidence['rawOutput'] = raw
    finally:
        if debugger is not None and debugger.poll() is None:
            try:
                os.killpg(debugger.pid, signal.SIGTERM)
                debugger.wait(timeout=5)
            except Exception:
                try:
                    os.killpg(debugger.pid, signal.SIGKILL)
                    debugger.wait(timeout=5)
                except Exception as error:
                    evidence['cleanup']['debuggerFailure'] = repr(error)
        evidence['cleanup']['debuggerStopped'] = debugger is None or debugger.poll() is not None
        try:
            if task.parent != runner_temp or not task.name.startswith('resident-startup-lldb-'):
                raise RuntimeError('task cleanup boundary mismatch')
            shutil.rmtree(task)
            evidence['cleanup']['taskRemoved'] = not task.exists()
        except Exception as error:
            evidence['cleanup']['failure'] = repr(error)
            evidence['cleanup']['taskRemoved'] = False
            evidence['classification'] = 'DIAGNOSTIC UNAVAILABLE'
            evidence['reason'] = 'bounded LLDB cleanup did not complete'
        evidence['finishedAt'] = now()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(evidence, indent=2) + '\n')
        output.with_suffix('.md').write_text(
            f"{evidence['classification']}\n\n{evidence.get('reason', '')}\n\n"
            f"Backtrace captured: {evidence.get('backtraceCaptured', False)}\n")
    return 0


if __name__ == '__main__':
    sys.exit(main())
