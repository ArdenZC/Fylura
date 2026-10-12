#!/usr/bin/env python3
"""Static-only contract validator for the #380 S1R4-P1 Windows path harness."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
BRANCH = "research/issue-380-s1r4-p1-windows-path-guard-preparation"
P0 = "e5e9ab5bc84136c15056d72db8904f26b24e2a64"
P0_TREE = "96cdd08012021c5409c8dcdedcb623644a55e541"
S1R3 = "08bad6b19831d40dda9d296de880aaae6c83c312"
S1R3_TREE = "5360d8c81e5949333c8d6ef16acd0c658d315598"
S0 = "1f4951f3c93b2eb549d159fed2dd288992c625c8"
PROBE = "scripts/issue380S0NsisObservabilityProbe.nsi"
RUNNER = "scripts/issue380S1R4SyntheticNsisRunner.ps1"
PATH_GUARD = "scripts/issue380S1R4PathGuard.ps1"
WINDOWS_TEST = "scripts/testIssue380S1R4PathGuard.ps1"
VALIDATOR = "scripts/validateIssue380S1R4Preparation.py"
REPORT = "docs/research/ISSUE-380-S1R4-P1-WINDOWS-PATH-GUARD-PREPARATION.md"
MANIFEST = "docs/research/evidence/issue-380-s1r4-p1/SHA256SUMS"
ALLOWED = {RUNNER, PATH_GUARD, WINDOWS_TEST, VALIDATOR, REPORT, MANIFEST}

PINNED_STRINGS = (
    "https://sourceforge.net/projects/nsis/files/NSIS%203/3.11/nsis-3.11.zip/download",
    "c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1",
    "f497e92deb9f179f7b8f7553fcb3bd04f511bb2949e5e4a2aee80a10f7b20431",
    "42850802704ecb11163f7e0329d35ee54bd288953200d4966e226d572848cfc5",
    "$script:LauncherLength = 2560L",
    "$script:CompilerLength = 468992L",
    "$script:ExpectedInstallerExitCode = 37",
    "$script:ExpectedUninstallerExitCode = 42",
)

REQUIRED_CASES = (
    "taskroot_working_directory",
    "direct_child_working_directory",
    "nested_child_working_directory",
    "shared_prefix_sibling_working_directory",
    "taskroot_parent_working_directory",
    "other_runner_temp_working_directory",
    "taskroot_outside_runner_temp_rejected",
    "traversal_that_resolves_back_inside",
    "traversal_to_parent",
    "uppercase_path_variant",
    "forward_slash_path_variant",
    "uppercase_forward_slash_path_variant",
    "uppercase_taskroot_variant",
    "forward_slash_taskroot_variant",
    "trailing_separator_taskroot_variant",
    "existing_in_root_executable",
    "uppercase_executable_path_variant",
    "forward_slash_executable_path_variant",
    "taskroot_is_not_an_executable",
    "missing_executable",
    "directory_cannot_be_executable",
    "outside_taskroot_executable",
    "another_runner_temp_directory_is_not_executable",
    "unc_path_rejected",
    "unc_taskroot_rejected",
    "extended_device_path_rejected",
    "device_namespace_taskroot_rejected",
    "win32_device_path_rejected",
    "nt_device_path_rejected",
    "relative_candidate_rejected",
    "drive_relative_candidate_rejected",
    "alternate_data_stream_rejected",
    "junction_working_directory_rejected",
    "junction_descendant_rejected",
    "directory_symlink_working_directory_rejected",
    "file_symlink_executable_rejected",
    "taskroot_symlink_rejected",
    "taskroot_reparse_ancestor_rejected",
    "compiler_version_working_directory_is_taskroot",
    "compiler_hdrinfo_working_directory_is_taskroot",
)


def git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=ROOT, text=True, capture_output=True, check=False
    )
    if result.returncode:
        raise RuntimeError(
            f"git {' '.join(args)} failed ({result.returncode}): "
            f"{result.stdout}{result.stderr}"
        )
    return result.stdout.strip()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def changed_paths() -> set[str]:
    committed = set(git("diff", "--name-only", f"{P0}...HEAD").splitlines())
    status = git("status", "--porcelain", "--untracked-files=all")
    working = {line[3:] for line in status.splitlines() if len(line) >= 4}
    return committed | working


def check_runtime_result(path: pathlib.Path) -> None:
    require(path.is_file(), f"Windows runtime result file is missing: {path}")
    payload = json.loads(path.read_text(encoding="utf-8"))
    require(payload.get("status") == "WINDOWS_RUNTIME_PASS", "runtime status is not WINDOWS_RUNTIME_PASS")
    require(payload.get("is_windows") is True, "runtime evidence does not identify Windows")
    require(payload.get("process_launch_count") == 0, "path test launched a process")
    require(payload.get("nsis_launched") is False, "path test launched NSIS")
    require(payload.get("unrelated_executable_launched") is False, "path test launched an unrelated executable")
    cases = payload.get("cases")
    require(isinstance(cases, list) and cases, "runtime case list is empty")
    by_name = {case.get("name"): case for case in cases if isinstance(case, dict)}
    require(set(REQUIRED_CASES).issubset(by_name), "runtime evidence omits required cases")
    require(all(by_name[name].get("passed") is True for name in REQUIRED_CASES), "one or more required runtime cases failed")
    require(not payload.get("failures"), "runtime evidence records failures")
    cleanup = payload.get("cleanup") or {}
    require(cleanup.get("owned_scope_root_removed") is True, "owned scope root cleanup is not verified")
    require(cleanup.get("owned_external_fixture_removed") is True, "external fixture cleanup is not verified")
    require(cleanup.get("owned_links_removed") is True, "owned reparse-point fixture cleanup is not verified")


def check_sha256_manifest() -> None:
    manifest_path = ROOT / MANIFEST
    require(manifest_path.is_file(), f"SHA256 manifest is missing: {manifest_path}")
    entries: dict[str, str] = {}
    for line in manifest_path.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        require(match is not None, f"malformed SHA256 manifest line: {line!r}")
        digest, relative = match.groups()
        require(relative not in entries, f"duplicate SHA256 manifest path: {relative}")
        entries[relative] = digest
    expected = ALLOWED - {MANIFEST}
    require(set(entries) == expected, f"manifest paths differ: expected={sorted(expected)} actual={sorted(entries)}")
    for relative, expected_hash in entries.items():
        file_path = ROOT / relative
        require(file_path.is_file(), f"manifest target is missing: {relative}")
        actual_hash = hashlib.sha256(file_path.read_bytes()).hexdigest()
        require(actual_hash == expected_hash, f"SHA256 mismatch for {relative}: {actual_hash}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--windows-runtime-results",
        type=pathlib.Path,
        help="optional JSON emitted by testIssue380S1R4PathGuard.ps1 on Windows",
    )
    args = parser.parse_args()

    branch = git("branch", "--show-current")
    head = git("rev-parse", "HEAD")
    tree = git("rev-parse", "HEAD^{tree}")
    require(branch == BRANCH, f"unexpected branch: {branch}")
    require(git("rev-parse", f"{P0}^{{tree}}") == P0_TREE, "frozen S1R4-P0 Tree mismatch")
    require(git("rev-parse", f"{S1R3}^{{tree}}") == S1R3_TREE, "frozen S1R3 Tree mismatch")
    git("merge-base", "--is-ancestor", P0, "HEAD")
    git("merge-base", "--is-ancestor", S1R3, "HEAD")

    actual_probe = git("hash-object", PROBE)
    frozen_probe = git("rev-parse", f"{S0}:{PROBE}")
    require(actual_probe == frozen_probe, f"frozen S0 Probe blob changed: {actual_probe}")

    paths = changed_paths()
    require(paths <= ALLOWED, f"out-of-scope paths changed: {sorted(paths - ALLOWED)}")
    require({RUNNER, PATH_GUARD, WINDOWS_TEST, VALIDATOR, REPORT, MANIFEST}.issubset(paths),
            "research deliverables are incomplete")
    require(not any(path.startswith(".github/workflows/") for path in paths), "Workflow changes are forbidden in P1")

    runner = (ROOT / RUNNER).read_text(encoding="utf-8")
    guard = (ROOT / PATH_GUARD).read_text(encoding="utf-8")
    windows_test = (ROOT / WINDOWS_TEST).read_text(encoding="utf-8")
    report = (ROOT / REPORT).read_text(encoding="utf-8")
    check_sha256_manifest()

    require("issue380S1R4PathGuard.ps1" in runner, "Runner does not import the shared path guard")
    process_function = runner.split("function Invoke-CapturedProcess", 1)[1].split("\nfunction ", 1)[0]
    require("Resolve-Issue380S1R4Path" in process_function, "actual process launch does not use the shared path guard")
    require("-Kind Executable" in process_function and "-Kind WorkingDirectory" in process_function,
            "process launch does not validate both image and working directory")
    snapshot_function = runner.split("function Get-TaskOwnedProcessSnapshot", 1)[1].split("\nfunction ", 1)[0]
    require("Resolve-Issue380S1R4Path" in snapshot_function, "process cleanup identity does not use the same path guard")
    require("$taskRootPrefix" not in process_function and "StartsWith($taskRootPrefix" not in process_function,
            "the old string-prefix process guard remains")
    require("OrdinalIgnoreCase" in guard and "StartsWith($rootPrefix" in guard,
            "guard lacks case-insensitive, separator-bounded containment")
    require("IsPathFullyQualified" in guard and "GetFullPath" in guard, "guard lacks absolute path canonicalization")
    require("TaskRoot is outside RUNNER_TEMP" in guard, "guard does not bind TaskRoot to the task temp area")
    require("UNC syntax" in guard and "device namespace" in guard, "UNC/device path policy is not explicit")
    require("DriveType]::Network" in guard, "mapped network drives are not rejected")
    require("ReparsePoint" in guard, "guard does not reject reparse points")
    require("$Kind -eq 'WorkingDirectory'" in guard and "$Kind -eq 'Executable'" in guard,
            "guard does not distinguish working directories and executables")
    require("$isExactRoot" in guard and "$isDescendant" in guard, "working-directory root equality is missing")
    require("GetAttributes" in guard, "guard does not inspect filesystem objects")
    require("$runnerTempFullPath = Resolve-Issue380S1R4Path" in runner,
            "Runner does not validate RUNNER_TEMP before creating evidence or task roots")
    require("-CandidatePath $outputFullPath -Kind ExistingDirectory" in runner,
            "Runner does not validate the evidence directory before enumeration/writes")
    require("-CandidatePath $taskRoot -Kind ExistingDirectory" in runner,
            "Runner does not validate the owned task root after creation")
    require("-Kind ExistingFile" in runner, "evidence copy sources are not checked as in-root files")
    require("s1r4-task-owned.txt" in runner and "all_task_owned_processes_exited" in runner,
            "task-root ownership/process cleanup contract is incomplete")
    require("issue380S1R4PathGuard.ps1" in windows_test, "Windows tests do not import the Runner's path guard")
    require("$IsWindows" in windows_test, "Windows test does not reject non-Windows runtime")
    require("#requires -Version 7.0" in windows_test, "Windows test does not pin the PowerShell runtime contract")
    require("process_launch_count = 0" in windows_test and "nsis_launched = $false" in windows_test,
            "Windows path test does not report its no-process contract")
    require("s1r4-path-test-owned.txt" in windows_test and "s1r4-path-test-external-owned.txt" in windows_test,
            "Windows fixtures lack separate ownership markers")
    require("Remove-OwnedLink" in windows_test and "Remove-Item -LiteralPath $scopeRoot -Recurse" in windows_test and
            "Remove-Item -LiteralPath $externalTemp -Recurse" in windows_test,
            "Windows cleanup is not restricted to the marked test fixture roots")
    for forbidden in (
        "Start-Process",
        "[System.Diagnostics.Process]",
        ".Start()",
        "Invoke-CapturedProcess",
        "Invoke-Expression",
        "Start-Job",
        "HttpClient",
        "makensis.exe",
    ):
        require(forbidden not in windows_test, f"Windows path test can launch code/processes: {forbidden}")
    for case in REQUIRED_CASES:
        require(re.search(rf"Add-PathCase\s+'{re.escape(case)}'", windows_test) is not None,
                f"Windows path test lacks required case: {case}")

    for value in PINNED_STRINGS:
        require(value in runner, f"Runner changed frozen NSIS/tool/exit-code value: {value}")
    require("ProcessStartInfo]::new()" in runner and "$startInfo.ArgumentList.Add($argument)" in runner,
            "ProcessStartInfo.ArgumentList contract changed")
    require('$uninstallerArguments.Add("_?=$absoluteInstallDirectory")' in runner,
            "uninstaller _?= argument is missing")
    require('$uninstallerArguments[$uninstallerArguments.Count - 1].Equals("_?=$absoluteInstallDirectory"' in runner,
            "uninstaller _?= last-argument assertion is missing")
    require("foreach ($traceMode in @('file', 'detail'))" in runner,
            "file-mode-first assertion sequence changed")
    require("SetErrorLevel 37" in (ROOT / PROBE).read_text(encoding="utf-8") and
            "SetErrorLevel 42" in (ROOT / PROBE).read_text(encoding="utf-8"),
            "frozen Probe exit levels changed")
    require("WINDOWS_RUNTIME_PASS" in windows_test and "STATIC_PREPARED" in report,
            "static and runtime status labels are not separately recorded")
    require("Windows runtime status: **NOT RUN**" in report,
            "report must label the unexecuted Windows runtime status explicitly")
    require("NOT RUN" in report and "Hosted" in report, "report lacks unexecuted-test/Hosted boundary")

    for diff_args in (
        ["git", "diff", "--check", P0, "HEAD"],
        ["git", "diff", "--cached", "--check"],
        ["git", "diff", "--check"],
    ):
        diff_check = subprocess.run(diff_args, cwd=ROOT, text=True, capture_output=True, check=False)
        require(diff_check.returncode == 0, f"{' '.join(diff_args)} failed: {diff_check.stdout}{diff_check.stderr}")

    if args.windows_runtime_results is None:
        status = "STATIC_PREPARED"
        windows_runtime = "NOT_RUN"
    else:
        check_runtime_result(args.windows_runtime_results)
        status = "WINDOWS_RUNTIME_PASS"
        windows_runtime = "PASS_VERIFIED_FROM_WINDOWS_RESULT_JSON"

    print(f"status={status}")
    print(f"windows_runtime={windows_runtime}")
    print(f"branch={branch}")
    print(f"head={head}")
    print(f"tree={tree}")
    print(f"frozen_s1r4_p0={P0} tree={P0_TREE}")
    print(f"frozen_s1r3={S1R3} tree={S1R3_TREE}")
    print(f"probe_blob={actual_probe}")
    print(f"changed_paths={','.join(sorted(paths))}")
    print("workflow_changes=NONE; hosted_jobs=NOT_STARTED")
    print("process_launches=NONE; NSIS=NOT_RUN")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # Static preparation must fail closed.
        print(f"S1R4-P1 validation failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
