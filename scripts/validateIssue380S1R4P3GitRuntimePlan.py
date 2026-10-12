#!/usr/bin/env python3
"""Static checks for the inactive #380 S1R4-P3 PowerShell runtime plan.

This validator never runs PowerShell, Git capture, NSIS, a product binary, or a
Hosted workflow. It verifies frozen identities and review-plan boundaries only.
"""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import unittest
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[1]
BASE_HEAD = "8b441df124ba1c64a806037b9461370f27ab1ef0"
BASE_TREE = "a2ba19477ca9ed6817b86c1da8b73db41838f274"
CODE_HEAD = "af305adb8fba017dd9284a829b5101a209a9b4e2"
CODE_TREE = "9d7716add5fde99c6e797065f336e0d0cf86a510"
RUNNER_PATH = "scripts/issue380S1R4P3SyntheticNsisRunner.ps1"
RUNNER_BLOB = "c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa"
RUNNER_SHA256 = "92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece"
P1_HEAD = "7a6fa69e5521d9759702c1923a9ade60a75819c3"
P1_PATH_GUARD = "scripts/issue380S1R4PathGuard.ps1"
P1_PATH_TEST = "scripts/testIssue380S1R4PathGuard.ps1"
S0_HEAD = "1f4951f3c93b2eb549d159fed2dd288992c625c8"
S0_PROBE = "scripts/issue380S0NsisObservabilityProbe.nsi"
OLD_WORKFLOW = ".github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml"
REPORT = "docs/research/ISSUE-380-S1R4-P3-POWERSHELL-RUNTIME-QUALIFICATION.md"
DRAFT = "docs/research/ISSUE-380-S1R4-P3-POWERSHELL-RUNTIME-WORKFLOW.md"
HARNESS = "scripts/issue380S1R4P3GitRuntimeQualification.ps1"
CASE03_REPRODUCER = "scripts/verifyIssue380S1R4P3Case03GitCommands.py"
CHILD = "scripts/issue380S1R4P3GitRuntimeChild.ps1"
CONTROLLER = "scripts/issue380S1R4P3GitRuntimeController.ps1"
VALIDATOR = "scripts/validateIssue380S1R4P3GitRuntimePlan.py"
EVIDENCE = "docs/research/evidence/issue-380-s1r4-p3-pwsh-runtime"
IDENTITY = f"{EVIDENCE}/plan-identity.json"
STATIC_RESULT = f"{EVIDENCE}/static-validation.txt"
SHA_SUMS = f"{EVIDENCE}/SHA256SUMS"
CASE03_OBSERVATION = f"{EVIDENCE}/case03-real-git-observation.json"
WORKFLOW_PATH = ".github/workflows/issue-380-s1r4-p3-git-runtime.yml"
FUTURE_BRANCH = "research/issue-380-s1r4-p3-git-runtime-once"
PREPARATION_BRANCH = "research/issue-380-s1r4-p3-git-runtime-case03-amendment"

ALLOWED_ADDITIONS = {
    REPORT,
    DRAFT,
    HARNESS,
    CASE03_REPRODUCER,
    CHILD,
    CONTROLLER,
    VALIDATOR,
    IDENTITY,
    STATIC_RESULT,
    SHA_SUMS,
    CASE03_OBSERVATION,
}


def git(*args: str, text: bool = True) -> str | bytes:
    result = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=text,
    )
    return result.stdout.strip() if text else result.stdout


def blob(revision: str, path: str) -> str:
    return str(git("rev-parse", f"{revision}:{path}"))


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def workflow_yaml() -> str:
    draft = (ROOT / DRAFT).read_text(encoding="utf-8")
    match = re.search(r"```yaml\n(.*?)\n```", draft, re.DOTALL)
    if not match:
        raise AssertionError("Inactive Workflow draft must contain one complete YAML document.")
    return match.group(1) + "\n"


class RuntimePlanContracts(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.report = (ROOT / REPORT).read_text(encoding="utf-8")
        cls.draft = (ROOT / DRAFT).read_text(encoding="utf-8")
        cls.harness = (ROOT / HARNESS).read_text(encoding="utf-8")
        cls.case03_reproducer = (ROOT / CASE03_REPRODUCER).read_text(encoding="utf-8")
        cls.case03_observation = json.loads((ROOT / CASE03_OBSERVATION).read_text(encoding="utf-8"))
        cls.runner = (ROOT / RUNNER_PATH).read_text(encoding="utf-8")
        cls.child = (ROOT / CHILD).read_text(encoding="utf-8")
        cls.controller = (ROOT / CONTROLLER).read_text(encoding="utf-8")
        cls.workflow = workflow_yaml()

    def test_01_freeze_and_research_parent_identities(self) -> None:
        self.assertEqual(str(git("branch", "--show-current")), PREPARATION_BRANCH)
        self.assertEqual(str(git("show", "-s", "--format=%T", CODE_HEAD)), CODE_TREE)
        self.assertEqual(str(git("show", "-s", "--format=%T", BASE_HEAD)), BASE_TREE)
        self.assertEqual(blob(CODE_HEAD, RUNNER_PATH), RUNNER_BLOB)
        self.assertEqual(sha(git("show", f"{CODE_HEAD}:{RUNNER_PATH}", text=False)), RUNNER_SHA256)
        current = str(git("rev-parse", "HEAD"))
        self.assertNotEqual(current, "c474517ff2ae9fe91932c61468fbdf6e2baf28a2", "Do not prepare on the historical activated branch.")
        self.assertEqual(str(git("rev-parse", f"{BASE_HEAD}^{{tree}}")), BASE_TREE)
        if current != BASE_HEAD:
            self.assertEqual(str(git("show", "-s", "--format=%P", current)), BASE_HEAD)

    def test_02_allowed_paths_and_no_active_workflow(self) -> None:
        current = str(git("rev-parse", "HEAD"))
        if current == BASE_HEAD:
            changed = set()
            status = subprocess.run(
                ["git", "status", "--porcelain=v1", "--untracked-files=all"],
                cwd=ROOT,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            ).stdout
            for line in status.splitlines():
                if not line:
                    continue
                if len(line) < 4:
                    self.fail(f"Malformed porcelain status record: {line!r}")
                path = line[3:]
                changed.add(path)
        else:
            ancestor = subprocess.run(
                ["git", "merge-base", "--is-ancestor", BASE_HEAD, current], cwd=ROOT
            )
            self.assertEqual(ancestor.returncode, 0)
            changed = set(str(git("diff", "--name-only", BASE_HEAD, current)).splitlines())
        self.assertLessEqual(changed, ALLOWED_ADDITIONS)
        self.assertNotIn(WORKFLOW_PATH, changed)
        self.assertTrue(all(path.startswith("docs/") or path.startswith("scripts/") for path in changed))

    def test_03_frozen_sources_and_historical_workflow_are_unchanged(self) -> None:
        protected = (RUNNER_PATH, P1_PATH_GUARD, P1_PATH_TEST, S0_PROBE, OLD_WORKFLOW)
        for path in protected:
            committed = git("show", f"{BASE_HEAD}:{path}", text=False)
            self.assertEqual((ROOT / path).read_bytes(), committed, path)
        self.assertEqual(blob(P1_HEAD, P1_PATH_GUARD), "42f38d81b314e6faa3426920b7671179015b26c3")
        self.assertEqual(blob(P1_HEAD, P1_PATH_TEST), "28b62abac73d08b71828482bfd26ef0da60faac2")
        self.assertEqual(blob(S0_HEAD, S0_PROBE), "5805f8d04cd37420a63f3e156c3b6fd5b4012f33")

    def test_04_extraction_uses_the_four_frozen_functions_without_rewrite(self) -> None:
        for name in (
            "Invoke-GitCapture",
            "Get-RequiredGitScalar",
            "Assert-GitNoOutputSuccess",
            "Assert-GitWorktreeClean",
        ):
            self.assertIn(f"'{name}'", self.harness)
            self.assertIn(f"$node.Name -ceq $Name", self.harness)
            self.assertIn("$found[0].Extent.Text", self.harness)
        self.assertIn("$utf8.GetString($runnerBytes)", self.harness)
        self.assertIn("$sourceSliceBytes", self.harness)
        self.assertIn("source_byte_start", self.harness)
        self.assertIn("exact_extent_text_copied = $true", self.harness)
        self.assertIn("[System.IO.File]::WriteAllBytes($fragmentPath, $fragmentBytes)", self.harness)
        self.assertNotRegex(self.harness + self.child + self.controller, r"(?i)Invoke-Expression|Add-Type|Registry::|HKCU")

    def test_05_real_git_and_wrapper_matrix_is_explicit(self) -> None:
        for case in (
            "01_real_git_zero_empty_streams",
            "02_real_git_nonempty_stdout_and_scalar_wrapper",
            "03_real_git_nonzero_with_stderr",
            "04_real_git_normal_exit_and_stream_close",
            "05_real_git_alias_delayed_streams_under_five_seconds",
            "06_git_exit_with_inherited_pipe_holder",
            "07_git_process_timeout_and_tree_termination",
            "08_termination_failure_injection_limit",
            "09_output_deadline_and_task_terminal_state",
            "10_consecutive_invocation_records_are_independent",
            "11_evidence_json_temporal_and_status_consistency",
            "12_exception_fails_closed_without_a_live_test_child",
            "13_final_evidence_json_temporal_and_status_consistency",
        ):
            self.assertIn(case, self.harness)
        self.assertIn("Assert-GitWorktreeClean -WorkingDirectory $script:RepoPath", self.harness)
        self.assertIn("Assert-GitNoOutputSuccess -Arguments", self.harness)
        self.assertIn("Get-RequiredGitScalar -Arguments", self.harness)

    def test_06_error_paths_cannot_return_a_runtime_pass(self) -> None:
        self.assertIn("UNPROVEN", self.harness)
        self.assertIn("PARTIAL_BLOCKED_FAULT_INJECTION_UNPROVEN", self.harness)
        self.assertIn("if ($script:Result.status -ne 'RUNTIME_MATRIX_PASS') { exit 1 }", self.harness)
        self.assertIn("status -notin @('EXITED_ZERO', 'EXITED_NONZERO')", self.harness)
        self.assertIn("output_timed_out", self.harness)
        self.assertIn("process_exit_confirmed", self.harness)

    def test_07_process_ownership_and_cleanup_are_scoped(self) -> None:
        self.assertIn("CommandLine -notlike \"*$($script:Nonce)*\"", self.harness)
        self.assertIn("issue380S1R4P3GitRuntimeChild.ps1", self.harness)
        self.assertIn("$process.Kill($true)", self.harness)
        self.assertIn("WaitForExit(5000)", self.harness)
        self.assertIn("Refusing to act on PID", self.harness)
        self.assertIn("Assert-NoOwnedProcessRemains", self.harness)
        self.assertIn("Complete-OwnedRuntimeProcessCleanup", self.controller)
        self.assertIn("owned_process_cleanup_status", self.controller)
        self.assertIn("CommandLine -like \"*$nonce*\"", self.controller)
        self.assertNotRegex(self.harness + self.child + self.controller, r"(?i)Remove-Item.*(System32|Program Files|HKCU|Registry)")

    def test_08_wait_budgets_are_not_called_a_total_upper_bound(self) -> None:
        self.assertIn("configured_explicit_wait_budget_ms", self.harness)
        self.assertIn("harness_elapsed_ms", self.harness)
        self.assertIn("not a proven whole-function worst-case completion time", self.report)
        self.assertIn("not been established", self.report)
        self.assertIn("Task.WaitAll", self.report)

    def test_09_inactive_workflow_has_one_narrow_hosted_job(self) -> None:
        yaml_text = self.workflow
        parsed = yaml.load(yaml_text, Loader=yaml.BaseLoader)
        self.assertIsInstance(parsed, dict)
        self.assertEqual(list(parsed["jobs"]), ["powershell-runtime"])
        self.assertEqual(parsed["jobs"]["powershell-runtime"]["runs-on"], "windows-2022")
        self.assertEqual(parsed["jobs"]["powershell-runtime"]["timeout-minutes"], "10")
        self.assertEqual(parsed["permissions"]["contents"], "read")
        self.assertIn("branches:\n      - " + FUTURE_BRANCH, yaml_text)
        self.assertIn("runs-on: windows-2022", yaml_text)
        self.assertIn("timeout-minutes: 10", yaml_text)
        self.assertIn("contents: read", yaml_text)
        self.assertIn("if-no-files-found: error", yaml_text)
        self.assertIn("GITHUB_RUN_ATTEMPT", yaml_text)
        self.assertIn("OWNER_REVIEWED_PLAN_HEAD_REQUIRED", yaml_text)
        self.assertNotIn("workflow_dispatch:", yaml_text)
        self.assertNotRegex(yaml_text, r"(?i)makensis|nsis-3\.11|installer|service|schema|database|cargo|npm|tauri")
        jobs = re.search(r"(?m)^jobs:\s*\n(.*?)(?=^\S|\Z)", yaml_text, re.DOTALL)
        self.assertIsNotNone(jobs)
        self.assertEqual(len(re.findall(r"(?m)^  [A-Za-z0-9_-]+:\s*$", jobs.group(1))), 1)

    def test_10_workflow_byte_policy_and_failure_evidence_contract(self) -> None:
        yaml_text = self.workflow
        self.assertIn("GIT_CONFIG_KEY_0: core.autocrlf", yaml_text)
        self.assertIn("GIT_CONFIG_VALUE_0: 'false'", yaml_text)
        self.assertIn("GIT_CONFIG_KEY_1: core.eol", yaml_text)
        self.assertIn("GIT_CONFIG_VALUE_1: lf", yaml_text)
        self.assertIn("steps.preflight.outcome == 'success'", yaml_text)
        self.assertIn("if: always()", yaml_text)
        self.assertIn("controller.stdout.raw.txt", self.controller)
        self.assertIn("controller.stderr.raw.txt", self.controller)
        self.assertIn("reparse", yaml_text.lower())
        self.assertIn("runtime-owner.json", yaml_text)
        self.assertIn(CONTROLLER, yaml_text)

    def test_11_current_workflow_triggers_do_not_match_future_branch(self) -> None:
        workflow_dir = ROOT / ".github/workflows"
        self.assertFalse((ROOT / WORKFLOW_PATH).exists())
        for path in workflow_dir.glob("*.yml"):
            text = path.read_text(encoding="utf-8")
            self.assertNotIn(FUTURE_BRANCH, text, path.name)
        self.assertIn("- master", (workflow_dir / "ci.yml").read_text(encoding="utf-8"))

    def test_12_history_and_not_run_boundaries_are_explicit(self) -> None:
        self.assertIn("38276002263", self.report)
        self.assertIn("NOT RUN", self.report)
        self.assertIn("does not activate a workflow", self.report)
        self.assertIn("HOSTED_NSIS_NO_GO", self.report)
        self.assertIn("U2_PRODUCTION_NO_GO", self.report)

    def test_13_sha_manifest_matches_all_review_payload_files(self) -> None:
        sums_path = ROOT / SHA_SUMS
        self.assertTrue(sums_path.is_file())
        for line in sums_path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            expected, relpath = line.split("  ", 1)
            actual = sha((ROOT / relpath).read_bytes())
            self.assertEqual(expected, actual, relpath)

    def test_14_workflow_raw_hash_and_git_blob_pins_match_test_tools(self) -> None:
        expected = {
            HARNESS: ("ebbdefb0bdb79e3ce26678dffae776e3ba5ef7fb", "fe1e5dd7af4cd5a5c2a236f249065a39442a1ce9594211b4a555d91d44c14aeb"),
            CHILD: ("b2f9895172335160315cfd5cd4d1715406704bcb", "fb453ed1d79a2079dc3eae0c82d4433090511654a5e366d4620ff2fa07e3575e"),
            CONTROLLER: ("4b126cf6958eb079736d8f05999d90438ea6fd9a", "9d2f377250422c5ddf788b3589c79b25d2019bb447e3995f4cc24e6d87b23f76"),
        }
        for path, (expected_blob, expected_sha) in expected.items():
            self.assertIn(expected_blob, self.workflow)
            self.assertIn(expected_sha, self.workflow)
            self.assertEqual(str(git("hash-object", path)), expected_blob, path)
            self.assertEqual(sha((ROOT / path).read_bytes()), expected_sha, path)
        validator_blob = re.search(r"(?m)^\s*\$validatorBlob = '([0-9a-f]{40})'", self.workflow)
        validator_sha = re.search(r"(?m)^\s*\$validatorSha = '([0-9a-f]{64})'", self.workflow)
        self.assertIsNotNone(validator_blob)
        self.assertIsNotNone(validator_sha)
        self.assertEqual(str(git("hash-object", VALIDATOR)), validator_blob.group(1))
        self.assertEqual(sha((ROOT / VALIDATOR).read_bytes()), validator_sha.group(1))

    def test_15_case03_harness_has_both_git_failure_contracts(self) -> None:
        self.assertIn("Invoke-TestCase -Name '03_real_git_nonzero_with_stderr'", self.harness)
        self.assertIn("@('cat-file', '-p', '0000000000000000000000000000000000000000')", self.harness)
        self.assertIn("Invoke-TestCase -Name '03b_real_git_nonzero_empty_stderr_is_not_success'", self.harness)
        self.assertIn("@('cat-file', '-e', '0000000000000000000000000000000000000000')", self.harness)
        self.assertIn("Assert-GitNoOutputSuccess -Arguments $arguments", self.harness)
        self.assertIn("$wrapperError.Contains(\"exit=$($wrapperCapture.exit_code)\")", self.harness)
        self.assertNotRegex(self.harness, r"(?i)Not a valid object name|fatal:|usage: git")

    def test_16_case03_real_git_evidence_has_positive_and_negative_assertions(self) -> None:
        observation = self.case03_observation
        identity = json.loads((ROOT / IDENTITY).read_text(encoding="utf-8"))
        self.assertEqual("PASS", observation["status"])
        self.assertEqual("NOT RUN", observation["powershell_runtime"])
        assertions = {item["name"]: item for item in observation["assertions"]}
        self.assertTrue(assertions["original_nonzero_empty_stderr_is_observed"]["passed"])
        self.assertTrue(assertions["candidate_nonzero_stderr_is_observed_without_pinning_text"]["passed"])
        original = observation["commands"]["original_cat_file_e_missing_object"]
        candidate = observation["commands"]["candidate_cat_file_p_missing_object"]
        self.assertNotEqual(0, original["exit_code"])
        self.assertEqual(0, original["stdout_bytes"])
        self.assertEqual(0, original["stderr_bytes"])
        self.assertNotEqual(0, candidate["exit_code"])
        self.assertEqual(0, candidate["stdout_bytes"])
        self.assertGreater(candidate["stderr_bytes"], 0)
        self.assertTrue(observation["cleanup"]["temporary_repository_removed"])
        self.assertIn("subprocess.run", self.case03_reproducer)
        self.assertIn("cat-file", self.case03_reproducer)
        self.assertNotRegex(self.case03_reproducer, r"(?i)Not a valid object name|fatal:|usage: git")
        self.assertEqual(str(git("hash-object", CASE03_REPRODUCER)), identity["case03_reproducer_git_blob"])
        self.assertEqual(sha((ROOT / CASE03_REPRODUCER).read_bytes()), identity["case03_reproducer_sha256"])
        self.assertEqual(sha((ROOT / CASE03_OBSERVATION).read_bytes()), identity["case03_real_git_observation_sha256"])

    def test_17_empty_stderr_is_rejected_by_frozen_wrapper_and_case08_stays_partial(self) -> None:
        match = re.search(
            r"(?ms)^function Assert-GitNoOutputSuccess\b.*?(?=^function |\Z)",
            self.runner,
        )
        self.assertIsNotNone(match)
        wrapper = match.group(0)
        exit_guard = wrapper.index("$capture.exit_code -ne 0")
        stderr_guard = wrapper.index("if (-not [string]::IsNullOrEmpty($capture.stderr))")
        self.assertLess(exit_guard, stderr_guard)
        self.assertIn("03b_real_git_nonzero_empty_stderr_is_not_success", self.harness)
        self.assertIn("08_termination_failure_injection_limit", self.harness)
        self.assertIn("UNPROVEN", self.harness)
        self.assertIn("$script:Result.status = 'PARTIAL_BLOCKED_FAULT_INJECTION_UNPROVEN'", self.harness)
        self.assertIn("$script:Result.status = 'RUNTIME_MATRIX_PASS'", self.harness)
        self.assertIn("if ($script:Result.status -ne 'RUNTIME_MATRIX_PASS') { exit 1 }", self.harness)
        self.assertIn("UNPROVEN_WITH_FROZEN_DIRECT_PROCESS_IMPLEMENTATION", (ROOT / IDENTITY).read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
