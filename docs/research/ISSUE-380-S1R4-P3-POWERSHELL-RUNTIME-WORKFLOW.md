# #380 S1R4-P3 Inactive PowerShell Runtime Workflow Draft

This document contains a complete, **inactive** one-shot workflow proposal. The YAML remains documentation and is not present under `.github/workflows`. No Hosted job is authorized by this draft.

The `OWNER_REVIEWED_PLAN_HEAD_REQUIRED` parent value is an intentional activation lock. After a separate Owner GO, activation would replace it with the exact accepted plan HEAD, re-run the static checks, and add only the resulting workflow file as one direct child commit. A missing or unchanged sentinel fails before the harness starts.

```yaml
name: Issue 380 S1R4-P3 bounded Git capture PowerShell 7 runtime

on:
  push:
    branches:
      - qualification/s1r4-p3-windows-runtime-r2
    paths:
      - .github/workflows/issue-380-s1r4-p3-git-runtime-r2.yml

permissions:
  contents: read

concurrency:
  group: issue-380-s1r4-p3-git-runtime-r2
  cancel-in-progress: false

jobs:
  powershell-runtime:
    name: R2 candidate Git capture functions only
    if: >-
      github.repository == 'ArdenZC/Zen-Canvas' &&
      github.ref == 'refs/heads/qualification/s1r4-p3-windows-runtime-r2' &&
      github.run_attempt == 1
    runs-on: windows-2022
    timeout-minutes: 10
    env:
      GIT_CONFIG_COUNT: '2'
      GIT_CONFIG_KEY_0: core.autocrlf
      GIT_CONFIG_VALUE_0: 'false'
      GIT_CONFIG_KEY_1: core.eol
      GIT_CONFIG_VALUE_1: lf
    steps:
      - name: Bootstrap run-owned temporary roots
        id: bootstrap
        shell: pwsh
        run: |
          Set-StrictMode -Version Latest
          $ErrorActionPreference = 'Stop'
          if ($env:GITHUB_REPOSITORY -ne 'ArdenZC/Zen-Canvas') { throw "Unexpected repository: $env:GITHUB_REPOSITORY" }
          if ($env:GITHUB_REF -ne 'refs/heads/qualification/s1r4-p3-windows-runtime-r2') { throw "Unexpected ref: $env:GITHUB_REF" }
          if ($env:GITHUB_RUN_ATTEMPT -ne '1') { throw "One-shot contract refuses attempt $env:GITHUB_RUN_ATTEMPT" }
          $runRoot = Join-Path $env:RUNNER_TEMP ("issue380-s1r4-p3-git-runtime-{0}" -f $env:GITHUB_RUN_ID)
          if (Test-Path -LiteralPath $runRoot) { throw "Refusing to reuse run root: $runRoot" }
          $evidenceRoot = Join-Path $runRoot 'evidence'
          $taskRoot = Join-Path $runRoot 'task'
          New-Item -ItemType Directory -Path $evidenceRoot -ErrorAction Stop | Out-Null
          New-Item -ItemType Directory -Path $taskRoot -ErrorAction Stop | Out-Null
          $owner = [ordered]@{
            repository = $env:GITHUB_REPOSITORY
            ref = $env:GITHUB_REF
            run_id = $env:GITHUB_RUN_ID
            run_attempt = $env:GITHUB_RUN_ATTEMPT
            triggering_sha = $env:GITHUB_SHA
            owner_token = ($env:GITHUB_REPOSITORY + '|' + $env:GITHUB_REF + '|' + $env:GITHUB_RUN_ID + '|' + $env:GITHUB_RUN_ATTEMPT + '|' + $env:GITHUB_SHA)
            created_utc = [DateTime]::UtcNow.ToString('o')
          }
          foreach ($path in @((Join-Path $evidenceRoot 'runtime-owner.json'), (Join-Path $taskRoot 'runtime-task-owner.json'))) {
            [System.IO.File]::WriteAllText($path, ((ConvertTo-Json -InputObject $owner -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))
          }
          $workflowRun = [ordered]@{
            repository = $env:GITHUB_REPOSITORY
            workflow_ref = $env:GITHUB_WORKFLOW_REF
            ref = $env:GITHUB_REF
            sha = $env:GITHUB_SHA
            run_id = $env:GITHUB_RUN_ID
            run_attempt = $env:GITHUB_RUN_ATTEMPT
            runner_os = $env:RUNNER_OS
            runner_arch = $env:RUNNER_ARCH
            image_os = $env:ImageOS
            image_version = $env:ImageVersion
            windows_version = [System.Environment]::OSVersion.VersionString
            powershell_version = $PSVersionTable.PSVersion.ToString()
            powershell_path = Join-Path $PSHOME 'pwsh.exe'
            git_path = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Path
            created_utc = [DateTime]::UtcNow.ToString('o')
          }
          [System.IO.File]::WriteAllText((Join-Path $evidenceRoot 'workflow-run.json'), ((ConvertTo-Json -InputObject $workflowRun -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))

      - name: Checkout exact activation child with raw LF policy in effect
        if: ${{ steps.bootstrap.outcome == 'success' }}
        uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4
        with:
          fetch-depth: 0
          persist-credentials: false

      - name: Verify one-shot activation, source identity, and checkout bytes
        id: preflight
        if: ${{ steps.bootstrap.outcome == 'success' }}
        continue-on-error: true
        shell: pwsh
        run: |
          Set-StrictMode -Version Latest
          $ErrorActionPreference = 'Stop'
          $PSNativeCommandUseErrorActionPreference = $false
          $freezeHead = 'af305adb8fba017dd9284a829b5101a209a9b4e2'
          $freezeTree = '9d7716add5fde99c6e797065f336e0d0cf86a510'
          $runnerPath = 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'
          $historicalRunnerBlob = 'c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa'
          $historicalRunnerSha = '92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece'
          $runnerBlob = '7d6081468d5fce46a18d1af2aaaf1d4091a89d5f'
          $runnerSha = 'd8466b47ea548b8482c30db88e1fbb47025f64d54b3f18d465c320658deca995'
          $p1Head = '7a6fa69e5521d9759702c1923a9ade60a75819c3'
          $p1Guard = 'scripts/issue380S1R4PathGuard.ps1'
          $p1GuardBlob = '42f38d81b314e6faa3426920b7671179015b26c3'
          $p1GuardSha = 'd38865aba8eccb444daba77ec4de572e64bcf2530f8e4f16c09b3aa7e7225ba9'
          $p1Test = 'scripts/testIssue380S1R4PathGuard.ps1'
          $p1TestBlob = '28b62abac73d08b71828482bfd26ef0da60faac2'
          $p1TestSha = 'e0a6620456a04e08e3ef62cd001a01d6511dcd05e677680556862181221d4610'
          $s0Head = '1f4951f3c93b2eb549d159fed2dd288992c625c8'
          $s0Probe = 'scripts/issue380S0NsisObservabilityProbe.nsi'
          $s0Blob = '5805f8d04cd37420a63f3e156c3b6fd5b4012f33'
          $s0Sha = '518b729388e36bbcb99dd8a3d82c60cad48a92660676230495d12e0a3437bade'
          $oldWorkflow = '.github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml'
          $oldWorkflowBlob = '05bcc48e531f662d7aa52e4887f329d2b9710739'
          $oldWorkflowSha = '19bdbce52a890fd340fb9d0d84d45666298080fc2934a0ff1d551d553eaab022'
          $harnessPath = 'scripts/issue380S1R4P3GitRuntimeQualification.ps1'
          $harnessBlob = 'ab92741ba7836f7b8f99b877d62dd650fa2e7d8f'
          $harnessSha = 'e130d8772c9afa927b00c4e18b8ed629db9f261ec50f307b7eb5e6c715dc91d9'
          $childPath = 'scripts/issue380S1R4P3GitRuntimeChild.ps1'
          $childBlob = 'b2f9895172335160315cfd5cd4d1715406704bcb'
          $childSha = 'fb453ed1d79a2079dc3eae0c82d4433090511654a5e366d4620ff2fa07e3575e'
          $controllerPath = 'scripts/issue380S1R4P3GitRuntimeController.ps1'
          $controllerBlob = 'b52d1304aeb1e273cd6a264a2a2d80e6e2f0fa94'
          $controllerSha = '7bd66ecac889bcf0e552a83b905a2a1973ba0b6face7bed482b42e4517970068'
          $artifactPath = 'scripts/issue380S1R4P3GitRuntimeArtifact.ps1'
          $artifactBlob = 'fc9f946f28ea6f1fd9b8fe0ecece8a6642e5f014'
          $artifactSha = '9738659b1829592ffbe62f17952c30dd5cefdd12559eff4fdbca7e9e2000b81f'
          $validatorPath = 'scripts/validateIssue380S1R4P3GitRuntimePlan.py'
          $validatorBlob = '613df81be88ebf421e24d1a8e05634184e0219dd'
          $validatorSha = '1f7cb5720e44597925b4ffc7822307099cf661e7650b983e1183b93817799d53'
          $activationWorkflow = '.github/workflows/issue-380-s1r4-p3-git-runtime-r2.yml'
          $approvedPlanParent = 'OWNER_REVIEWED_PLAN_HEAD_REQUIRED'
          if ($approvedPlanParent -eq 'OWNER_REVIEWED_PLAN_HEAD_REQUIRED') { throw 'Owner-reviewed plan parent has not been bound; refusing activation.' }
          if ($env:GITHUB_REPOSITORY -ne 'ArdenZC/Zen-Canvas' -or $env:GITHUB_REF -ne 'refs/heads/qualification/s1r4-p3-windows-runtime-r2') { throw 'Repository or branch does not match the one-shot contract.' }
          if ($env:GITHUB_RUN_ATTEMPT -ne '1') { throw "One-shot contract refuses attempt $env:GITHUB_RUN_ATTEMPT" }

          function Get-GitText([string[]]$Arguments) {
            $lines = & git @Arguments
            $exitCode = $LASTEXITCODE
            if ($exitCode -ne 0) { throw "git $($Arguments -join ' ') exited $exitCode" }
            if ($null -eq $lines) { return [string]::Empty }
            $parts = [System.Collections.Generic.List[string]]::new()
            foreach ($line in @($lines)) { if ($null -ne $line) { $parts.Add([string]$line) } }
            return [string]::Join("`n", $parts.ToArray())
          }
          function Get-GitScalar([string[]]$Arguments) {
            $value = Get-GitText $Arguments
            if ([string]::IsNullOrWhiteSpace($value) -or $value.Contains("`n")) { throw "git $($Arguments -join ' ') did not return one non-empty scalar." }
            return $value.Trim()
          }
          function Get-RawSha([string]$Path) {
            return (Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
          }

          $head = Get-GitScalar @('rev-parse', 'HEAD')
          if ($head -cne $env:GITHUB_SHA) { throw "Checkout SHA differs from event SHA: $head / $env:GITHUB_SHA" }
          $parent = Get-GitScalar @('rev-parse', 'HEAD^')
          if ($parent -cne $approvedPlanParent) { throw "Activation parent mismatch: expected=$approvedPlanParent actual=$parent" }
          $changed = Get-GitText @('diff-tree', '--no-commit-id', '--name-status', '-r', $parent, 'HEAD')
          if ($changed -cne "A`t$activationWorkflow") { throw "Activation commit must add only $activationWorkflow; actual=[$changed]" }
          if ((Get-GitText @('status', '--porcelain=v1', '--untracked-files=all')) -ne '') { throw 'Activation checkout is not clean.' }
          if ((Get-GitScalar @('rev-parse', "$freezeHead^{tree}")) -cne $freezeTree) { throw 'Frozen code Tree mismatch.' }
          Get-GitText @('merge-base', '--is-ancestor', $freezeHead, 'HEAD') | Out-Null
          if ((Get-GitScalar @('rev-parse', "$freezeHead`:$runnerPath")) -cne $historicalRunnerBlob) { throw 'Historical frozen Runner Blob mismatch.' }
          $runnerBlobActual = Get-GitScalar @('rev-parse', "HEAD`:$runnerPath")
          if ($runnerBlobActual -cne $runnerBlob) { throw 'Activation Runner Blob mismatch.' }
          $runnerWorktreeBlob = Get-GitScalar @('hash-object', '--', $runnerPath)
          if ($runnerWorktreeBlob -cne $runnerBlob) { throw 'Runner working-file Git Blob mismatch.' }
          $runnerShaActual = Get-RawSha $runnerPath
          if ($runnerShaActual -cne $runnerSha) { throw 'Runner raw SHA-256 mismatch.' }

          $fixedFiles = @(
            @{ path = $p1Guard; revision = $p1Head; blob = $p1GuardBlob; sha = $p1GuardSha },
            @{ path = $p1Test; revision = $p1Head; blob = $p1TestBlob; sha = $p1TestSha },
            @{ path = $s0Probe; revision = $s0Head; blob = $s0Blob; sha = $s0Sha }
          )
          foreach ($file in $fixedFiles) {
            if ((Get-GitScalar @('rev-parse', "$($file.revision):$($file.path)")) -cne $file.blob) { throw "Frozen historical Blob changed: $($file.path)" }
            if ((Get-GitScalar @('hash-object', '--', $file.path)) -cne $file.blob) { throw "Working-file Blob changed: $($file.path)" }
            if ((Get-RawSha $file.path) -cne $file.sha) { throw "Raw SHA-256 changed: $($file.path)" }
          }
          if ((Get-GitScalar @('rev-parse', "HEAD`:$oldWorkflow")) -cne $oldWorkflowBlob) { throw 'Historical consumed NSIS workflow changed.' }
          if ((Get-RawSha $oldWorkflow) -cne $oldWorkflowSha) { throw 'Historical consumed workflow raw SHA-256 changed.' }
          foreach ($file in @(
            @{ path = $harnessPath; blob = $harnessBlob; sha = $harnessSha },
            @{ path = $childPath; blob = $childBlob; sha = $childSha },
            @{ path = $controllerPath; blob = $controllerBlob; sha = $controllerSha },
            @{ path = $validatorPath; blob = $validatorBlob; sha = $validatorSha },
            @{ path = $artifactPath; blob = $artifactBlob; sha = $artifactSha }
          )) {
            if ((Get-GitScalar @('rev-parse', "HEAD`:$($file.path)")) -cne $file.blob) { throw "Committed test-tool Blob mismatch: $($file.path)" }
            if ((Get-GitScalar @('hash-object', '--', $file.path)) -cne $file.blob) { throw "Working test-tool Blob mismatch: $($file.path)" }
            if ((Get-RawSha $file.path) -cne $file.sha) { throw "Test tool raw SHA-256 mismatch: $($file.path)" }
          }

          $autocrlf = Get-GitScalar @('config', '--get', 'core.autocrlf')
          $eol = Get-GitScalar @('config', '--get', 'core.eol')
          if ($autocrlf -cne 'false' -or $eol -cne 'lf') { throw "Unexpected checkout byte policy: autocrlf=$autocrlf eol=$eol" }
          $attributeOutput = Get-GitText @('check-attr', 'text', 'eol', 'filter', 'working-tree-encoding', '--', $runnerPath, $p1Guard, $p1Test, $s0Probe, $harnessPath, $childPath, $controllerPath, $validatorPath, $artifactPath)
          if ($attributeOutput -match '(?m): filter: (?!unspecified$)' -or
              $attributeOutput -match '(?m): working-tree-encoding: (?!unspecified$)' -or
              $attributeOutput -match '(?m): eol: crlf$') { throw "Git attributes can transform a frozen source file:`n$attributeOutput" }

          $gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Path
          $gitRoot = Split-Path -Parent (Split-Path -Parent $gitPath)
          $gitShellPaths = @((Join-Path $gitRoot 'bin/sh.exe'), (Join-Path $gitRoot 'usr/bin/sh.exe')) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | ForEach-Object { [System.IO.Path]::GetFullPath($_) }
          if (@($gitShellPaths).Count -eq 0) { throw 'Git for Windows sh.exe was not found for the controlled Git alias tests.' }
          $gitVersion = Get-GitText @('--version')
          $evidenceRoot = Join-Path $env:RUNNER_TEMP ("issue380-s1r4-p3-git-runtime-{0}\evidence" -f $env:GITHUB_RUN_ID)
          $preflight = [ordered]@{
            status = 'PASS'
            head = $head
            tree = Get-GitScalar @('rev-parse', 'HEAD^{tree}')
            parent = $parent
            activation_diff = $changed
            freeze_head = $freezeHead
            freeze_tree = $freezeTree
            historical_runner_blob = $historicalRunnerBlob
            historical_runner_sha256 = $historicalRunnerSha
            runner_identity_kind = 'R2_CANDIDATE_NOT_WINDOWS_QUALIFIED'
            runner_blob_expected = $runnerBlob
            runner_blob_actual = $runnerWorktreeBlob
            runner_raw_sha_expected = $runnerSha
            runner_sha256_actual = $runnerShaActual
            p1_source_blobs = @($fixedFiles | ForEach-Object { @{ path = $_.path; blob = $_.blob; sha256 = $_.sha } })
            old_workflow_blob = $oldWorkflowBlob
            autocrlf = $autocrlf
            eol = $eol
            attributes = $attributeOutput
            git_path = $gitPath
            git_shell_paths = @($gitShellPaths)
            git_version = $gitVersion
            powershell_version = $PSVersionTable.PSVersion.ToString()
            windows_version = [System.Environment]::OSVersion.VersionString
            recorded_utc = [DateTime]::UtcNow.ToString('o')
          }
          [System.IO.File]::WriteAllText((Join-Path $evidenceRoot 'preflight.json'), ((ConvertTo-Json -InputObject $preflight -Depth 12) + "`n"), [System.Text.UTF8Encoding]::new($false))

      - name: Run only the R2 candidate Git capture qualification harness
        id: runtime
        if: ${{ steps.preflight.outcome == 'success' }}
        shell: pwsh
        run: |
          $runRoot = Join-Path $env:RUNNER_TEMP ("issue380-s1r4-p3-git-runtime-{0}" -f $env:GITHUB_RUN_ID)
          $evidenceRoot = Join-Path $runRoot 'evidence'
          $taskRoot = Join-Path $runRoot 'task'
          $controller = Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4P3GitRuntimeController.ps1'
          & pwsh -NoLogo -NoProfile -NonInteractive -File $controller `
            -HarnessPath (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4P3GitRuntimeQualification.ps1') `
            -EvidenceRoot $evidenceRoot -TaskRoot $taskRoot -WorkerDeadlineMilliseconds 420000
          if ($LASTEXITCODE -ne 0) { throw "Runtime controller failed with exit code $LASTEXITCODE" }

      - name: Preserve failure task files; clean only a fully verified task root
        id: task_cleanup
        if: always() && steps.bootstrap.outcome == 'success'
        shell: pwsh
        run: |
          Set-StrictMode -Version Latest
          $ErrorActionPreference = 'Stop'
          $runRoot = Join-Path $env:RUNNER_TEMP ("issue380-s1r4-p3-git-runtime-{0}" -f $env:GITHUB_RUN_ID)
          $evidenceRoot = Join-Path $runRoot 'evidence'
          $taskRoot = Join-Path $runRoot 'task'
          $resultPath = Join-Path $evidenceRoot 'runtime-qualification.json'
          $cleanup = [ordered]@{ status = 'PRESERVED_FOR_FAILURE_EVIDENCE'; task_root = $taskRoot; removed = $false; reason = $null; recorded_utc = [DateTime]::UtcNow.ToString('o') }
          try {
            $owner = Get-Content -LiteralPath (Join-Path $taskRoot 'runtime-task-owner.json') -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
            if ([string]$owner.run_id -ne $env:GITHUB_RUN_ID -or [string]$owner.run_attempt -ne $env:GITHUB_RUN_ATTEMPT -or [string]$owner.triggering_sha -ne $env:GITHUB_SHA) { throw 'TaskRoot owner identity mismatch.' }
            $result = if (Test-Path -LiteralPath $resultPath -PathType Leaf) { Get-Content -LiteralPath $resultPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable } else { $null }
            $cleanupVerified = $true
            if ($null -ne $result) {
              foreach ($entry in @($result.cleanup)) {
                if (-not $entry.exit_confirmed -or $null -ne $entry.error) { $cleanupVerified = $false }
              }
            }
            if ($null -ne $result -and $result.status -eq 'RUNTIME_MATRIX_PASS' -and $cleanupVerified) {
              $items = @(Get-ChildItem -LiteralPath $taskRoot -Force -Recurse -ErrorAction Stop)
              $reparse = @($items | Where-Object { ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 })
              if ($reparse.Count -ne 0) { throw 'TaskRoot contains a reparse point.' }
              Remove-Item -LiteralPath $taskRoot -Recurse -Force -ErrorAction Stop
              $cleanup.status = 'REMOVED_AFTER_FULL_PASS'
              $cleanup.removed = $true
            } else {
              $cleanup.reason = 'A failed, partial, or missing runtime result is retained for forensic review.'
            }
          } catch { $cleanup.status = 'CLEANUP_CHECK_FAILED'; $cleanup.reason = $_.Exception.ToString() }
          [System.IO.File]::WriteAllText((Join-Path $evidenceRoot 'cleanup.json'), ((ConvertTo-Json -InputObject $cleanup -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))
          if ($cleanup.status -eq 'CLEANUP_CHECK_FAILED') { throw $cleanup.reason }

      - name: Independently qualify the run-owned evidence directory
        id: upload_qualification
        if: always() && steps.bootstrap.outcome == 'success'
        shell: pwsh
        run: |
          Set-StrictMode -Version Latest
          $ErrorActionPreference = 'Stop'
          $runRoot = Join-Path $env:RUNNER_TEMP ("issue380-s1r4-p3-git-runtime-{0}" -f $env:GITHUB_RUN_ID)
          $evidenceRoot = Join-Path $runRoot 'evidence'
          $taskRoot = Join-Path $runRoot 'task'
          $tempFull = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd([char[]]@('\', '/'))
          $prefix = $tempFull + [System.IO.Path]::DirectorySeparatorChar
          $runFull = [System.IO.Path]::GetFullPath($runRoot)
          if (-not $runFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) { throw 'Evidence run root escaped RUNNER_TEMP.' }
          foreach ($root in @($runRoot, $evidenceRoot)) {
            if (-not (Test-Path -LiteralPath $root -PathType Container)) { throw "Owned evidence root is missing: $root" }
            $item = Get-Item -LiteralPath $root -Force -ErrorAction Stop
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Evidence root is a reparse point: $root" }
          }
          $owner = Get-Content -LiteralPath (Join-Path $evidenceRoot 'runtime-owner.json') -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
          if ([string]$owner.repository -ne $env:GITHUB_REPOSITORY -or [string]$owner.ref -ne $env:GITHUB_REF -or [string]$owner.run_id -ne $env:GITHUB_RUN_ID -or [string]$owner.run_attempt -ne $env:GITHUB_RUN_ATTEMPT -or [string]$owner.triggering_sha -ne $env:GITHUB_SHA) { throw 'Evidence owner does not match the current run.' }
          $entries = @(Get-ChildItem -LiteralPath $runRoot -Force -Recurse -ErrorAction Stop)
          $reparse = @($entries | Where-Object { ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 })
          if ($reparse.Count -ne 0) { throw 'Evidence payload contains a reparse point.' }
          $files = @($entries | Where-Object { -not $_.PSIsContainer })
          if ($files.Count -gt 250) { throw "Evidence file limit exceeded: $($files.Count)" }
          $bytes = [long](($files | Measure-Object -Property Length -Sum).Sum)
          if ($bytes -gt 20MB) { throw "Evidence size limit exceeded: $bytes bytes" }
          $taskOwnerPath = Join-Path $taskRoot 'runtime-task-owner.json'
          if (Test-Path -LiteralPath $taskRoot -PathType Container) {
            $taskOwner = Get-Content -LiteralPath $taskOwnerPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
            if ([string]$taskOwner.run_id -ne $env:GITHUB_RUN_ID -or [string]$taskOwner.run_attempt -ne $env:GITHUB_RUN_ATTEMPT -or [string]$taskOwner.triggering_sha -ne $env:GITHUB_SHA) { throw 'TaskRoot owner mismatch.' }
          }
          $manifest = [ordered]@{ status = 'SAFE_TO_UPLOAD'; run_root = $runFull; file_count = $files.Count; bytes = $bytes; reparse_points = 0; owner_run_id = $owner.run_id; owner_attempt = $owner.run_attempt; owner_sha = $owner.triggering_sha; recorded_utc = [DateTime]::UtcNow.ToString('o') }
          [System.IO.File]::WriteAllText((Join-Path $evidenceRoot 'upload-qualification.json'), ((ConvertTo-Json -InputObject $manifest -Depth 8) + "`n"), [System.Text.UTF8Encoding]::new($false))
          & (Join-Path $env:GITHUB_WORKSPACE 'scripts/issue380S1R4P3GitRuntimeArtifact.ps1') `
            -RunRoot $runRoot `
            -ArchiveRoot (Join-Path $env:RUNNER_TEMP ("issue380-s1r4-p3-git-runtime-archive-{0}" -f $env:GITHUB_RUN_ID))

      - name: Upload run-owned runtime evidence
        if: always() && steps.upload_qualification.outcome == 'success'
        uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4
        with:
          name: issue-380-s1r4-p3-git-runtime-${{ github.run_id }}-${{ github.run_attempt }}
          path: ${{ runner.temp }}/issue380-s1r4-p3-git-runtime-archive-${{ github.run_id }}/**
          if-no-files-found: error
          include-hidden-files: false
          compression-level: 0

      - name: Fail the job unless full runtime evidence qualifies
        if: always()
        shell: pwsh
        run: |
          if ('${{ steps.preflight.outcome }}' -ne 'success') { throw 'Preflight failed; no runtime qualification ran.' }
          if ('${{ steps.runtime.outcome }}' -ne 'success') { throw 'Runtime qualification did not complete successfully.' }
          $runRoot = Join-Path $env:RUNNER_TEMP ("issue380-s1r4-p3-git-runtime-{0}" -f $env:GITHUB_RUN_ID)
          $summaryPath = Join-Path $runRoot 'evidence/runtime-qualification.json'
          $summary = Get-Content -LiteralPath $summaryPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
          if ($summary.status -ne 'RUNTIME_MATRIX_PASS') { throw "Runtime qualification status is not a full pass: $($summary.status)" }
```

## Activation and interpretation limits

- The only trigger is a push to `qualification/s1r4-p3-windows-runtime-r2` that changes this exact Workflow path. There is no `workflow_dispatch`, matrix, retry, or broad CI invocation.
- The activation commit is a direct child of the reviewed plan HEAD and adds only this Workflow. Its parent SHA must replace the sentinel above before activation; otherwise preflight fails before starting the harness.
- The plan pins the new R2 candidate Runner plus harness, child, controller, P1, S0, and historical Workflow identities. Windows Git checkout policy is set through Git environment configuration before `actions/checkout`; attributes, Git Blob IDs, and raw hashes are checked after checkout.
- Harness status `PARTIAL_BLOCKED_FAULT_INJECTION_UNPROVEN` is a failure, not a pass. No evidence from this workflow could close the deterministic kill/cleanup-error injection gap without a separate Owner decision.
- `WaitForExit`, `Task.WaitAll`, synchronous `Kill`, `Close`, and `Dispose` observations are recorded individually. This draft does not claim a strict total execution-time bound from configured waits.

Remediation archive rule: upload only the separately staged archive tree. SHA256SUMS covers every payload file; SHA256SUMS itself is the sole self-hash exception. Source and staged archive limits remain 250 files / 20 MB, with ownership and Reparse Point checks. Hidden paths and temporary Git databases are excluded. The helper verifies copies and exact manifest membership before upload. This remains an inactive draft; the Owner parent sentinel is unchanged.

R2 candidate boundary: historical freeze Head/Tree/Blob remain provenance only. The separately pinned Runner Blob/SHA above identify the new candidate, not the original freeze. No Windows R2 runtime evidence exists. This draft remains unactivated; Parent sentinel must be bound only after Owner authorization.
