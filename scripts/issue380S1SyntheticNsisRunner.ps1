[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:S0Head = '1f4951f3c93b2eb549d159fed2dd288992c625c8'
$script:ProbeRelativePath = 'scripts/issue380S0NsisObservabilityProbe.nsi'
$script:RegistrySubKey = 'Software\ZenCanvasS0Harness'
$script:RegistryValueName = 'EstimatedSize'
$script:ExpectedValue = 35150
$script:ActualValue = 35149
$script:ExpectedInstallerExitCode = 37
$script:ExpectedUninstallerExitCode = 42
$script:Failure = $null
$script:CanDeleteSyntheticKey = $false
$script:SyntheticKeyChecked = $false
$script:Assertions = [System.Collections.Generic.List[object]]::new()
$script:Executables = [System.Collections.Generic.List[object]]::new()
$script:Processes = [System.Collections.Generic.List[object]]::new()
$baseKey = $null

$outputFullPath = [System.IO.Path]::GetFullPath($OutputRoot)
if (Test-Path -LiteralPath $outputFullPath) {
    $existingItems = @(Get-ChildItem -LiteralPath $outputFullPath -Force -ErrorAction Stop)
    if ($existingItems.Count -ne 0) {
        throw "OutputRoot exists and is not empty: $outputFullPath"
    }
} else {
    New-Item -ItemType Directory -Path $outputFullPath -ErrorAction Stop | Out-Null
}

$runRoot = Join-Path $env:RUNNER_TEMP 'issue380-s1-run'
if (Test-Path -LiteralPath $runRoot) {
    throw "Task-owned run directory already exists: $runRoot"
}
New-Item -ItemType Directory -Path $runRoot -ErrorAction Stop | Out-Null

function Write-Utf8Text([string]$Path, [string]$Text) {
    $parent = Split-Path -Parent $Path
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    [System.IO.File]::WriteAllText($Path, $Text, [System.Text.UTF8Encoding]::new($false))
}

function Write-Utf8Json([string]$Path, [object]$Value) {
    $json = ConvertTo-Json -InputObject $Value -Depth 20
    Write-Utf8Text -Path $Path -Text ($json + "`n")
}

function Add-Assertion([string]$Name, [bool]$Passed, [string]$Details) {
    $record = [ordered]@{
        name = $Name
        passed = $Passed
        details = $Details
    }
    $script:Assertions.Add($record)
    if (-not $Passed) {
        throw "Assertion failed: $Name — $Details"
    }
}

function Invoke-CapturedProcess(
    [string]$FilePath,
    [string[]]$ArgumentList,
    [string]$WorkingDirectory,
    [string]$StdoutPath,
    [string]$StderrPath,
    [int]$TimeoutSeconds = 60
) {
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $ArgumentList) {
        [void]$startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw "Could not start process: $FilePath"
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $completed = $process.WaitForExit($TimeoutSeconds * 1000)
    if (-not $completed) {
        try { $process.Kill($true) } catch { }
        $process.WaitForExit()
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    Write-Utf8Text -Path $StdoutPath -Text $stdout
    Write-Utf8Text -Path $StderrPath -Text $stderr

    $result = [ordered]@{
        file = $FilePath
        arguments = @($ArgumentList)
        working_directory = $WorkingDirectory
        timed_out = (-not $completed)
        exit_code = [int]$process.ExitCode
        stdout_path = $StdoutPath
        stderr_path = $StderrPath
        stdout_utf8 = $stdout
        stderr_utf8 = $stderr
    }
    $script:Processes.Add($result)
    $process.Dispose()
    return $result
}

function Get-SyntheticRegistrySnapshot([Microsoft.Win32.RegistryKey]$BaseKey) {
    $key = $BaseKey.OpenSubKey($script:RegistrySubKey, $false)
    if ($null -eq $key) {
        return [ordered]@{ exists = $false; value_name = $script:RegistryValueName; kind = $null; value = $null }
    }
    try {
        $hasValue = $key.GetValueNames() -contains $script:RegistryValueName
        if (-not $hasValue) {
            return [ordered]@{ exists = $true; value_name = $script:RegistryValueName; kind = 'MISSING'; value = $null }
        }
        return [ordered]@{
            exists = $true
            value_name = $script:RegistryValueName
            kind = [string]$key.GetValueKind($script:RegistryValueName)
            value = $key.GetValue($script:RegistryValueName, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        }
    } finally {
        $key.Dispose()
    }
}

function Add-ExecutableHash([string]$Path, [string]$Role) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Expected generated executable is missing: $Path"
    }
    $item = Get-Item -LiteralPath $Path
    if ($item.Length -le 0) {
        throw "Generated executable is empty: $Path"
    }
    $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Executables.Add([ordered]@{
        role = $Role
        path = $Path
        length_bytes = [long]$item.Length
        sha256 = $hash
    })
}

function Assert-ProcessExit([string]$Name, [object]$ProcessResult, [int]$ExpectedCode) {
    $passed = (-not $ProcessResult.timed_out) -and ($ProcessResult.exit_code -eq $ExpectedCode)
    Add-Assertion -Name $Name -Passed $passed -Details "expected=$ExpectedCode actual=$($ProcessResult.exit_code) timed_out=$($ProcessResult.timed_out) stdout_path=$($ProcessResult.stdout_path) stderr_path=$($ProcessResult.stderr_path)"
}

function Compile-Probe([string]$Makensis, [string]$ProbePath, [string]$Mode, [string]$RunMode, [string]$OutputName, [string]$WorkingDirectory, [string]$LogStem) {
    $arguments = @(
        '-NOCD',
        '-V2',
        "-DTRACE_MODE=$Mode",
        "-DRUN_MODE=$RunMode",
        "-DOUTPUT_NAME=$OutputName",
        $ProbePath
    )
    $result = Invoke-CapturedProcess -FilePath $Makensis -ArgumentList $arguments -WorkingDirectory $WorkingDirectory `
        -StdoutPath (Join-Path $outputFullPath "$LogStem.stdout.txt") `
        -StderrPath (Join-Path $outputFullPath "$LogStem.stderr.txt") -TimeoutSeconds 90
    Add-Assertion -Name "compile_${Mode}_${RunMode}_exit_zero" -Passed ((-not $result.timed_out) -and ($result.exit_code -eq 0)) `
        -Details "compiler_exit=$($result.exit_code) timed_out=$($result.timed_out) stdout=$($LogStem).stdout.txt stderr=$($LogStem).stderr.txt"
    $generated = Join-Path $WorkingDirectory $OutputName
    Add-ExecutableHash -Path $generated -Role "compiler_output_${Mode}_${RunMode}"
    return $generated
}

function Get-MakensisPath {
    $candidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'),
        (Join-Path $env:ProgramFiles 'NSIS\makensis.exe')
    )
    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) { return $candidate }
    }
    $command = Get-Command makensis.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    throw 'NSIS 3.11 makensis.exe was not found in the expected install locations or PATH.'
}

$resultDocument = [ordered]@{
    phase = '#380-S1 SYNTHETIC NSIS RUNTIME TRACE PERSISTENCE'
    status = 'IN_PROGRESS'
    repository = $env:GITHUB_REPOSITORY
    ref = $env:GITHUB_REF
    workflow_sha = $env:GITHUB_SHA
    run_id = $env:GITHUB_RUN_ID
    run_attempt = $env:GITHUB_RUN_ATTEMPT
    job = $env:GITHUB_JOB
    frozen_s0_head = $script:S0Head
    tested_head = $null
    tested_tree = $null
    probe_blob = $null
    environment = [ordered]@{
        runner_os = $env:RUNNER_OS
        runner_arch = $env:RUNNER_ARCH
        image_os = $env:ImageOS
        image_version = $env:ImageVersion
        powershell = $PSVersionTable.PSVersion.ToString()
        windows_version = [System.Environment]::OSVersion.VersionString
    }
    compiler = $null
    compile_options = 'makensis.exe -NOCD -V2 -DTRACE_MODE=<detail|file> -DRUN_MODE=<stage|abort> -DOUTPUT_NAME=<name> <committed probe source>'
    executables = @()
    processes = @()
    registry_snapshots = @()
    log_comparison = @()
    assertions = @()
    cleanup = [ordered]@{ registry_key = 'NOT_ATTEMPTED'; task_temp_directory = 'CLEANUP_BY_WORKFLOW' }
    failure = $null
}

try {
    if ($env:GITHUB_REPOSITORY -ne 'ArdenZC/Zen-Canvas') {
        throw "Unexpected GitHub repository identity: $env:GITHUB_REPOSITORY"
    }
    $expectedRef = 'refs/heads/research/issue-380-s1-synthetic-nsis-runtime-trace'
    if ($env:GITHUB_REF -ne $expectedRef) {
        throw "Unexpected workflow ref: $env:GITHUB_REF"
    }
    if ($env:GITHUB_RUN_ATTEMPT -ne '1') {
        throw "This is a one-shot probe; run attempt must be 1, got $env:GITHUB_RUN_ATTEMPT"
    }

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
    Push-Location $repoRoot
    try {
        $head = (& git rev-parse HEAD).Trim()
        $tree = (& git rev-parse 'HEAD^{tree}').Trim()
        if ($LASTEXITCODE -ne 0) { throw 'git could not resolve the tested source identity.' }
        $ancestor = & git merge-base --is-ancestor $script:S0Head HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Frozen S0 is not an ancestor of this S1 candidate.' }
        $probeBlob = (& git rev-parse "$($script:S0Head):$($script:ProbeRelativePath)").Trim()
        $workingProbeBlob = (& git hash-object $script:ProbeRelativePath).Trim()
        if ($LASTEXITCODE -ne 0 -or $probeBlob -ne $workingProbeBlob) {
            throw 'The committed S0 NSIS Probe differs from the frozen S0 blob.'
        }
        if ($head -ne $env:GITHUB_SHA) { throw "Checked-out HEAD $head differs from workflow SHA $env:GITHUB_SHA" }
        $resultDocument.tested_head = $head
        $resultDocument.tested_tree = $tree
        $resultDocument.probe_blob = $probeBlob
    } finally {
        Pop-Location
    }

    $probePath = Join-Path $repoRoot $script:ProbeRelativePath
    $probeText = [System.IO.File]::ReadAllText($probePath)
    Add-Assertion 'probe_is_unicode_user_level' `
        (($probeText -match '(?m)^Unicode True\s*$') -and ($probeText -match '(?m)^RequestExecutionLevel user\s*$')) `
        'Committed probe must use Unicode and user-level execution.'
    Add-Assertion 'probe_uses_only_synthetic_hkcu_identity' `
        (($probeText.Contains($script:RegistrySubKey)) -and ($probeText -notmatch '(?i)HKLM|Program Files|CurrentControlSet|ZenCanvasIndexService')) `
        'Probe must use only the synthetic HKCU key; no machine registry, Program Files, or Service identity.'
    Add-Assertion 'probe_has_direct_file_trace_and_abort_codes' `
        (($probeText.Contains('FileOpen')) -and ($probeText.Contains('FileWrite')) -and ($probeText.Contains('FileClose')) -and ($probeText.Contains('Abort '))) `
        'Committed probe must write/close a direct file trace before the synthetic Abort.'

    $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::CurrentUser,
        [Microsoft.Win32.RegistryView]::Registry64
    )
    try {
        $initialSnapshot = Get-SyntheticRegistrySnapshot -BaseKey $baseKey
        $script:SyntheticKeyChecked = $true
        $resultDocument.registry_snapshots += @([ordered]@{ stage = 'initial'; snapshot = $initialSnapshot })
        $script:CanDeleteSyntheticKey = (-not $initialSnapshot.exists)
        Add-Assertion 'synthetic_hkcu_key_initially_absent' -Passed $script:CanDeleteSyntheticKey `
            -Details "key=HKCU\$($script:RegistrySubKey) exists=$($initialSnapshot.exists); existing state is never overwritten or deleted."
        Write-Utf8Text -Path (Join-Path $outputFullPath 'synthetic-key-cleanup-owned.txt') `
            -Text "Created only after proving HKCU\$($script:RegistrySubKey) was absent.`n"

        $makensis = Get-MakensisPath
        $compilerHash = (Get-FileHash -LiteralPath $makensis -Algorithm SHA256).Hash.ToLowerInvariant()
        $versionResult = Invoke-CapturedProcess -FilePath $makensis -ArgumentList @('-VERSION') -WorkingDirectory $runRoot `
            -StdoutPath (Join-Path $outputFullPath 'makensis-version.stdout.txt') `
            -StderrPath (Join-Path $outputFullPath 'makensis-version.stderr.txt')
        $hdrResult = Invoke-CapturedProcess -FilePath $makensis -ArgumentList @('-HDRINFO') -WorkingDirectory $runRoot `
            -StdoutPath (Join-Path $outputFullPath 'makensis-hdrinfo.stdout.txt') `
            -StderrPath (Join-Path $outputFullPath 'makensis-hdrinfo.stderr.txt')
        $versionText = ($versionResult.stdout_utf8 + $versionResult.stderr_utf8).Trim()
        $headerText = $hdrResult.stdout_utf8 + $hdrResult.stderr_utf8
        $resultDocument.compiler = [ordered]@{
            path = $makensis
            version = $versionText
            sha256 = $compilerHash
            hdrinfo_stdout_path = 'makensis-hdrinfo.stdout.txt'
            hdrinfo_stderr_path = 'makensis-hdrinfo.stderr.txt'
            x86 = ($headerText -match 'NSIS_CPU=x86')
            unicode = ($headerText -match 'NSIS_UNICODE')
        }
        Add-Assertion 'makensis_version_is_3_11' -Passed ($versionText -match '^v?3\.11(?:\s|$)') `
            -Details "actual version output: $versionText"
        Add-Assertion 'makensis_is_x86_unicode' -Passed (($headerText -match 'NSIS_CPU=x86') -and ($headerText -match 'NSIS_UNICODE')) `
            -Details 'NSIS -HDRINFO must identify NSIS_CPU=x86 and NSIS_UNICODE.'
        Add-Assertion 'makensis_identity_commands_succeeded' -Passed ((-not $versionResult.timed_out) -and ($versionResult.exit_code -eq 0) -and (-not $hdrResult.timed_out) -and ($hdrResult.exit_code -eq 0)) `
            -Details "version_exit=$($versionResult.exit_code), hdrinfo_exit=$($hdrResult.exit_code), compiler_sha256=$compilerHash"

        foreach ($mode in @('detail', 'file')) {
            $modeRoot = Join-Path $runRoot $mode
            $modeEvidence = Join-Path $outputFullPath $mode
            New-Item -ItemType Directory -Path $modeRoot -ErrorAction Stop | Out-Null
            New-Item -ItemType Directory -Path $modeEvidence -ErrorAction Stop | Out-Null

            $stageExe = Compile-Probe -Makensis $makensis -ProbePath $probePath -Mode $mode -RunMode 'stage' `
                -OutputName 'stage.exe' -WorkingDirectory $modeRoot -LogStem "$mode-compile-stage"
            $abortExe = Compile-Probe -Makensis $makensis -ProbePath $probePath -Mode $mode -RunMode 'abort' `
                -OutputName 'installer-abort.exe' -WorkingDirectory $modeRoot -LogStem "$mode-compile-installer-abort"

            $stageProcess = Invoke-CapturedProcess -FilePath $stageExe -ArgumentList @('/S') -WorkingDirectory $modeRoot `
                -StdoutPath (Join-Path $modeEvidence 'stage.stdout.txt') -StderrPath (Join-Path $modeEvidence 'stage.stderr.txt')
            Assert-ProcessExit -Name "${mode}_stage_exit_zero" -ProcessResult $stageProcess -ExpectedCode 0
            $generatedUninstaller = Join-Path $modeRoot 'generated-uninstaller.exe'
            Add-ExecutableHash -Path $generatedUninstaller -Role "stage_generated_uninstaller_$mode"

            $afterStage = Get-SyntheticRegistrySnapshot -BaseKey $baseKey
            $resultDocument.registry_snapshots += @([ordered]@{ stage = "${mode}_after_stage"; snapshot = $afterStage })
            Add-Assertion -Name "${mode}_stage_wrote_expected_synthetic_dword" `
                -Passed (($afterStage.exists) -and ($afterStage.kind -eq 'DWord') -and ([int]$afterStage.value -eq $script:ActualValue)) `
                -Details "expected_kind=DWord expected_value=$($script:ActualValue) actual_kind=$($afterStage.kind) actual_value=$($afterStage.value)"

            $installerLog = Join-Path $modeEvidence 'installer-runtime.log'
            $installerProcess = Invoke-CapturedProcess -FilePath $abortExe -ArgumentList @('/S', "/LOG=$installerLog") `
                -WorkingDirectory $modeRoot -StdoutPath (Join-Path $modeEvidence 'installer-abort.stdout.txt') `
                -StderrPath (Join-Path $modeEvidence 'installer-abort.stderr.txt')
            Assert-ProcessExit -Name "${mode}_installer_abort_expected_exit" -ProcessResult $installerProcess `
                -ExpectedCode $script:ExpectedInstallerExitCode

            $uninstallerLog = Join-Path $modeEvidence 'uninstaller-runtime.log'
            $uninstallerProcess = Invoke-CapturedProcess -FilePath $generatedUninstaller `
                -ArgumentList @('/S', "/LOG=$uninstallerLog") -WorkingDirectory $modeRoot `
                -StdoutPath (Join-Path $modeEvidence 'uninstaller-abort.stdout.txt') `
                -StderrPath (Join-Path $modeEvidence 'uninstaller-abort.stderr.txt')
            Assert-ProcessExit -Name "${mode}_uninstaller_abort_expected_exit" -ProcessResult $uninstallerProcess `
                -ExpectedCode $script:ExpectedUninstallerExitCode

            $afterAborts = Get-SyntheticRegistrySnapshot -BaseKey $baseKey
            $resultDocument.registry_snapshots += @([ordered]@{ stage = "${mode}_after_aborts"; snapshot = $afterAborts })
            Add-Assertion -Name "${mode}_abort_paths_preserved_synthetic_dword" `
                -Passed (($afterAborts.exists) -and ($afterAborts.kind -eq 'DWord') -and ([int]$afterAborts.value -eq $script:ActualValue)) `
                -Details "expected_kind=DWord expected_value=$($script:ActualValue) actual_kind=$($afterAborts.kind) actual_value=$($afterAborts.value)"

            foreach ($exe in @($stageExe, $abortExe, $generatedUninstaller)) {
                Copy-Item -LiteralPath $exe -Destination $modeEvidence -Force
            }
            $runTrace = Join-Path $modeRoot 'direct-evidence.log'
            if (Test-Path -LiteralPath $runTrace -PathType Leaf) {
                Copy-Item -LiteralPath $runTrace -Destination $modeEvidence -Force
            }
            $detailTrace = Join-Path $modeEvidence 'direct-evidence.log'
            $detailLines = @()
            if (Test-Path -LiteralPath $detailTrace -PathType Leaf) {
                $detailLines = @(Get-Content -LiteralPath $detailTrace)
            }
            $resultDocument.log_comparison += @([ordered]@{
                mode = $mode
                installer_log_exists = (Test-Path -LiteralPath $installerLog -PathType Leaf)
                installer_log_bytes = if (Test-Path -LiteralPath $installerLog -PathType Leaf) { (Get-Item -LiteralPath $installerLog).Length } else { 0 }
                installer_log_contains_detailprint_failure = if (Test-Path -LiteralPath $installerLog -PathType Leaf) { (Get-Content -Raw -LiteralPath $installerLog).Contains('first_failure=EstimatedSize') } else { $false }
                uninstaller_log_exists = (Test-Path -LiteralPath $uninstallerLog -PathType Leaf)
                uninstaller_log_bytes = if (Test-Path -LiteralPath $uninstallerLog -PathType Leaf) { (Get-Item -LiteralPath $uninstallerLog).Length } else { 0 }
                uninstaller_log_contains_detailprint_failure = if (Test-Path -LiteralPath $uninstallerLog -PathType Leaf) { (Get-Content -Raw -LiteralPath $uninstallerLog).Contains('uninstaller_first_failure=EstimatedSize') } else { $false }
                direct_trace_exists = (Test-Path -LiteralPath $detailTrace -PathType Leaf)
                direct_trace_lines = $detailLines
            })
        }

        $fileTracePath = Join-Path (Join-Path $outputFullPath 'file') 'direct-evidence.log'
        Add-Assertion -Name 'file_trace_exists_after_both_abort_processes_exited' `
            -Passed ((Test-Path -LiteralPath $fileTracePath -PathType Leaf) -and ((Get-Item -LiteralPath $fileTracePath).Length -gt 0)) `
            -Details "trace_path=$fileTracePath; must persist after installer and uninstaller exit."
        $traceLines = @(Get-Content -LiteralPath $fileTracePath)
        $requiredFields = @(
            'predicate=HKCU\Software\ZenCanvasS0Harness\EstimatedSize',
            'type=REG_DWORD',
            'expected=35150',
            'actual=35149',
            'result=FAIL'
        )
        foreach ($context in @('installer', 'uninstaller')) {
            $contextLines = @($traceLines | Where-Object { $_ -match "(^|\s)context=$context(\s|$)" })
            $complete = ($contextLines.Count -eq 1)
            if ($complete) {
                foreach ($field in $requiredFields) {
                    if (-not $contextLines[0].Contains($field)) { $complete = $false }
                }
            }
            Add-Assertion -Name "file_trace_${context}_record_complete" -Passed $complete `
                -Details "exactly one post-exit line must contain context=$context plus predicate, type, expected, actual, and result=FAIL; matching_lines=$($contextLines -join ' | ')"
        }
        Add-Assertion -Name 'file_trace_has_exactly_two_abort_records' -Passed ($traceLines.Count -eq 2) `
            -Details "expected=2 actual=$($traceLines.Count)"
    } finally {
        if ($null -ne $baseKey) { $baseKey.Dispose() }
    }
} catch {
    $script:Failure = $_.Exception.ToString()
} finally {
    try {
        if ($script:CanDeleteSyntheticKey) {
            $cleanupBase = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
                [Microsoft.Win32.RegistryHive]::CurrentUser,
                [Microsoft.Win32.RegistryView]::Registry64
            )
            try {
                $cleanupBase.DeleteSubKeyTree($script:RegistrySubKey, $false)
                $remaining = Get-SyntheticRegistrySnapshot -BaseKey $cleanupBase
                if ($remaining.exists) { throw "Synthetic HKCU cleanup left key in place: $($script:RegistrySubKey)" }
                $resultDocument.cleanup.registry_key = 'REMOVED_AND_VERIFIED_ABSENT'
            } finally {
                $cleanupBase.Dispose()
            }
        } elseif ($script:SyntheticKeyChecked) {
            $resultDocument.cleanup.registry_key = 'NOT_REMOVED_PREEXISTING_KEY'
        } else {
            $resultDocument.cleanup.registry_key = 'NOT_CHECKED'
        }
    } catch {
        $cleanupError = $_.Exception.ToString()
        $resultDocument.cleanup.registry_key = "FAILED: $cleanupError"
        if ($null -eq $script:Failure) { $script:Failure = "Registry cleanup failed: $cleanupError" }
    }

    $resultDocument.executables = @($script:Executables.ToArray())
    $resultDocument.processes = @($script:Processes.ToArray())
    $resultDocument.assertions = @($script:Assertions.ToArray())
    if ($null -eq $script:Failure) {
        $resultDocument.status = 'PASS'
    } else {
        $resultDocument.status = 'FAIL'
        $resultDocument.failure = $script:Failure
        try { Write-Utf8Text -Path (Join-Path $outputFullPath 'failure.txt') -Text ($script:Failure + "`n") } catch { }
    }
    try {
        Write-Utf8Json -Path (Join-Path $outputFullPath 'results.json') -Value $resultDocument
        $sumLines = [System.Collections.Generic.List[string]]::new()
        foreach ($file in (Get-ChildItem -LiteralPath $outputFullPath -Recurse -File | Sort-Object FullName)) {
            if ($file.Name -eq 'SHA256SUMS') { continue }
            $relative = $file.FullName.Substring($outputFullPath.Length).TrimStart('\','/').Replace('\','/')
            $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            $sumLines.Add("$hash  $relative")
        }
        Write-Utf8Text -Path (Join-Path $outputFullPath 'SHA256SUMS') -Text (($sumLines -join "`n") + "`n")
    } catch {
        if ($null -eq $script:Failure) { $script:Failure = "Evidence finalization failed: $($_.Exception.ToString())" }
    }
}

if ($null -ne $script:Failure) {
    Write-Error -Message $script:Failure -ErrorAction Continue
    exit 1
}
exit 0
