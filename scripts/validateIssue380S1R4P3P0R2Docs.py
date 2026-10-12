#!/usr/bin/env python3
"""Static-only validator for the P3-P0R2 unactivated research plan."""

from __future__ import annotations

import fnmatch
import hashlib
import pathlib
import re
import subprocess
import sys

import yaml
from tree_sitter import Language, Parser
import tree_sitter_powershell


ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = "7deeed026c92beb0ae9db09a639971c3bd06a111"
BASE_TREE = "95d60b0f947ba4e9ceb091560699a6dee8ee32d6"
BRANCH = "research/issue-380-s1r4-p3-p0r2-activation-safety-docs"
P3_BRANCH = "research/issue-380-s1r4-p3-p0-nsis-hosted-plan"
P2 = "86c21c14dd65bd2bbdb3056513489e0d4139bb47"
P2_TREE = "883a851cf8e75b4f39a4a46baea15ab109feb312"
P1 = "7a6fa69e5521d9759702c1923a9ade60a75819c3"
P1_TREE = "c5465ef93eae268e2c683876d959db90fac5da08"
S0 = "1f4951f3c93b2eb549d159fed2dd288992c625c8"
PROBE = "scripts/issue380S0NsisObservabilityProbe.nsi"
P3_RUNNER = "scripts/issue380S1R4P3SyntheticNsisRunner.ps1"
P3_RUNNER_BLOB = "1d11c3add3c0d1f45e7d7ac082d09c616cca51a1"
P3_RUNNER_SHA256 = "c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd"
P1_GUARD = "scripts/issue380S1R4PathGuard.ps1"
P1_TEST = "scripts/testIssue380S1R4PathGuard.ps1"
REPORT = "docs/research/ISSUE-380-S1R4-P3-P0R2-ACTIVATION-SAFETY.md"
VALIDATOR = "scripts/validateIssue380S1R4P3P0R2Docs.py"
MANIFEST = "docs/research/evidence/issue-380-s1r4-p3-p0r2/SHA256SUMS"
ACTIVATION_FILE = ".github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml"
ALLOWED = {REPORT, VALIDATOR, MANIFEST}
CONTROLLED = {
    P3_RUNNER: ("0aa8be6d58fc0994b3be6aec2af8dc02318fcefb", P3_RUNNER_BLOB, P3_RUNNER_SHA256),
    P1_GUARD: (P1, "42f38d81b314e6faa3426920b7671179015b26c3", "d38865aba8eccb444daba77ec4de572e64bcf2530f8e4f16c09b3aa7e7225ba9"),
    P1_TEST: (P1, "28b62abac73d08b71828482bfd26ef0da60faac2", "e0a6620456a04e08e3ef62cd001a01d6511dcd05e677680556862181221d4610"),
    PROBE: (S0, "5805f8d04cd37420a63f3e156c3b6fd5b4012f33", "518b729388e36bbcb99dd8a3d82c60cad48a92660676230495d12e0a3437bade"),
}


def git(*args: str) -> str:
    result = subprocess.run(["git", *args], cwd=ROOT, text=True, capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError(
            f"git {' '.join(args)} failed ({result.returncode}): "
            f"{result.stdout}{result.stderr}"
        )
    return result.stdout.strip()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def parse_powershell(source: bytes, label: str):
    parser = Parser(Language(tree_sitter_powershell.language()))
    tree = parser.parse(source)
    pending = [tree.root_node]
    errors = []
    commands = []
    while pending:
        node = pending.pop()
        if node.type == "ERROR" or node.is_missing:
            errors.append(f"{node.type}@{node.start_point.row + 1}:{node.start_point.column + 1}")
        if node.type == "command":
            commands.append(node)
        pending.extend(node.children)
    require(not errors, f"PowerShell static parse failed in {label}: {errors}")
    return commands


def wait_job_has_incompatible_parameters(source: bytes) -> bool:
    for command in parse_powershell(source, "Wait-Job parameter scan"):
        name = command.child_by_field_name("command_name")
        if name is None or name.text.decode("utf-8", errors="replace").lower() != "wait-job":
            continue
        pending = [command]
        parameters = set()
        while pending:
            node = pending.pop()
            if node.type == "command_parameter":
                token = node.text.decode("utf-8", errors="replace").lstrip("-").split(":", 1)[0]
                parameters.add(token.lower())
            pending.extend(node.children)
        if {"job", "state"} <= parameters:
            return True
    return False


def verify_wait_job_negative_regression() -> None:
    known_bad = b"Wait-Job -Job $observerJob -State Running -Timeout 5"
    require(wait_job_has_incompatible_parameters(known_bad),
            "Known invalid Wait-Job parameter set was not rejected")
    known_good = b"$observerJob | Wait-Job -Timeout 5"
    require(not wait_job_has_incompatible_parameters(known_good),
            "Valid pipeline Wait-Job example was rejected")


def parse_workflow(report: str) -> dict:
    fence = chr(96) * 3
    match = re.search(re.escape(fence + "yaml\n") + r"(.*?)\n" + re.escape(fence), report, re.DOTALL)
    require(match is not None, "P3-P0R2 report lacks its complete Workflow draft")
    workflow = yaml.load(match.group(1), Loader=yaml.BaseLoader)
    require(isinstance(workflow, dict), "Workflow draft is not a YAML mapping")
    require("workflow_dispatch" not in str(workflow), "Draft must not be manually activatable")
    trigger = workflow.get("on", {}).get("push", {})
    require(trigger.get("branches") == [P3_BRANCH], "Draft must target the original P3-P0 branch")
    require(trigger.get("paths") == [ACTIVATION_FILE], "Draft trigger is broader than the designated file")
    require(workflow.get("permissions") == {"contents": "read"}, "Workflow permissions are not read-only")
    jobs = workflow.get("jobs")
    require(isinstance(jobs, dict) and list(jobs) == ["synthetic-nsis"], "Expected one isolated Job")
    job = jobs["synthetic-nsis"]
    require(job.get("runs-on") == "windows-2022", "Draft must use windows-2022")
    require(job.get("timeout-minutes") == "15", "Draft timeout changed")
    steps = job.get("steps")
    require(isinstance(steps, list), "Draft has no steps")
    by_id = {step.get("id"): step for step in steps if isinstance(step, dict) and step.get("id")}
    required = {"bootstrap", "byte_policy", "checkout", "preflight", "synthetic_nsis", "finalize", "upload_eligibility"}
    require(required <= set(by_id), f"Required guarded steps missing: {sorted(required - set(by_id))}")
    ids = [step.get("id") for step in steps if isinstance(step, dict)]
    require(ids.index("byte_policy") < ids.index("checkout") < ids.index("preflight") < ids.index("synthetic_nsis"),
            "Checkout byte policy and admission order is invalid")
    require("steps.bootstrap.outcome == 'success'" in str(by_id["byte_policy"].get("if", "")),
            "Raw-byte policy must follow successful bootstrap")
    require("core.autocrlf false" in str(by_id["byte_policy"].get("run", "")),
            "core.autocrlf=false is not set before checkout")
    require("core.eol lf" in str(by_id["byte_policy"].get("run", "")),
            "core.eol=lf is not set before checkout")
    require("steps.byte_policy.outcome == 'success'" in str(by_id["checkout"].get("if", "")),
            "Checkout can run before byte policy passes")
    require("steps.checkout.outcome == 'success'" in str(by_id["preflight"].get("if", "")),
            "Preflight can run without successful checkout")

    preflight = str(by_id["preflight"].get("run", ""))
    for token in (
        "GITHUB_RUN_ATTEMPT", "git-byte-policy.json", "core_autocrlf_effective", "core.eol",
        "check-attr text eol filter working-tree-encoding", ".gitattributes", "filter_configuration",
        "Get-NewlineFacts", "expected_newline_format = 'LF_ONLY'", "working_newline",
        "working_raw_sha256", "checkout_git_blob", "0aa8be6d58fc0994b3be6aec2af8dc02318fcefb",
        P3_RUNNER_BLOB, P3_RUNNER_SHA256,
        "42f38d81b314e6faa3426920b7671179015b26c3",
        "28b62abac73d08b71828482bfd26ef0da60faac2",
        "5805f8d04cd37420a63f3e156c3b6fd5b4012f33",
        "d38865aba8eccb444daba77ec4de572e64bcf2530f8e4f16c09b3aa7e7225ba9",
        "e0a6620456a04e08e3ef62cd001a01d6511dcd05e677680556862181221d4610",
        "518b729388e36bbcb99dd8a3d82c60cad48a92660676230495d12e0a3437bade",
        "workingBlob -cne $item.blob", "workingSha -cne $item.sha256",
        "activationDiff.Count -ne 1", "parentFields.Count -ne 2",
        "parentFields[1] -ne $p3p0Head", "activationDiff[0] -cne $expectedActivation",
        P2, P2_TREE, P1, P1_TREE, S0, "688bd53c3c44040f3ab86fe4cf71a7a3aa879274",
        "merge-base --is-ancestor",
    ):
        require(token in preflight, f"Preflight byte/admission gate is missing: {token}")

    runner = str(by_id["synthetic_nsis"].get("run", ""))
    require("P3 Runner identity changed immediately before process start" in runner,
            "Runner identity is not checked immediately before Process.Start")
    require(runner.index("P3 Runner identity changed immediately before process start") < runner.index("$process.Start()"),
            "Runner identity check occurs after Process.Start")
    for token in (
        "ProcessStartInfo", "ArgumentList.Add", "controlled_file_identities_rechecked_before_start",
        "Start-Job", "observer_ready", "AddSeconds(5)", "NotStarted", "Running",
        "Completed", "Failed", "Stopped", "try {", "catch {", "finally {",
        "Resource boundary:", "$process.Kill($true)", "WaitForExit(10000)", "Stop-Job",
        "Remove-Job", "Second Kill", "StandardOutput.Close()", "StandardError.Close()",
        "runner-error.raw.txt", "runner.stdout.raw.txt", "runner.stderr.raw.txt",
        "wrapper_failure", "cleanup_results", "process-observer.json",
        "outside_task_root_or_image_unavailable",
    ):
        require(token in runner, f"Runner wrapper resource/evidence contract is missing: {token}")
    require(runner.index("Resource boundary:") < runner.index("$process.Start()") < runner.index("} catch {") < runner.index("} finally {"),
            "Process start is not enclosed by the declared exception boundary")
    require(runner.index("$process.Kill($true)") > runner.index("} finally {"),
            "Process-tree kill is not in finally cleanup")
    finalizer = str(by_id["finalize"].get("run", ""))
    for token in ("Get-Process -Id", "Runner parent PID is still alive", "runner_process_exited",
                  "runner_wait_for_exit_succeeded", "processRecord.cleanup_results.wait_for_exit_succeeded",
                  "processRecord.process_exited -ne $true", "assessment.process_observer_status -ne 'COMPLETED'"):
        require(token in finalizer, f"Finalizer does not verify process cleanup: {token}")
    require(not wait_job_has_incompatible_parameters(runner.encode()),
            "Incompatible Wait-Job -Job and -State parameters occur in the draft")

    verify_wait_job_negative_regression()
    frozen_runner = (ROOT / P3_RUNNER).read_text(encoding="utf-8")
    require('$uninstallerArguments[$uninstallerArguments.Count - 1].Equals("_?=$absoluteInstallDirectory"' in frozen_runner,
            "Frozen Runner no longer puts the controlled _?= path last")
    require("100ms" in report and "does not prove" in report.lower(),
            "Observer limitation/no-copy statement is missing")

    bootstrap = str(by_id["bootstrap"].get("run", ""))
    require("bootstrap_owner_token" in bootstrap, "Bootstrap ownership token is missing")
    eligibility = by_id["upload_eligibility"]
    require(eligibility.get("if") == "always()", "Safe Artifact eligibility must run after failures")
    eligibility_run = str(eligibility.get("run", ""))
    for token in ("RUNNER_TEMP", "IsPathRooted", "UNC and device paths are rejected",
                  "Assert-NoReparsePath", "bootstrap_owner_token", "$maxFiles = 100",
                  "$maxEntries = 300", "$maxSingleBytes = 67108864",
                  "$maxTotalBytes = 268435456", "safe=true"):
        require(token in eligibility_run, f"Independent upload safety check missing: {token}")
    uploads = [step for step in steps if step.get("uses") == "actions/upload-artifact@v4"]
    require(len(uploads) == 1, "Expected exactly one evidence Artifact upload")
    upload = uploads[0]
    require("always()" in str(upload.get("if", "")), "Artifact must be considered after failure")
    require("steps.upload_eligibility.outputs.safe == 'true'" in str(upload.get("if", "")),
            "Artifact must require independent ownership eligibility")
    require("steps.finalize.outcome" not in str(upload.get("if", "")),
            "Artifact must not depend on successful finalization")
    require(upload.get("with", {}).get("if-no-files-found") == "error",
            "Missing evidence must fail upload")
    all_steps = "\n".join(str(step) for step in steps)
    require("cargo" not in all_steps.lower() and "npm" not in all_steps.lower(),
            "Draft includes unrelated build systems")
    for step in steps:
        if step.get("shell") == "pwsh":
            source = str(step.get("run", "")).encode()
            parse_powershell(source, str(step.get("name", "PowerShell step")))
            require(not wait_job_has_incompatible_parameters(source),
                    f"Invalid Wait-Job parameters in {step.get('name')}")
    return workflow


def newline_format(payload: bytes) -> str:
    crlf = payload.count(b"\r\n")
    lf = payload.count(b"\n")
    bare_lf = lf - crlf
    if lf == 0:
        return "NO_NEWLINE"
    if crlf == 0:
        return "LF_ONLY"
    if bare_lf == 0:
        return "CRLF"
    return "MIXED"


def verify_frozen_bytes() -> None:
    require(git("rev-parse", f"{BASE}:{P3_RUNNER}") == P3_RUNNER_BLOB,
            "Frozen P3 Runner Git blob mismatch")
    for path, (source, blob, raw_sha) in CONTROLLED.items():
        require(git("rev-parse", f"{source}:{path}") == blob, f"Frozen source blob changed: {path}")
        require(git("hash-object", "--", path) == blob, f"Working Git blob changed: {path}")
        payload = (ROOT / path).read_bytes()
        require(hashlib.sha256(payload).hexdigest() == raw_sha, f"Working raw SHA changed: {path}")
        require(newline_format(payload) == "LF_ONLY", f"Working newline format is not LF-only: {path}")
    attributes = git("check-attr", "text", "eol", "filter", "working-tree-encoding", "--", *CONTROLLED.keys())
    for row in attributes.splitlines():
        require(row.rsplit(": ", 1)[-1] in {"unspecified", "unset"},
                f"Effective Git attribute can transform controlled bytes: {row}")
    tracked_attributes = [path for path in git("ls-files").splitlines()
                          if pathlib.PurePosixPath(path).name == ".gitattributes"]
    require(all(not path.startswith("scripts/") for path in tracked_attributes),
            "Unexpected .gitattributes under scripts")
    filter_config = subprocess.run(
        ["git", "config", "--show-origin", "--get-regexp", r"^filter\..*\.(clean|smudge|process|required)$"],
        cwd=ROOT, text=True, capture_output=True, check=False
    )
    require(filter_config.returncode in (0, 1), "Git filter configuration could not be inspected")


def verify_manifest() -> None:
    path = ROOT / MANIFEST
    require(path.is_file(), "P3-P0R2 SHA256SUMS is missing")
    rows = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        require(match is not None, f"Malformed manifest row: {line!r}")
        digest, relative = match.groups()
        require(relative not in rows, f"Duplicate manifest path: {relative}")
        rows[relative] = digest
    require(set(rows) == {REPORT, VALIDATOR}, "Manifest must contain report and Validator only")
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
        if not isinstance(branches, list) or any(fnmatch.fnmatchcase(BRANCH, pattern) for pattern in branches):
            raise RuntimeError(f"Existing Workflow would run on the P3-P0R2 docs branch: {path.name}")
    return count


def main() -> None:
    branch = git("branch", "--show-current")
    require(branch == BRANCH, f"Unexpected branch: {branch}")
    require(git("rev-parse", f"{BASE}^{{tree}}") == BASE_TREE, "Frozen P3-P0R Tree mismatch")
    require(git("rev-parse", f"{P2}^{{tree}}") == P2_TREE, "Frozen P2 Tree mismatch")
    require(git("rev-parse", f"{P1}^{{tree}}") == P1_TREE, "Frozen P1 Tree mismatch")
    verify_frozen_bytes()
    report = (ROOT / REPORT).read_text(encoding="utf-8")
    workflow = parse_workflow(report)
    require(not (ROOT / ACTIVATION_FILE).exists(), "Executable Workflow was added")
    require("No Workflow was placed under .github/workflows" in report, "Draft activation status is missing")
    require("No Hosted job was authorized or started." in report, "No-Hosted declaration is missing")
    require("PowerShell runtime, NSIS" in report, "Unexecuted runtime items are missing")
    require("No-copy behavior remains unverified." in report, "No-copy limitation is missing")
    require("U2 production remain NO-GO" in report, "U2 gate status is missing")

    existing_push_workflows = audit_existing_push_workflows()
    changes = set(git("diff", "--name-only", BASE, "HEAD").splitlines())
    status = git("status", "--porcelain", "--untracked-files=all").splitlines()
    changes |= {line[3:] for line in status if len(line) >= 4}
    require(changes <= ALLOWED, f"Out-of-scope changes: {sorted(changes - ALLOWED)}")
    require(changes == ALLOWED, f"Missing deliverables: {sorted(ALLOWED - changes)}")
    require(not any(path.startswith(".github/workflows/") for path in changes),
            "P3-P0R2 must not install an active Workflow")
    verify_manifest()
    diff_check = subprocess.run(
        ["git", "diff", "--check", BASE, "HEAD"], cwd=ROOT, text=True, capture_output=True
    )
    require(diff_check.returncode == 0,
            f"git diff --check failed: {diff_check.stdout}{diff_check.stderr}")
    print("status=STATIC_PREPARED")
    print("windows_runtime=NOT_RUN; powershell_runtime=NOT_RUN; NSIS=NOT_RUN")
    print(f"branch={branch}")
    print(f"base={BASE} tree={BASE_TREE}")
    print(f"p3_runner_blob={P3_RUNNER_BLOB} raw_sha256={P3_RUNNER_SHA256} newline=LF_ONLY")
    print(f"controlled_byte_files={len(CONTROLLED)}; effective_transform_attributes=NONE")
    print(f"workflow_jobs={len(workflow['jobs'])}; executable_workflow=NONE")
    print(f"existing_push_workflows={existing_push_workflows}; docs_branch_matches=NONE")
    print(f"changed_paths={','.join(sorted(changes))}")
    print("negative_wait_job_parameter_set_regression=PASS_STATIC")
    print("hosted=NOT_STARTED; NSIS=NOT_RUN; HKCU=NOT_TOUCHED")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"P3-P0R2 static validation failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
