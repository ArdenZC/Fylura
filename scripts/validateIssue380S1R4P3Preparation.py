#!/usr/bin/env python3
"""Static-only P3 preparation validator. Does not execute PowerShell or NSIS."""

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
BRANCH = "research/issue-380-s1r4-p3-p0-nsis-hosted-plan"
P2 = "86c21c14dd65bd2bbdb3056513489e0d4139bb47"
P2_TREE = "883a851cf8e75b4f39a4a46baea15ab109feb312"
P1 = "7a6fa69e5521d9759702c1923a9ade60a75819c3"
P1_TREE = "c5465ef93eae268e2c683876d959db90fac5da08"
S0 = "1f4951f3c93b2eb549d159fed2dd288992c625c8"
S0_PROBE_BLOB = "5805f8d04cd37420a63f3e156c3b6fd5b4012f33"
S0_PROBE_SHA256 = "518b729388e36bbcb99dd8a3d82c60cad48a92660676230495d12e0a3437bade"
PROBE = "scripts/issue380S0NsisObservabilityProbe.nsi"
P1_GUARD = "scripts/issue380S1R4PathGuard.ps1"
P1_TEST = "scripts/testIssue380S1R4PathGuard.ps1"
P3_RUNNER = "scripts/issue380S1R4P3SyntheticNsisRunner.ps1"
VALIDATOR = "scripts/validateIssue380S1R4P3Preparation.py"
REPORT = "docs/research/ISSUE-380-S1R4-P3-P0-NSIS-HOSTED-PLAN.md"
MANIFEST = "docs/research/evidence/issue-380-s1r4-p3-p0/SHA256SUMS"
P2_RUNNER = "scripts/issue380S1R4SyntheticNsisRunner.ps1"
P2_WORKFLOW = ".github/workflows/issue-380-s1r4-p2-windows-path-matrix.yml"
ALLOWED = {P3_RUNNER, VALIDATOR, REPORT, MANIFEST}
P2_PRESERVED = {
    PROBE,
    P1_GUARD,
    P1_TEST,
    P2_RUNNER,
    P2_WORKFLOW,
}

PINNED_RUNNER_STRINGS = (
    "https://sourceforge.net/projects/nsis/files/NSIS%203/3.11/nsis-3.11.zip/download",
    "c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1",
    "f497e92deb9f179f7b8f7553fcb3bd04f511bb2949e5e4a2aee80a10f7b20431",
    "42850802704ecb11163f7e0329d35ee54bd288953200d4966e226d572848cfc5",
    "$script:LauncherLength = 2560L",
    "$script:CompilerLength = 468992L",
    "$script:ExpectedInstallerExitCode = 37",
    "$script:ExpectedUninstallerExitCode = 42",
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
    committed = set(git("diff", "--name-only", f"{P2}...HEAD").splitlines())
    status_result = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=all"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if status_result.returncode:
        raise RuntimeError(f"git status failed: {status_result.stderr}")
    working = {line[3:] for line in status_result.stdout.splitlines() if len(line) >= 4}
    return committed | working


def assert_powershell_syntax_text(source: bytes, label: str) -> None:
    parser = Parser(Language(tree_sitter_powershell.language()))
    tree = parser.parse(source)
    errors: list[str] = []
    stack = [tree.root_node]
    while stack:
        node = stack.pop()
        if node.type == "ERROR" or node.is_missing:
            start = node.start_point
            errors.append(f"{node.type}@{start.row + 1}:{start.column + 1}")
        stack.extend(node.children)
    require(not errors, f"PowerShell syntax parser errors in {label}: {errors}")


def assert_powershell_syntax(path: pathlib.Path) -> None:
    assert_powershell_syntax_text(path.read_bytes(), path.name)


def verify_manifest(expected: set[str]) -> None:
    path = ROOT / MANIFEST
    require(path.is_file(), "P3 SHA256SUMS is missing")
    entries: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        require(match is not None, f"Malformed SHA256SUMS line: {line!r}")
        digest, relative = match.groups()
        require(relative not in entries, f"Duplicate manifest path: {relative}")
        entries[relative] = digest
    require(set(entries) == expected, f"Manifest paths differ: {sorted(entries)}")
    for relative, digest in entries.items():
        actual = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        require(actual == digest, f"SHA-256 mismatch for {relative}: {actual}")


def parse_workflow_draft(report: str) -> dict:
    match = re.search(r"```yaml\n(.*?)\n```", report, re.DOTALL)
    require(match is not None, "Report does not contain the future Workflow YAML draft")
    workflow = yaml.load(match.group(1), Loader=yaml.BaseLoader)
    require(isinstance(workflow, dict), "Workflow draft did not parse to a YAML mapping")
    require(".github/workflows" not in "\n".join(changed_paths()), "Workflow path was changed")
    require(not (ROOT / ".github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml").exists(),
            "Executable P3 Workflow file exists; P3-P0 must remain inactive")
    require("workflow_dispatch" not in str(workflow), "Future draft must not include manual activation")
    trigger = workflow.get("on", {}).get("push", {})
    require(trigger.get("branches") == [BRANCH], "Draft push branch is not the unique P3 branch")
    require(trigger.get("paths") == [".github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml"],
            "Draft trigger path is not restricted to the activating Workflow")
    require(workflow.get("permissions") == {"contents": "read"}, "Draft permissions must be read-only")
    jobs = workflow.get("jobs")
    require(isinstance(jobs, dict) and list(jobs) == ["synthetic-nsis"], "Draft must contain exactly one job")
    job = jobs["synthetic-nsis"]
    require(job.get("runs-on") == "windows-2022", "Draft must use windows-2022")
    require(job.get("timeout-minutes") == "15", "Draft must enforce a 15-minute job timeout")
    steps = job.get("steps")
    require(isinstance(steps, list), "Draft job has no steps")
    ids = {step.get("id") for step in steps if isinstance(step, dict)}
    require({"bootstrap", "preflight", "synthetic_nsis"}.issubset(ids), "Draft preflight/runner step contract is incomplete")
    runner_steps = [step for step in steps if step.get("id") == "synthetic_nsis"]
    require(len(runner_steps) == 1 and runner_steps[0].get("timeout-minutes") == "12",
            "Draft Runner step must be separately bounded to 12 minutes")
    all_step_text = "\n".join(str(step) for step in steps)
    for needle in (
        "GITHUB_RUN_ATTEMPT",
        "PREFLIGHT_OUTCOME",
        "GITHUB_REF",
        "GITHUB_SHA",
        "issue380S1R4P3SyntheticNsisRunner.ps1",
        "runner.stdout.raw.txt",
        "runner.stderr.raw.txt",
        "runner-process.json",
        "WaitForExit(540000)",
        "if-no-files-found",
        "actions/upload-artifact@v4",
        "workflow-run.json",
        "P3HarnessOwner",
        "task_owned_processes",
        "issue380-s1r4-p3-task",
    ):
        require(needle in all_step_text, f"Draft Workflow is missing {needle}")
    require(any("$identity.status = 'RUNNING'" in str(step.get("run", "")) for step in steps),
            "Draft Workflow does not publish a RUNNING identity before the P3 Runner continues")
    upload_steps = [step for step in steps if step.get("uses") == "actions/upload-artifact@v4"]
    require(len(upload_steps) == 1 and "steps.finalize.outcome == 'success'" in upload_steps[0].get("if", ""),
            "Artifact upload must require the current owned evidence root to finalize successfully")
    for index, step in enumerate(steps):
        if isinstance(step, dict) and step.get("shell") == "pwsh":
            assert_powershell_syntax_text(str(step.get("run", "")).encode(), f"embedded workflow step {index}")
    require("cargo" not in all_step_text.lower() and "npm" not in all_step_text.lower(),
            "Draft unexpectedly launches unrelated build systems")
    return workflow


def audit_existing_push_triggers() -> int:
    workflows = sorted((ROOT / ".github/workflows").glob("*.yml")) + sorted(
        (ROOT / ".github/workflows").glob("*.yaml")
    )
    push_count = 0
    for path in workflows:
        payload = yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
        triggers = payload.get("on", {}) if isinstance(payload, dict) else {}
        push = triggers.get("push") if isinstance(triggers, dict) else None
        if push is None:
            continue
        push_count += 1
        if isinstance(push, dict):
            patterns = push.get("branches")
            if not isinstance(patterns, list) or not patterns:
                raise RuntimeError(f"Existing push Workflow is not branch-scoped: {path.relative_to(ROOT)}")
            if any(fnmatch.fnmatchcase(BRANCH, pattern) for pattern in patterns):
                raise RuntimeError(f"Existing push Workflow would match the P3 branch: {path.relative_to(ROOT)}")
        else:
            raise RuntimeError(f"Existing push Workflow does not have a branch filter: {path.relative_to(ROOT)}")
    return push_count


def main() -> int:
    branch = git("branch", "--show-current")
    head = git("rev-parse", "HEAD")
    tree = git("rev-parse", "HEAD^{tree}")
    require(branch == BRANCH, f"Unexpected P3 branch: {branch}")
    require(git("rev-parse", f"{P2}^{{tree}}") == P2_TREE, "Frozen P2 Tree mismatch")
    require(git("rev-parse", f"{P1}^{{tree}}") == P1_TREE, "Frozen P1 Tree mismatch")
    git("merge-base", "--is-ancestor", P2, "HEAD")
    git("merge-base", "--is-ancestor", P1, "HEAD")

    require(git("rev-parse", f"{P1}:{P1_GUARD}") == git("hash-object", P1_GUARD),
            "P1 path guard differs from the frozen P1 source")
    require(git("rev-parse", f"{P1}:{P1_TEST}") == git("hash-object", P1_TEST),
            "P1 Windows test differs from the frozen P1 source")
    require(git("rev-parse", f"{S0}:{PROBE}") == S0_PROBE_BLOB,
            "Frozen S0 Probe Git blob identity mismatch")
    require(git("rev-parse", f"{P2}:{PROBE}") == S0_PROBE_BLOB,
            "P2 Probe blob differs from frozen S0")
    require(git("hash-object", PROBE) == S0_PROBE_BLOB, "Working S0 Probe differs from frozen S0")
    probe_sha = hashlib.sha256((ROOT / PROBE).read_bytes()).hexdigest()
    require(probe_sha == S0_PROBE_SHA256, f"S0 Probe content SHA-256 mismatch: {probe_sha}")

    for path in P2_PRESERVED:
        require(git("rev-parse", f"{P2}:{path}") == git("hash-object", path),
                f"Accepted P2/P1/S0 file changed: {path}")
    paths = changed_paths()
    require(paths <= ALLOWED, f"Out-of-scope changes: {sorted(paths - ALLOWED)}")
    require(paths == ALLOWED, f"Research deliverables incomplete: expected={sorted(ALLOWED)} actual={sorted(paths)}")
    require(not any(path.startswith(".github/workflows/") for path in paths),
            "P3-P0 must not activate a Workflow")
    for path in (P3_RUNNER, P1_GUARD, P1_TEST):
        assert_powershell_syntax(ROOT / path)

    runner = (ROOT / P3_RUNNER).read_text(encoding="utf-8")
    report = (ROOT / REPORT).read_text(encoding="utf-8")
    require((ROOT / "scripts/issue380S1R4PathGuard.ps1").is_file(), "Frozen path guard is missing")
    for expected in (
        f"$script:P2Head = '{P2}'",
        f"$script:P2Tree = '{P2_TREE}'",
        f"$script:P1Head = '{P1}'",
        f"$script:P1Tree = '{P1_TREE}'",
        f"$script:ExpectedBranch = '{BRANCH}'",
        "$env:GITHUB_RUN_ATTEMPT -ne '1'",
        "$env:PREFLIGHT_OUTCOME -ne 'success'",
        "$env:GITHUB_RUN_ID)) { throw 'GITHUB_RUN_ID is required.' }",
        "$head -ne $env:GITHUB_SHA",
        "git merge-base --is-ancestor $script:P2Head HEAD",
        "git merge-base --is-ancestor $script:P1Head HEAD",
        "Resolve-Issue380S1R4Path",
        "ProcessStartInfo]::new()",
        "$startInfo.ArgumentList.Add($argument)",
        "function WaitForP3RunnerProcessRecord",
        "ParentProcessId",
        "expectedArguments = @('-NoLogo', '-NoProfile', '-NonInteractive', '-File'",
        "P3 process argument count differs from the fixed Runner invocation",
        "'runner.stdout.raw.txt', 'runner.stderr.raw.txt', 'runner-process.json'",
        "launcher_process = WaitForP3RunnerProcessRecord",
        '$uninstallerArguments.Add("_?=$absoluteInstallDirectory")',
        '$uninstallerArguments[$uninstallerArguments.Count - 1].Equals("_?=$absoluteInstallDirectory"',
        "$script:RegistrySubKey = 'Software\\ZenCanvasS0Harness'",
        "$script:RegistryOwnerValueName = 'P3HarnessOwner'",
        "function Get-SyntheticRegistryOwnerToken",
        "owner_kind = $ownerKind",
        "expected_owner_kind=String",
        "PRESERVED_BECAUSE_REGISTRY_STATE_CHANGED",
        "s1r4-p3-task-owned.txt",
        "REMOVED_AND_VERIFIED_ABSENT",
    ):
        require(expected in runner, f"P3 Runner contract missing: {expected}")
    for value in PINNED_RUNNER_STRINGS:
        require(value in runner, f"Pinned NSIS / exit contract changed: {value}")
    require("$script:P1PathGuardPath = 'scripts/issue380S1R4PathGuard.ps1'" in runner and
            "$script:P1PathTestPath = 'scripts/testIssue380S1R4PathGuard.ps1'" in runner,
            "P3 Runner does not pin P1 path source files")
    require("$preflightRecord.status -ne 'PASS'" in runner and "$workflowRunRecord.run_attempt -ne $env:GITHUB_RUN_ATTEMPT" in runner,
            "Runner does not cross-check preflight and run-identity JSON")
    require("foreach ($traceMode in @('file', 'detail'))" in runner,
            "Runner does not explicitly sequence file mode before optional Detail mode")
    loop = runner.split("foreach ($traceMode in @('file', 'detail'))", 1)[1].split("\n    $compilerOutputs", 1)[0]
    require(loop.count("Assert-PrimaryFileWriteEvidence -TracePath $evidenceTracePath") == 1,
            "Primary FileWrite evidence must be asserted within the trace-mode loop")
    require("if ($traceMode -eq 'file') {\n            Assert-PrimaryFileWriteEvidence" in loop,
            "Primary FileWrite assertion is not gated to the file iteration")
    primary = runner.split("function Assert-PrimaryFileWriteEvidence", 1)[1].split("\nfunction ", 1)[0]
    for token in ("context=", "predicate=HKCU\\Software\\ZenCanvasS0Harness\\EstimatedSize", "type=REG_DWORD",
                  "expected=35150", "actual=35149", "result=FAIL", "Add-Assertion"):
        require(token in primary, f"Primary direct trace assertion omits {token}")
    require("$detailTracePath" in loop and "TRACE_MODE=file" in runner,
            "Direct FileWrite remains the first trace mode")
    require('$uninstallerArguments.Add("/LOG=$uninstallerLog")' in runner and
            "_?=$absoluteInstallDirectory" in runner, "Uninstaller argument contract changed")
    require("$process.Kill($true)" in runner and "Wait-ForTaskOwnedProcessesToExit" in runner,
            "Runner process timeout/cleanup contract is missing")
    require("Get-Issue380S1R4PathAttributes" in (ROOT / P1_GUARD).read_text(encoding="utf-8"),
            "P3 does not retain the accepted path-guard reparse checks")

    workflow = parse_workflow_draft(report)
    existing_push_workflows = audit_existing_push_triggers()
    require("No P3 Hosted Workflow or NSIS job was started." in report,
            "Report must explicitly label the future runtime as not run")
    verify_manifest(ALLOWED - {MANIFEST})

    for args in (
        ["git", "diff", "--check", P2, "HEAD"],
        ["git", "diff", "--cached", "--check"],
        ["git", "diff", "--check"],
    ):
        result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=False)
        require(result.returncode == 0, f"{' '.join(args)} failed: {result.stdout}{result.stderr}")

    print("status=STATIC_PREPARED")
    print("windows_runtime=NOT_RUN")
    print(f"branch={branch}")
    print(f"head={head}")
    print(f"tree={tree}")
    print(f"frozen_p2={P2} tree={P2_TREE}")
    print(f"frozen_p1={P1} tree={P1_TREE}")
    print(f"frozen_s0_probe_blob={S0_PROBE_BLOB} raw_sha256={S0_PROBE_SHA256}")
    print(f"changed_paths={','.join(sorted(paths))}")
    print(f"workflow_draft_jobs={len(workflow['jobs'])}; executable_workflow=NONE")
    print(f"existing_push_workflows={existing_push_workflows}; p3_branch_trigger_matches=NONE")
    print("hosted_jobs=NOT_STARTED; NSIS=NOT_RUN; HKCU=NOT_TOUCHED")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # Preparation must fail closed.
        print(f"S1R4-P3-P0 validation failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
