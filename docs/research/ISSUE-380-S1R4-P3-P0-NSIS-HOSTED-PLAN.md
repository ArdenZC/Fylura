# #380 S1R4-P3-P0 Synthetic NSIS Hosted Plan Preparation

## Status

`#380-S1R4-P3-P0 READY_FOR_OWNER_NSIS_HOSTED_PLAN_REVIEW`

Preparation only. **No Windows Hosted Job, NSIS executable, Probe, installer, or uninstaller was run.** This does not open `NSIS_RUNTIME` or U2 production gates.

## Frozen identities

| Source | HEAD | Tree |
|---|---|---|
| Accepted P2 | `86c21c14dd65bd2bbdb3056513489e0d4139bb47` | `883a851cf8e75b4f39a4a46baea15ab109feb312` |
| Accepted P1 path source | `7a6fa69e5521d9759702c1923a9ade60a75819c3` | `c5465ef93eae268e2c683876d959db90fac5da08` |
| Frozen S0 Probe commit | `1f4951f3c93b2eb549d159fed2dd288992c625c8` | Probe blob `5805f8d04cd37420a63f3e156c3b6fd5b4012f33` |

The Probe file's raw-content SHA-256 is `518b729388e36bbcb99dd8a3d82c60cad48a92660676230495d12e0a3437bade`. The Git blob ID and file SHA-256 are different encodings and are both recorded. The P3 runner verifies the Git blob against S0 and the working file before any tool download.

The P3 branch is `research/issue-380-s1r4-p3-p0-nsis-hosted-plan`, created directly from accepted P2. The P1 path guard and Windows matrix scripts are byte-for-byte unchanged from P1 and P2. The P2 Workflow, accepted 40/40 run evidence, P2 Runner, S0 Probe, and all production files are unchanged.

A static push-trigger audit of the tracked Workflows found no existing push trigger that matches the P3 branch: `ci.yml` is master-only; the earlier S0/S1/S1R/S1R2/S1R3/P2/U1/U1R and resident qualification jobs are each pinned to other branch names. PR-only CI and benchmark Workflows are not triggered by this branch push. No P3 Workflow is installed or active.

## Research files

| File | Purpose |
|---|---|
| [`scripts/issue380S1R4P3SyntheticNsisRunner.ps1`](../../scripts/issue380S1R4P3SyntheticNsisRunner.ps1) | P3-specific future Runner. It does not run in this phase. |
| [`scripts/validateIssue380S1R4P3Preparation.py`](../../scripts/validateIssue380S1R4P3Preparation.py) | Static preparation contract checker, including non-activation of the future Workflow. |
| [`docs/research/evidence/issue-380-s1r4-p3-p0/SHA256SUMS`](evidence/issue-380-s1r4-p3-p0/SHA256SUMS) | Hash manifest for the P3 research Runner, validator, and report. |

No file is added under `.github/workflows`. The following complete future Workflow draft is documentation only.

## Runner's future entry contract

The existing P1 Runner hard-codes `research/issue-380-s1r4-p1-windows-path-guard-preparation`; using it on a later research branch fails before the test. P3 uses a separate Runner with the exact future branch name. It accepts only:

- `GITHUB_REPOSITORY=ArdenZC/Zen-Canvas`, `GITHUB_REF=refs/heads/research/issue-380-s1r4-p3-p0-nsis-hosted-plan`, a nonempty `GITHUB_SHA`, and `GITHUB_RUN_ATTEMPT=1`.
- `RUNNER_OS=Windows`, PowerShell 7, nonempty local-drive `RUNNER_TEMP`, and `PREFLIGHT_OUTCOME=success`.
- A clean checkout where `HEAD == GITHUB_SHA`, the accepted P2 HEAD and Tree match exactly and are ancestors, and the P1 HEAD and Tree match exactly and are ancestors.
- The current path guard and path test blobs equal the frozen P1 files; the S0 Probe Git blob equals the frozen S0 blob.
- An existing OutputRoot exactly at `%RUNNER_TEMP%\issue380-s1r4-p3-evidence`, containing only the current run's `workflow-run.json`, `preflight.json`, empty-or-captured raw Runner stdout/stderr, and `runner-process.json` on entry. These files must be ordinary in-root files; the P3 process waits for the wrapper's running record and proves the recorded launcher PID is its actual parent before continuing. The JSON records must match the current event identity and P2 base.
- An absent `%RUNNER_TEMP%\issue380-s1r4-p3-task` and an absent `HKCU\Software\ZenCanvasS0Harness` key. Existing task data or Registry state is never adopted, overwritten, or deleted.

The shared, accepted P1 path resolver remains the only filesystem path policy. OutputRoot is a separate, exact RUNNER_TEMP child. The private TaskRoot is created only after the pinned ZIP is downloaded and verified by the Runner's fixed URL, SHA-256, archive structure, compiler paths, file lengths, and compiler hashes. The TaskRoot marker binds repository, ref, run ID, attempt, workflow SHA, and canonical path. The Runner refuses recursive cleanup if the marker differs, a process still has an image under TaskRoot, or any reparse point exists below TaskRoot.

The runner first waits for `runner-process.json` to identify the exact PowerShell parent process by PID, image path/SHA-256, event identity, and fixed `-File <P3 Runner> -OutputRoot <owned root>` argument list. It captures each NSIS child image path, PID, argument list, working directory, exit code, timeout state, standard output/error, and image SHA-256. It uses `ProcessStartInfo.ArgumentList`; `_?=<absolute controlled install directory>` remains the last uninstaller argument. NSIS Abort exit assertions remain exactly 37 and 42. After the initial absence check, the runner adds a run-bound owner value and sidecar marker under this synthetic key. Cleanup requires the exact current-run owner token, no unexpected Registry values or subkeys, an expected Probe value if present, and all TaskRoot processes exited. It then deletes only that synthetic key and verifies absence. It removes TaskRoot only after verifying exact ownership and no task process or reparse point, then verifies absence. If any ownership or process fact is unclear, cleanup fails closed and preserves the original failure evidence.

### Fail-fast order

1. Verify run identity, preflight records, checkout identity, frozen P2/P1 ancestry and Tree, P1 file blobs, and S0 Probe blob.
2. Verify the synthetic HKCU key is absent. Only after this does the Runner create its ownership marker.
3. Download the pinned NSIS 3.11 ZIP into TaskRoot, validate its SHA-256 before reading/extracting it, then validate its two pinned compiler entries and extracted files.
4. Run pinned `makensis -VERSION` and `-HDRINFO`; require NSIS 3.11 x86 Unicode.
5. Compile the frozen Probe in `TRACE_MODE=file` for stage and installer Abort. Run stage, require exit 0 and the synthetic DWORD, then run installer Abort and require exit 37.
6. Run the generated uninstaller from its controlled TaskRoot directory with `_?=` last; require exit 42. Verify the synthetic DWORD still has its expected staged value.
7. After both Abort processes exit, require the direct FileWrite trace to exist with exactly one complete installer and uninstaller record, each containing context, predicate, type, expected, actual, and `result=FAIL`. Any failure throws before the optional DetailPrint/`/LOG` pass can begin.
8. Only after the primary file trace passes, run optional DetailPrint/`/LOG` comparison. Its absence is informational and never substitutes for FileWrite evidence.
9. Copy any produced output, raw streams and identity hashes to OutputRoot, verify process exit and HKCU cleanup, remove only the owned TaskRoot, and write result/cleanup records and a final hash manifest.

`Add-Assertion` throws on failure. The FileWrite assertions execute inside the `file` iteration, before the loop can enter `detail`. The static validator checks this call order. This is source-level preparation evidence, not runtime proof.

## Future one-shot Hosted Workflow draft (not activated)

This YAML is embedded only in this report. It must not be copied into `.github/workflows` without a later Owner decision. Its only trigger is the exact P3 branch plus the activating Workflow path. It has one `windows-2022` job, read-only repository permission, a 15-minute job deadline, a 12-minute Runner-step deadline with a 9-minute child-process deadline, and runner-side attempt-1 enforcement. It does not invoke product builds, CI matrices, the P2 test, or production software.

```yaml
name: Issue 380 S1R4-P3 synthetic NSIS trace probe

on:
  push:
    branches:
      - research/issue-380-s1r4-p3-p0-nsis-hosted-plan
    paths:
      - .github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml

permissions:
  contents: read

concurrency:
  group: issue-380-s1r4-p3-synthetic-nsis-${{ github.ref }}
  cancel-in-progress: false

jobs:
  synthetic-nsis:
    name: Frozen S0 synthetic NSIS trace probe
    runs-on: windows-2022
    timeout-minutes: 15
    steps:
      - name: Create private evidence root and record event identity
        id: bootstrap
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          if ($env:GITHUB_REPOSITORY -ne 'ArdenZC/Zen-Canvas') { throw "Unexpected repository: $env:GITHUB_REPOSITORY" }
          if ($env:GITHUB_REF -ne 'refs/heads/research/issue-380-s1r4-p3-p0-nsis-hosted-plan') { throw "Unexpected ref: $env:GITHUB_REF" }
          if ([string]::IsNullOrWhiteSpace($env:GITHUB_RUN_ID) -or [string]::IsNullOrWhiteSpace($env:GITHUB_SHA)) { throw 'GITHUB_RUN_ID and GITHUB_SHA are required.' }
          if ($env:GITHUB_RUN_ATTEMPT -ne '1') { throw "One-shot contract refuses attempt $env:GITHUB_RUN_ATTEMPT" }
          $evidenceRoot = Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-evidence'
          if (Test-Path -LiteralPath $evidenceRoot) { throw "Refusing to reuse evidence root: $evidenceRoot" }
          New-Item -ItemType Directory -Path $evidenceRoot -ErrorAction Stop | Out-Null
          $record = [ordered]@{
            repository = $env:GITHUB_REPOSITORY
            workflow_ref = $env:GITHUB_WORKFLOW_REF
            triggering_ref = $env:GITHUB_REF
            triggering_head = $env:GITHUB_SHA
            run_id = $env:GITHUB_RUN_ID
            run_attempt = $env:GITHUB_RUN_ATTEMPT
            runner_os = $env:RUNNER_OS
            runner_arch = $env:RUNNER_ARCH
            image_os = $env:ImageOS
            image_version = $env:ImageVersion
            powershell = $PSVersionTable.PSVersion.ToString()
            windows_version = [System.Environment]::OSVersion.VersionString
          }
          [System.IO.File]::WriteAllText((Join-Path $evidenceRoot 'workflow-run.json'), ((ConvertTo-Json -InputObject $record -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))

      - name: Checkout the exact activation commit
        if: ${{ steps.bootstrap.outcome == 'success' }}
        uses: actions/checkout@v4
        with:
          fetch-depth: 0
          persist-credentials: false

      - name: Verify P3, P2, P1, path, and HKCU preconditions
        id: preflight
        if: ${{ steps.bootstrap.outcome == 'success' }}
        continue-on-error: true
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          $evidenceRoot = Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-evidence'
          $record = [ordered]@{
            status = 'FAIL'
            expected_p2_head = '86c21c14dd65bd2bbdb3056513489e0d4139bb47'
            expected_p2_tree = '883a851cf8e75b4f39a4a46baea15ab109feb312'
            expected_p1_head = '7a6fa69e5521d9759702c1923a9ade60a75819c3'
            expected_p1_tree = 'c5465ef93eae268e2c683876d959db90fac5da08'
            triggering_ref = $env:GITHUB_REF
            triggering_head = $env:GITHUB_SHA
            checked_out_head = $null
            checked_out_tree = $null
            current_p2_tree = $null
            p1_path_guard_sha256 = $null
            p1_path_test_sha256 = $null
            probe_blob = $null
            runner_temp = $env:RUNNER_TEMP
            task_root = (Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-task')
            output_root = $evidenceRoot
            hkcu_key_initially_absent = $false
            failure = $null
          }
          try {
            if ($env:GITHUB_REPOSITORY -ne 'ArdenZC/Zen-Canvas') { throw "Unexpected repository: $env:GITHUB_REPOSITORY" }
            if ($env:GITHUB_REF -ne 'refs/heads/research/issue-380-s1r4-p3-p0-nsis-hosted-plan') { throw "Unexpected ref: $env:GITHUB_REF" }
            if ($env:GITHUB_RUN_ATTEMPT -ne '1') { throw "One-shot contract refuses attempt $env:GITHUB_RUN_ATTEMPT" }
            if ($env:RUNNER_OS -ne 'Windows' -or $PSVersionTable.PSVersion.Major -ne 7) { throw 'windows-2022 PowerShell 7 is required.' }
            if ([string]::IsNullOrWhiteSpace($env:GITHUB_RUN_ID) -or [string]::IsNullOrWhiteSpace($env:GITHUB_SHA) -or [string]::IsNullOrWhiteSpace($env:GITHUB_WORKSPACE) -or [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { throw 'GITHUB_RUN_ID, GITHUB_SHA, GITHUB_WORKSPACE, and RUNNER_TEMP are required.' }
            $initialEvidence = @(Get-ChildItem -LiteralPath $evidenceRoot -Force -ErrorAction Stop)
            if ($initialEvidence.Count -ne 1 -or $initialEvidence[0].Name -ne 'workflow-run.json') { throw 'Evidence root is not fresh and bootstrap-owned.' }
            if (Test-Path -LiteralPath $record.task_root) { throw "Refusing preexisting TaskRoot: $($record.task_root)" }

            . (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4PathGuard.ps1')
            $runnerTemp = Resolve-Issue380S1R4Path -TaskRoot $env:RUNNER_TEMP -CandidatePath $env:RUNNER_TEMP -Kind WorkingDirectory
            $null = Resolve-Issue380S1R4Path -TaskRoot $runnerTemp -CandidatePath $evidenceRoot -Kind ExistingDirectory
            $expectedTaskRoot = Join-Path $runnerTemp 'issue380-s1r4-p3-task'
            if (-not $record.task_root.Equals($expectedTaskRoot, [System.StringComparison]::OrdinalIgnoreCase)) { throw 'TaskRoot does not match the fixed private child path.' }

            $head = (& git -C $env:GITHUB_WORKSPACE rev-parse HEAD).Trim()
            if ($LASTEXITCODE -ne 0 -or $head -ne $env:GITHUB_SHA) { throw "Checkout HEAD mismatch: checkout=$head event=$env:GITHUB_SHA" }
            $tree = (& git -C $env:GITHUB_WORKSPACE rev-parse 'HEAD^{tree}').Trim()
            if ($LASTEXITCODE -ne 0) { throw 'Could not resolve checkout Tree.' }
            $record.checked_out_head = $head
            $record.checked_out_tree = $tree
            $p2Tree = (& git -C $env:GITHUB_WORKSPACE rev-parse '86c21c14dd65bd2bbdb3056513489e0d4139bb47^{tree}').Trim()
            if ($LASTEXITCODE -ne 0 -or $p2Tree -ne $record.expected_p2_tree) { throw "Frozen P2 Tree mismatch: $p2Tree" }
            $record.current_p2_tree = $p2Tree
            & git -C $env:GITHUB_WORKSPACE merge-base --is-ancestor $record.expected_p2_head HEAD
            if ($LASTEXITCODE -ne 0) { throw 'Frozen P2 HEAD is not an ancestor of the activation commit.' }
            $p1Tree = (& git -C $env:GITHUB_WORKSPACE rev-parse '7a6fa69e5521d9759702c1923a9ade60a75819c3^{tree}').Trim()
            if ($LASTEXITCODE -ne 0 -or $p1Tree -ne $record.expected_p1_tree) { throw "Frozen P1 Tree mismatch: $p1Tree" }
            & git -C $env:GITHUB_WORKSPACE merge-base --is-ancestor $record.expected_p1_head HEAD
            if ($LASTEXITCODE -ne 0) { throw 'Frozen P1 HEAD is not an ancestor of the activation commit.' }
            foreach ($path in @('scripts/issue380S1R4PathGuard.ps1', 'scripts/testIssue380S1R4PathGuard.ps1')) {
              $frozen = (& git -C $env:GITHUB_WORKSPACE rev-parse "7a6fa69e5521d9759702c1923a9ade60a75819c3:$path").Trim()
              $actual = (& git -C $env:GITHUB_WORKSPACE hash-object (Join-Path $env:GITHUB_WORKSPACE $path)).Trim()
              if ($LASTEXITCODE -ne 0 -or $frozen -ne $actual) { throw "Frozen P1 source changed: $path" }
            }
            $probe = (& git -C $env:GITHUB_WORKSPACE rev-parse '1f4951f3c93b2eb549d159fed2dd288992c625c8:scripts/issue380S0NsisObservabilityProbe.nsi').Trim()
            $actualProbe = (& git -C $env:GITHUB_WORKSPACE hash-object (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S0NsisObservabilityProbe.nsi')).Trim()
            if ($LASTEXITCODE -ne 0 -or $probe -ne $actualProbe) { throw 'Frozen S0 Probe blob changed.' }
            $record.probe_blob = $probe
            $record.p1_path_guard_sha256 = (Get-FileHash -LiteralPath (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4PathGuard.ps1') -Algorithm SHA256).Hash.ToLowerInvariant()
            $record.p1_path_test_sha256 = (Get-FileHash -LiteralPath (Join-Path $env:GITHUB_WORKSPACE 'scripts/testIssue380S1R4PathGuard.ps1') -Algorithm SHA256).Hash.ToLowerInvariant()
            if (-not (Test-Path -LiteralPath (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1') -PathType Leaf)) { throw 'P3 Runner is missing.' }

            $registryBase = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, [Microsoft.Win32.RegistryView]::Registry64)
            try {
              $key = $registryBase.OpenSubKey('Software\ZenCanvasS0Harness', $false)
              if ($null -ne $key) { $key.Dispose(); throw 'Synthetic HKCU key already exists; refusing mutation.' }
              $record.hkcu_key_initially_absent = $true
            } finally { $registryBase.Dispose() }
            $record.status = 'PASS'
          } catch {
            $record.failure = $_.Exception.ToString()
          } finally {
            [System.IO.File]::WriteAllText((Join-Path $evidenceRoot 'preflight.json'), ((ConvertTo-Json -InputObject $record -Depth 12) + "`n"), [System.Text.UTF8Encoding]::new($false))
          }
          if ($record.status -ne 'PASS') { throw "P3 preflight failed: $($record.failure)" }

      - name: Run P3 isolated synthetic Runner and capture raw process streams
        id: synthetic_nsis
        if: ${{ always() && steps.preflight.outcome == 'success' }}
        continue-on-error: true
        timeout-minutes: 12
        shell: pwsh
        env:
          PREFLIGHT_OUTCOME: ${{ steps.preflight.outcome }}
        run: |
          $ErrorActionPreference = 'Stop'
          $evidenceRoot = Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-evidence'
          $runner = Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'
          $pwsh = Join-Path $PSHOME 'pwsh.exe'
          $stdoutPath = Join-Path $evidenceRoot 'runner.stdout.raw.txt'
          $stderrPath = Join-Path $evidenceRoot 'runner.stderr.raw.txt'
          $processPath = Join-Path $evidenceRoot 'runner-process.json'
          $arguments = @('-NoLogo', '-NoProfile', '-NonInteractive', '-File', $runner, '-OutputRoot', $evidenceRoot)
          $identity = [ordered]@{
            status = 'NOT_STARTED'
            repository = $env:GITHUB_REPOSITORY
            triggering_ref = $env:GITHUB_REF
            workflow_sha = $env:GITHUB_SHA
            run_id = $env:GITHUB_RUN_ID
            run_attempt = $env:GITHUB_RUN_ATTEMPT
            image = $pwsh
            image_sha256 = $null
            process_id = $null
            arguments = $arguments
            working_directory = $env:GITHUB_WORKSPACE
            started_utc = $null
            finished_utc = $null
            timed_out = $null
            process_exited = $false
            exit_code = $null
            stdout_complete = $false
            stderr_complete = $false
          }
          [System.IO.File]::WriteAllText($stdoutPath, '', [System.Text.UTF8Encoding]::new($false))
          [System.IO.File]::WriteAllText($stderrPath, '', [System.Text.UTF8Encoding]::new($false))
          [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))
          if (Test-Path -LiteralPath $pwsh -PathType Leaf) { $identity.image_sha256 = (Get-FileHash -LiteralPath $pwsh -Algorithm SHA256).Hash.ToLowerInvariant() }
          [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))
          $start = [System.Diagnostics.ProcessStartInfo]::new()
          $start.FileName = $pwsh
          $start.WorkingDirectory = $env:GITHUB_WORKSPACE
          $start.UseShellExecute = $false
          $start.CreateNoWindow = $true
          $start.RedirectStandardOutput = $true
          $start.RedirectStandardError = $true
          $start.Environment['PREFLIGHT_OUTCOME'] = $env:PREFLIGHT_OUTCOME
          foreach ($arg in $arguments) { [void]$start.ArgumentList.Add($arg) }
          $process = [System.Diagnostics.Process]::new()
          $process.StartInfo = $start
          $startedAt = [DateTime]::UtcNow
          if (-not $process.Start()) { throw 'Could not start the P3 Runner process.' }
          $identity.status = 'RUNNING'
          $identity.process_id = $process.Id
          $identity.started_utc = $startedAt.ToString('o')
          [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))
          $stdoutTask = $process.StandardOutput.ReadToEndAsync()
          $stderrTask = $process.StandardError.ReadToEndAsync()
          $completed = $process.WaitForExit(540000)
          if (-not $completed) { try { $process.Kill($true) } catch { }; $process.WaitForExit() }
          $stdoutComplete = $stdoutTask.Wait(5000)
          $stderrComplete = $stderrTask.Wait(5000)
          $stdout = if ($stdoutComplete) { $stdoutTask.GetAwaiter().GetResult() } else { '[stdout did not close within 5 seconds]' }
          $stderr = if ($stderrComplete) { $stderrTask.GetAwaiter().GetResult() } else { '[stderr did not close within 5 seconds]' }
          [System.IO.File]::WriteAllText($stdoutPath, $stdout, [System.Text.UTF8Encoding]::new($false))
          [System.IO.File]::WriteAllText($stderrPath, $stderr, [System.Text.UTF8Encoding]::new($false))
          $exitCode = if ($process.HasExited) { [int]$process.ExitCode } else { $null }
          $identity.status = if ($completed) { 'COMPLETED' } else { 'TIMED_OUT_TREE_KILLED' }
          $identity.finished_utc = [DateTime]::UtcNow.ToString('o')
          $identity.timed_out = (-not $completed)
          $identity.process_exited = $process.HasExited
          $identity.exit_code = $exitCode
          $identity.stdout_complete = $stdoutComplete
          $identity.stderr_complete = $stderrComplete
          [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))
          $process.Dispose()
          if (-not $completed -or $exitCode -ne 0 -or -not $stdoutComplete -or -not $stderrComplete) { throw "P3 Runner failed or timed out: exit=$exitCode timeout=$(-not $completed)" }

      - name: Finalize evidence and verify cleanup (fail closed)
        id: finalize
        if: always()
        continue-on-error: true
        shell: pwsh
        env:
          PREFLIGHT_OUTCOME: ${{ steps.preflight.outcome }}
          RUNNER_OUTCOME: ${{ steps.synthetic_nsis.outcome }}
        run: |
          $ErrorActionPreference = 'Stop'
          $evidenceRoot = Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-evidence'
          if (-not (Test-Path -LiteralPath $evidenceRoot -PathType Container)) { throw 'Owned evidence root is missing; Actions log is the only available failure record.' }
          . (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4PathGuard.ps1')
          $null = Resolve-Issue380S1R4Path -TaskRoot $env:RUNNER_TEMP -CandidatePath $evidenceRoot -Kind ExistingDirectory
          $workflowRunPath = Join-Path $evidenceRoot 'workflow-run.json'
          $null = Resolve-Issue380S1R4Path -TaskRoot $env:RUNNER_TEMP -CandidatePath $workflowRunPath -Kind ExistingFile
          if (-not (Test-Path -LiteralPath $workflowRunPath -PathType Leaf)) { throw 'Evidence root has no bootstrap ownership record; refusing to modify it.' }
          $workflowRun = Get-Content -LiteralPath $workflowRunPath -Raw | ConvertFrom-Json
          if ($workflowRun.repository -ne $env:GITHUB_REPOSITORY -or $workflowRun.triggering_ref -ne $env:GITHUB_REF -or $workflowRun.triggering_head -ne $env:GITHUB_SHA -or $workflowRun.run_id -ne $env:GITHUB_RUN_ID -or $workflowRun.run_attempt -ne $env:GITHUB_RUN_ATTEMPT) { throw 'Evidence root belongs to a different run; preserving it.' }
          $evidenceReparse = @(Get-ChildItem -LiteralPath $evidenceRoot -Force -Recurse -ErrorAction Stop | Where-Object { ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 })
          if ($evidenceReparse.Count -ne 0) { throw "Evidence root contains reparse points; refusing recursive hashing: $($evidenceReparse.FullName -join ', ')" }
          $assessment = [ordered]@{
            status = 'FAIL_CLOSED'
            preflight_outcome = $env:PREFLIGHT_OUTCOME
            runner_outcome = $env:RUNNER_OUTCOME
            workflow_sha = $env:GITHUB_SHA
            run_id = $env:GITHUB_RUN_ID
            run_attempt = $env:GITHUB_RUN_ATTEMPT
            runner_exit_code = $null
            runner_timed_out = $null
            runner_result_status = $null
            required_primary_assertions_pass = $false
            hkcu_absent_after_run = $false
            task_root_absent_after_run = $false
            task_root_cleanup = 'NOT_CHECKED'
            task_owned_processes = @()
            failure = $null
          }
          try {
            $processPath = Join-Path $evidenceRoot 'runner-process.json'
            $resultPath = Join-Path $evidenceRoot 'results.json'
            $processRecord = if (Test-Path -LiteralPath $processPath -PathType Leaf) { Get-Content -LiteralPath $processPath -Raw | ConvertFrom-Json } else { $null }
            $runnerResult = if (Test-Path -LiteralPath $resultPath -PathType Leaf) { Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json } else { $null }
            if ($null -ne $processRecord) { $assessment.runner_exit_code = $processRecord.exit_code; $assessment.runner_timed_out = $processRecord.timed_out }
            if ($null -ne $runnerResult) { $assessment.runner_result_status = $runnerResult.status }
            $requiredNames = @('makensis_version_is_exactly_3_11', 'makensis_is_x86_unicode', 'file_installer_abort_expected_exit_37', 'file_uninstaller_abort_expected_exit_42', 'file_direct_trace_exists_after_both_abort_processes_exit', 'file_trace_installer_record_complete', 'file_trace_uninstaller_record_complete', 'file_trace_has_exactly_two_abort_records')
            $assertions = if ($null -ne $runnerResult) { @($runnerResult.assertions) } else { @() }
            $assessment.required_primary_assertions_pass = ($null -ne $runnerResult) -and (@($assertions | Where-Object { $_.name -in $requiredNames -and $_.passed -eq $true }).Count -eq $requiredNames.Count)
            $taskRoot = Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-task'
            . (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4PathGuard.ps1')
            $taskRootPrefix = [System.IO.Path]::GetFullPath($taskRoot).TrimEnd([char[]]@('\', '/')) + '\'
            $taskOwnedProcesses = [System.Collections.Generic.List[object]]::new()
            foreach ($processInfo in (Get-CimInstance -ClassName Win32_Process -Property @('ProcessId', 'ExecutablePath') -ErrorAction Stop)) {
              if ([string]::IsNullOrWhiteSpace([string]$processInfo.ExecutablePath)) { continue }
              $imagePath = [System.IO.Path]::GetFullPath([string]$processInfo.ExecutablePath)
              if ($imagePath.StartsWith($taskRootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                try { $validImage = Resolve-Issue380S1R4Path -TaskRoot $taskRoot -CandidatePath $imagePath -Kind Executable }
                catch { throw "Task process image could not be validated: pid=$($processInfo.ProcessId) image=$imagePath $($_.Exception.Message)" }
                $taskOwnedProcesses.Add([ordered]@{ process_id = [int]$processInfo.ProcessId; image_path = $validImage })
              }
            }
            $assessment.task_owned_processes = @($taskOwnedProcesses.ToArray())
            if ($taskOwnedProcesses.Count -ne 0) { throw 'Task-owned process remains; preserving Registry and TaskRoot.' }

            if (Test-Path -LiteralPath $taskRoot) {
              $expectedTaskMarker = "repository=$env:GITHUB_REPOSITORY`nref=$env:GITHUB_REF`nrun_id=$env:GITHUB_RUN_ID`nattempt=$env:GITHUB_RUN_ATTEMPT`nhead=$env:GITHUB_SHA`nroot=$taskRoot`n"
              $taskMarker = Join-Path $taskRoot 's1r4-p3-task-owned.txt'
              if (-not (Test-Path -LiteralPath $taskMarker -PathType Leaf) -or [System.IO.File]::ReadAllText($taskMarker) -cne $expectedTaskMarker) { throw 'TaskRoot owner marker is missing or mismatched; preserving it.' }
              $null = Resolve-Issue380S1R4Path -TaskRoot $env:RUNNER_TEMP -CandidatePath $taskRoot -Kind ExistingDirectory
              $reparseEntries = @(Get-ChildItem -LiteralPath $taskRoot -Force -Recurse -ErrorAction Stop | Where-Object { ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 })
              if ($reparseEntries.Count -ne 0) { throw "TaskRoot contains reparse points; preserving it: $($reparseEntries.FullName -join ', ')" }
              Remove-Item -LiteralPath $taskRoot -Recurse -Force -ErrorAction Stop
              if (Test-Path -LiteralPath $taskRoot) { throw 'TaskRoot remained after owned cleanup.' }
              $assessment.task_root_cleanup = 'FINALIZER_REMOVED_VERIFIED_OWNED_TASKROOT'
            } else {
              $assessment.task_root_cleanup = 'ALREADY_ABSENT'
            }
            $assessment.task_root_absent_after_run = -not (Test-Path -LiteralPath $taskRoot)

            $preflightPath = Join-Path $evidenceRoot 'preflight.json'
            $preflightRecord = if (Test-Path -LiteralPath $preflightPath -PathType Leaf) { Get-Content -LiteralPath $preflightPath -Raw | ConvertFrom-Json } else { $null }
            $registryBase = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, [Microsoft.Win32.RegistryView]::Registry64)
            try {
              $key = $registryBase.OpenSubKey('Software\ZenCanvasS0Harness', $false)
              if ($null -ne $key) {
                try {
                  $mayClean = ($null -ne $preflightRecord) -and $preflightRecord.status -eq 'PASS' -and $preflightRecord.hkcu_key_initially_absent -eq $true
                  $ownerToken = "$env:GITHUB_REPOSITORY|$env:GITHUB_REF|$env:GITHUB_RUN_ID|$env:GITHUB_RUN_ATTEMPT|$env:GITHUB_SHA"
                  $ownerMarker = Join-Path $evidenceRoot 'synthetic-key-owned.txt'
                  $expectedOwnerMarker = "key=HKCU\Software\ZenCanvasS0Harness`nowner_token=$ownerToken`n"
                  $valueNames = @($key.GetValueNames())
                  $subkeyNames = @($key.GetSubKeyNames())
                  $ownerKindOkay = ($valueNames -contains 'P3HarnessOwner') -and $key.GetValueKind('P3HarnessOwner') -eq [Microsoft.Win32.RegistryValueKind]::String
                  $ownerValue = if ($ownerKindOkay) { [string]$key.GetValue('P3HarnessOwner', $null) } else { $null }
                  $unexpectedValues = @($valueNames | Where-Object { $_ -notin @('P3HarnessOwner', 'EstimatedSize') })
                  $estimatedStateOkay = if ($valueNames -contains 'EstimatedSize') { $key.GetValueKind('EstimatedSize') -eq [Microsoft.Win32.RegistryValueKind]::DWord -and [int]$key.GetValue('EstimatedSize', $null) -eq 35149 } else { $true }
                  if (-not $mayClean -or -not (Test-Path -LiteralPath $ownerMarker -PathType Leaf) -or [System.IO.File]::ReadAllText($ownerMarker) -cne $expectedOwnerMarker -or -not $ownerKindOkay -or $ownerValue -cne $ownerToken -or $unexpectedValues.Count -ne 0 -or $subkeyNames.Count -ne 0 -or -not $estimatedStateOkay) {
                    throw 'HKCU key remains but exact P3 ownership and expected synthetic state are not proven; preserving it.'
                  }
                } finally { $key.Dispose() }
                $registryBase.DeleteSubKeyTree('Software\ZenCanvasS0Harness', $false)
              }
              $remainingKey = $registryBase.OpenSubKey('Software\ZenCanvasS0Harness', $false)
              $assessment.hkcu_absent_after_run = ($null -eq $remainingKey)
              if ($null -ne $remainingKey) { $remainingKey.Dispose() }
            } finally { $registryBase.Dispose() }
            if ($env:PREFLIGHT_OUTCOME -ne 'success' -or $env:RUNNER_OUTCOME -ne 'success' -or $null -eq $processRecord -or $processRecord.exit_code -ne 0 -or $processRecord.timed_out -or $null -eq $runnerResult -or $runnerResult.status -ne 'PASS' -or -not $assessment.required_primary_assertions_pass -or -not $assessment.hkcu_absent_after_run -or -not $assessment.task_root_absent_after_run) {
              throw 'One or more P3 fail-closed evidence or cleanup conditions failed.'
            }
            $assessment.status = 'SYNTHETIC_TRACE_PERSISTENCE_PROVEN'
          } catch {
            $assessment.failure = $_.Exception.ToString()
          }
          [System.IO.File]::WriteAllText((Join-Path $evidenceRoot 'assessment.json'), ((ConvertTo-Json -InputObject $assessment -Depth 12) + "`n"), [System.Text.UTF8Encoding]::new($false))
          $manifest = Join-Path $evidenceRoot 'SHA256SUMS'
          $lines = @(Get-ChildItem -LiteralPath $evidenceRoot -Recurse -File | Where-Object { $_.FullName -ne $manifest } | Sort-Object FullName | ForEach-Object { $relative = $_.FullName.Substring($evidenceRoot.Length).TrimStart([char[]]@('\', '/')).Replace('\', '/'); "$( (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $relative" })
          [System.IO.File]::WriteAllLines($manifest, [string[]]$lines, [System.Text.UTF8Encoding]::new($false))

      - name: Upload raw evidence even when a gate fails
        if: ${{ always() && steps.bootstrap.outcome == 'success' && steps.finalize.outcome == 'success' }}
        uses: actions/upload-artifact@v4
        with:
          name: issue380-s1r4-p3-${{ github.run_id }}-attempt-${{ github.run_attempt }}
          path: ${{ runner.temp }}/issue380-s1r4-p3-evidence/
          if-no-files-found: warn
          retention-days: 14

      - name: Enforce one-shot synthetic result
        if: always()
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          $assessmentPath = Join-Path (Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-evidence') 'assessment.json'
          if (-not (Test-Path -LiteralPath $assessmentPath -PathType Leaf)) { throw 'P3 assessment is missing; refusing PASS.' }
          $assessment = Get-Content -LiteralPath $assessmentPath -Raw | ConvertFrom-Json
          if ($assessment.status -ne 'SYNTHETIC_TRACE_PERSISTENCE_PROVEN' -or $assessment.run_id -ne $env:GITHUB_RUN_ID -or $assessment.run_attempt -ne $env:GITHUB_RUN_ATTEMPT -or $assessment.workflow_sha -ne $env:GITHUB_SHA) { throw "P3 synthetic NSIS result failed closed or belongs to another run: $($assessment.failure)" }
          Write-Host 'SYNTHETIC_TRACE_PERSISTENCE_PROVEN'
```

## Deterministic precondition review

| Contract | P3 source / future Workflow check | Static status |
|---|---|---|
| `GITHUB_REF`, branch, repository | Exact P3 branch/repository in draft bootstrap, preflight, and Runner. | VERIFIED_IN_SOURCE |
| `GITHUB_SHA` | Bootstrap captures the event SHA; preflight and Runner require `HEAD == GITHUB_SHA`. | VERIFIED_IN_SOURCE |
| `GITHUB_RUN_ATTEMPT` | Draft and Runner both require `1`; retry does not reach NSIS. | VERIFIED_IN_SOURCE |
| `PREFLIGHT_OUTCOME` | Runner requires `success`; draft passes the step outcome and requires a matching PASS JSON. | VERIFIED_IN_SOURCE |
| Git HEAD / Tree / ancestry | P2 HEAD/Tree and P1 HEAD/Tree fixed; P1 path guard/test blobs and S0 Probe blob compared exactly. | VERIFIED_IN_SOURCE |
| `RUNNER_TEMP` / TaskRoot / OutputRoot | Fixed P3 child paths; P1 helper validates local path containment and reparse points; pre-existing TaskRoot is rejected; the five wrapper inputs at OutputRoot are explicitly allowlisted and path-validated. | VERIFIED_IN_SOURCE |
| ownership markers / pre-existing directory | OutputRoot begins with only current `workflow-run.json` and `preflight.json`; TaskRoot must be absent and marker binds current run identity. | VERIFIED_IN_SOURCE |
| HKCU synthetic key | Preflight and Runner both require absence before mutation; cleanup requires ownership and exited processes. | VERIFIED_IN_SOURCE |
| process launch / cleanup | `ArgumentList`, finite waits, kill-tree timeout, image/PID/args/output capture, task path snapshots, fail-closed cleanup. | VERIFIED_IN_SOURCE |
| primary FileWrite before Detail | File trace completeness is asserted before the Runner advances to `detail`; assertion failure throws. | VERIFIED_IN_SOURCE |
| failure evidence | Runner preserves partial compiler/log outputs; future Workflow captures raw Runner stdout/stderr and process identity, then uploads with `always()`. | VERIFIED_IN_SOURCE |
| no activation | YAML exists only inside this Markdown file; `.github/workflows` has no P3 file. | VERIFIED_IN_SOURCE |
| existing push trigger audit | All tracked Workflow push filters are branch-scoped; none matches the P3 branch. | VERIFIED_IN_SOURCE |

The static contract checker parses the embedded YAML with PyYAML and parses both the P3 Runner/P1 path scripts and every embedded PowerShell Workflow step with the Tree-sitter PowerShell grammar. The local static run used Python 3.12.14, PyYAML 6.0.3, Tree-sitter 0.25.2, and Tree-sitter PowerShell 0.26.4; the YAML parsed as one job, and the checked PowerShell source blocks had zero syntax error/missing nodes. It also verifies frozen Git objects/blob identities, enforces the changed-file allowlist, verifies source order for primary FileWrite assertions, and checks fail-closed cleanup markers. This does not execute PowerShell, download the ZIP, run makensis, touch HKCU, launch any process, or validate GitHub's hosted runtime behavior.

## Planned primary result assertions

The one future Hosted run should count as a pass only if all of these are true: correct P3/P2/P1/S0 identities; exact pinned NSIS ZIP and compiler identities; `-VERSION` identifies 3.11; `-HDRINFO` identifies x86 Unicode; frozen Probe compile outputs are nonempty; stage returns 0 and writes the expected synthetic DWORD; installer returns exactly 37; uninstaller uses a controlled absolute `_?=` argument last and returns exactly 42; the direct FileWrite log survives both process exits and contains exactly the two complete records; no child remains; the HKCU key and owned TaskRoot are absent after cleanup; raw streams, compiler outputs, generated uninstallers, logs, and SHA manifest are present. A critical failure stops before optional Detail mode and the final assessment remains fail-closed.

## Not run / remaining risks

- No P2 Windows job was rerun; accepted Run `38258879642` remains untouched.
- No P3 Hosted Workflow or NSIS job was started. The embedded YAML is not executable.
- No PowerShell runtime exists in the current Linux environment; Tree-sitter parsing is static syntax evidence only.
- SourceForge availability, GitHub Windows image contents, Windows Registry and filesystem behavior, NSIS forwarding launcher behavior, Abort persistence, process tree termination, and cleanup remain unverified runtime behavior.
- GitHub Actions expression scheduling, artifact behavior, Windows runner image timing, and network download outcome cannot be proved offline.
- This deliverable establishes a reviewed plan only. It neither establishes synthetic runtime persistence nor authorizes a product installer test, U2 implementation, Schema/IPC change, or production fix.
