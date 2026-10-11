"""Run the bounded M0-M3 direct-bundle startup matrix on hosted Apple Silicon macOS."""
import datetime
import hashlib
import json
import os
import pathlib
import platform
import plistlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time


STARTUP_CHECKPOINT_RE = re.compile(r"^native_qa startup_checkpoint=([a-z_]+)$", re.MULTILINE)
MAC_CHECKPOINT_RE = re.compile(
    r"^native_qa macos_checkpoint=([a-z_]+) elapsed_ms=(\d+) main_thread=(true|false)$",
    re.MULTILINE,
)
TRAY_STAGE_RE = re.compile(r"^native_qa mac_tray_stage=([a-z_]+)(?: (.*))?$", re.MULTILINE)
SELECTOR_RE = re.compile(r"^native_qa mac_tray_selector case=(m[0-3]) source=(default|environment)$", re.MULTILINE)
WEBVIEW_RE = re.compile(r"ui_runtime startup_mode=background webview_count=(\d+)")
EXCEPTION_SUMMARY_RE = re.compile(
    r"uncaught exception ['\"]([^'\"]+)['\"].*?reason:\s*['\"]([^'\"]*)['\"]"
)
HISTORICAL_ABORT = "fatal runtime error: Rust cannot catch foreign exceptions, aborting"
STABLE_SURVIVAL_SECONDS = 30
STARTUP_TIMEOUT_SECONDS = 120
RESOURCE_SAMPLE_PERIOD_SECONDS = 5
RUNTIME_PACKAGES = ("tauri", "tao", "wry", "tray-icon", "objc2", "objc2-exception-helper")


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def run_command(*args, timeout=15):
    try:
        return subprocess.run(args, capture_output=True, text=True, timeout=timeout, check=False)
    except Exception:
        return None


def command_text(*args):
    result = run_command(*args)
    if result is None or result.returncode != 0:
        return None
    return result.stdout.strip()


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def bundle_identity(bundle, executable):
    digest = hashlib.sha256()
    file_count = 0
    total_bytes = 0
    for current, directories, files in os.walk(bundle, topdown=True, followlinks=False):
        directories.sort()
        files.sort()
        names = directories + files
        for name in names:
            path = pathlib.Path(current) / name
            relative = path.relative_to(bundle).as_posix()
            item = path.lstat()
            mode = item.st_mode & 0o777
            digest.update(relative.encode("utf-8", errors="surrogateescape"))
            digest.update(b"\0")
            digest.update(str(mode).encode("ascii"))
            digest.update(b"\0")
            if path.is_symlink():
                digest.update(b"symlink\0")
                digest.update(os.readlink(path).encode("utf-8", errors="surrogateescape"))
                digest.update(b"\0")
            elif path.is_dir():
                digest.update(b"directory\0")
            elif path.is_file():
                file_count += 1
                total_bytes += item.st_size
                digest.update(b"file\0")
                digest.update(str(item.st_size).encode("ascii"))
                digest.update(b"\0")
                with path.open("rb") as source:
                    for block in iter(lambda: source.read(1024 * 1024), b""):
                        digest.update(block)
                digest.update(b"\0")
            else:
                digest.update(b"other\0")
    info_path = bundle / "Contents" / "Info.plist"
    with info_path.open("rb") as source:
        info = plistlib.load(source)
    return {
        "bundleTreeSha256": digest.hexdigest(),
        "bundleFileCount": file_count,
        "bundleFileBytes": total_bytes,
        "executableSha256": sha256_file(executable),
        "infoPlistSha256": sha256_file(info_path),
        "bundleIdentity": {
            "CFBundleIdentifier": info.get("CFBundleIdentifier"),
            "CFBundleExecutable": info.get("CFBundleExecutable"),
            "CFBundleName": info.get("CFBundleName"),
            "CFBundleVersion": info.get("CFBundleVersion"),
            "CFBundleShortVersionString": info.get("CFBundleShortVersionString"),
        },
    }


def locked_runtime_versions(lockfile):
    versions = {name: [] for name in RUNTIME_PACKAGES}
    contents = lockfile.read_text(encoding="utf-8")
    for package in re.split(r"(?=^\[\[package\]\]$)", contents, flags=re.MULTILINE):
        name = re.search(r'^name = "([^\"]+)"$', package, flags=re.MULTILINE)
        version = re.search(r'^version = "([^\"]+)"$', package, flags=re.MULTILINE)
        if name and version and name.group(1) in versions:
            versions[name.group(1)].append(version.group(1))
    return {name: sorted(set(found)) for name, found in versions.items() if found}


def runner_context():
    version = command_text("sw_vers", "-productVersion")
    build = command_text("sw_vers", "-buildVersion")
    uid = str(os.getuid())
    manager = command_text("launchctl", "managername")
    gui_domain = run_command("launchctl", "print", f"gui/{uid}")
    window_server = run_command("pgrep", "-x", "WindowServer")
    return {
        "platform": platform.platform(),
        "system": platform.system(),
        "architecture": platform.machine(),
        "macOSVersion": version,
        "macOSBuild": build,
        "runnerOS": os.environ.get("RUNNER_OS"),
        "runnerArchitecture": os.environ.get("RUNNER_ARCH"),
        "runnerImageOS": os.environ.get("ImageOS"),
        "runnerImageVersion": os.environ.get("ImageVersion"),
        "launchctlManagerName": manager,
        "guiLaunchAgentDomainAvailable": bool(gui_domain and gui_domain.returncode == 0),
        "windowServerProcessPresent": bool(window_server and window_server.returncode == 0),
        "guiSessionObserved": bool(
            gui_domain and gui_domain.returncode == 0 and window_server and window_server.returncode == 0
        ),
        "launchDescription": "direct .app/Contents/MacOS/zen-canvas --background",
        "finderLaunch": False,
        "loginAutostartLaunch": False,
    }


def candidate_processes():
    result = run_command("ps", "-axo", "pid=", "-o", "comm=", timeout=15)
    if result is None or result.returncode != 0:
        return None
    rows = []
    for line in result.stdout.splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) < 2:
            continue
        try:
            pid = int(fields[0])
        except ValueError:
            continue
        comm = fields[1]
        if pathlib.Path(comm).name == "zen-canvas":
            rows.append({"pid": pid, "processName": pathlib.Path(comm).name})
    return rows


def terminate_task_candidate_processes():
    processes = candidate_processes()
    if processes is None:
        return None
    for row in processes:
        try:
            os.kill(row["pid"], signal.SIGTERM)
        except ProcessLookupError:
            pass
        except PermissionError:
            continue
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        remaining = candidate_processes()
        if remaining == []:
            return []
        if remaining is None:
            return None
        time.sleep(0.1)
    remaining = candidate_processes()
    for row in remaining or []:
        try:
            os.kill(row["pid"], signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            continue
    time.sleep(0.1)
    return candidate_processes()


def parse_startup(log):
    checkpoints = STARTUP_CHECKPOINT_RE.findall(log)
    timed = [
        {"name": name, "elapsedMs": int(elapsed), "mainThread": is_main == "true"}
        for name, elapsed, is_main in MAC_CHECKPOINT_RE.findall(log)
    ]
    stages = [{"name": name, "details": details or ""} for name, details in TRAY_STAGE_RE.findall(log)]
    selector_matches = SELECTOR_RE.findall(log)
    webview_match = WEBVIEW_RE.search(log)
    return {
        "startupCheckpointOrder": checkpoints,
        "macOSCheckpointTimeline": timed,
        "lastSuccessfulCheckpoint": checkpoints[-1] if checkpoints else None,
        "timeToLastCheckpointMs": timed[-1]["elapsedMs"] if timed else None,
        "mainThreadVerification": (
            "VERIFIED_MAIN_THREAD"
            if timed and all(entry["mainThread"] for entry in timed)
            else "NOT_MAIN_THREAD"
            if any(not entry["mainThread"] for entry in timed)
            else "DIAGNOSTIC UNAVAILABLE"
        ),
        "mainThreadCheckpointCount": len(timed),
        "selectorObservations": [{"case": case, "source": source} for case, source in selector_matches],
        "trayStageEvents": stages,
        "menuConstructionCompleted": any(item["name"] == "menu_construction_complete" for item in stages),
        "productIconDecodeCompleted": any(
            item["name"] == "icon_decode_complete" and "variant=product_png" in item["details"]
            for item in stages
        ),
        "knownGoodIconValidated": any(
            item["name"] == "icon_rgba_validated"
            and all(
                value in item["details"]
                for value in ("variant=m3", "width=20", "height=20", "length=1600")
            )
            for item in stages
        ),
        "knownGoodIconConstructed": any(
            item["name"] == "icon_construction_complete" and "variant=m3" in item["details"]
            for item in stages
        ),
        "iconOmitted": any(
            item["name"] == "icon_omitted" and "variant=m2" in item["details"]
            for item in stages
        ),
        "trayBuilderCompleted": any(item["name"] == "tray_builder_complete" for item in stages),
        "coreRuntimeOwnersReady": "core_runtime_owners_ready" in checkpoints,
        "trayReady": "tray_ready" in checkpoints,
        "traySkippedForControl": "tray_skipped_for_control" in log,
        "setupComplete": "setup_complete" in checkpoints,
        "webviewCount": int(webview_match.group(1)) if webview_match else None,
    }


def cpu_seconds(value):
    parts = value.replace("-", ":").split(":")
    if len(parts) == 4:
        return int(parts[0]) * 86400 + int(parts[1]) * 3600 + int(parts[2]) * 60 + float(parts[3])
    return sum(float(part) * 60 ** index for index, part in enumerate(reversed(parts)))


def process_observation(pid, started):
    result = {"timestamp": now(), "elapsedSeconds": round(time.monotonic() - started, 3)}
    ps = run_command("ps", "-p", str(pid), "-o", "rss=", "-o", "time=", "-o", "etime=")
    if ps and ps.returncode == 0 and ps.stdout.strip():
        fields = ps.stdout.split()
        if len(fields) >= 3:
            try:
                result["rssBytes"] = int(fields[0]) * 1024
                result["cpuTotalSeconds"] = cpu_seconds(fields[1])
                result["elapsedProcessTime"] = fields[2]
            except (ValueError, IndexError):
                result["processMetrics"] = "DIAGNOSTIC UNAVAILABLE"
    else:
        result["processMetrics"] = "DIAGNOSTIC UNAVAILABLE"

    lsof = run_command("lsof", "-a", "-p", str(pid), "-Ff", timeout=15)
    if lsof and lsof.returncode == 0:
        result["fdCount"] = sum(
            line.startswith("f") and line[1:].isdigit() for line in lsof.stdout.splitlines()
        )
    else:
        result["fdCount"] = "DIAGNOSTIC UNAVAILABLE"

    children = run_command("pgrep", "-P", str(pid))
    if children and children.returncode in (0, 1):
        child_rows = []
        for child in children.stdout.split():
            comm = run_command("ps", "-p", child, "-o", "comm=")
            if comm and comm.returncode == 0:
                child_rows.append(comm.stdout.strip())
        webkit = [name for name in child_rows if "WebKit" in name]
        result["webkitChildProcessCount"] = len(webkit)
    else:
        result["webkitChildProcessCount"] = "DIAGNOSTIC UNAVAILABLE"
    return result


def process_exit(returncode):
    if returncode is None:
        return {"exitCode": None, "signalNumber": None, "signalName": None}
    if returncode < 0:
        number = -returncode
        try:
            name = signal.Signals(number).name
        except ValueError:
            name = f"signal_{number}"
        return {"exitCode": None, "signalNumber": number, "signalName": name}
    return {"exitCode": returncode, "signalNumber": None, "signalName": None}


def sanitize_log(contents, task_root, candidate):
    for path in (str(task_root), str(candidate.parent.parent), str(candidate)):
        contents = contents.replace(path, "<TASK_PATH>")
    return contents


def read_text(path):
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return ""


def validation_errors(case, parsed):
    errors = []
    expected_source = "default" if case == "m0" else "environment"
    if not any(item == {"case": case, "source": expected_source} for item in parsed["selectorObservations"]):
        errors.append("expected_runtime_selector_observation_missing")
    if parsed["setupComplete"] and parsed["webviewCount"] != 0:
        errors.append("background_startup_did_not_report_zero_webviews")
    if case == "m1":
        if parsed["trayReady"]:
            errors.append("M1_forbidden_tray_ready_checkpoint")
        if parsed["setupComplete"] and not parsed["traySkippedForControl"]:
            errors.append("M1_missing_tray_skipped_for_control")
        if parsed["trayStageEvents"]:
            errors.append("M1_unexpected_tray_construction_stage")
    else:
        if parsed["traySkippedForControl"]:
            errors.append("non_M1_logged_tray_skipped_for_control")
        if parsed["setupComplete"] and not parsed["trayReady"]:
            errors.append("Tray_mode_setup_completed_without_tray_ready")
        if parsed["setupComplete"]:
            expected_stages = {
                "m0": ["menu_construction_complete", "icon_decode_complete", "tray_builder_complete"],
                "m2": ["menu_construction_complete", "icon_omitted", "tray_builder_complete"],
                "m3": [
                    "menu_construction_complete",
                    "icon_rgba_validated",
                    "icon_construction_complete",
                    "tray_builder_complete",
                ],
            }[case]
            actual_stages = [item["name"] for item in parsed["trayStageEvents"]]
            if actual_stages != expected_stages:
                errors.append(f"{case}_unexpected_tray_stage_sequence")
            if not parsed["menuConstructionCompleted"]:
                errors.append(f"{case}_menu_construction_checkpoint_missing")
            if not parsed["trayBuilderCompleted"]:
                errors.append(f"{case}_tray_builder_checkpoint_missing")
            if case == "m0" and not parsed["productIconDecodeCompleted"]:
                errors.append("m0_product_png_decode_checkpoint_missing")
            if case == "m2" and not parsed["iconOmitted"]:
                errors.append("m2_icon_omission_checkpoint_missing")
            if case == "m3" and not parsed["knownGoodIconValidated"]:
                errors.append("m3_known_good_rgba_validation_missing")
            if case == "m3" and not parsed["knownGoodIconConstructed"]:
                errors.append("m3_known_good_rgba_construction_missing")
    return errors


def attempt_result_class(attempt):
    stages = attempt.get("trayStageEvents", [])
    last_tray_stage = stages[-1]["name"] if stages else "no_tray_stage"
    if attempt.get("validationErrors"):
        return (
            "protocol_error:"
            + ",".join(attempt["validationErrors"])
            + f":{last_tray_stage}"
        )
    if attempt["outcome"] == "stable_survived_30s":
        return "stable"
    if attempt["outcome"] in ("exited_before_setup", "exited_after_setup"):
        exit_details = attempt.get("processExitDetails", {})
        exit_signature = exit_details.get("signalName") or exit_details.get("exitCode")
        checkpoint = attempt.get("lastSuccessfulCheckpoint") or "no_checkpoint"
        return f"{attempt['outcome']}:{exit_signature}:{checkpoint}:{last_tray_stage}"
    return f"incomplete:{attempt['outcome']}:{last_tray_stage}"


def run_attempt(case, ordinal, executable, frozen_identity, artifact_root, runner_temp):
    label = f"{case}-{ordinal:02d}"
    task_root = pathlib.Path(tempfile.mkdtemp(prefix=f"macos-tray-{label}-", dir=runner_temp)).resolve()
    profile = task_root / "profile"
    home = task_root / "home"
    temp = task_root / "temp"
    trace = task_root / "global-index-trace.log"
    for directory in (profile, home, temp):
        directory.mkdir()
    profile_entries_before = list(profile.iterdir())
    stdout_path = artifact_root / f"{label}.stdout.log"
    stderr_path = artifact_root / f"{label}.stderr.log"
    before_processes = candidate_processes()
    current_identity = bundle_identity(executable.parent.parent.parent, executable)
    identity_matches = current_identity == frozen_identity
    attempt = {
        "mode": case,
        "attempt": ordinal,
        "startedAt": now(),
        "launchMethod": "direct .app/Contents/MacOS/zen-canvas --background",
        "command": ["<APP_BUNDLE>/Contents/MacOS/zen-canvas", "--background"],
        "selectorEnvironment": None if case == "m0" else case,
        "candidateIdentity": frozen_identity,
        "candidateIdentityUnchangedBeforeLaunch": identity_matches,
        "previousCandidateProcesses": before_processes,
        "freshProfile": not profile_entries_before,
        "profileTaskOwned": True,
        "stdoutArtifact": stdout_path.name,
        "stderrArtifact": stderr_path.name,
        "resourceSamples": [],
        "cleanup": {},
        "protocolErrors": [],
    }
    process = None
    started = time.monotonic()
    natural_returncode = None
    stable_since = None
    last_sample = 0.0
    try:
        if not identity_matches:
            attempt["outcome"] = "candidate_identity_changed"
        elif before_processes is None:
            attempt["outcome"] = "prelaunch_process_check_unavailable"
        elif before_processes:
            attempt["outcome"] = "not_launched_preexisting_instance"
        elif profile_entries_before:
            attempt["outcome"] = "profile_not_fresh"
        else:
            environment = dict(os.environ)
            environment["ZC_NATIVE_QA_PROFILE_ROOT"] = str(profile)
            environment["ZC_GLOBAL_INDEX_QA_TRACE"] = str(trace)
            environment["HOME"] = str(home)
            environment["CFFIXED_USER_HOME"] = str(home)
            environment["TMPDIR"] = str(temp)
            environment["XDG_CONFIG_HOME"] = str(home / ".config")
            environment["XDG_DATA_HOME"] = str(home / ".local" / "share")
            environment["XDG_CACHE_HOME"] = str(home / ".cache")
            if case == "m0":
                environment.pop("ZC_NATIVE_QA_MAC_TRAY_CASE", None)
            else:
                environment["ZC_NATIVE_QA_MAC_TRAY_CASE"] = case
            for path in (home / ".config", home / ".local" / "share", home / ".cache"):
                path.mkdir(parents=True, exist_ok=True)
            with stdout_path.open("w", encoding="utf-8") as stdout, stderr_path.open(
                "w", encoding="utf-8"
            ) as stderr:
                process = subprocess.Popen(
                    [str(executable), "--background"],
                    cwd=task_root,
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    stdout=stdout,
                    stderr=stderr,
                )
                attempt["pid"] = process.pid
                attempt["outcome"] = "startup_pending"
                while True:
                    elapsed = time.monotonic() - started
                    log = read_text(stderr_path)
                    parsed = parse_startup(log)
                    if process.poll() is not None:
                        natural_returncode = process.returncode
                        attempt["outcome"] = (
                            "exited_after_setup" if parsed["setupComplete"] else "exited_before_setup"
                        )
                        break
                    if parsed["setupComplete"]:
                        if stable_since is None:
                            stable_since = time.monotonic()
                            attempt["setupCompleteObservedAtSeconds"] = round(
                                stable_since - started, 3
                            )
                        stable_elapsed = time.monotonic() - stable_since
                        attempt["stableSurvivalSecondsObserved"] = round(stable_elapsed, 3)
                        if stable_elapsed - last_sample >= 0:
                            if not attempt["resourceSamples"] or stable_elapsed - last_sample >= RESOURCE_SAMPLE_PERIOD_SECONDS:
                                attempt["resourceSamples"].append(
                                    process_observation(process.pid, started)
                                )
                                last_sample = stable_elapsed
                        if stable_elapsed >= STABLE_SURVIVAL_SECONDS:
                            attempt["outcome"] = "stable_survived_30s"
                            break
                    elif elapsed >= STARTUP_TIMEOUT_SECONDS:
                        attempt["outcome"] = "startup_timeout"
                        attempt["startupTimeoutSeconds"] = STARTUP_TIMEOUT_SECONDS
                        break
                    time.sleep(0.2)
                attempt["naturalProcessExit"] = process.poll() is not None
                attempt["naturalProcessExitDetails"] = process_exit(natural_returncode)
    except Exception as error:
        attempt["outcome"] = "attempt_exception"
        attempt["failure"] = repr(error)
    finally:
        if process is not None and process.poll() is None:
            attempt["cleanupSignalSent"] = "SIGTERM"
            try:
                process.terminate()
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                attempt["cleanupSignalSent"] = "SIGKILL"
                process.kill()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    attempt["cleanupProcessFailure"] = "process did not stop within bounded cleanup"
            except Exception as error:
                attempt["cleanupProcessFailure"] = repr(error)
        attempt["cleanup"]["processStopped"] = process is None or process.poll() is not None
        attempt["cleanupProcessExitDetails"] = (
            process_exit(process.returncode) if process is not None else None
        )
        if process is not None and not before_processes:
            remaining = terminate_task_candidate_processes()
        else:
            remaining = candidate_processes()
        attempt["cleanup"]["remainingCandidateProcesses"] = remaining
        attempt["cleanup"]["noPreviousInstanceSurvived"] = remaining == []
        final_log = read_text(stderr_path)
        sanitized_stderr = sanitize_log(final_log, task_root, executable)
        stderr_path.write_text(sanitized_stderr, encoding="utf-8")
        stdout_path.write_text(
            sanitize_log(read_text(stdout_path), task_root, executable), encoding="utf-8"
        )
        parsed = parse_startup(sanitized_stderr)
        attempt.update(parsed)
        attempt["elapsedSeconds"] = round(time.monotonic() - started, 3)
        attempt["finishedAt"] = now()
        attempt["processExitDetails"] = process_exit(natural_returncode)
        attempt["historicalAbortSignature"] = HISTORICAL_ABORT in sanitized_stderr
        attempt["objcExceptionEvidenceInLog"] = any(
            token in sanitized_stderr
            for token in ("objc_exception_throw", "__rust_foreign_exception", "NSException")
        )
        attempt["validationErrors"] = validation_errors(case, parsed)
        attempt["profileRootEntriesAfter"] = sorted(path.name for path in profile.iterdir())
        attempt["profileDatabaseCreated"] = (profile / "zen-canvas.sqlite3").is_file()
        try:
            if task_root.parent != pathlib.Path(runner_temp).resolve() or not task_root.name.startswith(
                f"macos-tray-{label}-"
            ):
                raise RuntimeError("task cleanup boundary mismatch")
            shutil.rmtree(task_root)
            attempt["cleanup"]["taskProfileRemoved"] = not task_root.exists()
        except Exception as error:
            attempt["cleanup"]["taskProfileRemoved"] = False
            attempt["cleanup"]["failure"] = repr(error)
        attempt["resultClass"] = attempt_result_class(attempt)
        (artifact_root / f"{label}.json").write_text(
            json.dumps(attempt, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
    return attempt


def run_invalid_selector(executable, artifact_root, runner_temp):
    task_root = pathlib.Path(tempfile.mkdtemp(prefix="macos-tray-invalid-selector-", dir=runner_temp)).resolve()
    profile = task_root / "profile"
    home = task_root / "home"
    temp = task_root / "temp"
    for directory in (profile, home, temp):
        directory.mkdir()
    stdout_path = artifact_root / "invalid-selector.stdout.log"
    stderr_path = artifact_root / "invalid-selector.stderr.log"
    before = candidate_processes()
    result = {
        "launchMethod": "direct .app/Contents/MacOS/zen-canvas --background",
        "selectorEnvironment": "invalid",
        "previousCandidateProcesses": before,
        "protocolErrors": [],
        "cleanup": {},
    }
    process = None
    try:
        if before is None or before:
            result["outcome"] = "not_run_preexisting_or_unobservable_process"
        else:
            environment = dict(os.environ)
            environment["ZC_NATIVE_QA_PROFILE_ROOT"] = str(profile)
            environment["HOME"] = str(home)
            environment["CFFIXED_USER_HOME"] = str(home)
            environment["TMPDIR"] = str(temp)
            environment["ZC_NATIVE_QA_MAC_TRAY_CASE"] = "invalid"
            with stdout_path.open("w", encoding="utf-8") as stdout, stderr_path.open(
                "w", encoding="utf-8"
            ) as stderr:
                process = subprocess.Popen(
                    [str(executable), "--background"],
                    cwd=task_root,
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    stdout=stdout,
                    stderr=stderr,
                )
                result["pid"] = process.pid
                try:
                    returncode = process.wait(timeout=15)
                    result["outcome"] = "exited"
                    result["processExitDetails"] = process_exit(returncode)
                except subprocess.TimeoutExpired:
                    result["outcome"] = "timeout"
                    process.terminate()
                    process.wait(timeout=10)
            log = read_text(stderr_path)
            result["explicitSelectorError"] = "native_qa mac_tray_selector_error reason=invalid_selector allowed=m0,m1,m2,m3" in log
            result["tauriSetupEntered"] = "tauri_setup_entered" in log
            result["trayReady"] = "tray_ready" in log
            if process.returncode != 64 or not result["explicitSelectorError"] or result["tauriSetupEntered"] or result["trayReady"]:
                result["protocolErrors"].append("invalid_selector_did_not_fail_explicitly_before_tray_setup")
        result["stdoutArtifact"] = stdout_path.name
        result["stderrArtifact"] = stderr_path.name
        result["finishedAt"] = now()
    except Exception as error:
        result["outcome"] = "validation_exception"
        result["failure"] = repr(error)
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
        result["cleanup"]["processStopped"] = process is None or process.poll() is not None
        remaining = candidate_processes()
        result["cleanup"]["remainingCandidateProcesses"] = remaining
        result["cleanup"]["noPreviousInstanceSurvived"] = remaining == []
        if stdout_path.exists():
            stdout_path.write_text(
                sanitize_log(read_text(stdout_path), task_root, executable), encoding="utf-8"
            )
        if stderr_path.exists():
            stderr_path.write_text(
                sanitize_log(read_text(stderr_path), task_root, executable), encoding="utf-8"
            )
        try:
            if task_root.parent != pathlib.Path(runner_temp).resolve() or not task_root.name.startswith(
                "macos-tray-invalid-selector-"
            ):
                raise RuntimeError("invalid-selector cleanup boundary mismatch")
            shutil.rmtree(task_root)
            result["cleanup"]["taskProfileRemoved"] = not task_root.exists()
        except Exception as error:
            result["cleanup"]["taskProfileRemoved"] = False
            result["cleanup"]["failure"] = repr(error)
        (artifact_root / "invalid-selector.json").write_text(
            json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
    return result


def run_lldb_diagnostic(mode, executable, expected_sha, artifact_root):
    output_json = artifact_root / "lldb-diagnostic.json"
    stdout_path = artifact_root / "lldb-driver.stdout.log"
    stderr_path = artifact_root / "lldb-driver.stderr.log"
    result = {
        "mode": mode,
        "classification": "DIAGNOSTIC UNAVAILABLE",
        "breakpoints": ["objc_exception_throw", "__rust_foreign_exception"],
        "timeoutSeconds": 45,
    }
    before = candidate_processes()
    process = None
    if before is None or before:
        result["reason"] = "existing candidate process prevented isolated LLDB launch"
    else:
        environment = dict(os.environ)
        environment["EXPECTED_SOURCE_SHA"] = expected_sha
        if mode == "m0":
            environment.pop("ZC_NATIVE_QA_MAC_TRAY_CASE", None)
        else:
            environment["ZC_NATIVE_QA_MAC_TRAY_CASE"] = mode
        command = [sys.executable, "scripts/diagnoseMacosStartup.py", str(executable), str(output_json)]
        try:
            with stdout_path.open("w", encoding="utf-8") as stdout, stderr_path.open(
                "w", encoding="utf-8"
            ) as stderr:
                process = subprocess.Popen(
                    command,
                    cwd=pathlib.Path.cwd(),
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    stdout=stdout,
                    stderr=stderr,
                    start_new_session=True,
                )
                try:
                    process.wait(timeout=75)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=5)
                    result["reason"] = "bounded LLDB driver exceeded 75 seconds"
            result["driverExitCode"] = process.returncode
            if output_json.exists():
                diagnostic = json.loads(output_json.read_text(encoding="utf-8"))
                result.update(diagnostic)
                result["debuggerLaunched"] = bool(diagnostic.get("lldbPath"))
            elif "reason" not in result:
                result["reason"] = "LLDB driver did not write diagnostic output"
        except Exception as error:
            result["reason"] = repr(error)
        result["stdoutArtifact"] = stdout_path.name
        result["stderrArtifact"] = stderr_path.name
    raw_output = result.get("rawOutput", "")
    exception_summary = EXCEPTION_SUMMARY_RE.search(raw_output)
    if exception_summary:
        result["exceptionClassAndReason"] = {
            "class": exception_summary.group(1),
            "reason": exception_summary.group(2),
            "source": "explicit runtime text in bounded LLDB output",
        }
    else:
        result["exceptionClassAndReason"] = (
            "DIAGNOSTIC UNAVAILABLE — no safe exception class/reason text was emitted; no Objective-C description method was invoked"
        )
    result["finishedAt"] = now()
    if not before:
        after = terminate_task_candidate_processes()
    else:
        after = candidate_processes()
    previous_cleanup = result.get("cleanup", {})
    result["cleanup"] = {
        **previous_cleanup,
        "driverStopped": process is None or process.poll() is not None,
        "noCandidateProcessesRemain": after == [],
        "remainingCandidateProcesses": after,
    }
    (artifact_root / "lldb-diagnostic-summary.json").write_text(
        json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    return result


def mode_summary(case, attempts):
    classes = [attempt["resultClass"] for attempt in attempts]
    stable = sum(attempt["outcome"] == "stable_survived_30s" for attempt in attempts)
    exits = sum(attempt["outcome"] in ("exited_before_setup", "exited_after_setup") for attempt in attempts)
    incomplete = sum(
        attempt["resultClass"].startswith(("incomplete:", "protocol_error:"))
        for attempt in attempts
    )
    mixed = len(set(classes)) > 1
    if incomplete:
        classification = "INCOMPLETE"
    elif mixed:
        classification = "MIXED"
    elif stable == len(attempts):
        classification = "STABLE_SURVIVAL"
    elif exits == len(attempts):
        classification = "EXITED_ON_EACH_LAUNCH"
    else:
        classification = "INCOMPLETE"
    return {
        "mode": case,
        "classification": classification,
        "attemptCount": len(attempts),
        "launchedCount": sum(attempt.get("pid") is not None for attempt in attempts),
        "stableSurvivalCount": stable,
        "exitCount": exits,
        "incompleteOrProtocolErrorCount": incomplete,
        "mixedAfterInitialThree": mixed and len(attempts) == 5,
        "resultClasses": classes,
        "attempts": attempts,
    }


def causal_hypothesis(modes, historical_m0):
    if historical_m0 != "REPRODUCED":
        return (
            "No tray-cause conclusion: the historical M0 foreign-exception signature was not reproduced. "
            "Compare the recorded source, locked dependencies, macOS build, runner image, and GUI session "
            "with the historical candidate; this result does not mean #270 is fixed."
        )

    stable = {
        case: summary["stableSurvivalCount"] == summary["attemptCount"]
        and summary["incompleteOrProtocolErrorCount"] == 0
        for case, summary in modes.items()
    }
    if not stable["m1"]:
        return (
            "M1 did not consistently survive without Tray, so the observed failure is not isolated to Tray setup. "
            "Use the last checkpoint and native stack to locate the remaining startup path."
        )
    if stable["m2"] and stable["m3"]:
        return (
            "M0 reproduced while the no-Tray, no-icon Tray, and known-good-icon Tray controls survived. "
            "That comparison is consistent with the product PNG/icon conversion path, rather than Tray creation "
            "generally, as the narrow cause; it does not prove which layer throws."
        )
    if stable["m2"] and not stable["m3"]:
        return (
            "M1 and M2 survived consistently, while M0 reproduced and M3 did not consistently survive. "
            "The comparison is consistent with different behavior in icon-bearing Tray builds; the product "
            "PNG itself is not uniquely implicated."
        )
    if not stable["m2"] and stable["m3"]:
        return (
            "M1 and M3 survived consistently, while M0 reproduced and M2 did not consistently survive. "
            "The known-good RGBA icon changes the outcome, which points to icon handling, but the comparison "
            "does not isolate PNG decoding from the Tray builder's icon conversion."
        )
    if not stable["m2"] and not stable["m3"]:
        return (
            "M1 survived consistently, while neither Tray-building control survived consistently, including "
            "the known-good RGBA icon. That comparison is consistent with Tray construction or the hosted "
            "Tray/session boundary, not the product PNG alone."
        )
    return "The mode outcomes do not support a single causal attribution; preserve the mixed results for specialist review."


def markdown_summary(matrix):
    identity = matrix.get("candidateIdentity", {})
    lines = [
        "# macOS Tahoe Tray M0-M3 diagnostics",
        "",
        f"Matrix status: **{matrix.get('matrixStatus', 'INCOMPLETE')}**",
        "",
        f"Source HEAD/tree: `{matrix.get('sourceHead')}` / `{matrix.get('sourceTree')}`",
        f"Cargo.lock SHA-256: `{matrix.get('cargoLockSha256')}`",
        f"Executable SHA-256: `{identity.get('executableSha256')}`",
        f"Bundle tree SHA-256: `{identity.get('bundleTreeSha256')}`",
        f"Info.plist SHA-256: `{identity.get('infoPlistSha256')}`",
        f"Runner: `{matrix.get('runner', {}).get('macOSVersion')}` / `{matrix.get('runner', {}).get('architecture')}` / image `{matrix.get('runner', {}).get('runnerImageOS')}` `{matrix.get('runner', {}).get('runnerImageVersion')}`",
        f"GUI session observed: `{matrix.get('runner', {}).get('guiSessionObserved')}`; launch: direct bundle executable with `--background`",
        "",
        "| Mode | Result | Launches | Stable 30s | Exits | Incomplete / protocol errors |",
        "|---|---|---:|---:|---:|---:|",
    ]
    for case in ("m0", "m1", "m2", "m3"):
        summary = matrix.get("modes", {}).get(case, {})
        lines.append(
            f"| {case.upper()} | {summary.get('classification')} | {summary.get('launchedCount')} / {summary.get('attemptCount')} | {summary.get('stableSurvivalCount')} | {summary.get('exitCount')} | {summary.get('incompleteOrProtocolErrorCount')} |"
        )
    lines.extend(
        [
            "",
            f"Invalid selector check: `{matrix.get('invalidSelector', {}).get('outcome')}`; explicit error verified: `{matrix.get('invalidSelector', {}).get('explicitSelectorError')}`.",
            f"LLDB: **{matrix.get('lldbDiagnostic', {}).get('classification')}**; reason: {matrix.get('lldbDiagnostic', {}).get('reason', 'captured bounded trace')}",
            f"M0 historical abort: **{matrix.get('historicalM0Classification')}**.",
            f"Causal hypothesis: {matrix.get('causalHypothesis')}",
            "",
            "All per-attempt JSON and sanitized stdout/stderr logs are retained beside this summary. A stable startup result is diagnostic evidence only; it is not macOS support or release qualification.",
            "",
        ]
    )
    return "\n".join(lines)


def main():
    if len(sys.argv) != 3:
        print("usage: qualifyMacosTrayMatrix.py <bundle-executable> <artifact-directory>", file=sys.stderr)
        return 2
    executable = pathlib.Path(sys.argv[1]).resolve()
    bundle = executable.parent.parent.parent
    artifact_root = pathlib.Path(sys.argv[2]).resolve()
    artifact_root.mkdir(parents=True, exist_ok=True)
    attempts_root = artifact_root / "attempts"
    attempts_root.mkdir(exist_ok=True)
    source_head = command_text("git", "rev-parse", "HEAD")
    source_tree = command_text("git", "rev-parse", "HEAD^{tree}")
    expected_sha = os.environ.get("EXPECTED_SOURCE_SHA")
    runner = runner_context()
    matrix = {
        "startedAt": now(),
        "sourceHead": source_head,
        "sourceTree": source_tree,
        "expectedSourceSha": expected_sha,
        "runner": runner,
        "runtimeDependencies": locked_runtime_versions(pathlib.Path("src-tauri/Cargo.lock")),
        "cargoLockSha256": sha256_file(pathlib.Path("src-tauri/Cargo.lock")),
        "nativeCargoLockSha256": sha256_file(pathlib.Path("src-tauri/native/Cargo.lock")),
        "launchProtocol": "one locked .app candidate; direct executable launch; selector changed only in process environment",
        "requestedStableSurvivalSeconds": STABLE_SURVIVAL_SECONDS,
        "modes": {},
        "infrastructureErrors": [],
    }
    if expected_sha is None or source_head != expected_sha:
        matrix["infrastructureErrors"].append("source_head_did_not_match_expected_event_sha")
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        matrix["infrastructureErrors"].append("runner_not_native_Apple_Silicon_macOS")
    if not runner.get("macOSVersion", "").startswith("26."):
        matrix["infrastructureErrors"].append("runner_not_macos_Tahoe_26")
    if not executable.is_file() or not bundle.is_dir():
        matrix["infrastructureErrors"].append("candidate_app_or_executable_missing")
    runner_temp = pathlib.Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())).resolve()
    if not matrix["infrastructureErrors"]:
        try:
            frozen_identity = bundle_identity(bundle, executable)
            matrix["candidateIdentity"] = frozen_identity
            matrix["applicationBundle"] = bundle.name
            matrix["binaryRelativePath"] = "Contents/MacOS/zen-canvas"
            invalid_selector = run_invalid_selector(executable, attempts_root, runner_temp)
            matrix["invalidSelector"] = invalid_selector
            if invalid_selector.get("protocolErrors") or not invalid_selector.get("explicitSelectorError"):
                matrix["infrastructureErrors"].append("invalid_selector_contract_failed")
            for field in ("processStopped", "taskProfileRemoved", "noPreviousInstanceSurvived"):
                if invalid_selector.get("cleanup", {}).get(field) is not True:
                    matrix["infrastructureErrors"].append(f"invalid_selector_{field}_failed")

            for case in ("m0", "m1", "m2", "m3"):
                attempts = []
                for ordinal in range(1, 4):
                    attempt = run_attempt(
                        case, ordinal, executable, frozen_identity, attempts_root, runner_temp
                    )
                    attempts.append(attempt)
                if len({attempt["resultClass"] for attempt in attempts}) > 1:
                    for ordinal in range(4, 6):
                        attempts.append(
                            run_attempt(
                                case, ordinal, executable, frozen_identity, attempts_root, runner_temp
                            )
                        )
                matrix["modes"][case] = mode_summary(case, attempts)
                for attempt in attempts:
                    if attempt.get("candidateIdentityUnchangedBeforeLaunch") is not True:
                        matrix["infrastructureErrors"].append(f"{case}_{attempt['attempt']}_candidate_identity_changed")
                    if attempt.get("cleanup", {}).get("processStopped") is not True:
                        matrix["infrastructureErrors"].append(f"{case}_{attempt['attempt']}_process_cleanup_failed")
                    if attempt.get("cleanup", {}).get("taskProfileRemoved") is not True:
                        matrix["infrastructureErrors"].append(f"{case}_{attempt['attempt']}_profile_cleanup_failed")
                    if attempt.get("cleanup", {}).get("noPreviousInstanceSurvived") is not True:
                        matrix["infrastructureErrors"].append(f"{case}_{attempt['attempt']}_candidate_process_survived")
                    matrix["infrastructureErrors"].extend(
                        f"{case}_{attempt['attempt']}_{error}"
                        for error in attempt.get("validationErrors", [])
                    )
                    if attempt.get("resultClass", "").startswith("incomplete:"):
                        matrix["infrastructureErrors"].append(f"{case}_{attempt['attempt']}_attempt_incomplete")

            final_identity = bundle_identity(bundle, executable)
            matrix["candidateIdentityUnchangedAfterMatrix"] = final_identity == frozen_identity
            if final_identity != frozen_identity:
                matrix["infrastructureErrors"].append("candidate_identity_changed_after_matrix")
            m0_attempts = matrix["modes"]["m0"]["attempts"]
            if any(
                attempt["historicalAbortSignature"]
                and attempt["coreRuntimeOwnersReady"]
                and not attempt["trayReady"]
                for attempt in m0_attempts
            ):
                matrix["historicalM0Classification"] = "REPRODUCED"
            elif all(
                attempt["outcome"] == "stable_survived_30s"
                and not attempt.get("validationErrors")
                and attempt["coreRuntimeOwnersReady"]
                and attempt["trayReady"]
                and attempt["setupComplete"]
                and attempt["webviewCount"] == 0
                for attempt in m0_attempts
            ):
                matrix["historicalM0Classification"] = "NOT REPRODUCED"
            elif any(attempt["outcome"].startswith("exited_") for attempt in m0_attempts):
                matrix["historicalM0Classification"] = "NOT REPRODUCED; DIFFERENT STARTUP EXIT OBSERVED"
            else:
                matrix["historicalM0Classification"] = "INCOMPLETE"
            matrix["causalHypothesis"] = causal_hypothesis(
                matrix["modes"], matrix["historicalM0Classification"]
            )
            lldb_mode = next(
                (
                    case
                    for case in ("m0", "m1", "m2", "m3")
                    if any(attempt["outcome"] != "stable_survived_30s" for attempt in matrix["modes"][case]["attempts"])
                ),
                "m0",
            )
            matrix["lldbDiagnostic"] = run_lldb_diagnostic(
                lldb_mode, executable, expected_sha, attempts_root
            )
            if not matrix["lldbDiagnostic"].get("cleanup", {}).get("noCandidateProcessesRemain"):
                matrix["infrastructureErrors"].append("candidate_process_survived_lldb_cleanup")
            if (
                matrix["lldbDiagnostic"].get("debuggerLaunched")
                and not matrix["lldbDiagnostic"].get("cleanup", {}).get("debuggerStopped")
            ):
                matrix["infrastructureErrors"].append("lldb_debugger_cleanup_failed")
            if matrix["lldbDiagnostic"].get("cleanup", {}).get("driverStopped") is not True:
                matrix["infrastructureErrors"].append("lldb_driver_cleanup_failed")
            if matrix["lldbDiagnostic"].get("cleanup", {}).get("taskRemoved") is False:
                matrix["infrastructureErrors"].append("lldb_task_profile_cleanup_failed")
        except Exception as error:
            matrix["fatalFailure"] = repr(error)
            matrix["infrastructureErrors"].append("matrix_driver_exception")
    matrix["matrixStatus"] = "RUN COMPLETE" if not matrix["infrastructureErrors"] else "INCOMPLETE"
    matrix["finishedAt"] = now()
    matrix["historicalFailureConclusion"] = (
        "NOT REPRODUCED does not mean #270 is fixed; current source/dependency/runner identity must be compared with the historical candidate."
        if matrix.get("historicalM0Classification", "").startswith("NOT REPRODUCED")
        else None
    )
    (artifact_root / "matrix.json").write_text(
        json.dumps(matrix, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    (artifact_root / "matrix.md").write_text(markdown_summary(matrix), encoding="utf-8")
    return 0 if matrix["matrixStatus"] == "RUN COMPLETE" else 1


if __name__ == "__main__":
    sys.exit(main())
