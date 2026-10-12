#!/usr/bin/env python3
"""Offline validation for the inert #380 S1R4-P3 preflight remediation draft.

This validates draft structure and a small typed-output contract model. It does
not execute PowerShell, GitHub Actions, NSIS, or the frozen P3 Runner.
"""

from __future__ import annotations

import copy
import hashlib
import re
import subprocess
import sys
import tempfile
import unittest
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

import yaml
from tree_sitter import Language, Parser
import tree_sitter_powershell


ROOT = Path(__file__).resolve().parents[1]
BASE = "c474517ff2ae9fe91932c61468fbdf6e2baf28a2"
BASE_TREE = "b5ce99e3809b78555c1273fe5f984bbd7fb32c11"
ACTIVE_WORKFLOW = ".github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml"
DRAFT = ROOT / "docs/research/ISSUE-380-S1R4-P3-PREFLIGHT-NULL-SAFETY-WORKFLOW.md"
REPORT = ROOT / "docs/research/ISSUE-380-S1R4-P3-PREFLIGHT-NULL-SAFETY.md"
MANIFEST = ROOT / "docs/research/evidence/issue-380-s1r4-p3-preflight-null-safety/SHA256SUMS"
RUNNER = "scripts/issue380S1R4P3SyntheticNsisRunner.ps1"
EXPECTED_RUNNER_BLOB = "1d11c3add3c0d1f45e7d7ac082d09c616cca51a1"
EXPECTED_RUNNER_SHA256 = "c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd"
EXPECTED_ADDITIONS = {
    "docs/research/ISSUE-380-S1R4-P3-PREFLIGHT-NULL-SAFETY.md",
    "docs/research/ISSUE-380-S1R4-P3-PREFLIGHT-NULL-SAFETY-WORKFLOW.md",
    "scripts/validateIssue380S1R4P3PreflightNullSafety.py",
    "docs/research/evidence/issue-380-s1r4-p3-preflight-null-safety/SHA256SUMS",
}


def git(*args: str, input_bytes: bytes | None = None) -> bytes:
    result = subprocess.run(
        ["git", *args], cwd=ROOT, input=input_bytes, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, check=False,
    )
    if result.returncode:
        raise AssertionError(
            f"git {' '.join(args)} failed ({result.returncode}): "
            f"{result.stderr.decode(errors='replace')}"
        )
    return result.stdout


def yaml_load(text: str) -> dict:
    # BaseLoader preserves the GitHub Actions key `on` as a string.
    value = yaml.load(text, Loader=yaml.BaseLoader)
    if not isinstance(value, dict):
        raise AssertionError("Workflow YAML must have a mapping at its root.")
    return value


def fenced_workflow() -> str:
    text = DRAFT.read_text(encoding="utf-8")
    matches = re.findall(r"```yaml\n(.*?)\n```", text, re.DOTALL)
    if len(matches) != 1:
        raise AssertionError(f"Expected one complete YAML block; found {len(matches)}.")
    return matches[0] + "\n"


def steps_by_id(workflow: dict) -> dict[str, dict]:
    steps = workflow["jobs"]["synthetic-nsis"]["steps"]
    result: dict[str, dict] = {}
    for step in steps:
        step_id = step.get("id")
        if step_id:
            if step_id in result:
                raise AssertionError(f"Duplicate step id: {step_id}")
            result[step_id] = step
    return result


@dataclass(frozen=True)
class GitCapture:
    exit_code: int
    lines: tuple[str, ...]


class GitContractError(ValueError):
    pass


def normalize_git_capture(exit_code: int, raw_lines: Iterable[str] | None) -> GitCapture:
    """Model the draft contract: a no-output native process is an empty tuple."""
    if raw_lines is None:
        lines: tuple[str, ...] = ()
    else:
        lines = tuple(str(line) for line in raw_lines)
    return GitCapture(exit_code=exit_code, lines=lines)


def require_clean_status(result: GitCapture) -> None:
    if result.exit_code != 0:
        raise GitContractError(f"git status failed with exit {result.exit_code}")
    if result.lines:
        raise GitContractError("worktree is not clean")


def require_scalar(result: GitCapture, label: str) -> str:
    if result.exit_code != 0:
        raise GitContractError(f"{label} failed with exit {result.exit_code}")
    if len(result.lines) != 1 or not result.lines[0].strip():
        raise GitContractError(f"{label} must return exactly one non-empty line")
    return result.lines[0].strip()


def optional_filter_config(result: GitCapture) -> tuple[str, ...] | None:
    if result.exit_code == 1 and not result.lines:
        return None
    if result.exit_code == 0 and result.lines:
        return result.lines
    raise GitContractError(
        f"unexpected filter-config result: exit={result.exit_code}, lines={len(result.lines)}"
    )


def parse_powershell(source: str, label: str) -> None:
    parser = Parser(Language(tree_sitter_powershell.language()))
    tree = parser.parse(source.encode("utf-8"))
    if tree.root_node.has_error:
        errors: list[str] = []

        def visit(node) -> None:
            if node.type == "ERROR" or node.is_missing:
                errors.append(
                    f"{node.type} at {node.start_point}->{node.end_point}: "
                    f"{source.encode('utf-8')[node.start_byte:node.end_byte]!r}"
                )
            for child in node.children:
                visit(child)

        visit(tree.root_node)
        raise AssertionError(f"PowerShell parse failure in {label}: {'; '.join(errors[:8])}")


def validate_static_contract() -> dict[str, str]:
    base_tree = git("rev-parse", f"{BASE}^{{tree}}").decode().strip()
    if base_tree != BASE_TREE:
        raise AssertionError(f"Frozen activation Tree changed: {base_tree}")

    changed = set(git("diff", "--name-only", BASE, "HEAD").decode().splitlines())
    untracked = set(git("ls-files", "--others", "--exclude-standard").decode().splitlines())
    if changed | untracked != EXPECTED_ADDITIONS:
        raise AssertionError(
            f"Unexpected research diff: tracked={sorted(changed)} untracked={sorted(untracked)}"
        )
    if git("diff", "--quiet", BASE, "HEAD", "--", ACTIVE_WORKFLOW).decode():
        raise AssertionError("The activated workflow must remain byte-for-byte frozen.")

    original_bytes = git("show", f"{BASE}:{ACTIVE_WORKFLOW}")
    original = yaml_load(original_bytes.decode("utf-8"))
    draft_text = fenced_workflow()
    draft = yaml_load(draft_text)
    original_steps = steps_by_id(original)
    draft_steps = steps_by_id(draft)
    if set(original_steps) != set(draft_steps):
        raise AssertionError("The draft changed the activation Workflow step identity set.")
    original_step_list = original["jobs"]["synthetic-nsis"]["steps"]
    draft_step_list = draft["jobs"]["synthetic-nsis"]["steps"]
    if len(original_step_list) != len(draft_step_list):
        raise AssertionError("The draft changed the activation Workflow step count.")
    for original_step, draft_step in zip(original_step_list, draft_step_list):
        step_id = original_step.get("id")
        if step_id not in {"byte_policy", "preflight"} and original_step != draft_step:
            raise AssertionError(f"Unexpected non-target Workflow step change: {step_id or original_step.get('name')}")
        if step_id in {"byte_policy", "preflight"}:
            original_metadata = copy.deepcopy(original_step)
            draft_metadata = copy.deepcopy(draft_step)
            original_metadata.pop("run", None)
            draft_metadata.pop("run", None)
            if original_metadata != draft_metadata:
                raise AssertionError(f"Target step metadata changed outside its run body: {step_id}")
    for key in ("name", "on", "permissions", "concurrency"):
        if original.get(key) != draft.get(key):
            raise AssertionError(f"Workflow-level contract changed: {key}")
    original_job = copy.deepcopy(original["jobs"]["synthetic-nsis"])
    draft_job = copy.deepcopy(draft["jobs"]["synthetic-nsis"])
    original_job.pop("steps")
    draft_job.pop("steps")
    if original_job != draft_job:
        raise AssertionError("Job identity, runner image, timeout, or other job contract changed.")

    trigger = draft["on"]["push"]
    if trigger["branches"] != ["research/issue-380-s1r4-p3-p0-nsis-hosted-plan"]:
        raise AssertionError("One-shot branch trigger changed.")
    if trigger["paths"] != [ACTIVE_WORKFLOW]:
        raise AssertionError("One-shot path trigger changed.")
    if draft["permissions"] != {"contents": "read"}:
        raise AssertionError("Workflow permissions are not read-only.")
    if draft["jobs"]["synthetic-nsis"]["runs-on"] != "windows-2022":
        raise AssertionError("Workflow runner changed.")

    byte_policy_script = draft_steps["byte_policy"]["run"]
    preflight_script = draft_steps["preflight"]["run"]
    runner_step = draft_steps["synthetic_nsis"]
    if "-NoProfile" not in runner_step["run"] or "Start-Process" not in runner_step["run"] and "ProcessStartInfo" not in runner_step["run"]:
        raise AssertionError("Runner step identity/process invocation changed unexpectedly.")
    if "GITHUB_RUN_ATTEMPT" not in draft_steps["bootstrap"]["run"] or "-ne '1'" not in draft_steps["bootstrap"]["run"]:
        raise AssertionError("One-shot attempt guard is missing.")
    if "steps.preflight.outcome == 'success'" not in runner_step["if"]:
        raise AssertionError("Runner is not strictly gated on preflight success.")
    if "always()" not in runner_step["if"]:
        raise AssertionError("Existing final evidence behavior was not preserved.")

    for label, source in (("byte_policy", byte_policy_script), ("preflight", preflight_script)):
        parse_powershell(source, label)
        if re.search(r"\(&\s*git\b[^\r\n]*\)\.Trim\s*\(", source):
            raise AssertionError(f"{label} still calls Trim directly on native Git output.")
        if "$nativeItems = @(& git @Arguments 2>&1)" not in source:
            raise AssertionError(f"{label} does not capture native Git output into an array.")
        lines = source.splitlines()
        native_index = lines.index("  $nativeItems = @(& git @Arguments 2>&1)")
        if lines[native_index + 1].strip() != "$nativeExitCode = [int]$LASTEXITCODE":
            raise AssertionError(f"{label} does not capture LASTEXITCODE immediately.")
        if "OutputCount = $lines.Length" not in source or "Text = [string]::Join" not in source:
            raise AssertionError(f"{label} does not normalize empty output to a typed string/line count.")
        if "$PSNativeCommandUseErrorActionPreference = $false" not in source:
            raise AssertionError(f"{label} does not preserve native exit-code capture under Stop preference.")

    if "$statusResult.ExitCode -ne 0" not in preflight_script or "$statusResult.OutputCount -ne 0" not in preflight_script:
        raise AssertionError("Clean status must be exit 0 plus zero output lines; dirty output must reject.")
    if "filterResult.ExitCode -eq 1 -and $filterResult.OutputCount -eq 0" not in preflight_script:
        raise AssertionError("The legal missing optional filter-config case is not explicit.")
    if "Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'config', '--get', 'core.autocrlf')" not in preflight_script:
        raise AssertionError("Required worktree core.autocrlf must use strict scalar validation.")
    if "Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'config', '--get', 'core.eol')" not in preflight_script:
        raise AssertionError("Required worktree core.eol must use strict scalar validation.")
    if "$attributeResult.OutputCount -ne $expectedAttributeRows" not in preflight_script:
        raise AssertionError("git check-attr must reject incomplete output.")
    if "Activation must be a single direct child of frozen P3-P0" not in preflight_script:
        raise AssertionError("Direct-child activation check was lost.")
    for sentinel in (
        "1d11c3add3c0d1f45e7d7ac082d09c616cca51a1",
        "c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd",
        "86c21c14dd65bd2bbdb3056513489e0d4139bb47",
        "7a6fa69e5521d9759702c1923a9ade60a75819c3",
        "1f4951f3c93b2eb549d159fed2dd288992c625c8",
    ):
        if sentinel not in preflight_script:
            raise AssertionError(f"Frozen identity gate missing: {sentinel}")

    # The current frozen Runner has a separate, unchanged empty-status .Trim hazard.
    runner_text = git("show", f"{BASE}:{RUNNER}").decode("utf-8")
    runner_blob = git("rev-parse", f"{BASE}:{RUNNER}").decode().strip()
    runner_sha = hashlib.sha256(git("show", f"{BASE}:{RUNNER}")).hexdigest()
    if runner_blob != EXPECTED_RUNNER_BLOB or runner_sha != EXPECTED_RUNNER_SHA256:
        raise AssertionError("Frozen P3 Runner identity differs from Owner-approved identity.")
    if "(& git status --porcelain --untracked-files all).Trim()" not in runner_text:
        raise AssertionError("Expected downstream Runner status hazard was changed or not found.")
    report_text = REPORT.read_text(encoding="utf-8")
    if "frozen P3 Runner" not in report_text or "separate clean-status null-output hazard" not in report_text:
        raise AssertionError("Report must disclose the frozen Runner's separate null-output blocker.")

    for step_id, step in draft_steps.items():
        if "run" in step and step.get("shell") == "pwsh":
            parse_powershell(step["run"], f"workflow-step:{step_id}")

    expected_manifest_files = {
        "docs/research/ISSUE-380-S1R4-P3-PREFLIGHT-NULL-SAFETY.md",
        "docs/research/ISSUE-380-S1R4-P3-PREFLIGHT-NULL-SAFETY-WORKFLOW.md",
        "scripts/validateIssue380S1R4P3PreflightNullSafety.py",
    }
    manifest_entries: dict[str, str] = {}
    for line in MANIFEST.read_text(encoding="ascii").splitlines():
        if not line.strip():
            continue
        fields = line.split(maxsplit=1)
        if len(fields) != 2 or not re.fullmatch(r"[0-9a-f]{64}", fields[0]):
            raise AssertionError(f"Malformed SHA256SUMS row: {line!r}")
        relative_path = fields[1].lstrip("* ")
        if relative_path in manifest_entries:
            raise AssertionError(f"Duplicate SHA256SUMS path: {relative_path}")
        manifest_entries[relative_path] = fields[0]
    if set(manifest_entries) != expected_manifest_files:
        raise AssertionError(f"SHA256SUMS contents differ: {sorted(manifest_entries)}")
    for relative_path, expected_hash in manifest_entries.items():
        actual_hash = hashlib.sha256((ROOT / relative_path).read_bytes()).hexdigest()
        if actual_hash != expected_hash:
            raise AssertionError(f"SHA-256 mismatch: {relative_path}")

    return {
        "frozen_workflow_blob": git("rev-parse", f"{BASE}:{ACTIVE_WORKFLOW}").decode().strip(),
        "frozen_runner_blob": runner_blob,
        "frozen_runner_sha256": runner_sha,
        "powershell_parser": "tree-sitter-powershell 0.26.4",
        "workflow_yaml": "PyYAML BaseLoader",
        "sha256_manifest": "PASS (report, inert Workflow draft, Validator)",
        "unchanged_steps": ",".join(sorted(set(original_steps) - {"byte_policy", "preflight"})),
    }


class GitOutputContractTests(unittest.TestCase):
    def test_1_clean_status_empty_output_exit_zero(self) -> None:
        with self.assertRaises(AttributeError):
            None.strip()  # Model the old direct method call on no-output data.
        require_clean_status(normalize_git_capture(0, None))

    def test_2_status_with_modification_is_rejected(self) -> None:
        with self.assertRaises(GitContractError):
            require_clean_status(normalize_git_capture(0, [" M src/file.rs"]))

    def test_3_status_command_nonzero_fails_closed(self) -> None:
        with self.assertRaises(GitContractError):
            require_clean_status(normalize_git_capture(128, None))

    def test_4_required_sha_must_not_be_empty(self) -> None:
        with self.assertRaises(GitContractError):
            require_scalar(normalize_git_capture(0, None), "required SHA")

    def test_5_optional_filter_config_may_be_absent(self) -> None:
        self.assertIsNone(optional_filter_config(normalize_git_capture(1, None)))

    def test_6_required_core_config_may_not_be_absent(self) -> None:
        with self.assertRaises(GitContractError):
            require_scalar(normalize_git_capture(1, None), "required core.eol")

    def test_7_preflight_failure_does_not_authorize_runner(self) -> None:
        workflow = yaml_load(fenced_workflow())
        runner = steps_by_id(workflow)["synthetic_nsis"]
        self.assertIn("steps.preflight.outcome == 'success'", runner["if"])
        self.assertNotEqual(runner.get("if"), "${{ always() }}")

    def test_8_one_shot_and_evidence_contract_remain_static(self) -> None:
        workflow = yaml_load(fenced_workflow())
        self.assertEqual(workflow["permissions"], {"contents": "read"})
        self.assertEqual(workflow["on"]["push"]["branches"], ["research/issue-380-s1r4-p3-p0-nsis-hosted-plan"])
        self.assertIn("GITHUB_RUN_ATTEMPT", steps_by_id(workflow)["bootstrap"]["run"])
        upload_steps = [
            step for step in workflow["jobs"]["synthetic-nsis"]["steps"]
            if "upload-artifact" in str(step.get("uses", ""))
        ]
        self.assertEqual(len(upload_steps), 1)

    def test_9_clean_git_worktree_emits_zero_status_bytes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="issue380-clean-git-") as tmp:
            subprocess.run(["git", "init", "-q", tmp], check=True)
            subprocess.run(["git", "-C", tmp, "config", "user.name", "Contract Test"], check=True)
            subprocess.run(["git", "-C", tmp, "config", "user.email", "contract@example.invalid"], check=True)
            Path(tmp, "tracked.txt").write_text("tracked\n", encoding="utf-8")
            subprocess.run(["git", "-C", tmp, "add", "tracked.txt"], check=True)
            subprocess.run(["git", "-C", tmp, "commit", "-qm", "fixture"], check=True)
            result = subprocess.run(
                ["git", "-C", tmp, "status", "--porcelain", "--untracked-files", "all"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
            )
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout, b"")
            self.assertEqual(result.stderr, b"")


def main() -> int:
    print("Static workflow/source contract:")
    facts = validate_static_contract()
    for key, value in facts.items():
        print(f"  PASS {key}: {value}")
    print("Offline Git output contract model:")
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(GitOutputContractTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
