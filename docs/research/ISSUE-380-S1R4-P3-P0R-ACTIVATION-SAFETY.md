# #380 S1R4-P3-P0R Activation Safety Amendment

## Status

P3-P0R is documentation-only. No Workflow was placed under .github/workflows; no Hosted task, NSIS executable, Probe, installer, or uninstaller was run. Hosted NSIS runtime and U2 production remain NO-GO.

Future activation, if separately approved, must be a single direct child commit on the original P3-P0 branch research/issue-380-s1r4-p3-p0-nsis-hosted-plan. It must not be activated from this P3-P0R documentation branch: the frozen Runner hard-codes the original P3-P0 branch identity.

## Frozen identities and preserved files

| Identity | Value |
|---|---|
| P3-P0 HEAD | 0aa8be6d58fc0994b3be6aec2af8dc02318fcefb |
| P3-P0 Tree | 688bd53c3c44040f3ab86fe4cf71a7a3aa879274 |
| P2 HEAD / Tree | 86c21c14dd65bd2bbdb3056513489e0d4139bb47 / 883a851cf8e75b4f39a4a46baea15ab109feb312 |
| P1 path source HEAD / Tree | 7a6fa69e5521d9759702c1923a9ade60a75819c3 / c5465ef93eae268e2c683876d959db90fac5da08 |
| S0 Probe HEAD / Git blob | 1f4951f3c93b2eb549d159fed2dd288992c625c8 / 5805f8d04cd37420a63f3e156c3b6fd5b4012f33 |
| P3 Runner Git blob at P3-P0 | 1d11c3add3c0d1f45e7d7ac082d09c616cca51a1 |
| P3 Runner file SHA-256 | c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd |

The original P3-P0 report, Runner, P1 path guard/tests, P2 Workflow, and S0 Probe are unchanged.

## P3 Runner identity lock before future activation

The future Workflow preflight fails before its P3 Runner step unless all conditions hold:

1. Event checkout HEAD equals GITHUB_SHA, the frozen P3-P0 Tree equals the listed Tree, and the activation commit has exactly one parent equal to P3-P0 HEAD.
2. git diff-tree against P3-P0 reports exactly one change: addition of .github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml. No modifications, deletions, renames, or extra files are permitted.
3. P3 Runner Git blob from P3-P0, activation HEAD, and checked-out work file equals 1d11c3add3c0d1f45e7d7ac082d09c616cca51a1. Its working-file SHA-256 equals c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd.
4. Immediately before Process.Start, the draft repeats the frozen Runner Git blob, working Git blob, and raw SHA-256 checks. Any mismatch stops before NSIS download or execution.
5. Existing P2/P1 Tree and ancestry checks, P1 file blobs, S0 Probe blob, run attempt, branch, repository, PREFLIGHT_OUTCOME, clean checkout, TaskRoot/OutputRoot ownership and HKCU absence checks remain required.

Activation is restricted to a single direct child. Any additional edits require a new Owner decision and an exact-diff review. The frozen Runner is not modified to accommodate this branch.

## Failure evidence upload contract

Artifact eligibility is a separate read-only step with always(). It does not depend on Runner or finalizer success. The eligibility step initializes its output to false, then validates:

- RUNNER_TEMP is a fully-qualified local-drive path; UNC and device paths are rejected.
- The evidence path is the one fixed directory directly under RUNNER_TEMP, canonical containment is checked, and reparse points in the path chain are rejected.
- No evidence file is read until the root exists as an ordinary directory and the bootstrap owner record is an ordinary in-root file.
- Repository, ref, run ID, attempt, workflow SHA and exact owner token match the current GitHub event.
- Iterative enumeration does not follow reparse points and rejects path escapes, more than 300 entries, more than 100 files, any file over 64 MiB, or total bytes over 256 MiB.
- safe=true is emitted only after every check passes and at least one file exists.

Artifact upload uses always() only with the independent safe output and uses if-no-files-found: error. It no longer requires finalize success. If ownership is unproven, upload is skipped and Actions logs remain available; the upload check will not access an unknown directory. The upload step uses only the fixed evidence root.

## Process identity and cleanup evidence contract

| Process / boundary | Required record |
|---|---|
| NSIS launcher | Fixed ZIP member path, byte length, SHA-256, actual path, -VERSION and -HDRINFO outputs and exit codes. |
| Underlying compiler | Fixed Bin/makensis.exe path, byte length and SHA-256, distinct from forwarding launcher. |
| P3 Runner parent | PID, actual PowerShell image path/SHA-256, fixed ArgumentList, working directory, start/end UTC, stdout/stderr completion, timeout and exit code. Runner Git blob and working-file SHA-256 are separate fields. |
| Stage / installer / uninstaller | Runner process PID, requested image path and SHA-256, ArgumentList, working directory, captured streams, timeout and exit code; observer-recorded image path, command line, parent PID, creation and observation times where available. |
| Uninstaller | Preserve _?=<controlled absolute TaskRoot directory> as the final argument and record the exact array. This argument alone does not prove NSIS skipped self-copy/restart. |
| Descendants | Read-only 100 ms Win32_Process descendant polling during the Runner plus three final snapshots. Record PID, parent PID, image path, command line, creation date and observation time. Join by PID to Runner process evidence. |
| Possible copied uninstaller | Mark an observed descendant outside TaskRoot with an uninstaller-related image name/command line or a matching generated-uninstaller SHA as suspicious. Record path and identity; never execute, delete, or hash arbitrary out-of-scope files. |
| Cleanup | Preserve Runner's pre-cleanup TaskRoot process snapshot, Registry/TaskRoot cleanup state, and wrapper snapshots before and after finalizer cleanup. Any remaining task PID, unowned path, reparse point or uncertain cleanup fails closed. |

The observer is a bounded polling trace, not a kernel process audit. A short-lived process can start and exit between snapshots; PID reuse and inaccessible image paths are further limits. An empty observer list therefore does not prove no-copy. The future run must never claim no-copy based only on _?=. If a no-copy conclusion becomes a release gate, Owner must require a separately reviewed reliable process-start trace before treating it as proven.

## Full future Workflow draft (documentation only)

This complete amended draft is not executable in this branch. It can only be considered for separately approved one-time activation on the original P3-P0 branch. Its exact-diff gate permits only the designated Workflow file to be added.

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
            bootstrap_owner_token = ($env:GITHUB_REPOSITORY + '|' + $env:GITHUB_REF + '|' + $env:GITHUB_RUN_ID + '|' + $env:GITHUB_RUN_ATTEMPT + '|' + $env:GITHUB_SHA)
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
            p3p0_head = '0aa8be6d58fc0994b3be6aec2af8dc02318fcefb'
            p3p0_tree = '688bd53c3c44040f3ab86fe4cf71a7a3aa879274'
            activation_parent = $null
            activation_diff = @()
            p3_runner_git_blob_sha = $null
            p3_runner_file_sha256 = $null
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
            $p3p0Head = '0aa8be6d58fc0994b3be6aec2af8dc02318fcefb'
            $p3p0Tree = '688bd53c3c44040f3ab86fe4cf71a7a3aa879274'
            $p3RunnerPath = 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'
            $expectedRunnerBlob = '1d11c3add3c0d1f45e7d7ac082d09c616cca51a1'
            $expectedRunnerFileSha256 = 'c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd'
            $actualP3Tree = (& git -C $env:GITHUB_WORKSPACE rev-parse ($p3p0Head + '^{tree}')).Trim()
            if ($LASTEXITCODE -ne 0 -or $actualP3Tree -ne $p3p0Tree) { throw "Frozen P3-P0 Tree mismatch: $actualP3Tree" }
            $parentLine = (& git -C $env:GITHUB_WORKSPACE rev-list --parents -n 1 $head).Trim()
            if ($LASTEXITCODE -ne 0) { throw 'Could not resolve activation commit parents.' }
            $parentFields = @($parentLine -split '\s+' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            if ($parentFields.Count -ne 2 -or $parentFields[0] -ne $head -or $parentFields[1] -ne $p3p0Head) {
              throw "Activation must be a single direct child of frozen P3-P0: $parentLine"
            }
            $record.activation_parent = $parentFields[1]
            $activationDiff = @(& git -C $env:GITHUB_WORKSPACE diff-tree --no-commit-id --no-renames --name-status -r $p3p0Head $head)
            if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate the activation diff.' }
            $record.activation_diff = @($activationDiff)
            $expectedActivation = 'A' + [char]9 + '.github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml'
            if ($activationDiff.Count -ne 1 -or [string]$activationDiff[0] -cne $expectedActivation) {
              throw "Activation diff must add only the designated Workflow file: $($activationDiff -join '; ')"
            }
            $checkoutStatus = (& git -C $env:GITHUB_WORKSPACE status --porcelain --untracked-files all).Trim()
            if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrEmpty($checkoutStatus)) { throw "Activation checkout is not clean: $checkoutStatus" }
            $p3RunnerFile = Join-Path $env:GITHUB_WORKSPACE $p3RunnerPath
            $frozenRunnerBlob = (& git -C $env:GITHUB_WORKSPACE rev-parse ($p3p0Head + ':' + $p3RunnerPath)).Trim()
            $activationRunnerBlob = (& git -C $env:GITHUB_WORKSPACE rev-parse ($head + ':' + $p3RunnerPath)).Trim()
            $workingRunnerBlob = (& git -C $env:GITHUB_WORKSPACE hash-object -- $p3RunnerFile).Trim()
            if ($LASTEXITCODE -ne 0) { throw 'Could not resolve frozen or working P3 Runner identity.' }
            $workingRunnerSha = (Get-FileHash -LiteralPath $p3RunnerFile -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($frozenRunnerBlob -ne $expectedRunnerBlob -or $activationRunnerBlob -ne $expectedRunnerBlob -or $workingRunnerBlob -ne $expectedRunnerBlob -or $workingRunnerSha -ne $expectedRunnerFileSha256) {
              throw "P3 Runner identity mismatch before NSIS: frozen_blob=$frozenRunnerBlob activation_blob=$activationRunnerBlob working_blob=$workingRunnerBlob raw_sha256=$workingRunnerSha"
            }
            $record.p3_runner_git_blob_sha = $workingRunnerBlob
            $record.p3_runner_file_sha256 = $workingRunnerSha
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
            p3_runner_git_blob_sha = $null
            p3_runner_file_sha256 = $null
            p3_runner_identity_rechecked_before_start = $false
            process_observer_status = 'NOT_STARTED'
            process_observer_record_count = 0
            process_observer_error = $null
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
          $p3p0Head = '0aa8be6d58fc0994b3be6aec2af8dc02318fcefb'
          $expectedRunnerBlob = '1d11c3add3c0d1f45e7d7ac082d09c616cca51a1'
          $expectedRunnerFileSha256 = 'c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd'
          $runnerRelative = 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'
          $frozenRunnerBlob = (& git -C $env:GITHUB_WORKSPACE rev-parse ($p3p0Head + ':' + $runnerRelative)).Trim()
          $workingRunnerBlob = (& git -C $env:GITHUB_WORKSPACE hash-object -- $runner).Trim()
          $workingRunnerSha = (Get-FileHash -LiteralPath $runner -Algorithm SHA256).Hash.ToLowerInvariant()
          if ($LASTEXITCODE -ne 0 -or $frozenRunnerBlob -ne $expectedRunnerBlob -or $workingRunnerBlob -ne $expectedRunnerBlob -or $workingRunnerSha -ne $expectedRunnerFileSha256) {
            throw "P3 Runner identity changed immediately before process start: frozen_blob=$frozenRunnerBlob working_blob=$workingRunnerBlob raw_sha256=$workingRunnerSha"
          }
          $identity.p3_runner_git_blob_sha = $workingRunnerBlob
          $identity.p3_runner_file_sha256 = $workingRunnerSha
          $identity.p3_runner_identity_rechecked_before_start = $true
          $process = [System.Diagnostics.Process]::new()
          $process.StartInfo = $start
          $startedAt = [DateTime]::UtcNow
          if (-not $process.Start()) { throw 'Could not start the P3 Runner process.' }
          $observerJob = Start-Job -ArgumentList ([int]$process.Id), (Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-task') -ScriptBlock {
            param([int]$rootProcessId, [string]$controlledTaskRoot)
            $ErrorActionPreference = 'Stop'
            $knownPids = [System.Collections.Generic.HashSet[int]]::new()
            [void]$knownPids.Add($rootProcessId)
            $seenPids = [System.Collections.Generic.HashSet[int]]::new()
            $taskPathPrefix = [System.IO.Path]::GetFullPath($controlledTaskRoot).TrimEnd([char[]]@([char]92, [char]47)) + [char]92
            $deadline = [DateTime]::UtcNow.AddMinutes(10)
            while ([DateTime]::UtcNow -lt $deadline -and $null -ne (Get-Process -Id $rootProcessId -ErrorAction SilentlyContinue)) {
              $snapshot = @(Get-CimInstance -ClassName Win32_Process -Property @('ProcessId', 'ParentProcessId', 'ExecutablePath', 'CommandLine', 'CreationDate') -ErrorAction Stop)
              foreach ($item in $snapshot) {
                $childPid = [int]$item.ProcessId
                $parentPid = [int]$item.ParentProcessId
                if ($knownPids.Contains($childPid)) { continue }
                if ($knownPids.Contains($parentPid)) {
                  [void]$knownPids.Add($childPid)
                  if ($seenPids.Add($childPid)) {
                    $imagePath = [string]$item.ExecutablePath
                    $outsideTaskRoot = [string]::IsNullOrWhiteSpace($imagePath)
                    if (-not $outsideTaskRoot) {
                      $fullImagePath = [System.IO.Path]::GetFullPath($imagePath)
                      $outsideTaskRoot = -not $fullImagePath.StartsWith($taskPathPrefix, [System.StringComparison]::OrdinalIgnoreCase)
                    }
                    [pscustomobject]@{
                      observed_utc = [DateTime]::UtcNow.ToString('o')
                      process_id = $childPid
                      parent_process_id = $parentPid
                      image_path = $imagePath
                      command_line = [string]$item.CommandLine
                      creation_date = [string]$item.CreationDate
                      outside_task_root_or_image_unavailable = $outsideTaskRoot
                    }
                  }
                }
              }
              Start-Sleep -Milliseconds 100
            }
            for ($tail = 0; $tail -lt 3; $tail++) {
              $snapshot = @(Get-CimInstance -ClassName Win32_Process -Property @('ProcessId', 'ParentProcessId', 'ExecutablePath', 'CommandLine', 'CreationDate') -ErrorAction Stop)
              foreach ($item in $snapshot) {
                $childPid = [int]$item.ProcessId
                $parentPid = [int]$item.ParentProcessId
                if (-not $knownPids.Contains($childPid) -and $knownPids.Contains($parentPid)) {
                  [void]$knownPids.Add($childPid)
                  if ($seenPids.Add($childPid)) {
                    $imagePath = [string]$item.ExecutablePath
                    $outsideTaskRoot = [string]::IsNullOrWhiteSpace($imagePath)
                    if (-not $outsideTaskRoot) {
                      $fullImagePath = [System.IO.Path]::GetFullPath($imagePath)
                      $outsideTaskRoot = -not $fullImagePath.StartsWith($taskPathPrefix, [System.StringComparison]::OrdinalIgnoreCase)
                    }
                    [pscustomobject]@{
                      observed_utc = [DateTime]::UtcNow.ToString('o')
                      process_id = $childPid
                      parent_process_id = $parentPid
                      image_path = $imagePath
                      command_line = [string]$item.CommandLine
                      creation_date = [string]$item.CreationDate
                      outside_task_root_or_image_unavailable = $outsideTaskRoot
                    }
                  }
                }
              }
              Start-Sleep -Milliseconds 100
            }
          }
          if (-not (Wait-Job -Job $observerJob -State Running -Timeout 5)) {
            try { $process.Kill($true) } catch { }
            try { $process.WaitForExit() } catch { }
            $identity.status = 'OBSERVER_START_FAILED'
            $identity.process_id = $process.Id
            $identity.finished_utc = [DateTime]::UtcNow.ToString('o')
            $identity.timed_out = $false
            $identity.process_exited = $process.HasExited
            $identity.exit_code = if ($process.HasExited) { [int]$process.ExitCode } else { $null }
            [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 8) + [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false))
            Stop-Job -Job $observerJob -ErrorAction SilentlyContinue
            Remove-Job -Job $observerJob -Force -ErrorAction SilentlyContinue
            throw 'Read-only process observer did not start within five seconds; the P3 Runner was terminated before it could continue.'
          }
          $identity.process_observer_status = 'RUNNING_DESCENDANT_POLL'
          $identity.status = 'RUNNING'
          $identity.process_id = $process.Id
          $identity.started_utc = $startedAt.ToString('o')
          [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))
          $stdoutTask = $process.StandardOutput.ReadToEndAsync()
          $stderrTask = $process.StandardError.ReadToEndAsync()
          $runnerDeadline = [DateTime]::UtcNow.AddMilliseconds(540000)
          $completed = $false
          $observerFailedDuringRun = $false
          while ([DateTime]::UtcNow -lt $runnerDeadline) {
            if ($process.WaitForExit(250)) { $completed = $true; break }
            if ($observerJob.State -in @('Failed', 'Stopped')) {
              $observerFailedDuringRun = $true
              try { $process.Kill($true) } catch { }
              try { $process.WaitForExit() } catch { }
              break
            }
          }
          if (-not $completed -and -not $process.HasExited) {
            try { $process.Kill($true) } catch { }
            $process.WaitForExit()
          }
          $stdoutComplete = $stdoutTask.Wait(5000)
          $stderrComplete = $stderrTask.Wait(5000)
          $stdout = if ($stdoutComplete) { $stdoutTask.GetAwaiter().GetResult() } else { '[stdout did not close within 5 seconds]' }
          $stderr = if ($stderrComplete) { $stderrTask.GetAwaiter().GetResult() } else { '[stderr did not close within 5 seconds]' }
          $observerFinished = $observerJob | Wait-Job -Timeout 5
          if ($null -eq $observerFinished) { Stop-Job -Job $observerJob -ErrorAction SilentlyContinue }
          $observerRows = @(Receive-Job -Job $observerJob -Keep -ErrorAction SilentlyContinue)
          $observerErrors = @($observerJob.ChildJobs | ForEach-Object { $_.Error } | ForEach-Object { $_.ToString() })
          Remove-Job -Job $observerJob -Force -ErrorAction SilentlyContinue
          $observerPath = Join-Path $evidenceRoot 'process-observer.json'
          $observerRecord = [ordered]@{
            status = if ($null -ne $observerFinished -and $observerErrors.Count -eq 0) { 'COMPLETED' } else { 'INCOMPLETE' }
            method = '100ms Win32_Process descendant polling plus three post-exit snapshots; best-effort, not proof that no short-lived child existed'
            runner_process_id = [int]$process.Id
            completed_utc = [DateTime]::UtcNow.ToString('o')
            record_count = $observerRows.Count
            descendant_processes = @($observerRows)
            errors = @($observerErrors)
          }
          [System.IO.File]::WriteAllText($observerPath, ((ConvertTo-Json -InputObject $observerRecord -Depth 12) + [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false))
          $identity.process_observer_status = $observerRecord.status
          $identity.process_observer_record_count = $observerRows.Count
          $identity.process_observer_error = ($observerErrors -join [Environment]::NewLine)
          [System.IO.File]::WriteAllText($stdoutPath, $stdout, [System.Text.UTF8Encoding]::new($false))
          [System.IO.File]::WriteAllText($stderrPath, $stderr, [System.Text.UTF8Encoding]::new($false))
          $exitCode = if ($process.HasExited) { [int]$process.ExitCode } else { $null }
          $identity.status = if ($observerFailedDuringRun) { 'OBSERVER_FAILED_TREE_KILLED' } elseif ($completed) { 'COMPLETED' } else { 'TIMED_OUT_TREE_KILLED' }
          $identity.finished_utc = [DateTime]::UtcNow.ToString('o')
          $identity.timed_out = (-not $completed -and -not $observerFailedDuringRun)
          $identity.process_exited = $process.HasExited
          $identity.exit_code = $exitCode
          $identity.stdout_complete = $stdoutComplete
          $identity.stderr_complete = $stderrComplete
          [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))
          $process.Dispose()
          if (-not $completed -or $exitCode -ne 0 -or -not $stdoutComplete -or -not $stderrComplete -or $identity.process_observer_status -ne 'COMPLETED' -or $observerFailedDuringRun) { throw "P3 Runner or process observer failed: exit=$exitCode timeout=$(-not $completed) observer=$($identity.process_observer_status)" }

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
            process_observer_status = 'NOT_AVAILABLE'
            observed_descendant_processes = @()
            suspected_out_of_root_children = @()
            direct_process_identity_map = @()
            pre_cleanup_task_process_snapshot = @()
            post_cleanup_task_process_snapshot = @()
            failure = $null
          }
          try {
            $processPath = Join-Path $evidenceRoot 'runner-process.json'
            $resultPath = Join-Path $evidenceRoot 'results.json'
            $processRecord = if (Test-Path -LiteralPath $processPath -PathType Leaf) { Get-Content -LiteralPath $processPath -Raw | ConvertFrom-Json } else { $null }
            $runnerResult = if (Test-Path -LiteralPath $resultPath -PathType Leaf) { Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json } else { $null }
            $observerPath = Join-Path $evidenceRoot 'process-observer.json'
            $observerRecord = if (Test-Path -LiteralPath $observerPath -PathType Leaf) { Get-Content -LiteralPath $observerPath -Raw | ConvertFrom-Json } else { $null }
            if ($null -ne $processRecord) { $assessment.runner_exit_code = $processRecord.exit_code; $assessment.runner_timed_out = $processRecord.timed_out }
            if ($null -ne $runnerResult) { $assessment.runner_result_status = $runnerResult.status }
            if ($null -ne $observerRecord) {
              $assessment.process_observer_status = $observerRecord.status
              $assessment.observed_descendant_processes = @($observerRecord.descendant_processes)
              $assessment.suspected_out_of_root_children = @($observerRecord.descendant_processes | Where-Object { $_.outside_task_root_or_image_unavailable -eq $true })
              if ($null -ne $runnerResult) {
                $assessment.direct_process_identity_map = @(
                  foreach ($direct in $runnerResult.processes) {
                    $observed = @($observerRecord.descendant_processes | Where-Object { [int]$_.process_id -eq [int]$direct.process_id })
                    [ordered]@{
                      process_id = $direct.process_id
                      requested_image_path = $direct.process_image_path
                      requested_image_sha256 = $direct.file_sha256
                      argument_list = $direct.argument_list
                      exit_code = $direct.exit_code
                      timed_out = $direct.timed_out
                      observed_process_instances = $observed
                      observation_status = if ($observed.Count -gt 0) { 'OBSERVED' } else { 'NOT_OBSERVED_BY_POLLING' }
                    }
                  }
                )
              }
            }
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
            $assessment.pre_cleanup_task_process_snapshot = @($taskOwnedProcesses.ToArray())
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
            $postCleanupProcesses = [System.Collections.Generic.List[object]]::new()
            foreach ($processInfo in (Get-CimInstance -ClassName Win32_Process -Property @('ProcessId', 'ExecutablePath') -ErrorAction Stop)) {
              if ([string]::IsNullOrWhiteSpace([string]$processInfo.ExecutablePath)) { continue }
              $fullImage = [System.IO.Path]::GetFullPath([string]$processInfo.ExecutablePath)
              if ($fullImage.StartsWith($taskRootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                $postCleanupProcesses.Add([ordered]@{ process_id = [int]$processInfo.ProcessId; image_path = $fullImage })
              }
            }
            $assessment.post_cleanup_task_process_snapshot = @($postCleanupProcesses.ToArray())
            if ($postCleanupProcesses.Count -ne 0) { throw 'Task-path process remained after cleanup.' }

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

      - name: Independently validate safe evidence upload eligibility
        id: upload_eligibility
        if: always()
        continue-on-error: true
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          $outputFile = $env:GITHUB_OUTPUT
          $maxFiles = 100
          $maxEntries = 300
          $maxSingleBytes = 67108864
          $maxTotalBytes = 268435456
          $expectedRepository = 'ArdenZC/Zen-Canvas'
          $expectedRef = 'refs/heads/research/issue-380-s1r4-p3-p0-nsis-hosted-plan'
          if ($env:GITHUB_REPOSITORY -ne $expectedRepository -or $env:GITHUB_REF -ne $expectedRef) { throw 'Evidence upload run identity is outside the fixed P3 contract.' }
          if ($env:GITHUB_RUN_ATTEMPT -ne '1' -or [string]::IsNullOrWhiteSpace($env:GITHUB_RUN_ID) -or [string]::IsNullOrWhiteSpace($env:GITHUB_SHA)) { throw 'Evidence upload run ID, attempt, or SHA is invalid.' }
          if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or -not [System.IO.Path]::IsPathRooted($env:RUNNER_TEMP)) { throw 'RUNNER_TEMP is not an absolute path.' }
          if ($env:RUNNER_TEMP.StartsWith('\\') -or $env:RUNNER_TEMP.StartsWith('\\?\') -or $env:RUNNER_TEMP.StartsWith('\\.\')) { throw 'UNC and device paths are rejected for evidence upload.' }
          $runnerTemp = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP)
          if ([System.IO.Path]::GetPathRoot($runnerTemp) -notmatch '^[A-Za-z]:\\$') { throw 'RUNNER_TEMP must be on a local drive.' }
          $expectedRoot = [System.IO.Path]::GetFullPath((Join-Path $runnerTemp 'issue380-s1r4-p3-evidence'))
          $rootPrefix = $runnerTemp.TrimEnd([char[]]@([char]92, [char]47)) + [System.IO.Path]::DirectorySeparatorChar
          if (-not $expectedRoot.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) { throw 'Evidence root escaped RUNNER_TEMP.' }
          function Assert-NoReparsePath([string]$path) {
            $full = [System.IO.Path]::GetFullPath($path)
            $drive = [System.IO.Path]::GetPathRoot($full)
            $tail = $full.Substring($drive.Length).Split([char[]]@([char]92, [char]47), [System.StringSplitOptions]::RemoveEmptyEntries)
            $cursor = $drive
            foreach ($part in $tail) {
              $cursor = Join-Path $cursor $part
              if (-not (Test-Path -LiteralPath $cursor)) { throw "Required path component is missing: $cursor" }
              $attributes = [System.IO.File]::GetAttributes($cursor)
              if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse point rejected: $cursor" }
            }
          }
          Assert-NoReparsePath $runnerTemp
          if (-not (Test-Path -LiteralPath $expectedRoot -PathType Container)) { throw 'Owned evidence directory is missing.' }
          Assert-NoReparsePath $expectedRoot
          $ownerPath = Join-Path $expectedRoot 'workflow-run.json'
          Assert-NoReparsePath $ownerPath
          if (-not (Test-Path -LiteralPath $ownerPath -PathType Leaf)) { throw 'Bootstrap ownership file is missing.' }
          $owner = Get-Content -LiteralPath $ownerPath -Raw | ConvertFrom-Json
          $expectedOwnerToken = "$expectedRepository|$expectedRef|$env:GITHUB_RUN_ID|$env:GITHUB_RUN_ATTEMPT|$env:GITHUB_SHA"
          if ($owner.repository -ne $expectedRepository -or $owner.triggering_ref -ne $expectedRef -or $owner.triggering_head -ne $env:GITHUB_SHA -or $owner.run_id -ne $env:GITHUB_RUN_ID -or $owner.run_attempt -ne $env:GITHUB_RUN_ATTEMPT -or $owner.bootstrap_owner_token -cne $expectedOwnerToken) {
            throw 'Bootstrap ownership record does not match this exact run, attempt, and SHA.'
          }
          $pending = [System.Collections.Generic.Stack[string]]::new()
          $pending.Push($expectedRoot)
          $fileCount = 0
          $entryCount = 0
          $totalBytes = [long]0
          while ($pending.Count -gt 0) {
            $directory = $pending.Pop()
            foreach ($entry in [System.IO.Directory]::EnumerateFileSystemEntries($directory)) {
              $entryCount++
              if ($entryCount -gt $maxEntries) { throw "Evidence entry limit exceeded: $entryCount" }
              $fullEntry = [System.IO.Path]::GetFullPath($entry)
              if (-not $fullEntry.StartsWith(($expectedRoot.TrimEnd([char[]]@([char]92, [char]47)) + [System.IO.Path]::DirectorySeparatorChar), [System.StringComparison]::OrdinalIgnoreCase)) { throw "Evidence entry escaped the owned root: $fullEntry" }
              $attributes = [System.IO.File]::GetAttributes($fullEntry)
              if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Evidence reparse point rejected: $fullEntry" }
              if (($attributes -band [System.IO.FileAttributes]::Directory) -ne 0) { $pending.Push($fullEntry) }
              else {
                $fileCount++
                if ($fileCount -gt $maxFiles) { throw "Evidence file limit exceeded: $fileCount" }
                $length = [System.IO.FileInfo]::new($fullEntry).Length
                if ($length -gt $maxSingleBytes) { throw "Evidence file exceeds per-file limit: $fullEntry" }
                $totalBytes += $length
                if ($totalBytes -gt $maxTotalBytes) { throw "Evidence total-byte limit exceeded: $totalBytes" }
              }
            }
          }
          if ($fileCount -lt 1) { throw 'No evidence files are available to upload.' }
          Add-Content -LiteralPath $outputFile -Value 'safe=true' -Encoding utf8
          Add-Content -LiteralPath $outputFile -Value ("file_count=" + $fileCount) -Encoding utf8
          Add-Content -LiteralPath $outputFile -Value ("total_bytes=" + $totalBytes) -Encoding utf8

      - name: Upload raw evidence when the owned evidence root is independently safe
        if: ${{ always() && steps.upload_eligibility.outputs.safe == 'true' }}
        uses: actions/upload-artifact@v4
        with:
          name: issue380-s1r4-p3-${{ github.run_id }}-attempt-${{ github.run_attempt }}
          path: ${{ runner.temp }}/issue380-s1r4-p3-evidence/
          if-no-files-found: error
          retention-days: 14

      - name: Enforce one-shot synthetic result
        if: always()
        shell: pwsh
        env:
          UPLOAD_SAFE: ${{ steps.upload_eligibility.outputs.safe }}
        run: |
          $ErrorActionPreference = 'Stop'
          if ($env:UPLOAD_SAFE -ne 'true') { throw 'Evidence ownership was not proven; refusing to access the evidence directory. Actions logs are the failure record.' }
          $assessmentPath = Join-Path (Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-evidence') 'assessment.json'
          if (-not (Test-Path -LiteralPath $assessmentPath -PathType Leaf)) { throw 'P3 assessment is missing; refusing PASS.' }
          $assessment = Get-Content -LiteralPath $assessmentPath -Raw | ConvertFrom-Json
          if ($assessment.status -ne 'SYNTHETIC_TRACE_PERSISTENCE_PROVEN' -or $assessment.run_id -ne $env:GITHUB_RUN_ID -or $assessment.run_attempt -ne $env:GITHUB_RUN_ATTEMPT -or $assessment.workflow_sha -ne $env:GITHUB_SHA) { throw "P3 synthetic NSIS result failed closed or belongs to another run: $($assessment.failure)" }
          Write-Host 'SYNTHETIC_TRACE_PERSISTENCE_PROVEN'
```

## Static admission checks and evidence

The independent P3-P0R validator checks the frozen base, Tree, Runner Git blob and file SHA-256; exact branch and file diff restrictions; retained P2/P1/S0 identities; the full embedded Workflow, its one-job/permissions/trigger/attempt constraints, activation-diff gate and immediate Runner identity recheck; independent upload eligibility, ownership binding, entry/size limits and if-no-files-found:error; embedded PowerShell syntax; process evidence fields; and no activation.

A static audit of the checked-in push triggers found no existing Workflow matching the P3-P0R documentation branch. Static checks do not execute PowerShell, NSIS, Actions, or Windows process observation. No Hosted job was authorized or started.

## Not executed and remaining limits

- No Workflow was copied under .github/workflows; no activation commit was made to the original P3-P0 branch.
- No Hosted Windows job, NSIS download/compiler, Probe, installer/uninstaller, HKCU access or process observer ran.
- No frozen P3-P0 Runner, P1/P2/S0 source, production code, master or PR #386 changed.
- Artifact behavior, Windows path/reparse semantics, NSIS launcher behavior, abort persistence, process observer completeness and cleanup remain runtime-unknown.
- No-copy behavior remains unverified.

## Requested terminal state

#380-S1R4-P3-P0R READY_FOR_OWNER_ACTIVATION_SAFETY_REVIEW
