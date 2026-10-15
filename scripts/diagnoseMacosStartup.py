"""Capture one bounded Objective-C exception during native-QA macOS startup."""
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


def unavailable(reason, output, identity=None, raw_output=''):
    evidence = {
        'classification': 'DIAGNOSTIC_UNAVAILABLE',
        'reason': reason,
        'runner': platform.platform(),
        'architecture': platform.machine(),
        'startedAt': now(),
        'finishedAt': now(),
        'cleanup': {'debuggerStopped': True, 'taskRemoved': True},
    }
    if identity:
        evidence.update(identity)
    if raw_output:
        evidence['rawOutput'] = raw_output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(evidence, indent=2) + '\n')
    output.with_suffix('.md').write_text(
        f"{evidence['classification']}\n\n{reason}\n")
    return 0


LLDB_CALLBACK = r'''import lldb


def _command(label, command, include_value=False):
    print("ZC_LLDB_COMMAND_BEGIN=" + label)
    result = lldb.SBCommandReturnObject()
    try:
        lldb.debugger.GetCommandInterpreter().HandleCommand(command, result)
        output = result.GetOutput() or ""
        error = result.GetError() or ""
        if include_value:
            marker = "ZC_OBJC_" + label.upper()
            print(marker + "_BEGIN")
            if output:
                print(output, end="" if output.endswith("\n") else "\n")
            if error:
                print("LLDB_ERROR: " + error,
                      end="" if error.endswith("\n") else "\n")
            print(marker + "_END")
        else:
            if output:
                print(output, end="" if output.endswith("\n") else "\n")
            if error:
                print("LLDB_ERROR: " + error,
                      end="" if error.endswith("\n") else "\n")
        print("ZC_LLDB_COMMAND_SUCCEEDED=" + label + ":" +
              ("1" if result.Succeeded() else "0"))
        return result.Succeeded() and not error
    except Exception as error:
        print("ZC_LLDB_COMMAND_EXCEPTION=" + label + ":" + repr(error))
        return False


def capture_objc_exception(frame, bp_loc, internal_dict):
    function_name = frame.GetFunctionName() or ""
    symbol = frame.GetSymbol()
    symbol_name = symbol.GetName() if symbol.IsValid() else ""
    entry_frame = frame.GetFrameID() == 0 and (
        "objc_exception_throw" in function_name or
        "objc_exception_throw" in symbol_name)
    if not entry_frame:
        print("ZC_OBJC_ENTRY_NOT_CONFIRMED frame={} function={!r} symbol={!r}".format(
            frame.GetFrameID(), function_name, symbol_name))
        return False

    thread = frame.GetThread()
    capture_index = internal_dict.get("zc_objc_capture_count", 0) + 1
    internal_dict["zc_objc_capture_count"] = capture_index
    print("ZC_OBJC_ENTRY_CONFIRMED index={}".format(capture_index))
    print("ZC_OBJC_THREAD_ID={}".format(thread.GetThreadID()))
    print("ZC_OBJC_FRAME_ID={} FUNCTION={!r} SYMBOL={!r} PC=0x{:x}".format(
        frame.GetFrameID(), function_name, symbol_name, frame.GetPC()))

    commands_ok = []
    commands_ok.append(_command(
        "select_objc_thread", "thread select {}".format(thread.GetIndexID())))
    commands_ok.append(_command("select_objc_entry_frame", "frame select 0"))
    commands_ok.append(_command("thread_info", "thread info"))
    commands_ok.append(_command("frame_info", "frame info"))
    commands_ok.append(_command("register_x0", "register read x0"))
    commands_ok.append(_command("exception_object", "po $arg1"))
    name_ok = _command("exception_name", "po [$arg1 name]", include_value=True)
    reason_ok = _command("exception_reason", "po [$arg1 reason]", include_value=True)
    commands_ok.append(_command("objc_backtrace", "thread backtrace all"))
    if name_ok and reason_ok and all(commands_ok):
        print("ZC_OBJC_EVALUATION_SUCCEEDED index={}".format(capture_index))
    else:
        print("ZC_OBJC_EVALUATION_FAILED index={}".format(capture_index))
    return False


def capture_rust_foreign_exception(frame, bp_loc, internal_dict):
    thread = frame.GetThread()
    function_name = frame.GetFunctionName() or ""
    symbol = frame.GetSymbol()
    symbol_name = symbol.GetName() if symbol.IsValid() else ""
    print("ZC_FOREIGN_EXCEPTION_THREAD_ID={}".format(thread.GetThreadID()))
    print("ZC_FOREIGN_EXCEPTION_FRAME={!r} {!r}".format(
        function_name, symbol_name))
    _command("foreign_thread_info", "thread info")
    _command("foreign_backtrace", "thread backtrace all")
    return False
'''


def _captured_values(raw, marker):
    begin = marker + '_BEGIN\n'
    end = marker + '_END'
    values = []
    remainder = raw
    while begin in remainder and end in remainder:
        remainder = remainder.split(begin, 1)[1]
        value, remainder = remainder.split(end, 1)
        values.append(value.strip())
    return values


def main():
    candidate = pathlib.Path(sys.argv[1]).resolve()
    output = pathlib.Path(sys.argv[2]).resolve()
    source_head = subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], text=True, timeout=15).strip()
    source_tree = subprocess.check_output(
        ['git', 'rev-parse', 'HEAD^{tree}'], text=True, timeout=15).strip()
    identity = {'sourceHead': source_head, 'sourceTree': source_tree,
                'candidate': str(candidate)}
    if source_head != os.environ.get('EXPECTED_SOURCE_SHA'):
        return unavailable('exact source SHA mismatch', output, identity)
    lldb = shutil.which('lldb')
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':
        return unavailable('requires a hosted Apple Silicon macOS runner',
                           output, identity)
    if not lldb:
        return unavailable('LLDB is not installed on this runner', output,
                           identity)
    if not candidate.is_file():
        return unavailable('native-QA candidate executable is missing', output,
                           identity)

    runner_temp = pathlib.Path(os.environ.get(
        'RUNNER_TEMP', tempfile.gettempdir())).resolve()
    task = pathlib.Path(tempfile.mkdtemp(
        prefix='resident-startup-lldb-', dir=runner_temp)).resolve()
    for name in ('profile', 'home', 'temp'):
        (task / name).mkdir()
    callback = task / 'zc_lldb_callback.py'
    callback.write_text(LLDB_CALLBACK)
    env = dict(os.environ,
               ZC_NATIVE_QA_PROFILE_ROOT=str(task / 'profile'),
               ZC_GLOBAL_INDEX_QA_TRACE=str(task / 'global-index-trace.log'),
               HOME=str(task / 'home'),
               TMPDIR=str(task / 'temp'))
    evidence = {
        **identity,
        'classification': 'DIAGNOSTIC_UNAVAILABLE',
        'runner': platform.platform(),
        'architecture': platform.machine(),
        'candidateSha256': hashlib.sha256(candidate.read_bytes()).hexdigest(),
        'lldbPath': lldb,
        'startedAt': now(),
        'timeoutSeconds': 90,
        'breakpoints': ['objc_exception_throw', '__rust_foreign_exception'],
        'cleanup': {},
    }
    debugger = None
    raw = ''
    try:
        callback_path = callback.as_posix().replace('"', '\\"')
        command = [
            lldb, '--batch', '--no-lldbinit',
            '-o', f'target create "{candidate}"',
            '-o', f'command script import "{callback_path}"',
            '-o', 'breakpoint set --name objc_exception_throw',
            '-o', 'breakpoint command add -F zc_lldb_callback.capture_objc_exception 1',
            '-o', 'breakpoint set --name __rust_foreign_exception',
            '-o', 'breakpoint command add -F zc_lldb_callback.capture_rust_foreign_exception 2',
            '-o', 'process handle SIGABRT --notify true --stop true --pass false',
            '-o', 'settings set target.run-args -- --background',
            '-o', 'run',
            '-o', 'script print("ZC_FINAL_THREAD_ID=" + str(lldb.debugger.GetSelectedTarget().GetProcess().GetSelectedThread().GetThreadID()))',
            '-o', 'thread info',
            '-o', 'frame info',
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
        evidence['objcExceptionEntryCount'] = raw.count(
            'ZC_OBJC_ENTRY_CONFIRMED index=')
        evidence['objcThreadIds'] = [
            line.split('=', 1)[1].strip()
            for line in raw.splitlines()
            if line.startswith('ZC_OBJC_THREAD_ID=')
        ]
        evidence['objcExceptionNames'] = _captured_values(
            raw, 'ZC_OBJC_EXCEPTION_NAME')
        evidence['objcExceptionReasons'] = _captured_values(
            raw, 'ZC_OBJC_EXCEPTION_REASON')
        evidence['objcEvaluationSucceeded'] = (
            'ZC_OBJC_EVALUATION_SUCCEEDED' in raw and
            'ZC_OBJC_EVALUATION_FAILED' not in raw)
        foreign_threads = [
            line.split('=', 1)[1].strip()
            for line in raw.splitlines()
            if line.startswith('ZC_FOREIGN_EXCEPTION_THREAD_ID=')
        ]
        evidence['foreignExceptionThreadIds'] = foreign_threads
        final_threads = [
            line.split('=', 1)[1].strip()
            for line in raw.splitlines()
            if line.startswith('ZC_FINAL_THREAD_ID=')
        ]
        evidence['fatalThreadId'] = final_threads[-1] if final_threads else None
        evidence['sigabrtObserved'] = 'stop reason = signal SIGABRT' in raw
        evidence['foreignExceptionObserved'] = bool(foreign_threads)
        fatal_thread = evidence['fatalThreadId']
        evidence['sameThreadAsFatalStop'] = bool(
            fatal_thread and fatal_thread in evidence['objcThreadIds'] and
            evidence['sigabrtObserved'])
        evidence['backtraceCaptured'] = (
            evidence['objcExceptionEntryCount'] > 0 and
            'frame #0:' in raw)
        if evidence['objcExceptionEntryCount'] == 0:
            evidence['reason'] = (
                'LLDB did not confirm the objc_exception_throw entry frame')
        elif not evidence['objcEvaluationSucceeded']:
            evidence['reason'] = (
                'Objective-C exception expressions failed; see rawOutput')
        elif not evidence['sigabrtObserved']:
            evidence['reason'] = (
                'exception captured but a subsequent SIGABRT was not observed')
        elif not evidence['sameThreadAsFatalStop']:
            evidence['reason'] = (
                'captured Objective-C exception was not correlated to the '
                'SIGABRT thread')
        else:
            evidence['classification'] = 'DIAGNOSTIC AVAILABLE'
            evidence['reason'] = (
                'Objective-C name/reason captured at the confirmed throw '
                'entry and correlated to the SIGABRT thread')
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
        evidence['cleanup']['debuggerStopped'] = (
            debugger is None or debugger.poll() is not None)
        try:
            if task.parent != runner_temp or not task.name.startswith(
                    'resident-startup-lldb-'):
                raise RuntimeError('task cleanup boundary mismatch')
            shutil.rmtree(task)
            evidence['cleanup']['taskRemoved'] = not task.exists()
        except Exception as error:
            evidence['cleanup']['failure'] = repr(error)
            evidence['cleanup']['taskRemoved'] = False
            evidence['classification'] = 'DIAGNOSTIC_UNAVAILABLE'
            evidence['reason'] = 'bounded LLDB cleanup did not complete'
        evidence['finishedAt'] = now()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(evidence, indent=2) + '\n')
        output.with_suffix('.md').write_text(
            f"{evidence['classification']}\n\n"
            f"{evidence.get('reason', '')}\n\n"
            f"Objective-C entry confirmed: "
            f"{evidence.get('objcExceptionEntryCount', 0) > 0}\n"
            f"Same-thread SIGABRT: "
            f"{evidence.get('sameThreadAsFatalStop', False)}\n")
    return 0


if __name__ == '__main__':
    sys.exit(main())
