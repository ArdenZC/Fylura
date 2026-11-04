#!/usr/bin/env bash
set -euo pipefail
# No existing disks or user directories: use one task-owned directory on the
# runner's internal APFS volume, where hosted Spotlight can index scoped data.
task_root="${RUNNER_TEMP:?}/zen-smart-lean-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
source_root="$task_root/source"
phase_dir="$task_root/phases"
if test -e "$task_root"; then echo "Refusing existing task root: $task_root" >&2; exit 1; fi
mkdir -p "$source_root" "$phase_dir"
printf '%s' "$source_root" > "$task_root/owned-source-root.txt"
export ZEN_SMART_LEAN_TASK_ROOT="$task_root"
export ZEN_SMART_LEAN_VOLUME_ROOT="$source_root"
export ZEN_SMART_LEAN_NATIVE_REPORT="$PWD/smart-lean-native-macos.json"
export ZEN_SMART_LEAN_NATIVE_TRACE=1
export ZEN_SMART_LEAN_NATIVE_PHASE_DIR="$phase_dir"
# Compile before starting the bounded native run. On a stalled native call,
# sample only this task's test child, then fail rather than hanging the runner.
cargo build --locked --manifest-path src-tauri/Cargo.toml --features "desktop-runtime native-qa" --bin smart-lean-macos-qa
qa_binary="${CARGO_TARGET_DIR:-src-tauri/target}/debug/smart-lean-macos-qa"
python3 - "$qa_binary" "$PWD" <<'PYTHON'
import json, os, pathlib, shutil, subprocess, sys
root = pathlib.Path(sys.argv[2])
phase_dir = pathlib.Path(__import__("os").environ["ZEN_SMART_LEAN_NATIVE_PHASE_DIR"])
artifact_phase_dir = root / "smart-lean-native-phase"
task_home = pathlib.Path(os.environ["ZEN_SMART_LEAN_VOLUME_ROOT"]) / "smart-lean-native-home"
task_tmp = pathlib.Path(os.environ["ZEN_SMART_LEAN_VOLUME_ROOT"]) / "smart-lean-native-tmp"
task_home.mkdir(parents=True, exist_ok=True)
task_tmp.mkdir(parents=True, exist_ok=True)
os.environ["HOME"] = str(task_home)
os.environ["TMPDIR"] = str(task_tmp)

def read_phases():
    phases = {}
    for path in phase_dir.glob("*.txt"):
        try:
            phases[path.stem] = path.read_text().strip()
        except OSError as error:
            phases[path.stem] = f"phase_file_read_error={error}"
    if phases:
        artifact_phase_dir.mkdir(parents=True, exist_ok=True)
        for path in phase_dir.glob("*.txt"):
            shutil.copy2(path, artifact_phase_dir / path.name)
    return phases

def active_hosted_spotlight_phase(phases):
    for line in phases.values():
        fields = dict(part.split("=", 1) for part in line.split() if "=" in part)
        phase = fields.get("phase", "")
        event = fields.get("event", "")
        if (
            event not in {"end", "unavailable"}
            and phase.startswith("spotlight.")
            and not phase.endswith(".run_loop")
        ):
            return f"{phase}:{event}"
    return None

process = subprocess.Popen([sys.argv[1]])
try:
    result = process.wait(timeout=120)
except subprocess.TimeoutExpired:
    # The PID comes directly from our own child, never from a process-name scan.
    try:
        subprocess.run(["sample", str(process.pid), "3", "-file",
                        str(root / "smart-lean-native-macos-sample.txt")], timeout=15)
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"Could not sample native QA child: {error}", file=sys.stderr)
    process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            print("Native QA child did not reap after SIGKILL", file=sys.stderr)
    phases = read_phases()
    report_path = root / "smart-lean-native-macos.json"
    try:
        report = json.loads(report_path.read_text()) if report_path.exists() else {}
    except (OSError, json.JSONDecodeError):
        report = {}
    spotlight_phase = active_hosted_spotlight_phase(phases)
    report.update(
        status="UNVERIFIED" if spotlight_phase else "FAIL",
        error="native_qa_controlled_process_timeout",
        blockingPhase=spotlight_phase or phases,
        phaseByThread=phases,
    )
    report_path.write_text(json.dumps(report, indent=2))
    print(f"SMART_LEAN_NATIVE status={report['status']} blockingPhase={report['blockingPhase']}")
    sys.exit(1)
phases = read_phases()
report_path = root / "smart-lean-native-macos.json"
try:
    report = json.loads(report_path.read_text()) if report_path.exists() else {}
except (OSError, json.JSONDecodeError):
    report = {}
report["phaseByThread"] = phases
if phases:
    report_path.write_text(json.dumps(report, indent=2))
if result == 0 and report.get("status") == "UNVERIFIED":
    print(f"SMART_LEAN_NATIVE status=UNVERIFIED capabilityGap={report.get('capabilityGap', 'native source not verified')}")
    sys.exit(1)
sys.exit(result)
PYTHON
