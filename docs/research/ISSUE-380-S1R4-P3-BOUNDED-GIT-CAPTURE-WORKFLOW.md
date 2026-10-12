# Inert Workflow draft — #380 S1R4-P3 bounded Git capture candidate

This complete Workflow is a research document only. It is not under `.github/workflows` and must not be copied or activated without a new Owner GO. It binds a future single-use activation to the bounded Git capture freeze commit `af305adb8fba017dd9284a829b5101a209a9b4e2` and Tree `9d7716add5fde99c6e797065f336e0d0cf86a510`; the historical activation branch and consumed Run remain immutable. The frozen Runner Blob is `c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa` with raw SHA-256 `92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece`. The candidate still has no Windows PowerShell runtime validation, and this draft is not authorization to download or run NSIS.

The future activation must be a single direct child of that freeze commit and add only `.github/workflows/issue-380-s1r4-p3-bounded-git-capture.yml`. Its separate branch and unique workflow path do not match the consumed Workflow's branch/path filters. The draft retains the prior Windows LF checkout, safe evidence upload, observer, HKCU, task ownership, NSIS identity, FileWrite-first, and exit-code contracts while requiring the new Runner Git Blob and raw SHA before invocation.

```yaml
name: Issue 380 S1R4-P3 synthetic NSIS trace probe

on:
  push:
    branches:
      - research/issue-380-s1r4-p3-runner-null-safety-activation
    paths:
      - .github/workflows/issue-380-s1r4-p3-bounded-git-capture.yml

permissions:
  contents: read

concurrency:
  group: issue-380-s1r4-p3-bounded-git-capture-${{ github.ref }}
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
          if ($env:GITHUB_REF -ne 'refs/heads/research/issue-380-s1r4-p3-runner-null-safety-activation') { throw "Unexpected ref: $env:GITHUB_REF" }
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

      - name: Set and verify raw Git checkout byte policy
        id: byte_policy
        if: ${{ steps.bootstrap.outcome == 'success' }}
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          function Invoke-Issue380GitCapture {
            param([Parameter(Mandatory)][string[]]$Arguments)
            $PSNativeCommandUseErrorActionPreference = $false
            $nativeItems = @(& git @Arguments 2>&1)
            $nativeExitCode = [int]$LASTEXITCODE
            $lineList = [System.Collections.Generic.List[string]]::new()
            foreach ($item in $nativeItems) { $lineList.Add([string]$item) }
            $lines = $lineList.ToArray()
            return [pscustomobject]@{
              ExitCode = $nativeExitCode
              OutputCount = $lines.Length
              Lines = $lines
              Text = [string]::Join([Environment]::NewLine, $lines)
            }
          }
          function Assert-Issue380GitNoOutputSuccess {
            param([Parameter(Mandatory)]$Result, [Parameter(Mandatory)][string]$Label)
            if ($Result.ExitCode -ne 0 -or $Result.OutputCount -ne 0) {
              throw "$Label must succeed without output; exit=$($Result.ExitCode) output=$($Result.Text)"
            }
          }
          function Get-Issue380GitScalar {
            param([Parameter(Mandatory)][string[]]$Arguments, [Parameter(Mandatory)][string]$Label)
            $result = Invoke-Issue380GitCapture -Arguments $Arguments
            if ($result.ExitCode -ne 0) { throw "$Label failed with Git exit $($result.ExitCode): $($result.Text)" }
            if ($result.OutputCount -ne 1 -or [string]::IsNullOrWhiteSpace([string]$result.Text)) {
              throw "$Label must return exactly one non-empty line; count=$($result.OutputCount) output=$($result.Text)"
            }
            return ([string]$result.Text).Trim()
          }
          $setAutocrlf = Invoke-Issue380GitCapture -Arguments @('config', '--global', '--replace-all', 'core.autocrlf', 'false')
          Assert-Issue380GitNoOutputSuccess -Result $setAutocrlf -Label 'Set global core.autocrlf=false'
          $setEol = Invoke-Issue380GitCapture -Arguments @('config', '--global', '--replace-all', 'core.eol', 'lf')
          Assert-Issue380GitNoOutputSuccess -Result $setEol -Label 'Set global core.eol=lf'
          $autocrlf = Get-Issue380GitScalar -Arguments @('config', '--global', '--get', 'core.autocrlf') -Label 'Read required global core.autocrlf'
          $eol = Get-Issue380GitScalar -Arguments @('config', '--global', '--get', 'core.eol') -Label 'Read required global core.eol'
          if ($autocrlf -cne 'false' -or $eol -cne 'lf') { throw "Raw checkout policy did not persist: autocrlf=$autocrlf eol=$eol" }
          $evidenceRoot = Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-evidence'
          $policy = [ordered]@{ status = 'PASS'; core_autocrlf_global = $autocrlf; core_eol_global = $eol; set_before_checkout = $true; recorded_utc = [DateTime]::UtcNow.ToString('o') }
          [System.IO.File]::WriteAllText((Join-Path $evidenceRoot 'git-byte-policy.json'), ((ConvertTo-Json -InputObject $policy -Depth 6) + [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false))

      - name: Checkout the exact activation commit
        id: checkout
        if: ${{ steps.bootstrap.outcome == 'success' && steps.byte_policy.outcome == 'success' }}
        uses: actions/checkout@v4
        with:
          fetch-depth: 0
          persist-credentials: false

      - name: Verify P3, P2, P1, path, and HKCU preconditions
        id: preflight
        if: ${{ steps.bootstrap.outcome == 'success' && steps.byte_policy.outcome == 'success' && steps.checkout.outcome == 'success' }}
        continue-on-error: true
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          function Invoke-Issue380GitCapture {
            param([Parameter(Mandatory)][string[]]$Arguments)
            $PSNativeCommandUseErrorActionPreference = $false
            $nativeItems = @(& git @Arguments 2>&1)
            $nativeExitCode = [int]$LASTEXITCODE
            $lineList = [System.Collections.Generic.List[string]]::new()
            foreach ($item in $nativeItems) { $lineList.Add([string]$item) }
            $lines = $lineList.ToArray()
            return [pscustomobject]@{
              ExitCode = $nativeExitCode
              OutputCount = $lines.Length
              Lines = $lines
              Text = [string]::Join([Environment]::NewLine, $lines)
            }
          }
          function Get-Issue380GitScalar {
            param([Parameter(Mandatory)][string[]]$Arguments, [Parameter(Mandatory)][string]$Label)
            $result = Invoke-Issue380GitCapture -Arguments $Arguments
            if ($result.ExitCode -ne 0) { throw "$Label failed with Git exit $($result.ExitCode): $($result.Text)" }
            if ($result.OutputCount -ne 1 -or [string]::IsNullOrWhiteSpace([string]$result.Text)) {
              throw "$Label must return exactly one non-empty line; count=$($result.OutputCount) output=$($result.Text)"
            }
            return ([string]$result.Text).Trim()
          }
          function Assert-Issue380GitNoOutputSuccess {
            param([Parameter(Mandatory)]$Result, [Parameter(Mandatory)][string]$Label)
            if ($Result.ExitCode -ne 0 -or $Result.OutputCount -ne 0) {
              throw "$Label must succeed without output; exit=$($Result.ExitCode) output=$($Result.Text)"
            }
          }
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
            candidate_freeze_head = 'af305adb8fba017dd9284a829b5101a209a9b4e2'
            candidate_freeze_tree = '9d7716add5fde99c6e797065f336e0d0cf86a510'
            activation_parent = $null
            activation_diff = @()
            checkout_status = 'NOT_CHECKED'
            checkout_status_output_count = $null
            p3_runner_git_blob_sha = $null
            p3_runner_file_sha256 = $null
            controlled_file_identities = @()
            git_byte_policy = $null
            gitattributes_inventory = @()
            effective_transform_attributes = @()
            filter_configuration = @()
            filter_configuration_state = 'NOT_CHECKED'
            hkcu_key_initially_absent = $false
            failure = $null
          }
          try {
            if ($env:GITHUB_REPOSITORY -ne 'ArdenZC/Zen-Canvas') { throw "Unexpected repository: $env:GITHUB_REPOSITORY" }
            if ($env:GITHUB_REF -ne 'refs/heads/research/issue-380-s1r4-p3-runner-null-safety-activation') { throw "Unexpected ref: $env:GITHUB_REF" }
            if ($env:GITHUB_RUN_ATTEMPT -ne '1') { throw "One-shot contract refuses attempt $env:GITHUB_RUN_ATTEMPT" }
            if ($env:RUNNER_OS -ne 'Windows' -or $PSVersionTable.PSVersion.Major -ne 7) { throw 'windows-2022 PowerShell 7 is required.' }
            if ([string]::IsNullOrWhiteSpace($env:GITHUB_RUN_ID) -or [string]::IsNullOrWhiteSpace($env:GITHUB_SHA) -or [string]::IsNullOrWhiteSpace($env:GITHUB_WORKSPACE) -or [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { throw 'GITHUB_RUN_ID, GITHUB_SHA, GITHUB_WORKSPACE, and RUNNER_TEMP are required.' }
            $initialEvidence = @(Get-ChildItem -LiteralPath $evidenceRoot -Force -ErrorAction Stop | Sort-Object Name)
            $initialNames = @($initialEvidence | ForEach-Object { $_.Name })
            if ($initialNames.Count -ne 2 -or $initialNames[0] -cne 'git-byte-policy.json' -or $initialNames[1] -cne 'workflow-run.json') { throw 'Evidence root does not contain exactly the bootstrap and pre-checkout byte-policy records.' }
            $policyPath = Join-Path $evidenceRoot 'git-byte-policy.json'
            $bytePolicy = Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json
            $effectiveAutocrlf = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'config', '--get', 'core.autocrlf') -Label 'Read required worktree core.autocrlf'
            $effectiveEol = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'config', '--get', 'core.eol') -Label 'Read required worktree core.eol'
            if ($bytePolicy.status -ne 'PASS' -or $bytePolicy.set_before_checkout -ne $true -or $bytePolicy.core_autocrlf_global -cne 'false' -or $bytePolicy.core_eol_global -cne 'lf' -or $effectiveAutocrlf -cne 'false' -or $effectiveEol -cne 'lf') { throw "Git checkout byte policy mismatch: recorded=$($bytePolicy | ConvertTo-Json -Compress) effective_autocrlf=$effectiveAutocrlf effective_eol=$effectiveEol" }
            $record.git_byte_policy = [ordered]@{ core_autocrlf_global = $bytePolicy.core_autocrlf_global; core_eol_global = $bytePolicy.core_eol_global; core_autocrlf_effective = $effectiveAutocrlf; core_eol_effective = $effectiveEol; set_before_checkout = $bytePolicy.set_before_checkout }
            if (Test-Path -LiteralPath $record.task_root) { throw "Refusing preexisting TaskRoot: $($record.task_root)" }

            . (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4PathGuard.ps1')
            $runnerTemp = Resolve-Issue380S1R4Path -TaskRoot $env:RUNNER_TEMP -CandidatePath $env:RUNNER_TEMP -Kind WorkingDirectory
            $null = Resolve-Issue380S1R4Path -TaskRoot $runnerTemp -CandidatePath $evidenceRoot -Kind ExistingDirectory
            $expectedTaskRoot = Join-Path $runnerTemp 'issue380-s1r4-p3-task'
            if (-not $record.task_root.Equals($expectedTaskRoot, [System.StringComparison]::OrdinalIgnoreCase)) { throw 'TaskRoot does not match the fixed private child path.' }

            $head = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', 'HEAD') -Label 'Resolve checked-out HEAD'
            if ($head -ne $env:GITHUB_SHA) { throw "Checkout HEAD mismatch: checkout=$head event=$env:GITHUB_SHA" }
            $tree = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', 'HEAD^{tree}') -Label 'Resolve checked-out Tree'
            $record.checked_out_head = $head
            $record.checked_out_tree = $tree
            $candidateFreezeHead = 'af305adb8fba017dd9284a829b5101a209a9b4e2'
            $candidateFreezeTree = '9d7716add5fde99c6e797065f336e0d0cf86a510'
            $p3RunnerPath = 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'
            $expectedRunnerBlob = 'c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa'
            $expectedRunnerFileSha256 = '92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece'
            $actualP3Tree = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', ($candidateFreezeHead + '^{tree}')) -Label 'Resolve frozen Runner candidate Tree'
            if ($actualP3Tree -ne $candidateFreezeTree) { throw "Frozen Runner candidate Tree mismatch: $actualP3Tree" }
            $parentLine = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-list', '--parents', '-n', '1', $head) -Label 'Resolve activation commit parents'
            $parentFields = @($parentLine -split '\s+' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            if ($parentFields.Count -ne 2 -or $parentFields[0] -ne $head -or $parentFields[1] -ne $candidateFreezeHead) {
              throw "Activation must be a single direct child of frozen Runner candidate: $parentLine"
            }
            $record.activation_parent = $parentFields[1]
            $diffResult = Invoke-Issue380GitCapture -Arguments @('-C', $env:GITHUB_WORKSPACE, 'diff-tree', '--no-commit-id', '--no-renames', '--name-status', '-r', $candidateFreezeHead, $head)
            if ($diffResult.ExitCode -ne 0) { throw "Could not enumerate the activation diff (exit $($diffResult.ExitCode)): $($diffResult.Text)" }
            $activationDiff = [string[]]$diffResult.Lines
            $record.activation_diff = @($activationDiff)
            $expectedActivation = 'A' + [char]9 + '.github/workflows/issue-380-s1r4-p3-bounded-git-capture.yml'
            if ($activationDiff.Count -ne 1 -or [string]$activationDiff[0] -cne $expectedActivation) {
              throw "Activation diff must add only the designated Workflow file: $($activationDiff -join '; ')"
            }
            $statusResult = Invoke-Issue380GitCapture -Arguments @('-C', $env:GITHUB_WORKSPACE, 'status', '--porcelain', '--untracked-files=all')
            if ($statusResult.ExitCode -ne 0) { throw "Could not verify clean checkout (exit $($statusResult.ExitCode)): $($statusResult.Text)" }
            $record.checkout_status_output_count = $statusResult.OutputCount
            if ($statusResult.OutputCount -ne 0) { throw "Activation checkout is not clean: $($statusResult.Text)" }
            $record.checkout_status = 'CLEAN'
            $controlledFiles = @(
              [ordered]@{ path = 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'; source = 'af305adb8fba017dd9284a829b5101a209a9b4e2'; blob = 'c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa'; sha256 = '92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece' },
              [ordered]@{ path = 'scripts/issue380S1R4PathGuard.ps1'; source = '7a6fa69e5521d9759702c1923a9ade60a75819c3'; blob = '42f38d81b314e6faa3426920b7671179015b26c3'; sha256 = 'd38865aba8eccb444daba77ec4de572e64bcf2530f8e4f16c09b3aa7e7225ba9' },
              [ordered]@{ path = 'scripts/testIssue380S1R4PathGuard.ps1'; source = '7a6fa69e5521d9759702c1923a9ade60a75819c3'; blob = '28b62abac73d08b71828482bfd26ef0da60faac2'; sha256 = 'e0a6620456a04e08e3ef62cd001a01d6511dcd05e677680556862181221d4610' },
              [ordered]@{ path = 'scripts/issue380S0NsisObservabilityProbe.nsi'; source = '1f4951f3c93b2eb549d159fed2dd288992c625c8'; blob = '5805f8d04cd37420a63f3e156c3b6fd5b4012f33'; sha256 = '518b729388e36bbcb99dd8a3d82c60cad48a92660676230495d12e0a3437bade' }
            )
            $trackedResult = Invoke-Issue380GitCapture -Arguments @('-C', $env:GITHUB_WORKSPACE, 'ls-files')
            if ($trackedResult.ExitCode -ne 0 -or $trackedResult.OutputCount -eq 0) { throw "Could not enumerate tracked files (exit $($trackedResult.ExitCode), lines $($trackedResult.OutputCount))." }
            $gitattributesPaths = @($trackedResult.Lines | Where-Object { [System.IO.Path]::GetFileName([string]$_) -ceq '.gitattributes' })
            $attributeInventory = @()
            foreach ($attributePath in $gitattributesPaths) {
              $attributeFile = Join-Path $env:GITHUB_WORKSPACE $attributePath
              $attributeBlob = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', ('HEAD:' + [string]$attributePath)) -Label "Resolve .gitattributes blob $attributePath"
              $attributeSha = (Get-FileHash -LiteralPath $attributeFile -Algorithm SHA256).Hash.ToLowerInvariant()
              $attributeInventory += [ordered]@{ path = [string]$attributePath; git_blob = $attributeBlob; working_sha256 = $attributeSha }
            }
            $record.gitattributes_inventory = @($attributeInventory)
            if ($gitattributesPaths.Count -eq 0) { $record.gitattributes_inventory = @([ordered]@{ status = 'NO_TRACKED_GITATTRIBUTES' }) }
            $controlledPaths = @($controlledFiles | ForEach-Object { [string]$_.path })
            $attributeResult = Invoke-Issue380GitCapture -Arguments (@('-C', $env:GITHUB_WORKSPACE, 'check-attr', 'text', 'eol', 'filter', 'working-tree-encoding', '--') + $controlledPaths)
            $expectedAttributeRows = $controlledPaths.Count * 4
            if ($attributeResult.ExitCode -ne 0 -or $attributeResult.OutputCount -ne $expectedAttributeRows) {
              throw "git check-attr returned an incomplete result: exit=$($attributeResult.ExitCode) rows=$($attributeResult.OutputCount) expected=$expectedAttributeRows output=$($attributeResult.Text)"
            }
            $attributeRows = [string[]]$attributeResult.Lines
            $record.effective_transform_attributes = @($attributeRows)
            foreach ($attributeRow in $attributeRows) {
              $parts = [string]$attributeRow -split ': ', 3
              if ($parts.Count -ne 3 -or $parts[2] -notin @('unspecified', 'unset')) { throw "A Git attribute can transform controlled bytes: $attributeRow" }
            }
            $filterResult = Invoke-Issue380GitCapture -Arguments @('-C', $env:GITHUB_WORKSPACE, 'config', '--show-origin', '--get-regexp', '^filter\..*\.(clean|smudge|process|required)$')
            if ($filterResult.ExitCode -eq 1 -and $filterResult.OutputCount -eq 0) {
              $filterRows = @()
              $record.filter_configuration_state = 'NO_FILTERS_CONFIGURED'
            } elseif ($filterResult.ExitCode -eq 0 -and $filterResult.OutputCount -gt 0) {
              $filterRows = [string[]]$filterResult.Lines
              $record.filter_configuration_state = 'CONFIGURED'
            } else {
              throw "Could not inspect configured Git filters safely: exit=$($filterResult.ExitCode) lines=$($filterResult.OutputCount) output=$($filterResult.Text)"
            }
            $record.filter_configuration = @($filterRows)
            function Get-NewlineFacts([byte[]]$Bytes) {
              $lfCount = 0
              $crlfCount = 0
              for ($i = 0; $i -lt $Bytes.Length; $i++) {
                if ($Bytes[$i] -eq 10) {
                  $lfCount++
                  if ($i -gt 0 -and $Bytes[$i - 1] -eq 13) { $crlfCount++ }
                }
              }
              $bareLf = $lfCount - $crlfCount
              $format = if ($lfCount -eq 0) { 'NO_NEWLINE' } elseif ($crlfCount -eq 0) { 'LF_ONLY' } elseif ($bareLf -eq 0) { 'CRLF' } else { 'MIXED' }
              return [ordered]@{ format = $format; lf_count = $lfCount; crlf_count = $crlfCount; bare_lf_count = $bareLf }
            }
            foreach ($item in $controlledFiles) {
              $path = [string]$item.path
              $sourceBlob = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', ([string]$item.source + ':' + $path)) -Label "Resolve frozen source blob $path"
              $headBlob = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', ('HEAD:' + $path)) -Label "Resolve checked-out blob $path"
              $workingFile = Join-Path $env:GITHUB_WORKSPACE $path
              $workingBlob = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'hash-object', '--', $workingFile) -Label "Hash checked-out blob $path"
              $workingSha = (Get-FileHash -LiteralPath $workingFile -Algorithm SHA256).Hash.ToLowerInvariant()
              $newline = Get-NewlineFacts ([System.IO.File]::ReadAllBytes($workingFile))
              $identity = [ordered]@{ path = $path; frozen_source = [string]$item.source; expected_git_blob = [string]$item.blob; frozen_git_blob = $sourceBlob; checkout_git_blob = $headBlob; working_git_blob = $workingBlob; expected_raw_sha256 = [string]$item.sha256; working_raw_sha256 = $workingSha; expected_newline_format = 'LF_ONLY'; working_newline = $newline }
              if ($sourceBlob -cne $item.blob -or $headBlob -cne $item.blob -or $workingBlob -cne $item.blob -or $workingSha -cne $item.sha256 -or $newline.format -cne 'LF_ONLY') { throw "Frozen raw-byte identity mismatch before NSIS: $($identity | ConvertTo-Json -Compress)" }
              $record.controlled_file_identities += $identity
            }
            $p3RunnerFile = Join-Path $env:GITHUB_WORKSPACE $p3RunnerPath
            $frozenRunnerBlob = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', ($candidateFreezeHead + ':' + $p3RunnerPath)) -Label 'Resolve frozen P3 Runner blob'
            $activationRunnerBlob = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', ($head + ':' + $p3RunnerPath)) -Label 'Resolve activation P3 Runner blob'
            $workingRunnerBlob = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'hash-object', '--', $p3RunnerFile) -Label 'Hash working P3 Runner'
            $workingRunnerSha = (Get-FileHash -LiteralPath $p3RunnerFile -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($frozenRunnerBlob -ne $expectedRunnerBlob -or $activationRunnerBlob -ne $expectedRunnerBlob -or $workingRunnerBlob -ne $expectedRunnerBlob -or $workingRunnerSha -ne $expectedRunnerFileSha256) {
              throw "P3 Runner identity mismatch before NSIS: frozen_blob=$frozenRunnerBlob activation_blob=$activationRunnerBlob working_blob=$workingRunnerBlob raw_sha256=$workingRunnerSha"
            }
            $record.p3_runner_git_blob_sha = $workingRunnerBlob
            $record.p3_runner_file_sha256 = $workingRunnerSha
            $p2Tree = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', '86c21c14dd65bd2bbdb3056513489e0d4139bb47^{tree}') -Label 'Resolve frozen P2 Tree'
            if ($p2Tree -ne $record.expected_p2_tree) { throw "Frozen P2 Tree mismatch: $p2Tree" }
            $record.current_p2_tree = $p2Tree
            $p2Ancestor = Invoke-Issue380GitCapture -Arguments @('-C', $env:GITHUB_WORKSPACE, 'merge-base', '--is-ancestor', $record.expected_p2_head, 'HEAD')
            Assert-Issue380GitNoOutputSuccess -Result $p2Ancestor -Label 'Verify frozen P2 ancestry'
            $p1Tree = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', '7a6fa69e5521d9759702c1923a9ade60a75819c3^{tree}') -Label 'Resolve frozen P1 Tree'
            if ($p1Tree -ne $record.expected_p1_tree) { throw "Frozen P1 Tree mismatch: $p1Tree" }
            $p1Ancestor = Invoke-Issue380GitCapture -Arguments @('-C', $env:GITHUB_WORKSPACE, 'merge-base', '--is-ancestor', $record.expected_p1_head, 'HEAD')
            Assert-Issue380GitNoOutputSuccess -Result $p1Ancestor -Label 'Verify frozen P1 ancestry'
            foreach ($path in @('scripts/issue380S1R4PathGuard.ps1', 'scripts/testIssue380S1R4PathGuard.ps1')) {
              $frozen = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', "7a6fa69e5521d9759702c1923a9ade60a75819c3:$path") -Label "Resolve frozen P1 blob $path"
              $actual = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'hash-object', (Join-Path $env:GITHUB_WORKSPACE $path)) -Label "Hash working P1 file $path"
              if ($frozen -ne $actual) { throw "Frozen P1 source changed: $path" }
            }
            $probe = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', '1f4951f3c93b2eb549d159fed2dd288992c625c8:scripts/issue380S0NsisObservabilityProbe.nsi') -Label 'Resolve frozen S0 Probe blob'
            $actualProbe = Get-Issue380GitScalar -Arguments @('-C', $env:GITHUB_WORKSPACE, 'hash-object', (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S0NsisObservabilityProbe.nsi')) -Label 'Hash working S0 Probe'
            if ($probe -ne $actualProbe) { throw 'Frozen S0 Probe blob changed.' }
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
          $errorPath = Join-Path $evidenceRoot 'runner-error.raw.txt'
          $processPath = Join-Path $evidenceRoot 'runner-process.json'
          $observerPath = Join-Path $evidenceRoot 'process-observer.json'
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
            controlled_file_identities_rechecked_before_start = $false
            process_observer_status = 'NOT_STARTED'
            process_observer_states = @()
            process_observer_record_count = 0
            process_observer_error = $null
            process_id = $null
            arguments = $arguments
            working_directory = $env:GITHUB_WORKSPACE
            started_utc = $null
            finished_utc = $null
            timed_out = $false
            process_exited = $false
            exit_code = $null
            stdout_complete = $false
            stderr_complete = $false
            cleanup_results = [ordered]@{ kill_attempted = $false; kill_error = $null; wait_for_exit_succeeded = $false; observer_natural_wait_attempted = $false; observer_natural_wait_returned = $false; observer_natural_wait_outcome = 'NOT_ATTEMPTED'; observer_natural_completion_confirmed = $false; observer_natural_wait_error = $null; observer_output_collection_succeeded = $false; observer_error_collection_succeeded = $false; observer_stop_attempted = $false; observer_stop_error = $null; observer_remove_succeeded = $false; observer_remove_error = $null; stdout_closed = $false; stderr_closed = $false; process_disposed = $false; cleanup_errors = @() }
            wrapper_failure = $null
          }
          [System.IO.File]::WriteAllText($stdoutPath, '', [System.Text.UTF8Encoding]::new($false))
          [System.IO.File]::WriteAllText($stderrPath, '', [System.Text.UTF8Encoding]::new($false))
          [System.IO.File]::WriteAllText($errorPath, '', [System.Text.UTF8Encoding]::new($false))
          [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 10) + [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false))
          $process = $null
          $observerJob = $null
          $started = $false
          $completed = $false
          $timedOut = $false
          $observerFailedDuringRun = $false
          $stdoutTask = $null
          $stderrTask = $null
          $observerRows = @()
          $observerErrors = [System.Collections.Generic.List[string]]::new()
          $observerNaturalCompletionConfirmed = $false
          $cleanupErrors = [System.Collections.Generic.List[string]]::new()
          $runFailure = $null
          $startedAt = $null
          $observerStateHistory = [System.Collections.Generic.List[string]]::new()
          $gitIdentityChecks = [System.Collections.Generic.List[object]]::new()
          function Invoke-Issue380GitProcess([string[]]$GitArguments) {
            $gitCommand = Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1
            if ($null -eq $gitCommand -or [string]::IsNullOrWhiteSpace([string]$gitCommand.Path)) { throw 'Git executable is unavailable for the pre-start identity check.' }
            $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
            $startInfo.FileName = [string]$gitCommand.Path
            $startInfo.WorkingDirectory = $env:GITHUB_WORKSPACE
            $startInfo.UseShellExecute = $false
            $startInfo.CreateNoWindow = $true
            $startInfo.RedirectStandardOutput = $true
            $startInfo.RedirectStandardError = $true
            $startInfo.StandardOutputEncoding = [System.Text.UTF8Encoding]::new($false)
            $startInfo.StandardErrorEncoding = [System.Text.UTF8Encoding]::new($false)
            foreach ($argument in $GitArguments) { [void]$startInfo.ArgumentList.Add([string]$argument) }
            $gitProcess = [System.Diagnostics.Process]::new()
            $gitProcess.StartInfo = $startInfo
            $startedGit = $false
            $record = [ordered]@{ arguments = @($GitArguments); process_id = $null; exit_code = $null; timed_out = $false; stdout = ''; stderr = ''; launch_error = $null }
            try {
              if (-not $gitProcess.Start()) { throw 'Git Process.Start returned false.' }
              $startedGit = $true
              $record.process_id = $gitProcess.Id
              $stdoutTask = $gitProcess.StandardOutput.ReadToEndAsync()
              $stderrTask = $gitProcess.StandardError.ReadToEndAsync()
              if (-not $gitProcess.WaitForExit(20000)) {
                $record.timed_out = $true
                $gitProcess.Kill($true)
                if (-not $gitProcess.WaitForExit(5000)) { throw 'Git process survived bounded tree termination.' }
              }
              $record.exit_code = if ($gitProcess.HasExited) { [int]$gitProcess.ExitCode } else { $null }
              $record.stdout = $stdoutTask.GetAwaiter().GetResult()
              $record.stderr = $stderrTask.GetAwaiter().GetResult()
            } catch {
              $record.launch_error = $_.Exception.ToString()
            } finally {
              if ($startedGit) {
                try {
                  if (-not $gitProcess.HasExited) {
                    $gitProcess.Kill($true)
                    if (-not $gitProcess.WaitForExit(5000)) { throw 'Git process cleanup wait expired.' }
                  }
                } catch { $record.launch_error = "Git cleanup failed: $($_.Exception.ToString())" }
              }
              $gitProcess.Dispose()
              $gitIdentityChecks.Add($record)
              $identity.git_identity_checks = @($gitIdentityChecks.ToArray())
              [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 12) + [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false))
            }
            return [pscustomobject]$record
          }
          function Get-Issue380RequiredGitScalar([string[]]$GitArguments, [string]$Label) {
            $capture = Invoke-Issue380GitProcess -GitArguments $GitArguments
            if ($capture.timed_out -or $capture.exit_code -ne 0 -or $null -ne $capture.launch_error) { throw "$Label failed: exit=$($capture.exit_code) timeout=$($capture.timed_out) error=$($capture.launch_error) stderr=$($capture.stderr)" }
            if (-not [string]::IsNullOrEmpty($capture.stderr)) { throw "$Label wrote unexpected stderr: $($capture.stderr)" }
            $match = [regex]::Match([string]$capture.stdout, '\A([^\r\n]+)(?:\r?\n)?\z')
            if (-not $match.Success -or [string]::IsNullOrWhiteSpace($match.Groups[1].Value)) { throw "$Label must return exactly one non-empty line." }
            return $match.Groups[1].Value.Trim()
          }
          try {
            if (-not (Test-Path -LiteralPath $pwsh -PathType Leaf)) { throw "PowerShell image is missing: $pwsh" }
            $identity.image_sha256 = (Get-FileHash -LiteralPath $pwsh -Algorithm SHA256).Hash.ToLowerInvariant()
            $candidateFreezeHead = 'af305adb8fba017dd9284a829b5101a209a9b4e2'
            $expectedFiles = @(
              [ordered]@{ path = 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'; source = $candidateFreezeHead; blob = 'c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa'; sha256 = '92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece' },
              [ordered]@{ path = 'scripts/issue380S1R4PathGuard.ps1'; source = '7a6fa69e5521d9759702c1923a9ade60a75819c3'; blob = '42f38d81b314e6faa3426920b7671179015b26c3'; sha256 = 'd38865aba8eccb444daba77ec4de572e64bcf2530f8e4f16c09b3aa7e7225ba9' },
              [ordered]@{ path = 'scripts/testIssue380S1R4PathGuard.ps1'; source = '7a6fa69e5521d9759702c1923a9ade60a75819c3'; blob = '28b62abac73d08b71828482bfd26ef0da60faac2'; sha256 = 'e0a6620456a04e08e3ef62cd001a01d6511dcd05e677680556862181221d4610' },
              [ordered]@{ path = 'scripts/issue380S0NsisObservabilityProbe.nsi'; source = '1f4951f3c93b2eb549d159fed2dd288992c625c8'; blob = '5805f8d04cd37420a63f3e156c3b6fd5b4012f33'; sha256 = '518b729388e36bbcb99dd8a3d82c60cad48a92660676230495d12e0a3437bade' }
            )
            foreach ($item in $expectedFiles) {
              $workingPath = Join-Path $env:GITHUB_WORKSPACE $item.path
              $frozenBlob = Get-Issue380RequiredGitScalar -GitArguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', ($item.source + ':' + $item.path)) -Label "Resolve frozen Git blob: $($item.path)"
              $workingBlob = Get-Issue380RequiredGitScalar -GitArguments @('-C', $env:GITHUB_WORKSPACE, 'hash-object', '--', $workingPath) -Label "Resolve working Git blob: $($item.path)"
              $workingSha = (Get-FileHash -LiteralPath $workingPath -Algorithm SHA256).Hash.ToLowerInvariant()
              if ($frozenBlob -cne $item.blob -or $workingBlob -cne $item.blob -or $workingSha -cne $item.sha256) {
                throw "Controlled byte identity changed before Runner start: path=$($item.path) frozen_blob=$frozenBlob working_blob=$workingBlob raw_sha256=$workingSha"
              }
            }
            $runnerRelative = 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'
            $frozenRunnerBlob = Get-Issue380RequiredGitScalar -GitArguments @('-C', $env:GITHUB_WORKSPACE, 'rev-parse', ($candidateFreezeHead + ':' + $runnerRelative)) -Label 'Resolve frozen Runner blob immediately before process start'
            $workingRunnerBlob = Get-Issue380RequiredGitScalar -GitArguments @('-C', $env:GITHUB_WORKSPACE, 'hash-object', '--', $runner) -Label 'Resolve working Runner blob immediately before process start'
            $workingRunnerSha = (Get-FileHash -LiteralPath $runner -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($frozenRunnerBlob -ne 'c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa' -or $workingRunnerBlob -ne 'c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa' -or $workingRunnerSha -ne '92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece') {
              throw "P3 Runner identity changed immediately before process start: frozen_blob=$frozenRunnerBlob working_blob=$workingRunnerBlob raw_sha256=$workingRunnerSha"
            }
            $identity.p3_runner_git_blob_sha = $workingRunnerBlob
            $identity.p3_runner_file_sha256 = $workingRunnerSha
            $identity.p3_runner_identity_rechecked_before_start = $true
            $identity.controlled_file_identities_rechecked_before_start = $true
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
            # Resource boundary: every operation from Process.Start through result persistence is covered by this try/catch/finally.
            if (-not $process.Start()) { throw 'Could not start the P3 Runner process.' }
            $started = $true
            $identity.process_id = [int]$process.Id
            $identity.started_utc = $startedAt.ToString('o')
            $identity.status = 'RUNNING'
            $stdoutTask = $process.StandardOutput.ReadToEndAsync()
            $stderrTask = $process.StandardError.ReadToEndAsync()
            $observerJob = Start-Job -ArgumentList ([int]$process.Id), (Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-task') -ScriptBlock {
              param([int]$rootProcessId, [string]$controlledTaskRoot)
              $ErrorActionPreference = 'Stop'
              [pscustomobject]@{ record_type = 'observer_ready'; process_id = $rootProcessId; emitted_utc = [DateTime]::UtcNow.ToString('o') }
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
                      [pscustomobject]@{ record_type = 'descendant'; observed_utc = [DateTime]::UtcNow.ToString('o'); process_id = $childPid; parent_process_id = $parentPid; image_path = $imagePath; command_line = [string]$item.CommandLine; creation_date = [string]$item.CreationDate; outside_task_root_or_image_unavailable = $outsideTaskRoot }
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
                      [pscustomobject]@{ record_type = 'descendant'; observed_utc = [DateTime]::UtcNow.ToString('o'); process_id = $childPid; parent_process_id = $parentPid; image_path = $imagePath; command_line = [string]$item.CommandLine; creation_date = [string]$item.CreationDate; outside_task_root_or_image_unavailable = $outsideTaskRoot }
                    }
                  }
                }
                Start-Sleep -Milliseconds 100
              }
            }
            $readyDeadline = [DateTime]::UtcNow.AddSeconds(5)
            $observerReady = $false
            while ([DateTime]::UtcNow -lt $readyDeadline -and -not $observerReady) {
              $observerState = [string]$observerJob.State
              if ($observerState -notin $observerStateHistory) { $observerStateHistory.Add($observerState) }
              switch ($observerState) {
                'NotStarted' { Start-Sleep -Milliseconds 50 }
                'Running' {
                  $observerSignals = @(Receive-Job -Job $observerJob -Keep -ErrorAction SilentlyContinue)
                  $observerReady = @($observerSignals | Where-Object { $_.record_type -eq 'observer_ready' -and [int]$_.process_id -eq [int]$process.Id }).Count -gt 0
                  if (-not $observerReady) { Start-Sleep -Milliseconds 50 }
                }
                'Completed' { throw 'Observer completed before publishing readiness; terminating the Runner.' }
                'Failed' { throw 'Observer failed before publishing readiness; terminating the Runner.' }
                'Stopped' { throw 'Observer stopped before publishing readiness; terminating the Runner.' }
                default { throw "Observer entered unsupported readiness state: $observerState" }
              }
            }
            if (-not $observerReady) { throw 'Observer readiness handshake timed out after five seconds; terminating the Runner.' }
            $identity.process_observer_status = 'RUNNING_DESCENDANT_POLL'
            [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 10) + [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false))
            $runnerDeadline = [DateTime]::UtcNow.AddMilliseconds(540000)
            while ([DateTime]::UtcNow -lt $runnerDeadline) {
              if ($process.WaitForExit(250)) { $completed = $true; break }
              $observerState = [string]$observerJob.State
              if ($observerState -notin $observerStateHistory) { $observerStateHistory.Add($observerState) }
              switch ($observerState) {
                'NotStarted' { $observerFailedDuringRun = $true; $observerErrors.Add('Observer returned to NotStarted after readiness.') }
                'Running' { }
                'Completed' { $observerFailedDuringRun = $true; $observerErrors.Add('Observer completed while Runner was still active.') }
                'Failed' { $observerFailedDuringRun = $true; $observerErrors.Add('Observer failed while Runner was still active.') }
                'Stopped' { $observerFailedDuringRun = $true; $observerErrors.Add('Observer stopped while Runner was still active.') }
                default { $observerFailedDuringRun = $true; $observerErrors.Add("Observer entered unsupported state: $observerState") }
              }
              if ($observerFailedDuringRun) { throw 'Observer ceased to be healthy; the Runner process tree will be terminated.' }
            }
            if (-not $completed) { $timedOut = $true; throw 'P3 Runner exceeded its bounded 540-second deadline; the process tree will be terminated.' }
            if ($process.ExitCode -ne 0) { throw "P3 Runner returned exit code $($process.ExitCode)." }
            $identity.cleanup_results.observer_natural_wait_attempted = $true
            try {
              # Wait only after a zero-exit Runner; this bounded wait lets the three required tail snapshots finish naturally.
              $observerWaitResult = Wait-Job -Job $observerJob -Timeout 5 -ErrorAction Stop
              $identity.cleanup_results.observer_natural_wait_returned = $null -ne $observerWaitResult
              $observerState = [string]$observerJob.State
              if ($observerState -notin $observerStateHistory) { $observerStateHistory.Add($observerState) }
              if ($null -eq $observerWaitResult) {
                $identity.cleanup_results.observer_natural_wait_outcome = 'TIMED_OUT'
                $observerErrors.Add('Observer natural completion timed out five seconds after Runner exit.')
                throw 'Observer did not complete naturally within the bounded five-second post-exit wait.'
              }
              if ($observerState -ne 'Completed') {
                $identity.cleanup_results.observer_natural_wait_outcome = $observerState.ToUpperInvariant()
                $observerErrors.Add("Observer reached $observerState instead of natural Completed state.")
                throw "Observer did not complete naturally after Runner exit: state=$observerState"
              }
              $observerNaturalCompletionConfirmed = $true
              $identity.cleanup_results.observer_natural_completion_confirmed = $true
              $identity.cleanup_results.observer_natural_wait_outcome = 'COMPLETED'
            } catch {
              $identity.cleanup_results.observer_natural_wait_error = $_.Exception.ToString()
              throw
            }
          } catch {
            $runFailure = $_.Exception.ToString()
          } finally {
            if ($null -ne $process -and $started) {
              try {
                if (-not $process.HasExited) {
                  $identity.cleanup_results.kill_attempted = $true
                  try { $process.Kill($true) } catch { $identity.cleanup_results.kill_error = $_.Exception.ToString(); $cleanupErrors.Add("Kill: $($_.Exception.Message)") }
                }
              } catch { $cleanupErrors.Add("Process-state check before Kill: $($_.Exception.Message)") }
              try {
                $identity.cleanup_results.wait_for_exit_succeeded = $process.WaitForExit(10000)
                if (-not $identity.cleanup_results.wait_for_exit_succeeded) {
                  $identity.cleanup_results.kill_attempted = $true
                  try { $process.Kill($true) } catch { $cleanupErrors.Add("Second Kill: $($_.Exception.Message)") }
                  $identity.cleanup_results.wait_for_exit_succeeded = $process.WaitForExit(10000)
                }
                if (-not $identity.cleanup_results.wait_for_exit_succeeded) { $cleanupErrors.Add('Runner process tree remained alive after two tree-kill attempts and bounded waits.') }
              } catch { $cleanupErrors.Add("WaitForExit: $($_.Exception.Message)") }
            }
            if ($null -ne $observerJob) {
              try {
                $state = [string]$observerJob.State
                if ($state -in @('NotStarted', 'Running')) {
                  $identity.cleanup_results.observer_stop_attempted = $true
                  try { Stop-Job -Job $observerJob -ErrorAction Stop } catch { $identity.cleanup_results.observer_stop_error = $_.Exception.ToString(); $cleanupErrors.Add("Stop-Job: $($_.Exception.Message)") }
                }
              } catch { $cleanupErrors.Add("Observer state/stop: $($_.Exception.Message)") }
              try {
                $observerDeadline = [DateTime]::UtcNow.AddSeconds(5)
                while ([DateTime]::UtcNow -lt $observerDeadline -and [string]$observerJob.State -in @('NotStarted', 'Running')) { Start-Sleep -Milliseconds 50 }
                if ([string]$observerJob.State -in @('NotStarted', 'Running')) { $cleanupErrors.Add('Observer job did not reach a terminal state within five seconds.') }
              } catch { $cleanupErrors.Add("Observer terminal wait: $($_.Exception.Message)") }
              try {
                $observerSignals = @(Receive-Job -Job $observerJob -Keep -ErrorAction Stop)
                $observerRows = @($observerSignals | Where-Object { $_.record_type -eq 'descendant' })
                $identity.cleanup_results.observer_output_collection_succeeded = $true
              } catch { $cleanupErrors.Add("Observer output collection: $($_.Exception.Message)") }
              try {
                $jobErrors = @($observerJob.ChildJobs | ForEach-Object { $_.Error } | ForEach-Object { $_.ToString() })
                foreach ($jobError in $jobErrors) { $observerErrors.Add([string]$jobError) }
                $identity.cleanup_results.observer_error_collection_succeeded = $true
              } catch { $cleanupErrors.Add("Observer error stream collection: $($_.Exception.Message)") }
              $observerFinalState = [string]$observerJob.State
              switch ($observerFinalState) {
                'Completed' {
                  if ($observerNaturalCompletionConfirmed -and $identity.cleanup_results.observer_output_collection_succeeded -and $identity.cleanup_results.observer_error_collection_succeeded -and $observerErrors.Count -eq 0) {
                    $identity.process_observer_status = 'COMPLETED'
                  } elseif ($observerErrors.Count -gt 0) {
                    $identity.process_observer_status = 'COMPLETED_WITH_ERRORS'
                  } else {
                    $identity.process_observer_status = 'COMPLETED_UNCONFIRMED'
                  }
                }
                'Failed' { $identity.process_observer_status = 'FAILED' }
                'Stopped' { $identity.process_observer_status = 'STOPPED' }
                'NotStarted' { $identity.process_observer_status = 'NOT_STARTED' }
                'Running' { $identity.process_observer_status = 'STILL_RUNNING' }
                default { $identity.process_observer_status = 'UNSUPPORTED_STATE' }
              }
              if ($identity.process_observer_status -ne 'COMPLETED' -and $null -eq $runFailure) {
                $runFailure = "Observer completion contract failed: state=$observerFinalState status=$($identity.process_observer_status) errors=$($observerErrors -join '; ')"
              }
              try { Remove-Job -Job $observerJob -Force -ErrorAction Stop; $identity.cleanup_results.observer_remove_succeeded = $true } catch { $identity.cleanup_results.observer_remove_error = $_.Exception.ToString(); $cleanupErrors.Add("Remove-Job: $($_.Exception.Message)") }
            }
            $stdout = '[stdout was not captured]'
            $stderr = '[stderr was not captured]'
            if ($null -ne $stdoutTask) {
              try {
                $identity.stdout_complete = $stdoutTask.Wait(5000)
                if ($identity.stdout_complete) { $stdout = $stdoutTask.GetAwaiter().GetResult() } else { $stdout = '[stdout did not close within five seconds]' }
              } catch { $cleanupErrors.Add("stdout collection: $($_.Exception.Message)") }
            }
            if ($null -ne $stderrTask) {
              try {
                $identity.stderr_complete = $stderrTask.Wait(5000)
                if ($identity.stderr_complete) { $stderr = $stderrTask.GetAwaiter().GetResult() } else { $stderr = '[stderr did not close within five seconds]' }
              } catch { $cleanupErrors.Add("stderr collection: $($_.Exception.Message)") }
            }
            if ($null -ne $process) {
              try { $process.StandardOutput.Close(); $identity.cleanup_results.stdout_closed = $true } catch { $cleanupErrors.Add("stdout close: $($_.Exception.Message)") }
              try { $process.StandardError.Close(); $identity.cleanup_results.stderr_closed = $true } catch { $cleanupErrors.Add("stderr close: $($_.Exception.Message)") }
              try { $identity.process_exited = $started -and $process.HasExited; if ($identity.process_exited) { $identity.exit_code = [int]$process.ExitCode } } catch { $cleanupErrors.Add("Exit-state capture: $($_.Exception.Message)") }
              try { $process.Dispose(); $identity.cleanup_results.process_disposed = $true } catch { $cleanupErrors.Add("Process.Dispose: $($_.Exception.Message)") }
            }
            $identity.process_observer_states = @($observerStateHistory.ToArray())
            $identity.process_observer_record_count = $observerRows.Count
            $identity.process_observer_error = ($observerErrors -join [Environment]::NewLine)
            $identity.timed_out = $timedOut
            $identity.finished_utc = [DateTime]::UtcNow.ToString('o')
            $identity.wrapper_failure = $runFailure
            if (-not $started) { $identity.status = 'NOT_STARTED' }
            elseif (-not $identity.process_exited) { $identity.status = 'PROCESS_EXIT_UNCONFIRMED' }
            elseif ($timedOut) { $identity.status = 'TIMED_OUT_TREE_KILLED' }
            elseif ($null -ne $runFailure) { $identity.status = 'FAILED' }
            else { $identity.status = 'COMPLETED' }
            try { [System.IO.File]::WriteAllText($stdoutPath, $stdout, [System.Text.UTF8Encoding]::new($false)) } catch { $cleanupErrors.Add("stdout evidence write: $($_.Exception.Message)") }
            try { [System.IO.File]::WriteAllText($stderrPath, $stderr, [System.Text.UTF8Encoding]::new($false)) } catch { $cleanupErrors.Add("stderr evidence write: $($_.Exception.Message)") }
            try { [System.IO.File]::WriteAllText($errorPath, $(if ($null -eq $runFailure) { 'No wrapper exception.' } else { $runFailure }), [System.Text.UTF8Encoding]::new($false)) } catch { $cleanupErrors.Add("error evidence write: $($_.Exception.Message)") }
            $observerRecord = [ordered]@{ status = $identity.process_observer_status; method = '100ms Win32_Process descendant polling plus three post-exit snapshots; bounded polling only, not proof of no-copy'; runner_process_id = $identity.process_id; completed_utc = [DateTime]::UtcNow.ToString('o'); state_history = @($observerStateHistory.ToArray()); record_count = $observerRows.Count; descendant_processes = @($observerRows); errors = @($observerErrors.ToArray()) }
            try { [System.IO.File]::WriteAllText($observerPath, ((ConvertTo-Json -InputObject $observerRecord -Depth 14) + [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false)) } catch { $cleanupErrors.Add("observer evidence write: $($_.Exception.Message)") }
            $identity.cleanup_results.cleanup_errors = @($cleanupErrors.ToArray())
            try { [System.IO.File]::WriteAllText($processPath, ((ConvertTo-Json -InputObject $identity -Depth 12) + [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false)) } catch { $cleanupErrors.Add("process evidence write: $($_.Exception.Message)") }
          }
          if ($cleanupErrors.Count -ne 0) { throw "Wrapper cleanup was incomplete: $($cleanupErrors -join '; ')" }
          if ($null -ne $runFailure -or -not $identity.process_exited -or $identity.exit_code -ne 0 -or -not $identity.stdout_complete -or -not $identity.stderr_complete -or $identity.process_observer_status -ne 'COMPLETED') {
            throw "P3 Runner or observer failed closed: exit=$($identity.exit_code) timed_out=$timedOut observer=$($identity.process_observer_status) failure=$runFailure"
          }

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
            runner_process_exited = $false
            runner_wait_for_exit_succeeded = $false
            runner_wrapper_failure = $null
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
            if ($null -ne $processRecord) {
              $assessment.runner_exit_code = $processRecord.exit_code
              $assessment.runner_timed_out = $processRecord.timed_out
              $assessment.runner_process_exited = $processRecord.process_exited -eq $true
              $assessment.runner_wait_for_exit_succeeded = $processRecord.cleanup_results.wait_for_exit_succeeded -eq $true
              $assessment.runner_wrapper_failure = $processRecord.wrapper_failure
              if ($null -ne $processRecord.process_id) {
                $liveRunnerParent = Get-Process -Id ([int]$processRecord.process_id) -ErrorAction SilentlyContinue
                if ($null -ne $liveRunnerParent) { throw "Runner parent PID is still alive after wrapper cleanup: $($processRecord.process_id)" }
              }
              if (-not $assessment.runner_process_exited -or -not $assessment.runner_wait_for_exit_succeeded) { throw 'Runner parent exit was not proven; preserving TaskRoot and Registry state.' }
            }
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
            if ($env:PREFLIGHT_OUTCOME -ne 'success' -or $env:RUNNER_OUTCOME -ne 'success' -or $null -eq $processRecord -or $processRecord.exit_code -ne 0 -or $processRecord.timed_out -or $processRecord.process_exited -ne $true -or $processRecord.cleanup_results.wait_for_exit_succeeded -ne $true -or $assessment.process_observer_status -ne 'COMPLETED' -or $null -eq $runnerResult -or $runnerResult.status -ne 'PASS' -or -not $assessment.required_primary_assertions_pass -or -not $assessment.hkcu_absent_after_run -or -not $assessment.task_root_absent_after_run) {
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
          $expectedRef = 'refs/heads/research/issue-380-s1r4-p3-runner-null-safety-activation'
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
