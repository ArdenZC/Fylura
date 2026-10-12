#!/usr/bin/env python3
"""Offline scope and trigger preflight for the one-shot #380 S1 Windows probe."""

from __future__ import annotations

import pathlib
import subprocess
import sys

import yaml


ROOT = pathlib.Path(__file__).resolve().parents[1]
S0 = "1f4951f3c93b2eb549d159fed2dd288992c625c8"
BRANCH = "research/issue-380-s1-synthetic-nsis-runtime-trace"
WORKFLOW_PATH = ".github/workflows/issue-380-s1-synthetic-nsis-probe.yml"
RUNNER_PATH = "scripts/issue380S1SyntheticNsisRunner.ps1"
PROBE_PATH = "scripts/issue380S0NsisObservabilityProbe.nsi"
VALIDATOR_PATH = "scripts/validateIssue380S1Harness.py"
ALLOWED_PATHS = {WORKFLOW_PATH, RUNNER_PATH, PROBE_PATH, VALIDATOR_PATH}
EXPECTED_TRIGGER_PATHS = {WORKFLOW_PATH, RUNNER_PATH, PROBE_PATH}


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


def main() -> int:
    head = git("rev-parse", "HEAD")
    branch = git("branch", "--show-current")
    require(branch == BRANCH, f"expected branch {BRANCH}, got {branch}")
    require(git("merge-base", "--is-ancestor", S0, "HEAD") == "", "S0 must be an ancestor")
    require(git("rev-parse", f"{S0}:scripts/issue380S0NsisObservabilityProbe.nsi") ==
            git("hash-object", PROBE_PATH), "committed S0 Probe blob must remain unchanged")

    changed = set(git("diff", "--name-only", f"{S0}...HEAD").splitlines())
    staged = set(git("diff", "--cached", "--name-only").splitlines())
    require(changed | staged <= ALLOWED_PATHS,
            f"unexpected changed paths: {sorted((changed | staged) - ALLOWED_PATHS)}")

    workflow_text = (ROOT / WORKFLOW_PATH).read_text(encoding="utf-8")
    workflow = yaml.load(workflow_text, Loader=yaml.BaseLoader)
    require(isinstance(workflow, dict), "workflow YAML did not parse as a mapping")
    triggers = workflow.get("on", {})
    require(set(triggers) == {"push"}, f"workflow must have push as its only trigger: {set(triggers)}")
    push = triggers["push"]
    require(push.get("branches") == [BRANCH], "push trigger must target only the S1 research branch")
    require(set(push.get("paths", [])) == EXPECTED_TRIGGER_PATHS,
            "push path filter must be restricted to this workflow, the S1 runner, and frozen Probe")
    require("workflow_dispatch" not in workflow_text and "pull_request" not in workflow_text,
            "workflow must not expose an additional trigger")
    require(len(workflow.get("jobs", {})) == 1, "workflow must contain exactly one job")
    job = next(iter(workflow["jobs"].values()))
    require(job.get("runs-on") == "windows-2022", "job must use the bounded windows-2022 runner")
    require(job.get("timeout-minutes") == "15", "job must have a finite 15-minute timeout")
    require(job.get("permissions") is None, "workflow may not broaden token permissions at job level")
    require(workflow.get("permissions") == {"contents": "read"}, "workflow token must be contents:read only")
    step_names = [step.get("name", "") for step in job.get("steps", [])]
    require(len(step_names) == 8, f"expected eight narrow steps, got {len(step_names)}")
    require(any("Static identity and scope preflight" in name for name in step_names),
            "Hosted workflow must verify source identity before compiler installation")
    require(any("Install pinned NSIS compiler package" in name for name in step_names),
            "workflow must install only the pinned NSIS 3.11.0 package")
    require("'install', 'nsis', '--version=3.11.0'" in workflow_text,
            "compiler package/version is not pinned")
    forbidden_workflow = ("npm ", "cargo ", "tauri ", "benchmark", "matrix:", "Build Release")
    require(not any(word.lower() in workflow_text.lower() for word in forbidden_workflow),
            "workflow contains a forbidden build, benchmark, release, or matrix path")

    runner_text = (ROOT / RUNNER_PATH).read_text(encoding="utf-8")
    require("software\\zencanvass0harness" in runner_text.lower(),
            "runner must name the dedicated synthetic HKCU key")
    require("RegistryHive]::CurrentUser" in runner_text and "RegistryView]::Registry64" in runner_text,
            "runner must scope Registry access to the synthetic HKCU 64-bit view")
    require("ReadRegDWORD" in (ROOT / PROBE_PATH).read_text(encoding="utf-8"),
            "probe must exercise the expected Registry read predicate")
    require("RequestExecutionLevel user" in (ROOT / PROBE_PATH).read_text(encoding="utf-8"),
            "probe must not request elevation")
    require("HKLM" not in (ROOT / PROBE_PATH).read_text(encoding="utf-8"),
            "probe may not use machine Registry state")
    forbidden_runner = ("Start-Process", "sc.exe", "ServiceController", "Program Files\\Zen Canvas",
                        "src-tauri/", "npm run", "cargo test", "Invoke-WebRequest")
    require(not any(token.lower() in runner_text.lower() for token in forbidden_runner),
            "runner contains a product, Service, network-download, or build invocation")
    for token in ("'-NOCD'", "'-V2'", "TRACE_MODE", "RUN_MODE", "OUTPUT_NAME", "-HDRINFO", "-VERSION",
                  "FileOpen", "FileWrite", "FileClose", "expected=35150", "actual=35149", "result=FAIL"):
        require(token.lower() in runner_text.lower() or token.lower() in (ROOT / PROBE_PATH).read_text(encoding="utf-8").lower(),
                f"runner/probe lacks required probe detail: {token}")
    require("installer-abort.stdout.txt" in runner_text and "uninstaller-abort.stdout.txt" in runner_text,
            "runner must retain stdout for both abort contexts")
    require("installer_abort_expected_exit" in runner_text and "uninstaller_abort_expected_exit" in runner_text,
            "runner must assert both expected Abort exit codes")
    require('"file_trace_${context}_record_complete"' in runner_text and "@('installer', 'uninstaller')" in runner_text,
            "runner must assert complete trace records for both Abort contexts")
    require("Join-Path $modeRoot 'direct-evidence.log'" in runner_text and
            "Copy-Item -LiteralPath $runTrace -Destination $modeEvidence" in runner_text,
            "runner must collect the trace from the NSIS process EXEDIR into the retained evidence directory")

    diff_check = subprocess.run(["git", "diff", "--check", S0], cwd=ROOT, text=True, capture_output=True)
    require(diff_check.returncode == 0, f"git diff --check failed: {diff_check.stdout}{diff_check.stderr}")

    print(f"S1 static preflight: PASS")
    print(f"branch={branch}")
    print(f"base={S0}")
    print(f"head={head}")
    print(f"changed_paths={','.join(sorted(changed | staged))}")
    print("workflow=single push-triggered windows-2022 job, exact research branch and path filter")
    print("registry_scope=synthetic HKCU 64-bit view only")
    print("production_install/service/database/build paths=absent")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # noqa: BLE001 - test preflight reports one concise blocker.
        print(f"S1 static preflight: FAIL: {exc}", file=sys.stderr)
        raise SystemExit(1)
