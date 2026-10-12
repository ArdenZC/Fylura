#!/usr/bin/env python3
"""Static-only P3-P0R activation safety validator; never runs PowerShell or NSIS."""

from __future__ import annotations

import hashlib
import fnmatch
import pathlib
import re
import subprocess
import sys

import yaml
from tree_sitter import Language, Parser
import tree_sitter_powershell


ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = "0aa8be6d58fc0994b3be6aec2af8dc02318fcefb"
BASE_TREE = "688bd53c3c44040f3ab86fe4cf71a7a3aa879274"
BRANCH = "research/issue-380-s1r4-p3-p0r-activation-safety-docs"
P3_BRANCH = "research/issue-380-s1r4-p3-p0-nsis-hosted-plan"
P2 = "86c21c14dd65bd2bbdb3056513489e0d4139bb47"
P2_TREE = "883a851cf8e75b4f39a4a46baea15ab109feb312"
P1 = "7a6fa69e5521d9759702c1923a9ade60a75819c3"
P1_TREE = "c5465ef93eae268e2c683876d959db90fac5da08"
S0 = "1f4951f3c93b2eb549d159fed2dd288992c625c8"
PROBE = "scripts/issue380S0NsisObservabilityProbe.nsi"
PROBE_BLOB = "5805f8d04cd37420a63f3e156c3b6fd5b4012f33"
P3_RUNNER = "scripts/issue380S1R4P3SyntheticNsisRunner.ps1"
P3_RUNNER_BLOB = "1d11c3add3c0d1f45e7d7ac082d09c616cca51a1"
P3_RUNNER_SHA256 = "c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd"
P1_GUARD = "scripts/issue380S1R4PathGuard.ps1"
P1_TEST = "scripts/testIssue380S1R4PathGuard.ps1"
REPORT = "docs/research/ISSUE-380-S1R4-P3-P0R-ACTIVATION-SAFETY.md"
VALIDATOR = "scripts/validateIssue380S1R4P3P0RDocs.py"
MANIFEST = "docs/research/evidence/issue-380-s1r4-p3-p0r/SHA256SUMS"
ACTIVATION_FILE = ".github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml"
ALLOWED = {REPORT, VALIDATOR, MANIFEST}


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


def parse_workflow(report: str) -> dict:
    fence = chr(96) * 3
    match = re.search(
        re.escape(fence + "yaml\n") + r"(.*?)\n" + re.escape(fence),
        report,
        re.DOTALL,
    )
    require(match is not None, "P3-P0R report lacks the complete Workflow draft")
    workflow = yaml.load(match.group(1), Loader=yaml.BaseLoader)
    require(isinstance(workflow, dict), "Workflow draft is not a YAML mapping")
    require("workflow_dispatch" not in str(workflow), "Draft must not be manually activatable")
    trigger = workflow.get("on", {}).get("push", {})
    require(trigger.get("branches") == [P3_BRANCH], "Draft must target the original P3-P0 branch")
    require(trigger.get("paths") == [ACTIVATION_FILE], "Draft trigger path is broader than its file")
    require(workflow.get("permissions") == {"contents": "read"}, "Workflow permission is not read-only")
    jobs = workflow.get("jobs")
    require(isinstance(jobs, dict) and list(jobs) == ["synthetic-nsis"], "Expected one isolated Job")
    job = jobs["synthetic-nsis"]
    require(job.get("runs-on") == "windows-2022", "Draft must use windows-2022")
    require(job.get("timeout-minutes") == "15", "Draft job deadline changed")
    steps = job.get("steps")
    require(isinstance(steps, list), "Draft Job has no steps")
    ids = [step.get("id") for step in steps if isinstance(step, dict)]
    require(
        {"bootstrap", "preflight", "synthetic_nsis", "finalize", "upload_eligibility"}.issubset(ids),
        "Required fail-closed Workflow steps are missing",
    )
    by_id = {step.get("id"): step for step in steps if step.get("id")}
    require(ids.index("preflight") < ids.index("synthetic_nsis"), "Identity gate is after Runner")

    preflight = str(by_id["preflight"].get("run", ""))
    for token in (
        "rev-list --parents -n 1",
        "diff-tree --no-commit-id --no-renames --name-status -r",
        "activationDiff.Count -ne 1",
        "1d11c3add3c0d1f45e7d7ac082d09c616cca51a1",
        P3_RUNNER_SHA256,
        "hash-object -- $p3RunnerFile",
        "86c21c14dd65bd2bbdb3056513489e0d4139bb47",
        "883a851cf8e75b4f39a4a46baea15ab109feb312",
        "7a6fa69e5521d9759702c1923a9ade60a75819c3",
        "c5465ef93eae268e2c683876d959db90fac5da08",
        "issue380S0NsisObservabilityProbe.nsi",
        "GITHUB_RUN_ATTEMPT",
    ):
        require(token in preflight, f"Preflight is missing frozen admission gate: {token}")

    runner = str(by_id["synthetic_nsis"].get("run", ""))
    require(
        runner.index("P3 Runner identity changed immediately before process start")
        < runner.index("$process.Start()"),
        "Runner identity is not rechecked immediately before Process.Start",
    )
    for token in (
        "p3_runner_git_blob_sha",
        "p3_runner_file_sha256",
        "p3_runner_identity_rechecked_before_start",
        "Start-Job",
        "Win32_Process",
        "ParentProcessId",
        "ExecutablePath",
        "CommandLine",
        "process-observer.json",
        "ArgumentList",
        "process_observer_status -ne 'COMPLETED'",
    ):
        require(token in runner, f"Runner wrapper lacks process evidence contract: {token}")
    frozen_runner = (ROOT / P3_RUNNER).read_text(encoding="utf-8")
    require(
        '$uninstallerArguments[$uninstallerArguments.Count - 1].Equals("_?=$absoluteInstallDirectory"' in frozen_runner,
        "Frozen Runner no longer records the controlled _?= argument last",
    )

    bootstrap = str(by_id["bootstrap"].get("run", ""))
    require("bootstrap_owner_token" in bootstrap, "Bootstrap owner token is missing")
    eligibility = by_id["upload_eligibility"]
    require(eligibility.get("if") == "always()", "Upload eligibility must run after any failure")
    eligibility_run = str(eligibility.get("run", ""))
    for token in (
        "RUNNER_TEMP",
        "IsPathRooted",
        "UNC and device paths are rejected",
        "Assert-NoReparsePath",
        "bootstrap_owner_token",
        "$maxFiles = 100",
        "$maxEntries = 300",
        "$maxSingleBytes = 67108864",
        "$maxTotalBytes = 268435456",
        "safe=true",
    ):
        require(token in eligibility_run, f"Evidence upload guard is missing: {token}")
    upload = [step for step in steps if step.get("uses") == "actions/upload-artifact@v4"]
    require(len(upload) == 1, "Expected exactly one evidence Artifact upload")
    upload_step = upload[0]
    require("always()" in str(upload_step.get("if", "")), "Artifact must be eligible after failure")
    require("steps.upload_eligibility.outputs.safe == 'true'" in str(upload_step.get("if", "")),
            "Artifact depends on independent safe-path output")
    require("steps.finalize.outcome" not in str(upload_step.get("if", "")),
            "Artifact must not depend on finalize success")
    require(upload_step.get("with", {}).get("if-no-files-found") == "error",
            "Missing evidence must fail the Artifact step")

    enforce = [step for step in steps if step.get("name") == "Enforce one-shot synthetic result"]
    require(len(enforce) == 1, "Final enforcement step is missing")
    enforce_step = enforce[0]
    require("UPLOAD_SAFE" in str(enforce_step.get("env", {})), "Final enforcement lacks upload-safety gate")
    enforce_run = str(enforce_step.get("run", ""))
    require(
        enforce_run.index("UPLOAD_SAFE") < enforce_run.index("$assessmentPath"),
        "Final enforcement can access an unowned evidence path",
    )

    for step in steps:
        if step.get("shell") == "pwsh":
            assert_powershell_syntax(str(step.get("run", "")).encode(), str(step.get("name", "PowerShell step")))
    all_steps = "\n".join(str(step) for step in steps)
    require("cargo" not in all_steps.lower() and "npm" not in all_steps.lower(),
            "Draft includes unrelated build systems")
    return workflow


def assert_powershell_syntax(source: bytes, label: str) -> None:
    parser = Parser(Language(tree_sitter_powershell.language()))
    tree = parser.parse(source)
    pending = [tree.root_node]
    errors = []
    while pending:
        node = pending.pop()
        if node.type == "ERROR" or node.is_missing:
            errors.append(f"{node.type}@{node.start_point.row + 1}:{node.start_point.column + 1}")
        pending.extend(node.children)
    require(not errors, f"PowerShell syntax parse failed in {label}: {errors}")


def verify_manifest() -> None:
    path = ROOT / MANIFEST
    require(path.is_file(), "P3-P0R SHA256SUMS is missing")
    rows = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        require(match is not None, f"Malformed manifest row: {line!r}")
        digest, relative = match.groups()
        require(relative not in rows, f"Duplicate manifest path: {relative}")
        rows[relative] = digest
    require(set(rows) == {REPORT, VALIDATOR}, "Manifest contents do not match P3-P0R files")
    for relative, expected in rows.items():
        actual = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        require(actual == expected, f"SHA-256 mismatch for {relative}: {actual}")


def audit_existing_push_workflows() -> int:
    count = 0
    paths = sorted((ROOT / ".github/workflows").glob("*.yml")) + sorted(
        (ROOT / ".github/workflows").glob("*.yaml")
    )
    for path in paths:
        payload = yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
        triggers = payload.get("on", {}) if isinstance(payload, dict) else {}
        push = triggers.get("push") if isinstance(triggers, dict) else None
        if push is None:
            continue
        count += 1
        if not isinstance(push, dict):
            raise RuntimeError(f"Unscoped push trigger cannot be audited safely: {path.name}")
        branches = push.get("branches")
        if branches is None:
            if push.get("tags"):
                continue
            raise RuntimeError(f"Push trigger accepts all branches: {path.name}")
        if not isinstance(branches, list) or any(
            fnmatch.fnmatchcase(BRANCH, pattern) for pattern in branches
        ):
            raise RuntimeError(f"Existing Workflow would run on the P3-P0R branch: {path.name}")
    return count


def main() -> None:
    branch = git("branch", "--show-current")
    require(branch == BRANCH, f"Unexpected branch: {branch}")
    require(git("rev-parse", f"{BASE}^{{tree}}") == BASE_TREE, "Frozen P3-P0 Tree mismatch")
    require(git("rev-parse", f"{P2}^{{tree}}") == P2_TREE, "Frozen P2 Tree mismatch")
    require(git("rev-parse", f"{P1}^{{tree}}") == P1_TREE, "Frozen P1 Tree mismatch")
    require(git("rev-parse", f"{BASE}:{P3_RUNNER}") == P3_RUNNER_BLOB, "Frozen P3 Runner blob mismatch")
    runner_sha = hashlib.sha256((ROOT / P3_RUNNER).read_bytes()).hexdigest()
    require(runner_sha == P3_RUNNER_SHA256, "Frozen P3 Runner file SHA-256 mismatch")
    require(git("rev-parse", f"{S0}:{PROBE}") == PROBE_BLOB, "Frozen S0 Probe blob mismatch")
    require(git("hash-object", PROBE) == PROBE_BLOB, "Working S0 Probe differs from S0")
    for path in (P1_GUARD, P1_TEST):
        require(git("rev-parse", f"{P1}:{path}") == git("hash-object", path),
                f"Frozen P1 source changed: {path}")

    report = (ROOT / REPORT).read_text(encoding="utf-8")
    workflow = parse_workflow(report)
    require(".github/workflows" not in "\n".join(ALLOWED), "P3-P0R files include an active Workflow")
    require(not (ROOT / ACTIVATION_FILE).exists(), "Executable P3 Workflow was added")
    require("single direct child" in report and "original P3-P0 branch" in report,
            "Activation branch/diff restriction is not documented")
    require("No-copy behavior remains unverified." in report, "No-copy limitation is not disclosed")
    require("U2 production remain NO-GO" in report, "Production gate status is missing")
    require("Run this workflow only on the original P3-P0 branch" not in report,
            "Report has an ambiguous activation contract")
    existing_push_workflows = audit_existing_push_workflows()

    changes = set(git("diff", "--name-only", BASE, "HEAD").splitlines())
    status = git("status", "--porcelain", "--untracked-files=all").splitlines()
    work_paths = {line[3:] for line in status if len(line) >= 4}
    changes |= work_paths
    require(changes <= ALLOWED, f"Out-of-scope changes: {sorted(changes - ALLOWED)}")
    require(changes == ALLOWED, f"Missing P3-P0R deliverable(s): {sorted(ALLOWED - changes)}")
    require(not any(path.startswith(".github/workflows/") for path in changes),
            "P3-P0R must not install an executable Workflow")
    verify_manifest()

    diff_check = subprocess.run(
        ["git", "diff", "--check", BASE, "HEAD"], cwd=ROOT, text=True, capture_output=True
    )
    require(diff_check.returncode == 0, f"git diff --check failed: {diff_check.stdout}{diff_check.stderr}")
    print("status=STATIC_PREPARED")
    print("windows_runtime=NOT_RUN")
    print(f"branch={branch}")
    print(f"base={BASE} tree={BASE_TREE}")
    print(f"p3_runner_blob={P3_RUNNER_BLOB} raw_sha256={runner_sha}")
    print(f"workflow_jobs={len(workflow['jobs'])}; executable_workflow=NONE")
    print(f"existing_push_workflows={existing_push_workflows}; p3p0r_branch_matches=NONE")
    print(f"changed_paths={','.join(sorted(changes))}")
    print("hosted=NOT_STARTED; NSIS=NOT_RUN; HKCU=NOT_TOUCHED")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"P3-P0R static validation failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
