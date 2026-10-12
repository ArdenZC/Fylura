#!/usr/bin/env python3
"""Fail-closed offline source/ZIP preflight for the #380 S1R3 one-shot probe."""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import pathlib
import stat
import subprocess
import sys
import zipfile

import yaml


ROOT = pathlib.Path(__file__).resolve().parents[1]
S1R2 = "6fe01abae7640a93f4a694616d758fd54c714960"
S1R2_TREE = "af88309c65999d73564cf3876bb5bd03a15bf597"
S1R = "45a2dfe0e51cee5708dfaa5335affbad947c30c2"
S1R_TREE = "9bcc5a2a03a8335d2cb2e81c3d497efb275b8724"
S1 = "bd56788aa317d808dadf930260e191e9d1b45adb"
S0 = "1f4951f3c93b2eb549d159fed2dd288992c625c8"
BRANCH = "research/issue-380-s1r3-nsis-uninstaller-trace-verification"
WORKFLOW = ".github/workflows/issue-380-s1r3-synthetic-nsis-uninstaller-trace.yml"
RUNNER = "scripts/issue380S1R3SyntheticNsisRunner.ps1"
VALIDATOR = "scripts/validateIssue380S1R3Harness.py"
PROBE = "scripts/issue380S0NsisObservabilityProbe.nsi"
REPORT = "docs/research/ISSUE-380-S1R3-SYNTHETIC-TRACE-PERSISTENCE.md"
EVIDENCE_ROOT = "docs/research/evidence/issue-380-s1r3"
BASE_ALLOWED = {WORKFLOW, RUNNER, VALIDATOR}
TRIGGER_PATHS = {WORKFLOW, RUNNER, VALIDATOR, PROBE}
ZIP_URL = "https://sourceforge.net/projects/nsis/files/NSIS%203/3.11/nsis-3.11.zip/download"
ZIP_SHA256 = "c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1"
MEMBERS = {
    "nsis-3.11/makensis.exe": (
        2560,
        "f497e92deb9f179f7b8f7553fcb3bd04f511bb2949e5e4a2aee80a10f7b20431",
    ),
    "nsis-3.11/Bin/makensis.exe": (
        468992,
        "42850802704ecb11163f7e0329d35ee54bd288953200d4966e226d572848cfc5",
    ),
}


def git(*args: str) -> str:
    completed = subprocess.run(
        ["git", *args], cwd=ROOT, text=True, capture_output=True, check=False
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"git {' '.join(args)} failed ({completed.returncode}): "
            f"{completed.stdout}{completed.stderr}"
        )
    return completed.stdout.strip()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def worktree_paths() -> set[str]:
    lines = git("status", "--porcelain", "--untracked-files=all").splitlines()
    return {line[3:] for line in lines if len(line) >= 4}


def validate_pinned_zip(path: pathlib.Path) -> tuple[int, int]:
    require(path.is_file(), f"pinned NSIS ZIP not found: {path}")
    whole_hash = hashlib.sha256(path.read_bytes()).hexdigest()
    require(whole_hash == ZIP_SHA256, f"NSIS ZIP SHA mismatch: {whole_hash}")

    with zipfile.ZipFile(path) as archive:
        entries = archive.infolist()
        require(len(entries) == 441, f"unexpected NSIS ZIP entry count: {len(entries)}")
        names_seen: set[str] = set()
        makensis_members: set[str] = set()
        uncompressed = 0
        matched: dict[str, tuple[int, str]] = {}
        for entry in entries:
            original = entry.filename
            normalized = original.replace("\\", "/")
            directory = normalized.endswith("/")
            relative = normalized.rstrip("/")
            parts = relative.split("/")
            require(
                relative
                and not relative.startswith("/")
                and ":" not in relative
                and not any(part in ("", ".", "..") for part in parts),
                f"unsafe archive path: {original!r}",
            )
            require(
                relative == "nsis-3.11" or relative.startswith("nsis-3.11/"),
                f"archive member escaped nsis-3.11 root: {original!r}",
            )
            folded = relative.casefold()
            require(folded not in names_seen, f"duplicate/case-colliding entry: {relative}")
            names_seen.add(folded)
            mode = entry.external_attr >> 16
            require(not stat.S_ISLNK(mode), f"symlink archive entry: {relative}")
            require(not directory or entry.file_size == 0, f"directory has payload: {relative}")
            uncompressed += entry.file_size
            require(uncompressed <= 512 * 1024 * 1024, "archive expands beyond 512 MiB")

            if relative.casefold().endswith("makensis.exe"):
                makensis_members.add(relative)
            if relative in MEMBERS:
                expected_size, expected_hash = MEMBERS[relative]
                payload = archive.read(entry)
                actual_hash = hashlib.sha256(payload).hexdigest()
                require(
                    len(payload) == expected_size == entry.file_size,
                    f"member length mismatch: {relative}: {len(payload)}",
                )
                require(
                    actual_hash == expected_hash,
                    f"member SHA mismatch: {relative}: {actual_hash}",
                )
                matched[relative] = (len(payload), actual_hash)

        require(makensis_members == set(MEMBERS), f"unexpected makensis member set: {sorted(makensis_members)}")
        require(set(matched) == set(MEMBERS), "one or more pinned NSIS executable members are missing")
    return len(entries), uncompressed


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--zip-path", required=True, type=pathlib.Path)
    args = parser.parse_args()

    head = git("rev-parse", "HEAD")
    branch = git("branch", "--show-current")
    require(branch == BRANCH, f"expected branch {BRANCH}, got {branch}")
    require(git("rev-parse", f"{S1R2}^{{tree}}") == S1R2_TREE, "frozen S1R2 Tree mismatch")
    git("merge-base", "--is-ancestor", S1R2, "HEAD")
    git("merge-base", "--is-ancestor", S1R, "HEAD")
    git("merge-base", "--is-ancestor", S1, "HEAD")
    git("merge-base", "--is-ancestor", S0, "HEAD")
    require(git("rev-parse", f"{S0}:{PROBE}") == git("hash-object", PROBE), "frozen S0 Probe blob changed")

    changed = set(git("diff", "--name-only", f"{S1R2}...HEAD").splitlines())
    changed |= worktree_paths()
    allowed = set(BASE_ALLOWED)
    allowed.add(PROBE)
    allowed.add(REPORT)
    if (ROOT / EVIDENCE_ROOT).exists():
        allowed |= {
            path.relative_to(ROOT).as_posix()
            for path in (ROOT / EVIDENCE_ROOT).rglob("*")
            if path.is_file()
        }
    unexpected = changed - allowed
    require(not unexpected, f"out-of-scope paths: {sorted(unexpected)}")
    require(
        git("rev-parse", f"{S0}:{PROBE}") == git("hash-object", PROBE),
        "Probe must remain byte-identical to frozen S0",
    )

    workflow_text = (ROOT / WORKFLOW).read_text(encoding="utf-8")
    workflow = yaml.load(workflow_text, Loader=yaml.BaseLoader)
    require(isinstance(workflow, dict), "workflow YAML did not parse as a mapping")
    require(set(workflow.get("on", {})) == {"push"}, "push must be the only workflow trigger")
    push = workflow["on"]["push"]
    require(push.get("branches") == [BRANCH], "push trigger must target only the S1R3 branch")
    require(set(push.get("paths", [])) == TRIGGER_PATHS, "workflow path filter is broader or incomplete")
    require(workflow.get("permissions") == {"contents": "read"}, "workflow permission must be contents:read only")
    require(len(workflow.get("jobs", {})) == 1, "workflow must contain exactly one job")
    job = next(iter(workflow["jobs"].values()))
    require(job.get("runs-on") == "windows-2022", "only the bounded windows-2022 runner is allowed")
    require(job.get("timeout-minutes") == "15", "job must have a 15-minute timeout")
    steps = job.get("steps", [])
    require(len(steps) == 8, "workflow must contain exactly eight narrow steps")
    step_names = [step.get("name", "") for step in steps]
    probe_step = next(step for step in steps if step.get("id") == "probe")
    require(probe_step.get("timeout-minutes") == "13", "probe step timeout must be 13 minutes")
    for name in (
        "Static identity and tool pin preflight",
        "Run one-shot synthetic NSIS probe",
        "Clean synthetic Registry and task temp; retain cleanup evidence",
        "Upload raw synthetic probe evidence",
        "Enforce single-run result",
    ):
        require(name in step_names, f"workflow lacks required step: {name}")
    require("workflow_dispatch" not in workflow_text and "pull_request" not in workflow_text, "unauthorized trigger present")
    require("GITHUB_RUN_ATTEMPT" in workflow_text and "ne '1'" in workflow_text, "workflow lacks one-attempt guard")
    require("System.Management.Automation.Language.Parser]::ParseFile" in workflow_text,
            "Hosted preflight must run the Windows PowerShell parser before the NSIS probe")
    require("continue-on-error: true" in workflow_text, "probe failure evidence would not reach cleanup/upload")
    require("pre_cleanup_process_check_error" in workflow_text and "pre_cleanup_owned_processes" in workflow_text,
            "workflow must record the process gate before Registry cleanup")
    require(workflow_text.index("$taskProcessesCleanBeforeRegistryCleanup") < workflow_text.index("$base.DeleteSubKeyTree"),
            "synthetic Registry cleanup must be gated on task-owned process exit")
    require("OWNED_PROCESSES_REMAIN" in workflow_text and "OWNED_PROCESS_CHECK_FAILED" in workflow_text,
            "workflow must fail closed and preserve temp files if task processes remain or cannot be checked")
    forbidden_workflow = ("npm ", "cargo ", "tauri ", "benchmark", "matrix:", "Build Release", "Chocolatey/setup")
    require(not any(token.lower() in workflow_text.lower() for token in forbidden_workflow), "workflow contains forbidden build/package/benchmark path")
    require("actions/checkout@v4" in workflow_text and "actions/upload-artifact@v4" in workflow_text, "required source/evidence actions missing")

    matching_push_workflows: list[str] = []
    for candidate in sorted((ROOT / ".github/workflows").glob("*.yml")):
        candidate_data = yaml.load(candidate.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
        candidate_push = candidate_data.get("on", {}).get("push") if isinstance(candidate_data, dict) else None
        if candidate_push is None:
            continue
        if isinstance(candidate_push, str):
            candidate_push = {}
        candidate_branches = candidate_push.get("branches")
        if candidate_branches is None:
            branch_matches = True
        else:
            branch_matches = any(fnmatch.fnmatchcase(BRANCH, pattern) for pattern in candidate_branches)
        candidate_paths = candidate_push.get("paths")
        if candidate_paths is None:
            path_matches = True
        else:
            path_matches = any(
                fnmatch.fnmatchcase(path, pattern)
                for path in changed
                for pattern in candidate_paths
            )
        if branch_matches and path_matches:
            matching_push_workflows.append(candidate.relative_to(ROOT).as_posix())
    require(matching_push_workflows == [WORKFLOW],
            f"S1R3 push must trigger exactly its narrow workflow, got: {matching_push_workflows}")

    runner_text = (ROOT / RUNNER).read_text(encoding="utf-8")
    require(ZIP_URL in runner_text and ZIP_SHA256 in runner_text, "runner lacks exact official URL/SHA pin")
    require(S1R2 in runner_text and S1R2_TREE in runner_text, "runner lacks frozen S1R2 HEAD/Tree")
    require(S1R in runner_text and S1R_TREE in runner_text, "runner lacks frozen S1R HEAD/Tree")
    for member, (length, digest) in MEMBERS.items():
        require(member in runner_text and digest in runner_text, f"runner lacks pinned member identity: {member}")
        length_name = "LauncherLength" if "Bin/" not in member else "CompilerLength"
        require(f"$script:{length_name} = {length}L" in runner_text, f"runner lacks pinned member length: {member}")
    require("sha256_verified_before_extract" in runner_text and "Expand-VerifiedZipSafely" in runner_text, "archive hash must guard inspection/extraction")
    require("Get-ZipEntrySha256" in runner_text and "EXTRACTED_FILE_PRE_EXECUTION" in runner_text, "pre-extract and post-extract hashes are required")
    require("Get-ChildItem -LiteralPath $extractionRoot -Filter 'makensis.exe' -File -Recurse" not in runner_text, "runner still uses recursive compiler discovery")
    require("Get-Command makensis" not in runner_text and "choco" not in runner_text.lower(), "ambient compiler/Chocolatey fallback forbidden")
    require("Invoke-CapturedProcess -FilePath $binCompiler" not in runner_text, "underlying compiler must not be run as a fallback")
    require("Invoke-CapturedProcess -FilePath $makensis" in runner_text, "only the approved top-level launcher should be invoked")
    require("ProcessStartInfo]::new()" in runner_text and "$startInfo.ArgumentList.Add($argument)" in runner_text,
            "process invocation must use ProcessStartInfo.ArgumentList")
    require("$script:ExpectedInstallerExitCode = 37" in runner_text and "$script:ExpectedUninstallerExitCode = 42" in runner_text,
            "S1R3 must keep installer 37 and uninstaller 42 exit expectations")
    require("foreach ($traceMode in @('file', 'detail'))" in runner_text,
            "TRACE_MODE=file must run before optional DetailPrint and /LOG comparison")
    require('if ($traceMode -eq \'detail\') { $installerArguments += "/LOG=$installerLog" }' in runner_text,
            "primary file-mode installer run must not depend on /LOG")
    require('if ($traceMode -eq \'detail\') { $uninstallerArguments.Add("/LOG=$uninstallerLog") }' in runner_text,
            "primary file-mode uninstaller run must not depend on /LOG")
    require('$uninstallerArguments.Add("_?=$absoluteInstallDirectory")' in runner_text,
            "uninstaller must receive _?=<controlled absolute directory>")
    require('$uninstallerArguments[$uninstallerArguments.Count - 1].Equals("_?=$absoluteInstallDirectory"' in runner_text,
            "_?= must be asserted as the final uninstaller argument")
    require("ArgumentList @($uninstallerArguments.ToArray())" in runner_text,
            "uninstaller must be launched through the explicit argument list")
    require("Get-TaskOwnedProcessSnapshot" in runner_text and "Get-CimInstance -ClassName Win32_Process -Property @('ProcessId', 'ExecutablePath')" in runner_text,
            "task-owned process exit verification is missing")
    require("$startInfo.Environment['TEMP'] = $workingDirectoryFullPath" in runner_text and "$startInfo.Environment['TMP'] = $workingDirectoryFullPath" in runner_text,
            "NSIS child temp paths must remain under the controlled task directory")
    require("task_owned_processes_exited_after_pid_" in runner_text and "all_task_owned_processes_exited" in runner_text,
            "process evidence must preserve PID/path/exit/timeout and prove owned processes exited")
    forbidden_runner = (
        "HKLM", "Program Files\\Zen Canvas", "ServiceController", "sc.exe",
        "Start-Process", "cargo test", "npm run", "tauri build", "src-tauri",
    )
    require(not any(token.lower() in runner_text.lower() for token in forbidden_runner), "runner contains a product, Service, or application build path")
    for token in (
        "HKCU", "RegistryHive]::CurrentUser", "RegistryView]::Registry64",
        "-VERSION", "-HDRINFO", "NSIS_CPU=x86", "NSIS_UNICODE",
        "FileOpen", "FileWrite", "FileClose", "direct_file_trace_exists_after_both_abort_processes_exit",
        'file_trace_' + chr(36) + '{context}_record_complete', "@('installer', 'uninstaller')",
        "expected=35150", "actual=35149", "result=FAIL", "installer_abort_expected_exit",
        "uninstaller_abort_expected_exit", "SHA256SUMS", "nsis-3.11-download.zip",
    ):
        require(token.lower() in runner_text.lower() or token.lower() in (ROOT / PROBE).read_text(encoding="utf-8").lower(),
                f"runner/probe lacks required evidence contract: {token}")

    probe = (ROOT / PROBE).read_text(encoding="utf-8")
    require("RequestExecutionLevel user" in probe, "synthetic probe must not request elevation")
    require("HKLM" not in probe, "synthetic probe may not touch machine Registry")
    require("Software\\ZenCanvasS0Harness" in probe, "synthetic HKCU key is missing")
    require("SetErrorLevel 37" in probe and 'Abort "S0 synthetic Registry DWORD mismatch"' in probe,
            "frozen installer Abort error level 37 contract changed")
    require("SetErrorLevel 42" in probe and 'Abort "S0 synthetic uninstaller Registry DWORD mismatch"' in probe,
            "frozen uninstaller Abort error level 42 contract changed")

    zip_entries, zip_uncompressed = validate_pinned_zip(args.zip_path)
    diff = subprocess.run(["git", "diff", "--check", S1R2], cwd=ROOT, text=True, capture_output=True)
    require(diff.returncode == 0, f"git diff --check failed: {diff.stdout}{diff.stderr}")

    print("S1R3 offline preflight: PASS")
    print(f"branch={branch}")
    print(f"frozen_s1r2={S1R2} tree={S1R2_TREE}")
    print(f"frozen_s1r={S1R} tree={S1R_TREE}")
    print(f"head={head}")
    print(f"changed_paths={','.join(sorted(changed))}")
    print("workflow=push-only, exact branch/path filters, contents:read, one windows-2022 job, no retry")
    print(f"matching_push_workflows={matching_push_workflows}")
    print(f"nsis_zip_path={args.zip_path}")
    print(f"nsis_zip_sha256={ZIP_SHA256} entries={zip_entries} uncompressed_bytes={zip_uncompressed}")
    for member, (length, digest) in MEMBERS.items():
        print(f"member={member} length={length} sha256={digest}")
    print("registry_scope=synthetic HKCU 64-bit view only")
    print("production installer/service/database/build paths=absent")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # fail-closed preflight summary.
        print(f"S1R3 offline preflight: FAIL: {exc}", file=sys.stderr)
        raise SystemExit(1)
