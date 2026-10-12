#!/usr/bin/env python3
"""Offline scope and one-shot trigger preflight for the #380 S1R probe."""

from __future__ import annotations

import pathlib
import subprocess
import sys

import yaml


ROOT = pathlib.Path(__file__).resolve().parents[1]
S1 = "bd56788aa317d808dadf930260e191e9d1b45adb"
S0 = "1f4951f3c93b2eb549d159fed2dd288992c625c8"
BRANCH = "research/issue-380-s1r-nsis-zip-trace-persistence"
WORKFLOW = ".github/workflows/issue-380-s1r-synthetic-nsis-zip-probe.yml"
RUNNER = "scripts/issue380S1RSyntheticNsisRunner.ps1"
VALIDATOR = "scripts/validateIssue380S1RHarness.py"
PROBE = "scripts/issue380S0NsisObservabilityProbe.nsi"
REPORT = "docs/research/ISSUE-380-S1R-SYNTHETIC-TRACE-PERSISTENCE.md"
ALLOWED_PATHS = {WORKFLOW, RUNNER, VALIDATOR, PROBE, REPORT}
TRIGGER_PATHS = {WORKFLOW, RUNNER, VALIDATOR, PROBE}
ZIP_URL = "https://sourceforge.net/projects/nsis/files/NSIS%203/3.11/nsis-3.11.zip/download"
ZIP_SHA256 = "c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1"


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


def main() -> int:
    head = git("rev-parse", "HEAD")
    branch = git("branch", "--show-current")
    require(branch == BRANCH, f"expected branch {BRANCH}, got {branch}")
    require(git("rev-parse", f"{S1}^{{tree}}") == "45abf0197c7fab7ff39dcf37e644f7689bf50511",
            "frozen S1 Tree does not match the owner-provided baseline")
    git("merge-base", "--is-ancestor", S1, "HEAD")
    git("merge-base", "--is-ancestor", S0, "HEAD")
    require(
        git("rev-parse", f"{S0}:{PROBE}") == git("hash-object", PROBE),
        "the committed frozen S0 Probe blob changed",
    )

    changed = set(git("diff", "--name-only", f"{S1}...HEAD").splitlines())
    changed |= worktree_paths()
    require(changed <= ALLOWED_PATHS, f"out-of-scope paths: {sorted(changed - ALLOWED_PATHS)}")

    workflow_text = (ROOT / WORKFLOW).read_text(encoding="utf-8")
    workflow = yaml.load(workflow_text, Loader=yaml.BaseLoader)
    require(isinstance(workflow, dict), "workflow YAML did not parse as a mapping")
    require(set(workflow.get("on", {})) == {"push"}, "push must be the only workflow trigger")
    push = workflow["on"]["push"]
    require(push.get("branches") == [BRANCH], "push trigger must target only the S1R branch")
    require(set(push.get("paths", [])) == TRIGGER_PATHS, "workflow path filter is broader or incomplete")
    require(workflow.get("permissions") == {"contents": "read"}, "workflow permission must be contents:read only")
    require(len(workflow.get("jobs", {})) == 1, "workflow must contain exactly one job")
    job = next(iter(workflow["jobs"].values()))
    require(job.get("runs-on") == "windows-2022", "only the bounded windows-2022 runner is allowed")
    require(job.get("timeout-minutes") == "15", "job must have a 15-minute timeout")
    require(len(job.get("steps", [])) == 8, "workflow must have exactly eight narrow steps")
    step_names = [step.get("name", "") for step in job["steps"]]
    runner_step = next(step for step in job["steps"] if step.get("id") == "probe")
    require(runner_step.get("timeout-minutes") == "13", "probe step must leave bounded time for cleanup and evidence upload")
    for name in (
        "Static identity and tool pin preflight",
        "Run one-shot synthetic NSIS probe",
        "Clean synthetic Registry and task temp; retain cleanup evidence",
        "Upload raw synthetic probe evidence",
        "Enforce single-run result",
    ):
        require(name in step_names, f"workflow lacks required step: {name}")
    require("workflow_dispatch" not in workflow_text and "pull_request" not in workflow_text,
            "workflow exposes an unauthorized trigger")
    forbidden_workflow = ("npm ", "cargo ", "tauri ", "benchmark", "matrix:", "Build Release", "Chocolatey/setup")
    require(not any(token.lower() in workflow_text.lower() for token in forbidden_workflow),
            "workflow contains a forbidden build, package manager, benchmark, release, or matrix path")

    runner_text = (ROOT / RUNNER).read_text(encoding="utf-8")
    require(ZIP_URL in runner_text and ZIP_SHA256 in runner_text, "runner lacks the exact official URL/SHA pin")
    require("sha256_verified_before_extract" in runner_text and "Expand-VerifiedZipSafely" in runner_text,
            "extraction is not guarded by the archive SHA-256 result")
    require("Get-Command makensis" not in runner_text and "choco" not in runner_text.lower(),
            "runner may not use ambient NSIS or Chocolatey fallback")
    forbidden_runner = (
        "HKLM", "Program Files\\Zen Canvas", "ServiceController", "sc.exe",
        "Start-Process", "cargo test", "npm run", "tauri build", "src-tauri",
    )
    require(not any(token.lower() in runner_text.lower() for token in forbidden_runner),
            "runner contains a product, Service, or application build path")
    for token in (
        "HKCU", "RegistryHive]::CurrentUser", "RegistryView]::Registry64",
        "-VERSION", "-HDRINFO", "NSIS_CPU=x86", "NSIS_UNICODE",
        "FileOpen", "FileWrite", "FileClose", "direct_file_trace_exists_after_both_abort_processes_exit",
        '"file_trace_${context}_record_complete"', "@('installer', 'uninstaller')",
        "expected=35150", "actual=35149", "result=FAIL", "installer_abort_expected_exit",
        "uninstaller_abort_expected_exit", "SHA256SUMS", "nsis-3.11-download.zip",
    ):
        require(token.lower() in runner_text.lower() or token.lower() in (ROOT / PROBE).read_text(encoding="utf-8").lower(),
                f"runner/probe lacks required evidence contract: {token}")

    probe = (ROOT / PROBE).read_text(encoding="utf-8")
    require("RequestExecutionLevel user" in probe, "synthetic probe must not request elevation")
    require("HKLM" not in probe, "synthetic probe may not touch machine Registry state")
    require("Software\\ZenCanvasS0Harness" in probe, "frozen synthetic HKCU key is missing")
    require(ZIP_SHA256 in workflow_text, "workflow preflight must independently pin the ZIP SHA")

    diff = subprocess.run(["git", "diff", "--check", S1], cwd=ROOT, text=True, capture_output=True)
    require(diff.returncode == 0, f"git diff --check failed: {diff.stdout}{diff.stderr}")

    print("S1R static preflight: PASS")
    print(f"branch={branch}")
    print(f"base={S1}")
    print(f"head={head}")
    print(f"changed_paths={','.join(sorted(changed))}")
    print("workflow=one push-only windows-2022 job, exact branch and narrow path filter")
    print(f"nsis_zip_url={ZIP_URL}")
    print(f"nsis_zip_sha256={ZIP_SHA256}")
    print("registry_scope=synthetic HKCU 64-bit view only")
    print("production installer/service/database/build paths=absent")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # noqa: BLE001 - fail-closed preflight summary.
        print(f"S1R static preflight: FAIL: {exc}", file=sys.stderr)
        raise SystemExit(1)
