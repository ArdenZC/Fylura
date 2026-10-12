#!/usr/bin/env python3
"""Offline contract checks for the #380 S1R4-P3 Runner candidate.

This model/static suite does not execute PowerShell, Windows, NSIS, or the
synthetic installer. Its unit cases test the specified Git output contract;
the source checks ensure the candidate Runner is wired to that contract.
"""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import unittest
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BASE_HEAD = "740d99379c584589c92bd855f101e06858e8ee38"
OLD_ACTIVATION_HEAD = "c474517ff2ae9fe91932c61468fbdf6e2baf28a2"
OLD_RUNNER_HEAD = "0aa8be6d58fc0994b3be6aec2af8dc02318fcefb"
OLD_RUNNER_BLOB = "1d11c3add3c0d1f45e7d7ac082d09c616cca51a1"
OLD_RUNNER_SHA256 = "c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd"
OLD_WORKFLOW_PATH = ".github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml"
RUNNER_PATH = "scripts/issue380S1R4P3SyntheticNsisRunner.ps1"
P1_PATHS = (
    "scripts/issue380S1R4PathGuard.ps1",
    "scripts/testIssue380S1R4PathGuard.ps1",
)
PROBE_PATH = "scripts/issue380S0NsisObservabilityProbe.nsi"
REPORT_PATH = "docs/research/ISSUE-380-S1R4-P3-RUNNER-NULL-SAFETY.md"
DRAFT_PATH = "docs/research/ISSUE-380-S1R4-P3-RUNNER-NULL-SAFETY-WORKFLOW.md"
IDENTITY_PATH = "docs/research/evidence/issue-380-s1r4-p3-runner-null-safety/runner-identity.json"


class GitContractError(RuntimeError):
    pass


@dataclass(frozen=True)
class GitCapture:
    exit_code: int | None
    stdout: str = ""
    stderr: str = ""
    timed_out: bool = False
    launch_error: str | None = None


def require_command_success(capture: GitCapture, label: str) -> None:
    if capture.timed_out or capture.exit_code != 0 or capture.launch_error is not None:
        raise GitContractError(
            f"{label}: exit={capture.exit_code} timeout={capture.timed_out} "
            f"launch_error={capture.launch_error!r} stderr={capture.stderr!r}"
        )


def require_clean_status(capture: GitCapture) -> bool:
    require_command_success(capture, "git status")
    if capture.stderr:
        raise GitContractError(f"git status emitted stderr: {capture.stderr!r}")
    if capture.stdout:
        raise GitContractError(f"worktree is dirty: {capture.stdout!r}")
    return True


def require_scalar(capture: GitCapture, label: str) -> str:
    require_command_success(capture, label)
    if capture.stderr:
        raise GitContractError(f"{label} emitted stderr: {capture.stderr!r}")
    match = re.fullmatch(r"([^\r\n]+)(?:\r?\n)?", capture.stdout)
    if not match or not match.group(1).strip():
        raise GitContractError(f"{label} must return one non-empty line")
    return match.group(1).strip()


def require_no_output_success(capture: GitCapture, label: str) -> bool:
    require_command_success(capture, label)
    if capture.stderr or capture.stdout:
        raise GitContractError(f"{label} must have empty stdout and stderr")
    return True


def git_show(revision: str, path: str) -> bytes:
    result = subprocess.run(
        ["git", "show", f"{revision}:{path}"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout


def git_blob(revision: str, path: str) -> str:
    result = subprocess.run(
        ["git", "rev-parse", f"{revision}:{path}"],
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout.strip()


def frozen_value(name: str, source: str) -> str:
    match = re.search(
        rf"^\$script:{re.escape(name)}\s*=\s*(?:'([^']+)'|([0-9]+))\s*$",
        source,
        re.MULTILINE,
    )
    if not match:
        raise AssertionError(f"Missing frozen Runner assignment: {name}")
    return match.group(1) or match.group(2)


class RunnerNullSafetyContracts(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.runner = (ROOT / RUNNER_PATH).read_text(encoding="utf-8")
        cls.old_runner = git_show(OLD_RUNNER_HEAD, RUNNER_PATH).decode("utf-8")

    def test_01_clean_status_empty_stdout_and_exit_zero_is_clean(self) -> None:
        self.assertTrue(require_clean_status(GitCapture(exit_code=0, stdout="", stderr="")))

    def test_02_nonempty_status_is_rejected(self) -> None:
        with self.assertRaisesRegex(GitContractError, "dirty"):
            require_clean_status(GitCapture(exit_code=0, stdout=" M file.txt\n"))

    def test_03_nonzero_git_status_fails_closed_even_when_stdout_is_empty(self) -> None:
        with self.assertRaisesRegex(GitContractError, "exit=128"):
            require_clean_status(GitCapture(exit_code=128, stdout="", stderr="fatal: not a repository"))

    def test_04_required_identity_output_must_not_be_empty(self) -> None:
        with self.assertRaisesRegex(GitContractError, "one non-empty line"):
            require_scalar(GitCapture(exit_code=0, stdout=""), "required HEAD")

    def test_05_stderr_and_exit_code_are_checked_separately(self) -> None:
        with self.assertRaisesRegex(GitContractError, "stderr"):
            require_scalar(GitCapture(exit_code=0, stdout="abc123\n", stderr="warning"), "Tree")
        with self.assertRaisesRegex(GitContractError, "exit=2"):
            require_scalar(GitCapture(exit_code=2, stdout="abc123\n", stderr="fatal"), "Blob")

    def test_06_sequential_git_calls_keep_their_own_exit_codes(self) -> None:
        captures = [GitCapture(exit_code=1), GitCapture(exit_code=0, stdout="abc123\n")]
        with self.assertRaises(GitContractError):
            require_no_output_success(captures[0], "ancestor call")
        self.assertEqual(require_scalar(captures[1], "Tree call"), "abc123")
        self.assertIn("$process.ExitCode", self.runner)
        self.assertIn("exit_code = $capture.exit_code", self.runner)
        self.assertNotIn("$LASTEXITCODE", self.runner)
        self.assertNotRegex(self.runner, r"\(&\s*git\b")

    def test_07_runner_preflight_exception_stops_before_any_nsis_download(self) -> None:
        git_gate = self.runner.index("Assert-GitWorktreeClean -WorkingDirectory $repoRoot")
        download = self.runner.index("Download-And-VerifyPinnedZip", git_gate)
        failure_catch = self.runner.index("} catch {\n    $script:Failure = $_.Exception.ToString()", git_gate)
        self.assertLess(git_gate, download)
        self.assertLess(download, failure_catch)
        self.assertIn("exit 1", self.runner)
        self.assertIn("$result.status = 'FAIL'", self.runner)

    def test_08_preflight_and_runner_identity_fields_are_checked_against_git(self) -> None:
        self.assertIn("$preflightRecord.p3_runner_git_blob_sha -cne $runnerCommitBlob", self.runner)
        self.assertIn("$preflightRecord.p3_runner_file_sha256 -cne $runnerFileSha256", self.runner)
        self.assertIn("$runnerCommitBlob -cne $runnerWorktreeBlob", self.runner)
        self.assertIn("candidate_freeze_head", self.runner)
        self.assertIn("candidate_freeze_tree", self.runner)
        self.assertIn("$preflightRecord.activation_parent -cne $activationParent", self.runner)

    def test_09_frozen_runner_workflow_probe_and_p1_path_sources_remain_unchanged(self) -> None:
        original_runner_bytes = git_show(OLD_RUNNER_HEAD, RUNNER_PATH)
        self.assertEqual(git_blob(OLD_RUNNER_HEAD, RUNNER_PATH), OLD_RUNNER_BLOB)
        self.assertEqual(hashlib.sha256(original_runner_bytes).hexdigest(), OLD_RUNNER_SHA256)
        self.assertEqual(
            hashlib.sha256(git_show(OLD_ACTIVATION_HEAD, OLD_WORKFLOW_PATH)).hexdigest(),
            "19bdbce52a890fd340fb9d0d84d45666298080fc2934a0ff1d551d553eaab022",
        )
        for path in (*P1_PATHS, PROBE_PATH, OLD_WORKFLOW_PATH):
            self.assertEqual(
                hashlib.sha256((ROOT / path).read_bytes()).hexdigest(),
                hashlib.sha256(git_show(BASE_HEAD, path)).hexdigest(),
                path,
            )
        self.assertEqual(frozen_value("P2Head", self.runner), frozen_value("P2Head", self.old_runner))
        self.assertEqual(frozen_value("P2Tree", self.runner), frozen_value("P2Tree", self.old_runner))
        self.assertEqual(frozen_value("P1Head", self.runner), frozen_value("P1Head", self.old_runner))
        self.assertEqual(frozen_value("P1Tree", self.runner), frozen_value("P1Tree", self.old_runner))
        self.assertEqual(frozen_value("S0Head", self.runner), frozen_value("S0Head", self.old_runner))

    def test_10_nsis_filewrite_exit_code_and_cleanup_contracts_are_preserved(self) -> None:
        for assignment in (
            "ExpectedZipSha256",
            "LauncherSha256",
            "CompilerSha256",
            "ExpectedInstallerExitCode",
            "ExpectedUninstallerExitCode",
            "RegistrySubKey",
            "RegistryValueName",
        ):
            self.assertEqual(frozen_value(assignment, self.runner), frozen_value(assignment, self.old_runner))
        self.assertEqual(frozen_value("ExpectedInstallerExitCode", self.runner), "37")
        self.assertEqual(frozen_value("ExpectedUninstallerExitCode", self.runner), "42")
        self.assertIn("foreach ($traceMode in @('file', 'detail'))", self.runner)
        self.assertIn("Assert-ProcessExit -Name 'file_installer_abort_expected_exit_37'", self.runner)
        self.assertIn("Assert-ProcessExit -Name 'file_uninstaller_abort_expected_exit_42'", self.runner)
        self.assertIn("Assert-PrimaryFileWriteEvidence -TracePath $evidenceTracePath", self.runner)
        self.assertIn('$uninstallerArguments.Add("_?=$absoluteInstallDirectory")', self.runner)
        self.assertIn("$uninstallerArguments[$uninstallerArguments.Count - 1]", self.runner)
        self.assertIn("Wait-ForTaskOwnedProcessesToExit -TimeoutSeconds 2", self.runner)
        self.assertIn("Remove-Item -LiteralPath $taskRoot -Recurse -Force", self.runner)

    def test_11_frozen_candidate_identity_is_shared_by_report_and_workflow_draft(self) -> None:
        identity_path = ROOT / IDENTITY_PATH
        report_path = ROOT / REPORT_PATH
        draft_path = ROOT / DRAFT_PATH
        self.assertTrue(identity_path.is_file(), "runner-identity.json is required")
        self.assertTrue(report_path.is_file(), "research report is required")
        self.assertTrue(draft_path.is_file(), "inert workflow draft is required")
        identity = json.loads(identity_path.read_text(encoding="utf-8"))
        report = report_path.read_text(encoding="utf-8")
        draft = draft_path.read_text(encoding="utf-8")
        self.assertIn(identity["candidate_freeze_head"], report)
        self.assertIn(identity["candidate_freeze_tree"], report)
        self.assertIn(identity["runner_git_blob"], report)
        self.assertIn(identity["runner_sha256"], report)
        self.assertIn(identity["candidate_freeze_head"], draft)
        self.assertIn(identity["runner_git_blob"], draft)
        self.assertIn(identity["runner_sha256"], draft)
        self.assertIn("research/issue-380-s1r4-p3-runner-null-safety-activation", draft)

    def test_12_preflight_and_runner_define_the_same_clean_status_contract(self) -> None:
        draft = (ROOT / DRAFT_PATH).read_text(encoding="utf-8")
        self.assertIn("OutputCount = $lines.Length", draft)
        self.assertIn("Text = [string]::Join([Environment]::NewLine, $lines)", draft)
        self.assertIn("if ($statusResult.ExitCode -ne 0)", draft)
        self.assertIn("if ($statusResult.OutputCount -ne 0)", draft)
        self.assertIn("if ($capture.stdout_char_count -ne 0)", self.runner)
        self.assertIn("if (-not [string]::IsNullOrEmpty($capture.stderr))", self.runner)
        self.assertIn("if ($capture.timed_out -or $capture.exit_code -ne 0", self.runner)


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(RunnerNullSafetyContracts)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    raise SystemExit(0 if result.wasSuccessful() else 1)
