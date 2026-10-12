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
NULL_SAFETY_FREEZE_HEAD = "bfabd486bce84b7e5298276453dd01957049c91d"
BOUNDED_GIT_FREEZE_HEAD = "6e0c108aa5beeadceea531e7632d3667f18f3890"
BOUNDED_GIT_FREEZE_TREE = "1a566533f6cf6f3e1132d13cc4205ffa9a50cbe2"
BOUNDED_GIT_RUNNER_BLOB = "c413395bb029956b264cbd91189a9ac093e64a85"
BOUNDED_GIT_RUNNER_SHA256 = "fe9b62c1de6550a88fe74c1e2f09f72d178d652086f51f44b5b35c506e45d302"
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
BOUNDED_REPORT_PATH = "docs/research/ISSUE-380-S1R4-P3-BOUNDED-GIT-CAPTURE.md"
BOUNDED_DRAFT_PATH = "docs/research/ISSUE-380-S1R4-P3-BOUNDED-GIT-CAPTURE-WORKFLOW.md"
BOUNDED_IDENTITY_PATH = "docs/research/evidence/issue-380-s1r4-p3-bounded-git-capture/runner-identity.json"
BOUNDED_SUMS_PATH = "docs/research/evidence/issue-380-s1r4-p3-bounded-git-capture/SHA256SUMS"


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
        self.assertIn("$capture.timed_out -or $capture.output_timed_out -or $capture.exit_code -ne 0", self.runner)


@dataclass(frozen=True)
class BoundedCaptureModel:
    """Specification model only; it does not execute the PowerShell Runner."""

    exit_code: int | None
    process_exited_before_deadline: bool = True
    kill_succeeded: bool = True
    process_exit_confirmed_after_kill: bool = True
    stdout: str = ""
    stderr: str = ""
    stdout_delay_ms: int = 0
    stderr_delay_ms: int = 0
    output_deadline_ms: int = 5000
    stdout_fault: bool = False
    stderr_fault: bool = False
    cleanup_succeeded: bool = True
    post_close_tasks_stopped: bool = True


def classify_bounded_capture(model: BoundedCaptureModel) -> dict[str, object]:
    """Pure Python contract model used to exercise expected fail-closed states."""

    timed_out = not model.process_exited_before_deadline
    process_exit_confirmed = model.process_exited_before_deadline
    if timed_out:
        process_exit_confirmed = model.kill_succeeded and model.process_exit_confirmed_after_kill

    stdout_complete = model.stdout_delay_ms <= model.output_deadline_ms and not model.stdout_fault
    stderr_complete = model.stderr_delay_ms <= model.output_deadline_ms and not model.stderr_fault
    output_timed_out = (
        (not stdout_complete and not model.stdout_fault)
        or (not stderr_complete and not model.stderr_fault)
    )

    cleanup_completed = (
        model.cleanup_succeeded
        and process_exit_confirmed
        and (not output_timed_out or model.post_close_tasks_stopped)
    )
    if not cleanup_completed:
        status = "PROCESS_CLEANUP_FAILED"
    elif timed_out:
        status = "PROCESS_TIMEOUT"
    elif output_timed_out:
        status = "OUTPUT_CAPTURE_TIMEOUT"
    elif model.stdout_fault or model.stderr_fault or not stdout_complete or not stderr_complete:
        status = "OUTPUT_CAPTURE_FAILED"
    elif not process_exit_confirmed:
        status = "PROCESS_WAIT_FAILED"
    elif model.exit_code == 0:
        status = "EXITED_ZERO"
    else:
        status = "EXITED_NONZERO"

    accepted = (
        status == "EXITED_ZERO"
        and stdout_complete
        and stderr_complete
        and process_exit_confirmed
        and cleanup_completed
    )
    return {
        "status": status,
        "accepted": accepted,
        "timed_out": timed_out,
        "process_exit_confirmed": process_exit_confirmed,
        "stdout_complete": stdout_complete and not output_timed_out,
        "stderr_complete": stderr_complete and not output_timed_out,
        "output_timed_out": output_timed_out,
        "kill_succeeded": model.kill_succeeded if timed_out else False,
        "cleanup_completed": cleanup_completed,
    }


class BoundedGitCaptureContracts(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.runner = (ROOT / RUNNER_PATH).read_text(encoding="utf-8")
        cls.freeze_runner = git_show(NULL_SAFETY_FREEZE_HEAD, RUNNER_PATH).decode("utf-8")
        start = cls.runner.index("function Invoke-GitCapture(")
        end = cls.runner.index("\nfunction Get-RequiredGitScalar(", start)
        cls.capture_source = cls.runner[start:end]

    def test_13_normal_exit_drains_stdout_and_stderr_concurrently(self) -> None:
        result = classify_bounded_capture(
            BoundedCaptureModel(exit_code=0, stdout="head\n", stderr="")
        )
        self.assertEqual(result["status"], "EXITED_ZERO")
        self.assertTrue(result["accepted"])
        process_wait = self.capture_source.index("$process.WaitForExit($TimeoutSeconds * 1000)")
        self.assertLess(self.capture_source.index("$stdoutTask = $process.StandardOutput.ReadToEndAsync()"), process_wait)
        self.assertLess(self.capture_source.index("$stderrTask = $process.StandardError.ReadToEndAsync()"), process_wait)

    def test_14_empty_stdout_with_zero_exit_is_accepted(self) -> None:
        result = classify_bounded_capture(BoundedCaptureModel(exit_code=0, stdout="", stderr=""))
        self.assertEqual(result["status"], "EXITED_ZERO")
        self.assertTrue(result["accepted"])

    def test_15_delayed_streams_complete_when_they_finish_inside_the_bound(self) -> None:
        for model in (
            BoundedCaptureModel(exit_code=0, stdout_delay_ms=1800),
            BoundedCaptureModel(exit_code=0, stderr_delay_ms=2300),
            BoundedCaptureModel(exit_code=0, stdout_delay_ms=1800, stderr_delay_ms=2300),
        ):
            with self.subTest(stdout_delay_ms=model.stdout_delay_ms, stderr_delay_ms=model.stderr_delay_ms):
                result = classify_bounded_capture(model)
                self.assertEqual(result["status"], "EXITED_ZERO")
                self.assertTrue(result["stdout_complete"])
                self.assertTrue(result["stderr_complete"])
                self.assertTrue(result["accepted"])

    def test_16_process_timeout_and_successful_tree_termination_still_fail_closed(self) -> None:
        result = classify_bounded_capture(
            BoundedCaptureModel(
                exit_code=137,
                process_exited_before_deadline=False,
                kill_succeeded=True,
                process_exit_confirmed_after_kill=True,
            )
        )
        self.assertEqual(result["status"], "PROCESS_TIMEOUT")
        self.assertTrue(result["process_exit_confirmed"])
        self.assertTrue(result["kill_succeeded"])
        self.assertFalse(result["accepted"])

    def test_17_exited_process_with_output_stream_past_deadline_fails_closed(self) -> None:
        for model, expected_status in (
            (BoundedCaptureModel(exit_code=0, stdout_delay_ms=5001), "OUTPUT_CAPTURE_TIMEOUT"),
            (
                BoundedCaptureModel(exit_code=0, stderr_delay_ms=5001, post_close_tasks_stopped=False),
                "PROCESS_CLEANUP_FAILED",
            ),
        ):
            with self.subTest(stdout_delay_ms=model.stdout_delay_ms, stderr_delay_ms=model.stderr_delay_ms):
                result = classify_bounded_capture(model)
                self.assertEqual(result["status"], expected_status)
                self.assertTrue(result["output_timed_out"])
                self.assertFalse(result["stdout_complete"] or result["stderr_complete"])
                self.assertFalse(result["accepted"])
        self.assertIn("$capture.post_close_wait_timeout_ms", self.capture_source)
        self.assertIn("$capture.stream_close_attempted = $true", self.capture_source)

    def test_18_kill_and_resource_cleanup_failures_are_not_accepted(self) -> None:
        kill_failure = classify_bounded_capture(
            BoundedCaptureModel(
                exit_code=None,
                process_exited_before_deadline=False,
                kill_succeeded=False,
                process_exit_confirmed_after_kill=False,
            )
        )
        cleanup_failure = classify_bounded_capture(
            BoundedCaptureModel(exit_code=0, cleanup_succeeded=False)
        )
        self.assertEqual(kill_failure["status"], "PROCESS_CLEANUP_FAILED")
        self.assertFalse(kill_failure["accepted"])
        self.assertEqual(cleanup_failure["status"], "PROCESS_CLEANUP_FAILED")
        self.assertFalse(cleanup_failure["accepted"])
        self.assertIn("$capture.kill_error", self.capture_source)
        self.assertIn("$capture.cleanup_error", self.capture_source)

    def test_19_sequential_captures_keep_exit_and_error_records_independent(self) -> None:
        first = classify_bounded_capture(BoundedCaptureModel(exit_code=128, stderr="fatal"))
        second = classify_bounded_capture(BoundedCaptureModel(exit_code=0, stdout="tree\n"))
        self.assertEqual(first["status"], "EXITED_NONZERO")
        self.assertEqual(second["status"], "EXITED_ZERO")
        self.assertTrue(second["accepted"])
        self.assertIn("$script:GitInvocations.Add([ordered]@{", self.capture_source)
        self.assertIn("exit_code = $capture.exit_code", self.capture_source)
        self.assertIn("kill_attempted = $capture.kill_attempted", self.capture_source)
        self.assertIn("process_timeout_ms = $capture.process_timeout_ms", self.capture_source)
        self.assertIn("termination_timeout_ms = $capture.termination_timeout_ms", self.capture_source)
        self.assertNotIn("$LASTEXITCODE", self.runner)

    def test_20_output_waits_are_bounded_and_result_reads_are_completion_guarded(self) -> None:
        self.assertIn("Task]::WaitAll(", self.capture_source)
        self.assertIn("$capture.output_wait_timeout_ms", self.capture_source)
        self.assertIn("$capture.post_close_wait_timeout_ms", self.capture_source)
        self.assertIn("WaitAll($readTasks.ToArray(), $capture.output_wait_timeout_ms)", self.capture_source)
        self.assertIn("WaitAll($readTasks.ToArray(), $capture.post_close_wait_timeout_ms)", self.capture_source)
        self.assertIn("$stream.Task.GetAwaiter().GetResult()", self.capture_source)
        self.assertNotRegex(self.capture_source, r"\.WaitForExit\(\s*\)")
        read_guard = self.capture_source.index("if ($null -eq $stream.Task -or -not $stream.Task.IsCompleted) { continue }")
        result_read = self.capture_source.index("$stream.Task.GetAwaiter().GetResult()")
        self.assertLess(read_guard, result_read)
        for gate in (
            "Get-RequiredGitScalar",
            "Assert-GitNoOutputSuccess",
            "Assert-GitWorktreeClean",
        ):
            block_start = self.runner.index(f"function {gate}(")
            next_function = self.runner.find("\nfunction ", block_start + 1)
            block = self.runner[block_start:next_function if next_function >= 0 else len(self.runner)]
            self.assertIn("-not $capture.stdout_complete", block)
            self.assertIn("-not $capture.stderr_complete", block)
            self.assertIn("$capture.output_timed_out", block)

    def test_21_non_git_process_and_nsis_sections_are_unchanged(self) -> None:
        def function_segment(source: str, start_marker: str, end_marker: str) -> str:
            start = source.index(start_marker)
            end = source.index(end_marker, start)
            return source[start:end]

        git_section_start = self.runner.index("function Invoke-GitCapture(")
        git_section_end = self.runner.index("function Add-Assertion(", git_section_start)
        freeze_section_start = self.freeze_runner.index("function Invoke-GitCapture(")
        freeze_section_end = self.freeze_runner.index("function Add-Assertion(", freeze_section_start)
        reconstructed = (
            self.runner[:git_section_start]
            + self.freeze_runner[freeze_section_start:freeze_section_end]
            + self.runner[git_section_end:]
        )
        self.assertEqual(reconstructed, self.freeze_runner)
        self.assertEqual(
            function_segment(self.runner, "function Invoke-CapturedProcess(", "\nfunction Get-SyntheticRegistrySnapshot"),
            function_segment(self.freeze_runner, "function Invoke-CapturedProcess(", "\nfunction Get-SyntheticRegistrySnapshot"),
        )
        self.assertEqual(
            function_segment(self.runner, "function Download-And-VerifyPinnedZip", "\nfunction Get-ZipEntrySha256"),
            function_segment(self.freeze_runner, "function Download-And-VerifyPinnedZip", "\nfunction Get-ZipEntrySha256"),
        )

    def test_22_bounded_capture_freeze_identity_is_exact_in_git_objects(self) -> None:
        self.assertEqual(
            subprocess.run(
                ["git", "rev-parse", f"{BOUNDED_GIT_FREEZE_HEAD}^{{tree}}"],
                cwd=ROOT,
                check=True,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            ).stdout.strip(),
            BOUNDED_GIT_FREEZE_TREE,
        )
        self.assertEqual(git_blob(BOUNDED_GIT_FREEZE_HEAD, RUNNER_PATH), BOUNDED_GIT_RUNNER_BLOB)
        frozen_bytes = git_show(BOUNDED_GIT_FREEZE_HEAD, RUNNER_PATH)
        self.assertEqual(hashlib.sha256(frozen_bytes).hexdigest(), BOUNDED_GIT_RUNNER_SHA256)
        self.assertEqual((ROOT / RUNNER_PATH).read_bytes(), frozen_bytes)

    def test_23_new_report_draft_and_machine_identity_agree(self) -> None:
        report_path = ROOT / BOUNDED_REPORT_PATH
        draft_path = ROOT / BOUNDED_DRAFT_PATH
        identity_path = ROOT / BOUNDED_IDENTITY_PATH
        self.assertTrue(report_path.is_file(), "bounded capture report is required")
        self.assertTrue(draft_path.is_file(), "bounded capture workflow draft is required")
        self.assertTrue(identity_path.is_file(), "bounded capture identity record is required")
        identity = json.loads(identity_path.read_text(encoding="utf-8"))
        report = report_path.read_text(encoding="utf-8")
        draft = draft_path.read_text(encoding="utf-8")
        for value in (
            BOUNDED_GIT_FREEZE_HEAD,
            BOUNDED_GIT_FREEZE_TREE,
            BOUNDED_GIT_RUNNER_BLOB,
            BOUNDED_GIT_RUNNER_SHA256,
        ):
            self.assertIn(value, report)
            self.assertIn(value, draft)
        self.assertEqual(identity["candidate_freeze_head"], BOUNDED_GIT_FREEZE_HEAD)
        self.assertEqual(identity["candidate_freeze_tree"], BOUNDED_GIT_FREEZE_TREE)
        self.assertEqual(identity["runner_git_blob"], BOUNDED_GIT_RUNNER_BLOB)
        self.assertEqual(identity["runner_sha256"], BOUNDED_GIT_RUNNER_SHA256)
        activation_branch = identity["planned_activation"]["branch"]
        activation_workflow = identity["planned_activation"]["workflow_path"]
        self.assertIn(f"$script:ExpectedBranch = '{activation_branch}'", self.runner)
        self.assertIn(activation_branch, draft)
        self.assertIn(activation_workflow, draft)

    def test_24_new_evidence_hashes_match_and_old_research_package_is_immutable(self) -> None:
        sums_path = ROOT / BOUNDED_SUMS_PATH
        self.assertTrue(sums_path.is_file(), "bounded capture SHA256SUMS is required")
        entries: dict[str, str] = {}
        for line in sums_path.read_text(encoding="utf-8").splitlines():
            digest, relative = line.split("  ", 1)
            entries[relative] = digest
        for relative in (BOUNDED_REPORT_PATH, BOUNDED_DRAFT_PATH, BOUNDED_IDENTITY_PATH):
            self.assertEqual(hashlib.sha256((ROOT / relative).read_bytes()).hexdigest(), entries[relative])
        for relative, expected_digest in entries.items():
            artifact = ROOT / relative
            self.assertTrue(artifact.is_file(), f"SHA256SUMS names a missing file: {relative}")
            self.assertEqual(hashlib.sha256(artifact.read_bytes()).hexdigest(), expected_digest, relative)
        for relative in (REPORT_PATH, DRAFT_PATH, IDENTITY_PATH, "docs/research/evidence/issue-380-s1r4-p3-runner-null-safety/SHA256SUMS"):
            self.assertEqual((ROOT / relative).read_bytes(), git_show(NULL_SAFETY_FREEZE_HEAD, relative))


if __name__ == "__main__":
    suite = unittest.TestSuite(
        (
            unittest.defaultTestLoader.loadTestsFromTestCase(RunnerNullSafetyContracts),
            unittest.defaultTestLoader.loadTestsFromTestCase(BoundedGitCaptureContracts),
        )
    )
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    raise SystemExit(0 if result.wasSuccessful() else 1)
