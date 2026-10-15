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


def _thread_summary(thread, prefix):
    print("{}INDEX={} ID={} NAME={!r} QUEUE={!r}".format(
        prefix, thread.GetIndexID(), thread.GetThreadID(),
        thread.GetName() or "", thread.GetQueueName() or ""))


def _backtrace(thread, prefix):
    for index in range(thread.GetNumFrames()):
        frame = thread.GetFrameAtIndex(index)
        if not frame.IsValid():
            continue
        function_name = frame.GetFunctionName() or ""
        symbol = frame.GetSymbol()
        symbol_name = symbol.GetName() if symbol.IsValid() else ""
        print("{}FRAME={} PC=0x{:x} FUNCTION={!r} SYMBOL={!r}".format(
            prefix, index, frame.GetPC(), function_name, symbol_name))


def _objc_expression(frame, label, expression):
    marker = "ZC_OBJC_" + label.upper()
    print(marker + "_BEGIN")
    try:
        options = lldb.SBExpressionOptions()
        options.SetLanguage(lldb.eLanguageTypeObjC)
        options.SetTimeoutInMicroSeconds(5000000)
        options.SetIgnoreBreakpoints(True)
        options.SetUnwindOnError(True)
        value = frame.EvaluateExpression(expression, options)
        error = value.GetError()
        if error.Fail():
            print("LLDB_EXPRESSION_ERROR: " + (error.GetCString() or "unknown error"))
            print(marker + "_END")
            return False
        description = value.GetObjectDescription()
        if not description:
            description = value.GetSummary() or value.GetValue()
        if description:
            print(description)
            print(marker + "_END")
            return True
        print("LLDB_EXPRESSION_ERROR: expression returned no printable value")
    except Exception as error:
        print("LLDB_EXPRESSION_ERROR: " + repr(error))
    print(marker + "_END")
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
    print("ZC_OBJC_CAPTURE_START index={}".format(capture_index))
    print("ZC_OBJC_THREAD_ID={}".format(thread.GetThreadID()))
    print("ZC_OBJC_THREAD_NAME={}".format(thread.GetName() or ""))
    _thread_summary(thread, "ZC_OBJC_THREAD_")
    print("ZC_OBJC_FRAME_ID={} FUNCTION={!r} SYMBOL={!r} PC=0x{:x}".format(
        frame.GetFrameID(), function_name, symbol_name, frame.GetPC()))

    x0 = frame.FindRegister("x0")
    if not x0.IsValid():
        print("ZC_OBJC_X0_ERROR: x0 register is unavailable")
        print("ZC_OBJC_EVALUATION_FAILED index={}".format(capture_index))
        _backtrace(thread, "ZC_OBJC_BACKTRACE_")
        print("ZC_OBJC_CAPTURE_END index={}".format(capture_index))
        return False
    exception_address = x0.GetValueAsUnsigned(0)
    print("ZC_OBJC_X0=0x{:x}".format(exception_address))
    object_expression = "(id)0x{:x}".format(exception_address)
    object_ok = _objc_expression(
        frame, "exception_object", object_expression)
    name_ok = _objc_expression(
        frame, "exception_name", "[{} name]".format(object_expression))
    reason_ok = _objc_expression(
        frame, "exception_reason", "[{} reason]".format(object_expression))
    _backtrace(thread, "ZC_OBJC_BACKTRACE_")
    if object_ok and name_ok and reason_ok:
        print("ZC_OBJC_EVALUATION_SUCCEEDED index={}".format(capture_index))
    else:
        print("ZC_OBJC_EVALUATION_FAILED index={}".format(capture_index))
    print("ZC_OBJC_CAPTURE_END index={}".format(capture_index))
    return False


def capture_rust_foreign_exception(frame, bp_loc, internal_dict):
    thread = frame.GetThread()
    function_name = frame.GetFunctionName() or ""
    symbol = frame.GetSymbol()
    symbol_name = symbol.GetName() if symbol.IsValid() else ""
    print("ZC_FOREIGN_EXCEPTION_FRAME={!r} {!r}".format(
        function_name, symbol_name))
    print("ZC_FOREIGN_EXCEPTION_THREAD_ID={}".format(thread.GetThreadID()))
    _thread_summary(thread, "ZC_FOREIGN_EXCEPTION_THREAD_")
    _backtrace(thread, "ZC_FOREIGN_EXCEPTION_BACKTRACE_")
    return False


def capture_pthread_kill(frame, bp_loc, internal_dict):
    function_name = frame.GetFunctionName() or ""
    symbol = frame.GetSymbol()
    symbol_name = symbol.GetName() if symbol.IsValid() else ""
    if "__pthread_kill" not in function_name and "__pthread_kill" not in symbol_name:
        return False
    signal_register = frame.FindRegister("x1")
    signal_number = signal_register.GetValueAsUnsigned(0) if signal_register.IsValid() else 0
    if signal_number != 6:
        return False
    thread = frame.GetThread()
    print("ZC_SIGABRT_THREAD_ID={}".format(thread.GetThreadID()))
    print("ZC_SIGABRT_THREAD_NAME={!r}".format(thread.GetName() or ""))
    print("ZC_SIGABRT_THREAD_INDEX={}".format(thread.GetIndexID()))
    _backtrace(thread, "ZC_SIGABRT_BACKTRACE_")
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


def _line_value(raw, marker):
    for line in raw.splitlines():
        if line.startswith(marker):
            return line.split('=', 1)[1].strip()
    return None


def _exception_records(raw):
    records = []
    for capture in raw.split('ZC_OBJC_CAPTURE_START index=')[1:]:
        if 'ZC_OBJC_CAPTURE_END' not in capture:
            continue
        section = capture.split('ZC_OBJC_CAPTURE_END', 1)[0]
        index = section.splitlines()[0].strip()
        names = _captured_values(section, 'ZC_OBJC_EXCEPTION_NAME')
        reasons = _captured_values(section, 'ZC_OBJC_EXCEPTION_REASON')
        records.append({
            'index': index,
            'threadId': _line_value(section, 'ZC_OBJC_THREAD_ID='),
            'threadName': _line_value(section, 'ZC_OBJC_THREAD_NAME='),
            'x0': _line_value(section, 'ZC_OBJC_X0='),
            'name': names[0] if names else None,
            'reason': reasons[0] if reasons else None,
            'evaluationSucceeded': (
                'ZC_OBJC_EVALUATION_SUCCEEDED index={}'.format(index) in section),
            'backtraceCaptured': 'ZC_OBJC_BACKTRACE_FRAME=' in section,
        })
    return records


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
        'breakpoints': [
            'objc_exception_throw', '__rust_foreign_exception', '__pthread_kill'],
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
            '-o', 'breakpoint set --name __pthread_kill',
            '-o', 'breakpoint command add -F zc_lldb_callback.capture_pthread_kill 3',
            '-o', 'process handle SIGABRT --notify true --stop true --pass false',
            '-o', 'settings set target.run-args -- --background',
            '-o', 'run',
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
        exception_records = _exception_records(raw)
        evidence['objcExceptions'] = exception_records
        evidence['objcThreadIds'] = [
            record['threadId'] for record in exception_records
            if record['threadId'] is not None]
        evidence['objcThreadNames'] = [
            record['threadName'] for record in exception_records
            if record['threadName'] is not None]
        evidence['objcExceptionNames'] = [
            record['name'] for record in exception_records
            if record['name'] is not None]
        evidence['objcExceptionReasons'] = [
            record['reason'] for record in exception_records
            if record['reason'] is not None]
        foreign_threads = [
            line.split('=', 1)[1].strip()
            for line in raw.splitlines()
            if line.startswith('ZC_FOREIGN_EXCEPTION_THREAD_ID=')
        ]
        evidence['foreignExceptionThreadIds'] = foreign_threads
        evidence['fatalThreadId'] = _line_value(raw, 'ZC_SIGABRT_THREAD_ID=')
        evidence['fatalThreadName'] = _line_value(raw, 'ZC_SIGABRT_THREAD_NAME=')
        evidence['sigabrtObserved'] = 'stop reason = signal SIGABRT' in raw
        evidence['foreignExceptionObserved'] = bool(foreign_threads)
        fatal_thread = evidence['fatalThreadId']
        correlated_exceptions = [
            record for record in exception_records
            if fatal_thread and record['threadId'] == fatal_thread]
        evidence['sameThreadAsFatalStop'] = bool(
            correlated_exceptions and
            evidence['sigabrtObserved'])
        evidence['objcEvaluationSucceeded'] = any(
            record['evaluationSucceeded'] for record in exception_records)
        evidence['fatalExceptionDetailsCaptured'] = any(
            record['evaluationSucceeded'] for record in correlated_exceptions)
        evidence['backtraceCaptured'] = (
            evidence['objcExceptionEntryCount'] > 0 and
            any(record['backtraceCaptured'] for record in exception_records))
        if evidence['objcExceptionEntryCount'] == 0:
            evidence['reason'] = (
                'LLDB did not confirm the objc_exception_throw entry frame')
        elif not evidence['objcEvaluationSucceeded']:
            evidence['reason'] = (
                'Objective-C exception expressions failed; see rawOutput')
        elif not evidence['sigabrtObserved']:
            evidence['classification'] = 'DIAGNOSTIC AVAILABLE'
            evidence['reason'] = (
                'Objective-C exception details were captured, but no SIGABRT '
                'stop was observed')
        elif not evidence['sameThreadAsFatalStop']:
            evidence['classification'] = 'DIAGNOSTIC AVAILABLE'
            evidence['reason'] = (
                'Objective-C exception details were captured, but the fatal '
                'thread correlation was not established')
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
