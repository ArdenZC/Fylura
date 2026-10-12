[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'issue380S1R4PathGuard.ps1')

$script:S1R4P0Head = 'e5e9ab5bc84136c15056d72db8904f26b24e2a64'
$script:P2Head = '86c21c14dd65bd2bbdb3056513489e0d4139bb47'
$script:P2Tree = '883a851cf8e75b4f39a4a46baea15ab109feb312'
$script:P1Head = '7a6fa69e5521d9759702c1923a9ade60a75819c3'
$script:P1Tree = 'c5465ef93eae268e2c683876d959db90fac5da08'
$script:P1PathGuardPath = 'scripts/issue380S1R4PathGuard.ps1'
$script:P1PathTestPath = 'scripts/testIssue380S1R4PathGuard.ps1'
$script:S1R4P0Tree = '96cdd08012021c5409c8dcdedcb623644a55e541'
$script:S1R3Head = '08bad6b19831d40dda9d296de880aaae6c83c312'
$script:S1R3Tree = '5360d8c81e5949333c8d6ef16acd0c658d315598'
$script:S1R2Head = '6fe01abae7640a93f4a694616d758fd54c714960'
$script:S1R2Tree = 'af88309c65999d73564cf3876bb5bd03a15bf597'
$script:S1RHead = '45a2dfe0e51cee5708dfaa5335affbad947c30c2'
$script:S1RTree = '9bcc5a2a03a8335d2cb2e81c3d497efb275b8724'
$script:S1Head = 'bd56788aa317d808dadf930260e191e9d1b45adb'
$script:S0Head = '1f4951f3c93b2eb549d159fed2dd288992c625c8'
$script:ProbePath = 'scripts/issue380S0NsisObservabilityProbe.nsi'
$script:ExpectedBranch = 'research/issue-380-s1r4-p3-runner-null-safety-activation'
$script:RunnerPath = 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'
$script:ZipUrl = 'https://sourceforge.net/projects/nsis/files/NSIS%203/3.11/nsis-3.11.zip/download'
$script:ExpectedZipSha256 = 'c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1'
$script:LauncherRelativePath = 'nsis-3.11/makensis.exe'
$script:LauncherLength = 2560L
$script:LauncherSha256 = 'f497e92deb9f179f7b8f7553fcb3bd04f511bb2949e5e4a2aee80a10f7b20431'
$script:CompilerRelativePath = 'nsis-3.11/Bin/makensis.exe'
$script:CompilerLength = 468992L
$script:CompilerSha256 = '42850802704ecb11163f7e0329d35ee54bd288953200d4966e226d572848cfc5'
$script:RegistrySubKey = 'Software\ZenCanvasS0Harness'
$script:RegistryValueName = 'EstimatedSize'
$script:RegistryOwnerValueName = 'P3HarnessOwner'
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
$script:GitInvocations = [System.Collections.Generic.List[object]]::new()
$script:Acquisition = [ordered]@{
    source_url = $script:ZipUrl
    expected_sha256 = $script:ExpectedZipSha256
    final_url = $null
    http_status = $null
    content_length_header = $null
    downloaded_bytes = 0
    downloaded_sha256 = $null
    content_type = $null
    error_body_path = $null
    error_body_truncated = $false
    sha256_verified_before_extract = $false
    archive_entry_count = $null
    archive_uncompressed_bytes = $null
    validated_archive_members = @()
    extraction_root = $null
    extracted_members = @()
    extraction_status = 'NOT_STARTED'
    failure = $null
}
$script:Compiler = $null
$script:LogComparison = [System.Collections.Generic.List[object]]::new()
$script:RegistrySnapshots = [System.Collections.Generic.List[object]]::new()

$runnerTempFullPath = ConvertTo-Issue380S1R4LocalPath -Path $env:RUNNER_TEMP -Label 'RUNNER_TEMP'
$runnerTempFullPath = Resolve-Issue380S1R4Path -TaskRoot $runnerTempFullPath `
    -CandidatePath $runnerTempFullPath -Kind WorkingDirectory
$outputFullPath = ConvertTo-Issue380S1R4LocalPath -Path $OutputRoot -Label 'Evidence output'
$taskRoot = ConvertTo-Issue380S1R4LocalPath -Path (Join-Path $runnerTempFullPath 'issue380-s1r4-p3-task') -Label 'TaskRoot'
$script:TaskRoot = $taskRoot
$expectedEvidenceRoot = ConvertTo-Issue380S1R4LocalPath `
    -Path (Join-Path $runnerTempFullPath 'issue380-s1r4-p3-evidence') -Label 'Expected evidence root'
$taskMarker = Join-Path $taskRoot 's1r4-p3-task-owned.txt'
$syntheticKeyOwnerMarker = Join-Path $outputFullPath 'synthetic-key-owned.txt'
$zipPath = Join-Path $taskRoot 'nsis-3.11.zip'
$extractionRoot = Join-Path $taskRoot 'nsis-3.11-extracted'
$expectedArchiveFiles = [ordered]@{
    'nsis-3.11/makensis.exe' = [ordered]@{ length = $script:LauncherLength; sha256 = $script:LauncherSha256; role = 'forwarding_launcher' }
    'nsis-3.11/Bin/makensis.exe' = [ordered]@{ length = $script:CompilerLength; sha256 = $script:CompilerSha256; role = 'underlying_compiler' }
}

if (-not $outputFullPath.Equals($expectedEvidenceRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Evidence output must be inside the designated task temp directory: expected=$expectedEvidenceRoot actual=$outputFullPath"
}

if (Test-Path -LiteralPath $outputFullPath) {
    $outputFullPath = Resolve-Issue380S1R4Path -TaskRoot $runnerTempFullPath `
        -CandidatePath $outputFullPath -Kind ExistingDirectory
    $existing = @(Get-ChildItem -LiteralPath $outputFullPath -Force -ErrorAction Stop)
    $unexpectedExisting = @($existing | Where-Object { $_.Name -notin @('preflight.json', 'workflow-run.json', 'runner.stdout.raw.txt', 'runner.stderr.raw.txt', 'runner-process.json') })
    if ($unexpectedExisting.Count -ne 0) { throw "Evidence directory contains unexpected files: $($unexpectedExisting.Name -join ', ')" }
} else {
    New-Item -ItemType Directory -Path $outputFullPath -ErrorAction Stop | Out-Null
    $outputFullPath = Resolve-Issue380S1R4Path -TaskRoot $runnerTempFullPath `
        -CandidatePath $outputFullPath -Kind ExistingDirectory
}

function Write-Utf8Text([string]$Path, [string]$Text) {
    $parent = Split-Path -Parent $Path
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force -ErrorAction Stop | Out-Null
    }
    [System.IO.File]::WriteAllText($Path, $Text, [System.Text.UTF8Encoding]::new($false))
}

function Write-Utf8Json([string]$Path, [object]$Value) {
    Write-Utf8Text -Path $Path -Text ((ConvertTo-Json -InputObject $Value -Depth 30) + "`n")
}

function Invoke-GitCapture([string[]]$Arguments, [string]$WorkingDirectory, [int]$TimeoutSeconds = 20) {
    $capture = [ordered]@{
        executable = $null; arguments = @($Arguments); working_directory = $WorkingDirectory
        process_id = $null; exit_code = $null; timed_out = $false; output_timed_out = $false
        launch_error = $null; process_error = $null; capture_error = $null; kill_error = $null; cleanup_error = $null
        kill_attempted = $false; kill_succeeded = $false; process_exit_confirmed = $false
        stdout_task_status = 'NOT_STARTED'; stderr_task_status = 'NOT_STARTED'
        stdout_complete = $false; stderr_complete = $false
        process_timeout_ms = [Math]::Max(1, $TimeoutSeconds * 1000); termination_timeout_ms = 5000
        output_wait_timeout_ms = 5000
        post_close_wait_timeout_ms = 1000; stream_close_attempted = $false; stream_close_error = $null
        cleanup_completed = $false; status = 'INITIALIZING'
        stdout = [string]::Empty; stderr = [string]::Empty
        stdout_char_count = 0; stderr_char_count = 0; stdout_truncated = $false; stderr_truncated = $false
    }
    $process = $null
    $processStarted = $false
    $stdoutTask = $null
    $stderrTask = $null
    $stdoutText = $null
    $stderrText = $null

    try {
        if ([string]::IsNullOrWhiteSpace($WorkingDirectory)) { throw 'Git working directory is required.' }
        if ($TimeoutSeconds -lt 1) { throw 'Git process timeout must be positive.' }
        $gitCommand = Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1
        if ($null -eq $gitCommand -or [string]::IsNullOrWhiteSpace([string]$gitCommand.Path)) {
            throw 'Could not resolve a Git executable from PATH.'
        }
        $capture.executable = [string]$gitCommand.Path

        $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $capture.executable
        $startInfo.WorkingDirectory = $WorkingDirectory
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.StandardOutputEncoding = [System.Text.UTF8Encoding]::new($false)
        $startInfo.StandardErrorEncoding = [System.Text.UTF8Encoding]::new($false)
        foreach ($argument in $Arguments) { [void]$startInfo.ArgumentList.Add([string]$argument) }

        $process = [System.Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) { throw 'Process.Start returned false.' }
        $processStarted = $true
        $capture.process_id = $process.Id

        # Drain both redirected pipes concurrently before waiting for Git.
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        try {
            $capture.process_exit_confirmed = $process.WaitForExit($TimeoutSeconds * 1000)
        } catch {
            $capture.process_error = $_.Exception.ToString()
        }
        if (-not $capture.process_exit_confirmed -and $null -eq $capture.process_error) {
            $capture.timed_out = $true
            $capture.kill_attempted = $true
            try { $process.Kill($true); $capture.kill_succeeded = $true } catch { $capture.kill_error = $_.Exception.ToString() }
            try {
                $capture.process_exit_confirmed = $process.WaitForExit($capture.termination_timeout_ms)
                if (-not $capture.process_exit_confirmed) { $capture.process_error = 'Git process tree did not exit within its bounded termination wait.' }
            } catch { $capture.process_error = $_.Exception.ToString() }
        }
    } catch {
        if (-not $processStarted) { $capture.launch_error = $_.Exception.ToString() }
        elseif ($null -eq $capture.process_error) { $capture.process_error = $_.Exception.ToString() }
        else { $capture.process_error += "`n$($_.Exception.ToString())" }
    } finally {
        if ($processStarted -and $null -ne $process) {
            $stillRunning = $true
            try { $stillRunning = -not $process.HasExited } catch {
                $capture.cleanup_error = "Could not inspect Git process before cleanup: $($_.Exception.ToString())"
            }
            if ($stillRunning) {
                $capture.kill_attempted = $true
                try { $process.Kill($true); $capture.kill_succeeded = $true } catch {
                    $killMessage = "Final process-tree kill failed: $($_.Exception.ToString())"
                    if ($null -eq $capture.kill_error) { $capture.kill_error = $killMessage }
                    else { $capture.kill_error += "`n$killMessage" }
                }
                try {
                    $capture.process_exit_confirmed = $process.WaitForExit($capture.termination_timeout_ms)
                    if (-not $capture.process_exit_confirmed) {
                        $cleanupMessage = 'Git process tree remained alive after final bounded termination wait.'
                        if ($null -eq $capture.cleanup_error) { $capture.cleanup_error = $cleanupMessage }
                        else { $capture.cleanup_error += "`n$cleanupMessage" }
                    }
                } catch {
                    if ($null -eq $capture.cleanup_error) { $capture.cleanup_error = $_.Exception.ToString() }
                    else { $capture.cleanup_error += "`n$($_.Exception.ToString())" }
                }
            }
            try {
                if ($process.HasExited) {
                    $capture.process_exit_confirmed = $true
                    $capture.exit_code = [int]$process.ExitCode
                } elseif ($null -eq $capture.cleanup_error) {
                    $capture.cleanup_error = 'Git process exit could not be confirmed.'
                }
            } catch {
                $exitError = "Could not record Git exit state: $($_.Exception.ToString())"
                if ($null -eq $capture.cleanup_error) { $capture.cleanup_error = $exitError }
                else { $capture.cleanup_error += "`n$exitError" }
            }
        }

        $readTasks = [System.Collections.Generic.List[System.Threading.Tasks.Task]]::new()
        if ($null -ne $stdoutTask) { $readTasks.Add($stdoutTask) }
        if ($null -ne $stderrTask) { $readTasks.Add($stderrTask) }
        if ($readTasks.Count -eq 2) {
            try {
                [void][System.Threading.Tasks.Task]::WaitAll($readTasks.ToArray(), $capture.output_wait_timeout_ms)
            } catch { $capture.capture_error = $_.Exception.ToString() }
        }
        $stdoutDone = ($null -ne $stdoutTask -and $stdoutTask.IsCompletedSuccessfully)
        $stderrDone = ($null -ne $stderrTask -and $stderrTask.IsCompletedSuccessfully)
        if (-not $stdoutDone -or -not $stderrDone) {
            $capture.output_timed_out = ($null -ne $stdoutTask -and -not $stdoutTask.IsCompleted) -or
                ($null -ne $stderrTask -and -not $stderrTask.IsCompleted)
            if ($capture.output_timed_out -and $null -eq $capture.capture_error) {
                $capture.capture_error = 'One or both Git output reads exceeded the bounded wait.'
            }
            if ($null -ne $process) {
                $capture.stream_close_attempted = $true
                try { $process.StandardOutput.Close() } catch {
                    $closeError = "stdout: $($_.Exception.ToString())"
                    if ($null -eq $capture.stream_close_error) { $capture.stream_close_error = $closeError }
                    else { $capture.stream_close_error += "`n$closeError" }
                }
                try { $process.StandardError.Close() } catch {
                    if ($null -eq $capture.stream_close_error) { $capture.stream_close_error = "stderr: $($_.Exception.ToString())" }
                    else { $capture.stream_close_error += "`nstderr: $($_.Exception.ToString())" }
                }
            }
            if ($readTasks.Count -gt 0) {
                try { [void][System.Threading.Tasks.Task]::WaitAll($readTasks.ToArray(), $capture.post_close_wait_timeout_ms) } catch {
                    if ($null -eq $capture.capture_error) { $capture.capture_error = $_.Exception.ToString() }
                    else { $capture.capture_error += "`nPost-close wait: $($_.Exception.ToString())" }
                }
            }
            $capture.stdout_complete = $false
            $capture.stderr_complete = $false
        } else {
            $capture.stdout_complete = $true
            $capture.stderr_complete = $true
        }
        if ($null -ne $stdoutTask) { $capture.stdout_task_status = $stdoutTask.Status.ToString() }
        if ($null -ne $stderrTask) { $capture.stderr_task_status = $stderrTask.Status.ToString() }
        foreach ($stream in @(
            [pscustomobject]@{ Name = 'stdout'; Task = $stdoutTask },
            [pscustomobject]@{ Name = 'stderr'; Task = $stderrTask }
        )) {
            if ($null -eq $stream.Task -or -not $stream.Task.IsCompleted) { continue }
            try {
                $value = [string]$stream.Task.GetAwaiter().GetResult()
                if ($stream.Name -eq 'stdout') { $stdoutText = $value } else { $stderrText = $value }
            } catch {
                if ($null -eq $capture.capture_error) { $capture.capture_error = "$($stream.Name) read failed: $($_.Exception.ToString())" }
                else { $capture.capture_error += "`n$($stream.Name) read failed: $($_.Exception.ToString())" }
            }
        }
        if ($null -ne $process) {
            try { $process.Dispose() } catch {
                $disposeError = "Process.Dispose failed: $($_.Exception.ToString())"
                if ($null -eq $capture.cleanup_error) { $capture.cleanup_error = $disposeError }
                else { $capture.cleanup_error += "`n$disposeError" }
            }
        }
        $capture.cleanup_completed = ($null -eq $capture.cleanup_error -and $null -eq $capture.stream_close_error -and
            ((-not $processStarted) -or $capture.process_exit_confirmed) -and
            (($null -eq $stdoutTask) -or $stdoutTask.IsCompleted) -and (($null -eq $stderrTask) -or $stderrTask.IsCompleted))
    }

    if ($null -ne $stdoutText) {
        $capture.stdout_char_count = $stdoutText.Length
        $capture.stdout_truncated = ($stdoutText.Length -gt 8192)
        $capture.stdout = if ($capture.stdout_truncated) { $stdoutText.Substring(0, 8192) } else { $stdoutText }
    }
    if ($null -ne $stderrText) {
        $capture.stderr_char_count = $stderrText.Length
        $capture.stderr_truncated = ($stderrText.Length -gt 8192)
        $capture.stderr = if ($capture.stderr_truncated) { $stderrText.Substring(0, 8192) } else { $stderrText }
    }
    if (-not $capture.cleanup_completed) { $capture.status = 'PROCESS_CLEANUP_FAILED' }
    elseif ($null -ne $capture.launch_error) { $capture.status = 'LAUNCH_FAILED' }
    elseif ($capture.timed_out) { $capture.status = 'PROCESS_TIMEOUT' }
    elseif ($capture.output_timed_out) { $capture.status = 'OUTPUT_CAPTURE_TIMEOUT' }
    elseif ($null -ne $capture.process_error -or -not $capture.process_exit_confirmed) { $capture.status = 'PROCESS_WAIT_FAILED' }
    elseif ($null -ne $capture.capture_error -or -not $capture.stdout_complete -or -not $capture.stderr_complete) { $capture.status = 'OUTPUT_CAPTURE_FAILED' }
    elseif ($capture.exit_code -eq 0) { $capture.status = 'EXITED_ZERO' }
    else { $capture.status = 'EXITED_NONZERO' }

    $script:GitInvocations.Add([ordered]@{
        executable = $capture.executable; arguments = $capture.arguments; working_directory = $capture.working_directory
        process_id = $capture.process_id; exit_code = $capture.exit_code; timed_out = $capture.timed_out
        output_timed_out = $capture.output_timed_out; status = $capture.status
        launch_error = $capture.launch_error; process_error = $capture.process_error; capture_error = $capture.capture_error
        kill_error = $capture.kill_error; cleanup_error = $capture.cleanup_error
        kill_attempted = $capture.kill_attempted; kill_succeeded = $capture.kill_succeeded
        process_exit_confirmed = $capture.process_exit_confirmed; stdout_task_status = $capture.stdout_task_status
        stderr_task_status = $capture.stderr_task_status; stdout_complete = $capture.stdout_complete
        stderr_complete = $capture.stderr_complete; process_timeout_ms = $capture.process_timeout_ms
        termination_timeout_ms = $capture.termination_timeout_ms; output_wait_timeout_ms = $capture.output_wait_timeout_ms
        post_close_wait_timeout_ms = $capture.post_close_wait_timeout_ms; stream_close_attempted = $capture.stream_close_attempted
        stream_close_error = $capture.stream_close_error; cleanup_completed = $capture.cleanup_completed
        stdout = $capture.stdout; stderr = $capture.stderr
        stdout_char_count = $capture.stdout_char_count; stderr_char_count = $capture.stderr_char_count
        stdout_truncated = $capture.stdout_truncated; stderr_truncated = $capture.stderr_truncated
    })
    return [pscustomobject]$capture
}

function Get-RequiredGitScalar([string[]]$Arguments, [string]$WorkingDirectory, [string]$Label) {
    $capture = Invoke-GitCapture -Arguments $Arguments -WorkingDirectory $WorkingDirectory
    if ($capture.status -notin @('EXITED_ZERO', 'EXITED_NONZERO') -or -not $capture.stdout_complete -or -not $capture.stderr_complete -or
        $capture.timed_out -or $capture.output_timed_out -or $capture.exit_code -ne 0 -or
        $null -ne $capture.launch_error -or $null -ne $capture.process_error -or
        $null -ne $capture.capture_error -or $null -ne $capture.cleanup_error -or -not $capture.cleanup_completed) {
        throw "$Label failed: status=$($capture.status) exit=$($capture.exit_code) timed_out=$($capture.timed_out) output_timed_out=$($capture.output_timed_out) launch_error=$($capture.launch_error) process_error=$($capture.process_error) capture_error=$($capture.capture_error) cleanup_error=$($capture.cleanup_error) stderr=$($capture.stderr)"
    }
    if (-not [string]::IsNullOrEmpty($capture.stderr)) {
        throw "$Label wrote unexpected stderr despite exit 0: $($capture.stderr)"
    }
    if ($capture.stdout_truncated -or $capture.stderr_truncated) {
        throw "$Label output exceeded the bounded identity capture limit."
    }
    $lineMatch = [regex]::Match([string]$capture.stdout, '\A([^\r\n]+)(?:\r?\n)?\z')
    if (-not $lineMatch.Success -or [string]::IsNullOrWhiteSpace($lineMatch.Groups[1].Value)) {
        throw "$Label must return exactly one non-empty output line; exit=$($capture.exit_code) output_chars=$($capture.stdout_char_count) stderr=$($capture.stderr)"
    }
    return $lineMatch.Groups[1].Value.Trim()
}

function Assert-GitNoOutputSuccess([string[]]$Arguments, [string]$WorkingDirectory, [string]$Label) {
    $capture = Invoke-GitCapture -Arguments $Arguments -WorkingDirectory $WorkingDirectory
    if ($capture.status -notin @('EXITED_ZERO', 'EXITED_NONZERO') -or -not $capture.stdout_complete -or -not $capture.stderr_complete -or
        $capture.timed_out -or $capture.output_timed_out -or $capture.exit_code -ne 0 -or
        $null -ne $capture.launch_error -or $null -ne $capture.process_error -or
        $null -ne $capture.capture_error -or $null -ne $capture.cleanup_error -or -not $capture.cleanup_completed) {
        throw "$Label failed: status=$($capture.status) exit=$($capture.exit_code) timed_out=$($capture.timed_out) output_timed_out=$($capture.output_timed_out) launch_error=$($capture.launch_error) process_error=$($capture.process_error) capture_error=$($capture.capture_error) cleanup_error=$($capture.cleanup_error) stderr=$($capture.stderr)"
    }
    if (-not [string]::IsNullOrEmpty($capture.stderr)) {
        throw "$Label wrote unexpected stderr despite exit 0: $($capture.stderr)"
    }
    if (-not [string]::IsNullOrEmpty($capture.stdout)) {
        throw "$Label must succeed without stdout; output_chars=$($capture.stdout_char_count) output=$($capture.stdout)"
    }
}

function Assert-GitWorktreeClean([string]$WorkingDirectory) {
    $capture = Invoke-GitCapture -Arguments @('status', '--porcelain', '--untracked-files', 'all') -WorkingDirectory $WorkingDirectory
    if ($capture.status -notin @('EXITED_ZERO', 'EXITED_NONZERO') -or -not $capture.stdout_complete -or -not $capture.stderr_complete -or
        $capture.timed_out -or $capture.output_timed_out -or $capture.exit_code -ne 0 -or
        $null -ne $capture.launch_error -or $null -ne $capture.process_error -or
        $null -ne $capture.capture_error -or $null -ne $capture.cleanup_error -or -not $capture.cleanup_completed) {
        throw "Git worktree status failed: status=$($capture.status) exit=$($capture.exit_code) timed_out=$($capture.timed_out) output_timed_out=$($capture.output_timed_out) launch_error=$($capture.launch_error) process_error=$($capture.process_error) capture_error=$($capture.capture_error) cleanup_error=$($capture.cleanup_error) stderr=$($capture.stderr)"
    }
    if (-not [string]::IsNullOrEmpty($capture.stderr)) {
        throw "Git worktree status wrote unexpected stderr despite exit 0: $($capture.stderr)"
    }
    if ($capture.stdout_char_count -ne 0) {
        throw "Checkout is not clean before Runner starts: status_output=$($capture.stdout) output_chars=$($capture.stdout_char_count) truncated=$($capture.stdout_truncated)"
    }
}

function Add-Assertion([string]$Name, [bool]$Passed, [string]$Details) {
    $script:Assertions.Add([ordered]@{ name = $Name; passed = $Passed; details = $Details })
    if (-not $Passed) { throw "Assertion failed: $Name — $Details" }
}

function Get-TaskOwnedProcessSnapshot {
    $matching = [System.Collections.Generic.List[object]]::new()
    foreach ($processInfo in (Get-CimInstance -ClassName Win32_Process -Property @('ProcessId', 'ExecutablePath') -ErrorAction Stop)) {
        if ([string]::IsNullOrWhiteSpace([string]$processInfo.ExecutablePath)) { continue }
        try {
            $imagePath = Resolve-Issue380S1R4Path -TaskRoot $script:TaskRoot `
                -CandidatePath ([string]$processInfo.ExecutablePath) -Kind Executable
        }
        catch { continue }
        $matching.Add([ordered]@{
            process_id = [int]$processInfo.ProcessId
            image_path = $imagePath
        })
    }
    return @($matching.ToArray())
}

function Wait-ForTaskOwnedProcessesToExit([int]$TimeoutSeconds = 8) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $snapshot = @(Get-TaskOwnedProcessSnapshot)
        if ($snapshot.Count -eq 0) { return $snapshot }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    return @(Get-TaskOwnedProcessSnapshot)
}

function Invoke-CapturedProcess(
    [string]$FilePath,
    [string[]]$ArgumentList,
    [string]$WorkingDirectory,
    [string]$StdoutPath,
    [string]$StderrPath,
    [int]$TimeoutSeconds = 45
) {
    $fileFullPath = Resolve-Issue380S1R4Path -TaskRoot $script:TaskRoot -CandidatePath $FilePath -Kind Executable
    $workingDirectoryFullPath = Resolve-Issue380S1R4Path -TaskRoot $script:TaskRoot `
        -CandidatePath $WorkingDirectory -Kind WorkingDirectory
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $fileFullPath
    $startInfo.WorkingDirectory = $workingDirectoryFullPath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.Environment['TEMP'] = $workingDirectoryFullPath
    $startInfo.Environment['TMP'] = $workingDirectoryFullPath
    foreach ($argument in $ArgumentList) { [void]$startInfo.ArgumentList.Add($argument) }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "Could not start process: $FilePath" }
    $processId = $process.Id
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $completed = $process.WaitForExit($TimeoutSeconds * 1000)
    if (-not $completed) {
        try { $process.Kill($true) } catch { }
        $process.WaitForExit()
    }
    $processExited = $process.HasExited
    $exitCode = if ($processExited) { [int]$process.ExitCode } else { $null }
    $stdoutComplete = $stdoutTask.Wait(5000)
    $stderrComplete = $stderrTask.Wait(5000)
    if (-not $stdoutComplete -or -not $stderrComplete) {
        try { $process.StandardOutput.Close() } catch { }
        try { $process.StandardError.Close() } catch { }
        try { [void]$stdoutTask.Wait(1000) } catch { }
        try { [void]$stderrTask.Wait(1000) } catch { }
    }
    $stdout = if ($stdoutComplete) { $stdoutTask.GetAwaiter().GetResult() } else { '[stdout stream did not close within 5 seconds after process wait]' }
    $stderr = if ($stderrComplete) { $stderrTask.GetAwaiter().GetResult() } else { '[stderr stream did not close within 5 seconds after process wait]' }
    Write-Utf8Text -Path $StdoutPath -Text $stdout
    Write-Utf8Text -Path $StderrPath -Text $stderr

    $remainingTaskProcesses = @()
    $processCheckFailure = $null
    try { $remainingTaskProcesses = @(Wait-ForTaskOwnedProcessesToExit) }
    catch { $processCheckFailure = $_.Exception.ToString() }
    $record = [ordered]@{
        file = $fileFullPath
        process_image_path = $fileFullPath
        file_sha256 = if (Test-Path -LiteralPath $fileFullPath -PathType Leaf) { Get-FileSha256 $fileFullPath } else { $null }
        process_id = $processId
        argument_list = @($ArgumentList)
        arguments = @($ArgumentList)
        working_directory = $workingDirectoryFullPath
        child_temp_directory = $workingDirectoryFullPath
        timed_out = (-not $completed)
        timeout_state = if ($completed) { 'COMPLETED_BEFORE_TIMEOUT' } else { 'TIMED_OUT_OWNED_PROCESS_TREE_TERMINATED' }
        process_exited = $processExited
        exit_code = $exitCode
        stdout_stream_completed = $stdoutComplete
        stderr_stream_completed = $stderrComplete
        process_check_failure = $processCheckFailure
        task_owned_processes_remaining = $remainingTaskProcesses
        all_task_owned_processes_exited = ($remainingTaskProcesses.Count -eq 0)
        stdout_path = $StdoutPath
        stderr_path = $StderrPath
        stdout_utf8 = $stdout
        stderr_utf8 = $stderr
    }
    $script:Processes.Add($record)
    $process.Dispose()
    Add-Assertion -Name "task_owned_processes_exited_after_pid_${processId}" `
        -Passed (($processExited) -and $stdoutComplete -and $stderrComplete -and ($null -eq $processCheckFailure) -and ($remainingTaskProcesses.Count -eq 0)) `
        -Details "process_id=$processId process_exited=$processExited stdout_complete=$stdoutComplete stderr_complete=$stderrComplete process_check_failure=$processCheckFailure remaining=$($remainingTaskProcesses | ConvertTo-Json -Compress -Depth 4)"
    return $record
}

function Get-SyntheticRegistrySnapshot {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::CurrentUser,
        [Microsoft.Win32.RegistryView]::Registry64
    )
    try {
        $key = $base.OpenSubKey($script:RegistrySubKey, $false)
        if ($null -eq $key) {
            return [ordered]@{ exists = $false; value_name = $script:RegistryValueName; kind = $null; value = $null; owner_value = $null; owner_kind = $null; value_names = @(); subkey_names = @() }
        }
        try {
            $valueNames = @($key.GetValueNames())
            $subkeyNames = @($key.GetSubKeyNames())
            $ownerValue = if ($valueNames -contains $script:RegistryOwnerValueName) {
                [string]$key.GetValue($script:RegistryOwnerValueName, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            } else { $null }
            $ownerKind = if ($valueNames -contains $script:RegistryOwnerValueName) {
                $key.GetValueKind($script:RegistryOwnerValueName).ToString()
            } else { $null }
            if ($valueNames -notcontains $script:RegistryValueName) {
                return [ordered]@{ exists = $true; value_name = $script:RegistryValueName; kind = 'MISSING'; value = $null; owner_value = $ownerValue; owner_kind = $ownerKind; value_names = $valueNames; subkey_names = $subkeyNames }
            }
            return [ordered]@{
                exists = $true
                value_name = $script:RegistryValueName
                kind = $key.GetValueKind($script:RegistryValueName).ToString()
                value = $key.GetValue($script:RegistryValueName, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                owner_value = $ownerValue
                owner_kind = $ownerKind
                value_names = $valueNames
                subkey_names = $subkeyNames
            }
        } finally { $key.Dispose() }
    } finally { $base.Dispose() }
}

function Add-ExecutableEvidence([string]$SourcePath, [string]$EvidencePath, [string]$Role, [bool]$AllowEmpty = $false) {
    $SourcePath = Resolve-Issue380S1R4Path -TaskRoot $script:TaskRoot -CandidatePath $SourcePath -Kind ExistingFile
    $sourceItem = Get-Item -LiteralPath $SourcePath -Force
    if (-not $AllowEmpty -and $sourceItem.Length -le 0) { throw "Executable is empty: $SourcePath" }
    Copy-Item -LiteralPath $SourcePath -Destination $EvidencePath -Force
    $item = Get-Item -LiteralPath $EvidencePath
    $hash = (Get-FileHash -LiteralPath $EvidencePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Executables.Add([ordered]@{
        role = $Role
        evidence_path = $EvidencePath.Substring($outputFullPath.Length).TrimStart('\', '/').Replace('\', '/')
        source_path = $SourcePath
        length_bytes = [long]$item.Length
        sha256 = $hash
    })
}

function Assert-ProcessExit([string]$Name, [object]$Result, [int]$ExpectedCode) {
    Add-Assertion -Name $Name -Passed ((-not $Result.timed_out) -and ($Result.exit_code -eq $ExpectedCode)) `
        -Details "expected=$ExpectedCode actual=$($Result.exit_code) timed_out=$($Result.timed_out) pid=$($Result.process_id) path=$($Result.process_image_path) args=$($Result.argument_list -join ' | ') stdout=$($Result.stdout_path) stderr=$($Result.stderr_path)"
}

function Assert-PrimaryFileWriteEvidence([string]$TracePath) {
    Add-Assertion -Name 'file_direct_trace_exists_after_both_abort_processes_exit' `
        -Passed ((Test-Path -LiteralPath $TracePath -PathType Leaf) -and ((Get-Item -LiteralPath $TracePath).Length -gt 0)) `
        -Details "trace_path=$TracePath; both file-mode Abort processes have exited."
    $traceLines = @(Get-Content -LiteralPath $TracePath)
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
            foreach ($field in $requiredFields) { if (-not $contextLines[0].Contains($field)) { $complete = $false } }
        }
        Add-Assertion -Name "file_trace_${context}_record_complete" -Passed $complete `
            -Details "exactly one complete context=$context record required; matching_lines=$($contextLines -join ' | ')"
    }
    Add-Assertion -Name 'file_trace_has_exactly_two_abort_records' -Passed ($traceLines.Count -eq 2) `
        -Details "expected=2 actual=$($traceLines.Count)"
}

function Get-FileSha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-SyntheticRegistryOwnerToken {
    return "$env:GITHUB_REPOSITORY|$env:GITHUB_REF|$env:GITHUB_RUN_ID|$env:GITHUB_RUN_ATTEMPT|$env:GITHUB_SHA"
}

function WaitForP3RunnerProcessRecord([string]$Path) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $record = $null
        try { $record = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json } catch { }
        if ($null -ne $record -and $record.status -eq 'RUNNING' -and $record.process_id -gt 0) {
            if ($record.repository -ne $env:GITHUB_REPOSITORY -or $record.triggering_ref -ne $env:GITHUB_REF -or
                $record.workflow_sha -ne $env:GITHUB_SHA -or $record.run_id -ne $env:GITHUB_RUN_ID -or
                $record.run_attempt -ne $env:GITHUB_RUN_ATTEMPT) {
                throw 'Runner process record belongs to a different GitHub run.'
            }
            $currentProcess = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $PID" -Property @('ProcessId', 'ParentProcessId', 'ExecutablePath') -ErrorAction Stop
            if ($null -eq $currentProcess -or [int]$currentProcess.ParentProcessId -ne [int]$record.process_id) {
                throw "Runner process record does not identify this P3 process parent: current_pid=$PID recorded_parent=$($record.process_id)"
            }
            $actualImage = [System.IO.Path]::GetFullPath([string]$currentProcess.ExecutablePath)
            $recordImage = [System.IO.Path]::GetFullPath([string]$record.image)
            if (-not $actualImage.Equals($recordImage, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "P3 process image path does not match the captured launcher: current=$actualImage recorded=$recordImage"
            }
            if ((Get-FileSha256 $actualImage) -ne $record.image_sha256) {
                throw 'P3 process image SHA-256 does not match the captured launcher record.'
            }
            $expectedArguments = @('-NoLogo', '-NoProfile', '-NonInteractive', '-File', (Join-Path $PSScriptRoot 'issue380S1R4P3SyntheticNsisRunner.ps1'), '-OutputRoot', $outputFullPath)
            $actualArguments = @($record.arguments)
            if ($actualArguments.Count -ne $expectedArguments.Count) { throw 'P3 process argument count differs from the fixed Runner invocation.' }
            for ($index = 0; $index -lt $expectedArguments.Count; $index++) {
                if (-not ([string]$actualArguments[$index]).Equals($expectedArguments[$index], [System.StringComparison]::OrdinalIgnoreCase)) {
                    throw "P3 process argument $index differs from the fixed Runner invocation."
                }
            }
            return $record
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'The Workflow wrapper did not publish a matching running process record within five seconds.'
}

function Download-And-VerifyPinnedZip {
    if (Test-Path -LiteralPath $taskRoot) { throw "Task temp directory already exists: $taskRoot" }
    New-Item -ItemType Directory -Path $taskRoot -ErrorAction Stop | Out-Null
    $null = Resolve-Issue380S1R4Path -TaskRoot $runnerTempFullPath `
        -CandidatePath $taskRoot -Kind ExistingDirectory
    Write-Utf8Text -Path $taskMarker -Text "repository=$env:GITHUB_REPOSITORY`nref=$env:GITHUB_REF`nrun_id=$env:GITHUB_RUN_ID`nattempt=$env:GITHUB_RUN_ATTEMPT`nhead=$env:GITHUB_SHA`nroot=$taskRoot`n"
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $true
    $handler.MaxAutomaticRedirections = 5
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(120)
    $client.DefaultRequestHeaders.UserAgent.ParseAdd('Zen-Canvas-Issue-380-S1R/1.0')
    $response = $null
    $inputStream = $null
    $outputStream = $null
    try {
        $response = $client.GetAsync($script:ZipUrl, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
        $script:Acquisition.http_status = [int]$response.StatusCode
        if ($null -ne $response.RequestMessage -and $null -ne $response.RequestMessage.RequestUri) {
            $script:Acquisition.final_url = $response.RequestMessage.RequestUri.AbsoluteUri
        }
        $script:Acquisition.content_length_header = $response.Content.Headers.ContentLength
        if ($null -ne $response.Content.Headers.ContentType) { $script:Acquisition.content_type = $response.Content.Headers.ContentType.ToString() }
        if (-not $response.IsSuccessStatusCode) {
            $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
            $bodyMemory = [System.IO.MemoryStream]::new()
            $errorBuffer = [byte[]]::new(8192)
            $bodyBytes = 0
            while ($bodyBytes -lt 65536 -and ($read = $inputStream.Read($errorBuffer, 0, [Math]::Min($errorBuffer.Length, 65536 - $bodyBytes))) -gt 0) {
                $bodyMemory.Write($errorBuffer, 0, $read)
                $bodyBytes += $read
            }
            if ($bodyBytes -eq 65536) { $script:Acquisition.error_body_truncated = $true }
            $bodyPath = Join-Path $outputFullPath 'download-http-error-body.txt'
            Write-Utf8Text -Path $bodyPath -Text ([System.Text.Encoding]::UTF8.GetString($bodyMemory.ToArray()))
            $script:Acquisition.error_body_path = 'download-http-error-body.txt'
            $bodyMemory.Dispose()
            throw "NSIS ZIP download returned HTTP $([int]$response.StatusCode)"
        }
        if ($null -ne $response.Content.Headers.ContentLength -and $response.Content.Headers.ContentLength -gt 33554432) {
            throw "NSIS ZIP Content-Length exceeds 32 MiB: $($response.Content.Headers.ContentLength)"
        }

        $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
        $outputStream = [System.IO.FileStream]::new($zipPath, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
        $buffer = [byte[]]::new(65536)
        $total = [long]0
        while (($read = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $total += $read
            if ($total -gt 33554432) { throw "NSIS ZIP download exceeds 32 MiB: $total" }
            $outputStream.Write($buffer, 0, $read)
        }
        $outputStream.Flush($true)
        $script:Acquisition.downloaded_bytes = $total
    } catch {
        $script:Acquisition.failure = $_.Exception.ToString()
        throw
    } finally {
        if ($null -ne $outputStream) { $outputStream.Dispose() }
        if ($null -ne $inputStream) { $inputStream.Dispose() }
        if ($null -ne $response) { $response.Dispose() }
        $client.Dispose()
        $handler.Dispose()
        if (Test-Path -LiteralPath $zipPath -PathType Leaf) {
            $script:Acquisition.downloaded_bytes = (Get-Item -LiteralPath $zipPath).Length
            $script:Acquisition.downloaded_sha256 = Get-FileSha256 $zipPath
            Copy-Item -LiteralPath $zipPath -Destination (Join-Path $outputFullPath 'nsis-3.11-download.zip') -Force
        }
        Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
    }

    if (-not (Test-Path -LiteralPath $zipPath -PathType Leaf)) { throw 'NSIS ZIP download produced no file.' }
    $actualHash = Get-FileSha256 $zipPath
    $script:Acquisition.downloaded_sha256 = $actualHash
    if ($actualHash -ne $script:ExpectedZipSha256) {
        $script:Acquisition.failure = "SHA256 mismatch: expected=$($script:ExpectedZipSha256) actual=$actualHash"
        Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
        throw $script:Acquisition.failure
    }
    if ((Get-Item -LiteralPath $zipPath).Length -le 0) { throw 'NSIS ZIP is empty.' }
    $script:Acquisition.sha256_verified_before_extract = $true
    $script:Acquisition.failure = $null
    Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
}

function Get-ZipEntrySha256([System.IO.Compression.ZipArchiveEntry]$Entry) {
    $stream = $Entry.Open()
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try { return ([Convert]::ToHexString($sha.ComputeHash($stream))).ToLowerInvariant() }
    finally { $sha.Dispose(); $stream.Dispose() }
}

function Expand-VerifiedZipSafely {
    if (-not $script:Acquisition.sha256_verified_before_extract) { throw 'Refusing to inspect or extract an NSIS ZIP without a verified whole-archive SHA-256.' }
    if (Test-Path -LiteralPath $extractionRoot) { throw "Extraction directory already exists: $extractionRoot" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
    $validatedMembers = [System.Collections.Generic.List[object]]::new()
    try {
        if ($archive.Entries.Count -gt 2000) { throw "NSIS ZIP has too many entries: $($archive.Entries.Count)" }
        $script:Acquisition.archive_entry_count = $archive.Entries.Count
        $uncompressedTotal = [long]0
        $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
        $matched = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        foreach ($entry in $archive.Entries) {
            $normalized = $entry.FullName.Replace('\', '/')
            $isDirectory = $normalized.EndsWith('/')
            $relative = $normalized.TrimEnd('/')
            if ([string]::IsNullOrWhiteSpace($relative) -or $relative.StartsWith('/') -or $relative -match '^[A-Za-z]:' -or $relative.Contains(':') -or $relative.Contains([char]0)) {
                throw "Unsafe ZIP entry path: $($entry.FullName)"
            }
            $segments = $relative.Split('/')
            if (($segments -contains '..') -or ($segments -contains '.') -or ($segments -contains '')) { throw "Unsafe ZIP traversal or empty path segment: $($entry.FullName)" }
            if ($relative -ne 'nsis-3.11' -and -not $relative.StartsWith('nsis-3.11/', [System.StringComparison]::Ordinal)) {
                throw "ZIP entry is outside the pinned nsis-3.11 root: $($entry.FullName)"
            }
            if (-not $seen.Add($relative)) { throw "Duplicate or case-colliding ZIP entry path: $relative" }
            $mode = ($entry.ExternalAttributes -shr 16) -band 0xF000
            if ($mode -eq 0xA000) { throw "Symbolic link is not allowed in NSIS ZIP: $relative" }
            if ($isDirectory -and $entry.Length -ne 0) { throw "Directory entry has unexpected payload bytes: $relative" }
            $uncompressedTotal += [long]$entry.Length
            if ($uncompressedTotal -gt 536870912) { throw "NSIS ZIP expands beyond 512 MiB: $uncompressedTotal" }

            if ($expectedArchiveFiles.Contains($relative)) {
                if ($isDirectory) { throw "Expected executable is encoded as a directory: $relative" }
                $expected = $expectedArchiveFiles[$relative]
                if ([long]$entry.Length -ne [long]$expected.length) {
                    throw "Pinned archive member length mismatch: path=$relative expected=$($expected.length) actual=$($entry.Length)"
                }
                $memberHash = Get-ZipEntrySha256 -Entry $entry
                if ($memberHash -ne $expected.sha256) {
                    throw "Pinned archive member SHA-256 mismatch: path=$relative expected=$($expected.sha256) actual=$memberHash"
                }
                [void]$matched.Add($relative)
                $validatedMembers.Add([ordered]@{
                    path = $relative
                    role = $expected.role
                    length_bytes = [long]$entry.Length
                    sha256 = $memberHash
                    verification = 'ZIP_MEMBER_PRE_EXTRACTION'
                })
            }
            if ($relative -match '(?i)(^|/)makensis\.exe$' -and -not $expectedArchiveFiles.Contains($relative)) {
                throw "Unapproved makensis.exe member path: $relative"
            }
        }
        foreach ($memberPath in $expectedArchiveFiles.Keys) {
            if (-not $matched.Contains($memberPath)) { throw "Required pinned archive member is missing: $memberPath" }
        }
        $script:Acquisition.archive_uncompressed_bytes = $uncompressedTotal
        $script:Acquisition.validated_archive_members = @($validatedMembers.ToArray())
    } finally { $archive.Dispose() }

    New-Item -ItemType Directory -Path $extractionRoot -ErrorAction Stop | Out-Null
    $extractFull = Resolve-Issue380S1R4Path -TaskRoot $taskRoot -CandidatePath $extractionRoot -Kind ExistingDirectory
    $script:Acquisition.extraction_root = $extractFull
    try {
        [System.IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $extractFull)
        $extractedMembers = [System.Collections.Generic.List[object]]::new()
        foreach ($memberPath in $expectedArchiveFiles.Keys) {
            $relativeFs = $memberPath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
            $memberFull = Resolve-Issue380S1R4Path -TaskRoot $extractFull `
                -CandidatePath (Join-Path $extractFull $relativeFs) -Kind Executable
            $item = Get-Item -LiteralPath $memberFull -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Pinned extracted member is a reparse point: $memberPath" }
            $expected = $expectedArchiveFiles[$memberPath]
            $memberHash = Get-FileSha256 $memberFull
            if ([long]$item.Length -ne [long]$expected.length -or $memberHash -ne $expected.sha256) {
                throw "Extracted member identity mismatch: path=$memberPath expected_length=$($expected.length) actual_length=$($item.Length) expected_sha256=$($expected.sha256) actual_sha256=$memberHash"
            }
            $extractedMembers.Add([ordered]@{
                path = $memberPath
                absolute_path = $memberFull
                role = $expected.role
                length_bytes = [long]$item.Length
                sha256 = $memberHash
                verification = 'EXTRACTED_FILE_PRE_EXECUTION'
            })
        }
        $script:Acquisition.extracted_members = @($extractedMembers.ToArray())
        $script:Acquisition.extraction_status = 'EXTRACTED_AND_MEMBER_IDENTITIES_VERIFIED'
        Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
    } catch {
        $script:Acquisition.extraction_status = 'FAILED_OR_REJECTED'
        $script:Acquisition.failure = $_.Exception.ToString()
        Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
        throw
    }
}

function Compile-Probe([string]$Makensis, [string]$Probe, [string]$TraceMode, [string]$RunMode, [string]$OutputName, [string]$WorkingDirectory, [string]$EvidenceLogStem) {
    $args = @('-NOCD', '-V2', "-DTRACE_MODE=$TraceMode", "-DRUN_MODE=$RunMode", "-DOUTPUT_NAME=$OutputName", $Probe)
    $compile = Invoke-CapturedProcess -FilePath $Makensis -ArgumentList $args -WorkingDirectory $WorkingDirectory `
        -StdoutPath (Join-Path $outputFullPath "$EvidenceLogStem.stdout.txt") `
        -StderrPath (Join-Path $outputFullPath "$EvidenceLogStem.stderr.txt") -TimeoutSeconds 60
    $outputPath = Join-Path $WorkingDirectory $OutputName
    $evidencePath = Join-Path (Join-Path $outputFullPath $TraceMode) $OutputName
    $outputExists = Test-Path -LiteralPath $outputPath -PathType Leaf
    $outputLength = 0
    if ($outputExists) {
        Add-ExecutableEvidence -SourcePath $outputPath -EvidencePath $evidencePath `
            -Role "compiler_output_${TraceMode}_${RunMode}" -AllowEmpty $true
        $outputLength = (Get-Item -LiteralPath $outputPath).Length
    }
    Add-Assertion -Name "compile_${TraceMode}_${RunMode}_exit_zero_and_output_present" `
        -Passed ((-not $compile.timed_out) -and ($compile.exit_code -eq 0) -and $outputExists -and ($outputLength -gt 0)) `
        -Details "exit=$($compile.exit_code) timed_out=$($compile.timed_out) output_exists=$outputExists output_bytes=$outputLength stdout=$EvidenceLogStem.stdout.txt stderr=$EvidenceLogStem.stderr.txt"
    return $outputPath
}

$result = [ordered]@{
    phase = '#380-S1R4-P3 Runner null-safety candidate'
    status = 'IN_PROGRESS'
    repository = $env:GITHUB_REPOSITORY
    ref = $env:GITHUB_REF
    workflow_sha = $env:GITHUB_SHA
    run_id = $env:GITHUB_RUN_ID
    run_attempt = $env:GITHUB_RUN_ATTEMPT
    job = $env:GITHUB_JOB
    frozen_s1r4_p0_head = $script:S1R4P0Head
    frozen_s1r4_p0_tree = $script:S1R4P0Tree
    frozen_p2_head = $script:P2Head
    frozen_p2_tree = $script:P2Tree
    frozen_p1_head = $script:P1Head
    frozen_p1_tree = $script:P1Tree
    frozen_s1r3_head = $script:S1R3Head
    frozen_s1r3_tree = $script:S1R3Tree
    frozen_s1r2_head = $script:S1R2Head
    frozen_s1r2_tree = $script:S1R2Tree
    frozen_s1r_head = $script:S1RHead
    frozen_s1r_tree = $script:S1RTree
    frozen_s1_head = $script:S1Head
    frozen_s0_head = $script:S0Head
    candidate_freeze_head = $null
    candidate_freeze_tree = $null
    activation_parent = $null
    runner_git_blob_sha = $null
    runner_file_sha256 = $null
    tested_head = $null
    tested_tree = $null
    frozen_probe_blob = $null
    environment = [ordered]@{
        runner_os = $env:RUNNER_OS
        runner_arch = $env:RUNNER_ARCH
        image_os = $env:ImageOS
        image_version = $env:ImageVersion
        powershell = $PSVersionTable.PSVersion.ToString()
        windows_version = [System.Environment]::OSVersion.VersionString
    }
    acquisition = $script:Acquisition
    compiler = $null
    compile_options = 'NSIS 3.11 pinned forwarding launcher -NOCD -V2 -DTRACE_MODE=file first, then optional detail -DRUN_MODE=stage|abort -DOUTPUT_NAME=<name> frozen S0 Probe.'
    executables = @()
    processes = @()
    registry_snapshots = @()
    log_comparison = @()
    launcher_process = $null
    git_invocations = @()
    assertions = @()
    cleanup = [ordered]@{ registry_key = 'NOT_CHECKED'; task_temp = 'NOT_CREATED'; task_owned_processes = 'NOT_CHECKED' }
    failure = $null
}

try {
    if ($env:GITHUB_REPOSITORY -ne 'ArdenZC/Zen-Canvas') { throw "Unexpected repository identity: $env:GITHUB_REPOSITORY" }
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_SHA)) { throw 'GITHUB_SHA is required.' }
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_RUN_ID)) { throw 'GITHUB_RUN_ID is required.' }
    if ($env:GITHUB_REF -ne "refs/heads/$($script:ExpectedBranch)") { throw "Unexpected workflow ref: $env:GITHUB_REF" }
    if ($env:GITHUB_RUN_ATTEMPT -ne '1') { throw "One-shot gate refuses run attempt $env:GITHUB_RUN_ATTEMPT" }
    if ($env:PREFLIGHT_OUTCOME -ne 'success') { throw "Static Hosted preflight did not pass: $env:PREFLIGHT_OUTCOME" }
    if ($env:RUNNER_OS -ne 'Windows') { throw "Windows runner required: $env:RUNNER_OS" }
    if ($PSVersionTable.PSVersion.Major -ne 7) { throw "PowerShell 7 required: $($PSVersionTable.PSVersion)" }
    if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { throw 'RUNNER_TEMP is required.' }
    $preflightPath = Join-Path $outputFullPath 'preflight.json'
    $workflowRunPath = Join-Path $outputFullPath 'workflow-run.json'
    $runnerStdoutPath = Join-Path $outputFullPath 'runner.stdout.raw.txt'
    $runnerStderrPath = Join-Path $outputFullPath 'runner.stderr.raw.txt'
    $runnerProcessPath = Join-Path $outputFullPath 'runner-process.json'
    foreach ($path in @($preflightPath, $workflowRunPath, $runnerStdoutPath, $runnerStderrPath, $runnerProcessPath)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "P3 run input/evidence file is missing: $path" }
        $null = Resolve-Issue380S1R4Path -TaskRoot $runnerTempFullPath -CandidatePath $path -Kind ExistingFile
    }
    $preflightRecord = Get-Content -LiteralPath $preflightPath -Raw | ConvertFrom-Json
    $workflowRunRecord = Get-Content -LiteralPath $workflowRunPath -Raw | ConvertFrom-Json
    if ($preflightRecord.status -ne 'PASS' -or $preflightRecord.expected_p2_head -ne $script:P2Head -or
        $preflightRecord.expected_p2_tree -ne $script:P2Tree -or $preflightRecord.triggering_ref -ne $env:GITHUB_REF -or
        $preflightRecord.triggering_head -ne $env:GITHUB_SHA) {
        throw 'P3 static preflight JSON does not attest to this exact P2/P3 run identity.'
    }
    if ($workflowRunRecord.repository -ne $env:GITHUB_REPOSITORY -or $workflowRunRecord.triggering_ref -ne $env:GITHUB_REF -or
        $workflowRunRecord.triggering_head -ne $env:GITHUB_SHA -or $workflowRunRecord.run_id -ne $env:GITHUB_RUN_ID -or
        $workflowRunRecord.run_attempt -ne $env:GITHUB_RUN_ATTEMPT) {
        throw 'P3 workflow identity JSON does not match the current GitHub run.'
    }
    $result.launcher_process = WaitForP3RunnerProcessRecord -Path $runnerProcessPath

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_WORKSPACE)) { throw 'GITHUB_WORKSPACE is required.' }
    $workspaceRoot = (Resolve-Path -LiteralPath $env:GITHUB_WORKSPACE).Path
    if (-not $repoRoot.Equals($workspaceRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Runner source is outside GITHUB_WORKSPACE: runner_root=$repoRoot workspace=$workspaceRoot"
    }
    Push-Location $repoRoot
    try {
        $head = Get-RequiredGitScalar -Arguments @('rev-parse', 'HEAD') -WorkingDirectory $repoRoot -Label 'Resolve checked-out HEAD'
        $tree = Get-RequiredGitScalar -Arguments @('rev-parse', 'HEAD^{tree}') -WorkingDirectory $repoRoot -Label 'Resolve checked-out Tree'
        if ($head -ne $env:GITHUB_SHA) { throw "Checkout HEAD $head differs from workflow SHA $env:GITHUB_SHA" }
        Assert-GitWorktreeClean -WorkingDirectory $repoRoot

        $activationParent = Get-RequiredGitScalar -Arguments @('rev-parse', 'HEAD^') -WorkingDirectory $repoRoot -Label 'Resolve activation parent'
        $activationParentTree = Get-RequiredGitScalar -Arguments @('rev-parse', "$activationParent^{tree}") -WorkingDirectory $repoRoot -Label 'Resolve activation parent Tree'
        if ($preflightRecord.candidate_freeze_head -cne $activationParent -or
            $preflightRecord.candidate_freeze_tree -cne $activationParentTree -or
            $preflightRecord.activation_parent -cne $activationParent) {
            throw 'Preflight candidate freeze identity does not match the actual direct parent of this activation commit.'
        }

        $runnerCommitBlob = Get-RequiredGitScalar -Arguments @('rev-parse', "HEAD:$($script:RunnerPath)") -WorkingDirectory $repoRoot -Label 'Resolve committed P3 Runner blob'
        $runnerWorktreeBlob = Get-RequiredGitScalar -Arguments @('hash-object', '--', $script:RunnerPath) -WorkingDirectory $repoRoot -Label 'Resolve worktree P3 Runner blob'
        $runnerFileSha256 = Get-FileSha256 (Join-Path $repoRoot $script:RunnerPath)
        if ($runnerCommitBlob -cne $runnerWorktreeBlob -or
            $preflightRecord.p3_runner_git_blob_sha -cne $runnerCommitBlob -or
            $preflightRecord.p3_runner_file_sha256 -cne $runnerFileSha256) {
            throw "P3 Runner identity mismatch: committed_blob=$runnerCommitBlob worktree_blob=$runnerWorktreeBlob file_sha256=$runnerFileSha256"
        }

        $frozenP2Tree = Get-RequiredGitScalar -Arguments @('rev-parse', "$($script:P2Head)^{tree}") -WorkingDirectory $repoRoot -Label 'Resolve frozen P2 Tree'
        if ($frozenP2Tree -cne $script:P2Tree) { throw "Frozen P2 identity mismatch: expected_tree=$($script:P2Tree) actual_tree=$frozenP2Tree" }
        Assert-GitNoOutputSuccess -Arguments @('merge-base', '--is-ancestor', $script:P2Head, 'HEAD') -WorkingDirectory $repoRoot -Label 'Check frozen P2 ancestry'
        $frozenP1Tree = Get-RequiredGitScalar -Arguments @('rev-parse', "$($script:P1Head)^{tree}") -WorkingDirectory $repoRoot -Label 'Resolve frozen P1 Tree'
        if ($frozenP1Tree -cne $script:P1Tree) { throw "Frozen P1 identity mismatch: expected_tree=$($script:P1Tree) actual_tree=$frozenP1Tree" }
        Assert-GitNoOutputSuccess -Arguments @('merge-base', '--is-ancestor', $script:P1Head, 'HEAD') -WorkingDirectory $repoRoot -Label 'Check frozen P1 ancestry'
        foreach ($path in @($script:P1PathGuardPath, $script:P1PathTestPath)) {
            $frozenPathBlob = Get-RequiredGitScalar -Arguments @('rev-parse', "$($script:P1Head):$path") -WorkingDirectory $repoRoot -Label "Resolve frozen P1 blob for $path"
            $actualPathBlob = Get-RequiredGitScalar -Arguments @('hash-object', '--', $path) -WorkingDirectory $repoRoot -Label "Resolve current P1 blob for $path"
            if ($frozenPathBlob -cne $actualPathBlob) { throw "Accepted P1 source changed: $path" }
        }
        $result.candidate_freeze_head = $preflightRecord.candidate_freeze_head
        $result.candidate_freeze_tree = $preflightRecord.candidate_freeze_tree
        $result.activation_parent = $activationParent
        $result.runner_git_blob_sha = $runnerCommitBlob
        $result.runner_file_sha256 = $runnerFileSha256
        $result.frozen_p2_head = $script:P2Head
        $result.frozen_p2_tree = $frozenP2Tree
        $result.frozen_p1_head = $script:P1Head
        $result.frozen_p1_tree = $frozenP1Tree
        $frozenP0Tree = Get-RequiredGitScalar -Arguments @('rev-parse', "$($script:S1R4P0Head)^{tree}") -WorkingDirectory $repoRoot -Label 'Resolve frozen S1R4-P0 Tree'
        if ($frozenP0Tree -cne $script:S1R4P0Tree) { throw "Frozen S1R4-P0 identity mismatch: expected_tree=$($script:S1R4P0Tree) actual_tree=$frozenP0Tree" }
        Assert-GitNoOutputSuccess -Arguments @('merge-base', '--is-ancestor', $script:S1R4P0Head, 'HEAD') -WorkingDirectory $repoRoot -Label 'Check frozen S1R4-P0 ancestry'
        $frozenS1R3Tree = Get-RequiredGitScalar -Arguments @('rev-parse', "$($script:S1R3Head)^{tree}") -WorkingDirectory $repoRoot -Label 'Resolve frozen S1R3 Tree'
        if ($frozenS1R3Tree -cne $script:S1R3Tree) { throw "Frozen S1R3 identity mismatch: expected_tree=$($script:S1R3Tree) actual_tree=$frozenS1R3Tree" }
        Assert-GitNoOutputSuccess -Arguments @('merge-base', '--is-ancestor', $script:S1R3Head, 'HEAD') -WorkingDirectory $repoRoot -Label 'Check frozen S1R3 ancestry'
        $frozenS1R2Tree = Get-RequiredGitScalar -Arguments @('rev-parse', "$($script:S1R2Head)^{tree}") -WorkingDirectory $repoRoot -Label 'Resolve frozen S1R2 Tree'
        if ($frozenS1R2Tree -cne $script:S1R2Tree) { throw "Frozen S1R2 identity mismatch: expected_tree=$($script:S1R2Tree) actual_tree=$frozenS1R2Tree" }
        Assert-GitNoOutputSuccess -Arguments @('merge-base', '--is-ancestor', $script:S1R2Head, 'HEAD') -WorkingDirectory $repoRoot -Label 'Check frozen S1R2 ancestry'
        $frozenS1RTree = Get-RequiredGitScalar -Arguments @('rev-parse', "$($script:S1RHead)^{tree}") -WorkingDirectory $repoRoot -Label 'Resolve frozen S1R Tree'
        if ($frozenS1RTree -cne $script:S1RTree) { throw "Frozen S1R identity mismatch: expected_tree=$($script:S1RTree) actual_tree=$frozenS1RTree" }
        Assert-GitNoOutputSuccess -Arguments @('merge-base', '--is-ancestor', $script:S1RHead, 'HEAD') -WorkingDirectory $repoRoot -Label 'Check frozen S1R ancestry'
        Assert-GitNoOutputSuccess -Arguments @('merge-base', '--is-ancestor', $script:S1Head, 'HEAD') -WorkingDirectory $repoRoot -Label 'Check frozen S1 ancestry'
        $probeBlob = Get-RequiredGitScalar -Arguments @('rev-parse', "$($script:S0Head):$($script:ProbePath)") -WorkingDirectory $repoRoot -Label 'Resolve frozen S0 Probe blob'
        $actualProbeBlob = Get-RequiredGitScalar -Arguments @('hash-object', '--', $script:ProbePath) -WorkingDirectory $repoRoot -Label 'Resolve current S0 Probe blob'
        if ($probeBlob -cne $actualProbeBlob) { throw 'Frozen S0 Probe blob changed.' }
        $result.tested_head = $head
        $result.tested_tree = $tree
        $result.frozen_probe_blob = $probeBlob
    } finally { Pop-Location }

    $initialRegistry = Get-SyntheticRegistrySnapshot
    $script:SyntheticKeyChecked = $true
    $script:CanDeleteSyntheticKey = (-not $initialRegistry.exists)
    $script:RegistrySnapshots.Add([ordered]@{ stage = 'initial'; snapshot = $initialRegistry })
    Add-Assertion -Name 'synthetic_hkcu_key_initially_absent' -Passed $script:CanDeleteSyntheticKey `
        -Details "key=HKCU\$($script:RegistrySubKey) exists=$($initialRegistry.exists); existing state is never overwritten or deleted."
    $ownerToken = Get-SyntheticRegistryOwnerToken
    Write-Utf8Text -Path $syntheticKeyOwnerMarker -Text "key=HKCU\$($script:RegistrySubKey)`nowner_token=$ownerToken`n"
    $registryBase = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::CurrentUser,
        [Microsoft.Win32.RegistryView]::Registry64
    )
    try {
        $ownedKey = $registryBase.CreateSubKey($script:RegistrySubKey, $true)
        if ($null -eq $ownedKey) { throw "Could not create the isolated ownership key: HKCU\$($script:RegistrySubKey)" }
        try { $ownedKey.SetValue($script:RegistryOwnerValueName, $ownerToken, [Microsoft.Win32.RegistryValueKind]::String) }
        finally { $ownedKey.Dispose() }
    } finally { $registryBase.Dispose() }
    $afterOwnerCreate = Get-SyntheticRegistrySnapshot
    $script:RegistrySnapshots.Add([ordered]@{ stage = 'owner_created_before_probe'; snapshot = $afterOwnerCreate })
    Add-Assertion -Name 'synthetic_hkcu_owner_token_is_current_run' `
        -Passed ($afterOwnerCreate.exists -and $afterOwnerCreate.owner_kind -eq 'String' -and $afterOwnerCreate.owner_value -ceq $ownerToken) `
        -Details "expected_owner_kind=String expected_owner_token=$ownerToken actual_owner_kind=$($afterOwnerCreate.owner_kind) actual_owner_token=$($afterOwnerCreate.owner_value)"

    Download-And-VerifyPinnedZip
    Expand-VerifiedZipSafely
    Add-Assertion -Name 'two_pinned_archive_members_verified_before_extract' `
        -Passed (($script:Acquisition.validated_archive_members.Count -eq 2) -and
            (@($script:Acquisition.validated_archive_members | Where-Object { $_.path -eq $script:LauncherRelativePath -and $_.length_bytes -eq $script:LauncherLength -and $_.sha256 -eq $script:LauncherSha256 }).Count -eq 1) -and
            (@($script:Acquisition.validated_archive_members | Where-Object { $_.path -eq $script:CompilerRelativePath -and $_.length_bytes -eq $script:CompilerLength -and $_.sha256 -eq $script:CompilerSha256 }).Count -eq 1)) `
        -Details "members=$($script:Acquisition.validated_archive_members | ConvertTo-Json -Compress -Depth 5)"
    Add-Assertion -Name 'two_pinned_extracted_members_verified_under_task_root' `
        -Passed (($script:Acquisition.extracted_members.Count -eq 2) -and
            (@($script:Acquisition.extracted_members | Where-Object { $_.path -eq $script:LauncherRelativePath -and $_.length_bytes -eq $script:LauncherLength -and $_.sha256 -eq $script:LauncherSha256 }).Count -eq 1) -and
            (@($script:Acquisition.extracted_members | Where-Object { $_.path -eq $script:CompilerRelativePath -and $_.length_bytes -eq $script:CompilerLength -and $_.sha256 -eq $script:CompilerSha256 }).Count -eq 1)) `
        -Details "members=$($script:Acquisition.extracted_members | ConvertTo-Json -Compress -Depth 5)"

    $relativeLauncher = $script:LauncherRelativePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
    $relativeCompiler = $script:CompilerRelativePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
    $makensis = Resolve-Issue380S1R4Path -TaskRoot $extractionRoot `
        -CandidatePath (Join-Path $extractionRoot $relativeLauncher) -Kind Executable
    $binCompiler = Resolve-Issue380S1R4Path -TaskRoot $extractionRoot `
        -CandidatePath (Join-Path $extractionRoot $relativeCompiler) -Kind Executable
    $makensisHash = Get-FileSha256 $makensis
    $binCompilerHash = Get-FileSha256 $binCompiler
    if ($makensisHash -ne $script:LauncherSha256 -or (Get-Item -LiteralPath $makensis).Length -ne $script:LauncherLength) {
        throw "Top-level launcher identity changed before execution: path=$makensis sha256=$makensisHash"
    }
    if ($binCompilerHash -ne $script:CompilerSha256 -or (Get-Item -LiteralPath $binCompiler).Length -ne $script:CompilerLength) {
        throw "Bin compiler identity changed before execution: path=$binCompiler sha256=$binCompilerHash"
    }
    $identityRoot = Join-Path $outputFullPath 'compiler'
    New-Item -ItemType Directory -Path $identityRoot -ErrorAction Stop | Out-Null
    Add-ExecutableEvidence -SourcePath $makensis -EvidencePath (Join-Path $identityRoot 'makensis-launcher.exe') -Role 'pinned_forwarding_launcher'
    Add-ExecutableEvidence -SourcePath $binCompiler -EvidencePath (Join-Path $identityRoot 'Bin-makensis.exe') -Role 'pinned_underlying_compiler'
    $script:Compiler = [ordered]@{
        executed_path = $makensis
        executed_role = 'top-level forwarding launcher; the Bin compiler is not invoked directly'
        launcher = [ordered]@{ path = $makensis; relative_path = $script:LauncherRelativePath; length_bytes = $script:LauncherLength; sha256 = $makensisHash }
        underlying_compiler = [ordered]@{ path = $binCompiler; relative_path = $script:CompilerRelativePath; length_bytes = $script:CompilerLength; sha256 = $binCompilerHash }
        version_output = $null
        hdrinfo_output = $null
        version_exit = $null
        hdrinfo_exit = $null
        x86 = $false
        unicode = $false
        source_zip_sha256 = $script:Acquisition.downloaded_sha256
    }
    $versionResult = Invoke-CapturedProcess -FilePath $makensis -ArgumentList @('-VERSION') -WorkingDirectory $taskRoot `
        -StdoutPath (Join-Path $identityRoot 'makensis-version.stdout.txt') `
        -StderrPath (Join-Path $identityRoot 'makensis-version.stderr.txt')
    $versionText = ($versionResult.stdout_utf8 + $versionResult.stderr_utf8).Trim()
    $script:Compiler.version_output = $versionText
    $script:Compiler.version_exit = $versionResult.exit_code
    Add-Assertion -Name 'makensis_version_is_exactly_3_11' `
        -Passed ((-not $versionResult.timed_out) -and ($versionResult.exit_code -eq 0) -and ($versionText -match '(?m)^\s*v?3\.11(?:\s|$)')) `
        -Details "path=$makensis version=$versionText sha256=$makensisHash"

    $headerResult = Invoke-CapturedProcess -FilePath $makensis -ArgumentList @('-HDRINFO') -WorkingDirectory $taskRoot `
        -StdoutPath (Join-Path $identityRoot 'makensis-hdrinfo.stdout.txt') `
        -StderrPath (Join-Path $identityRoot 'makensis-hdrinfo.stderr.txt')
    $headerText = $headerResult.stdout_utf8 + $headerResult.stderr_utf8
    $script:Compiler.hdrinfo_output = $headerText
    $script:Compiler.hdrinfo_exit = $headerResult.exit_code
    $script:Compiler.x86 = ($headerText -match 'NSIS_CPU=x86')
    $script:Compiler.unicode = ($headerText -match 'NSIS_UNICODE')
    Add-Assertion -Name 'makensis_is_x86_unicode' `
        -Passed ((-not $headerResult.timed_out) -and ($headerResult.exit_code -eq 0) -and ($headerText -match 'NSIS_CPU=x86') -and ($headerText -match 'NSIS_UNICODE')) `
        -Details 'NSIS -HDRINFO must identify NSIS_CPU=x86 and NSIS_UNICODE.'

    $probePath = Join-Path $repoRoot $script:ProbePath
    foreach ($traceMode in @('file', 'detail')) {
        $modeTaskRoot = Join-Path (Join-Path $taskRoot 'compiled') $traceMode
        $modeEvidenceRoot = Join-Path $outputFullPath $traceMode
        New-Item -ItemType Directory -Path $modeTaskRoot -Force -ErrorAction Stop | Out-Null
        New-Item -ItemType Directory -Path $modeEvidenceRoot -Force -ErrorAction Stop | Out-Null

        $stageExe = Compile-Probe -Makensis $makensis -Probe $probePath -TraceMode $traceMode -RunMode 'stage' `
            -OutputName 'stage.exe' -WorkingDirectory $modeTaskRoot -EvidenceLogStem "$traceMode-compile-stage"
        $abortExe = Compile-Probe -Makensis $makensis -Probe $probePath -TraceMode $traceMode -RunMode 'abort' `
            -OutputName 'installer-abort.exe' -WorkingDirectory $modeTaskRoot -EvidenceLogStem "$traceMode-compile-installer-abort"

        $stageProcess = Invoke-CapturedProcess -FilePath $stageExe -ArgumentList @('/S') -WorkingDirectory $modeTaskRoot `
            -StdoutPath (Join-Path $modeEvidenceRoot 'stage.stdout.txt') -StderrPath (Join-Path $modeEvidenceRoot 'stage.stderr.txt')
        Assert-ProcessExit -Name "${traceMode}_stage_exit_zero" -Result $stageProcess -ExpectedCode 0
        $generatedUninstaller = Join-Path $modeTaskRoot 'generated-uninstaller.exe'
        Add-ExecutableEvidence -SourcePath $generatedUninstaller `
            -EvidencePath (Join-Path $modeEvidenceRoot 'generated-uninstaller.exe') -Role "generated_uninstaller_$traceMode"

        $afterStage = Get-SyntheticRegistrySnapshot
        $script:RegistrySnapshots.Add([ordered]@{ stage = "${traceMode}_after_stage"; snapshot = $afterStage })
        Add-Assertion -Name "${traceMode}_stage_wrote_synthetic_dword" `
            -Passed (($afterStage.exists) -and ($afterStage.kind -eq 'DWord') -and ([int]$afterStage.value -eq $script:ActualValue) -and ($afterStage.owner_kind -eq 'String') -and ($afterStage.owner_value -ceq $ownerToken)) `
            -Details "expected_kind=DWord expected_value=$($script:ActualValue) expected_owner_kind=String expected_owner=$ownerToken actual_kind=$($afterStage.kind) actual_value=$($afterStage.value) actual_owner_kind=$($afterStage.owner_kind) actual_owner=$($afterStage.owner_value)"

        $installerLog = Join-Path $modeEvidenceRoot 'installer-runtime.log'
        $installerArguments = @('/S')
        if ($traceMode -eq 'detail') { $installerArguments += "/LOG=$installerLog" }
        $installerAbort = Invoke-CapturedProcess -FilePath $abortExe -ArgumentList $installerArguments `
            -WorkingDirectory $modeTaskRoot -StdoutPath (Join-Path $modeEvidenceRoot 'installer-abort.stdout.txt') `
            -StderrPath (Join-Path $modeEvidenceRoot 'installer-abort.stderr.txt')
        if ($traceMode -eq 'file') {
            Assert-ProcessExit -Name 'file_installer_abort_expected_exit_37' -Result $installerAbort -ExpectedCode $script:ExpectedInstallerExitCode
        }

        $uninstallerLog = Join-Path $modeEvidenceRoot 'uninstaller-runtime.log'
        $absoluteInstallDirectory = Resolve-Issue380S1R4Path -TaskRoot $taskRoot `
            -CandidatePath $modeTaskRoot -Kind WorkingDirectory
        $absoluteUninstallerPath = Resolve-Issue380S1R4Path -TaskRoot $modeTaskRoot `
            -CandidatePath $generatedUninstaller -Kind Executable
        $uninstallerArguments = [System.Collections.Generic.List[string]]::new()
        $uninstallerArguments.Add('/S')
        if ($traceMode -eq 'detail') { $uninstallerArguments.Add("/LOG=$uninstallerLog") }
        $uninstallerArguments.Add("_?=$absoluteInstallDirectory")
        Add-Assertion -Name "${traceMode}_uninstaller_no_copy_argument_is_final" `
            -Passed (($uninstallerArguments.Count -ge 2) -and $uninstallerArguments[$uninstallerArguments.Count - 1].Equals("_?=$absoluteInstallDirectory", [System.StringComparison]::OrdinalIgnoreCase)) `
            -Details "process_image=$absoluteUninstallerPath controlled_absolute_directory=$absoluteInstallDirectory argument_list=$($uninstallerArguments -join ' | ')"
        $uninstallerAbort = Invoke-CapturedProcess -FilePath $absoluteUninstallerPath -ArgumentList @($uninstallerArguments.ToArray()) `
            -WorkingDirectory $modeTaskRoot -StdoutPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stdout.txt') `
            -StderrPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stderr.txt')
        if ($traceMode -eq 'file') {
            Assert-ProcessExit -Name 'file_uninstaller_abort_expected_exit_42' -Result $uninstallerAbort -ExpectedCode $script:ExpectedUninstallerExitCode
        }

        $afterAborts = Get-SyntheticRegistrySnapshot
        $script:RegistrySnapshots.Add([ordered]@{ stage = "${traceMode}_after_aborts"; snapshot = $afterAborts })
        Add-Assertion -Name "${traceMode}_abort_paths_preserved_synthetic_dword" `
            -Passed (($afterAborts.exists) -and ($afterAborts.kind -eq 'DWord') -and ([int]$afterAborts.value -eq $script:ActualValue) -and ($afterAborts.owner_kind -eq 'String') -and ($afterAborts.owner_value -ceq $ownerToken)) `
            -Details "expected_kind=DWord expected_value=$($script:ActualValue) expected_owner_kind=String expected_owner=$ownerToken actual_kind=$($afterAborts.kind) actual_value=$($afterAborts.value) actual_owner_kind=$($afterAborts.owner_kind) actual_owner=$($afterAborts.owner_value)"

        $detailTracePath = Join-Path $modeTaskRoot 'direct-evidence.log'
        $evidenceTracePath = Join-Path $modeEvidenceRoot 'direct-evidence.log'
        if (Test-Path -LiteralPath $detailTracePath -PathType Leaf) {
            Copy-Item -LiteralPath $detailTracePath -Destination $evidenceTracePath -Force
        }
        $directLines = @()
        if (Test-Path -LiteralPath $evidenceTracePath -PathType Leaf) { $directLines = @(Get-Content -LiteralPath $evidenceTracePath) }
        $script:LogComparison.Add([ordered]@{
            mode = $traceMode
            role = if ($traceMode -eq 'file') { 'primary direct FileWrite persistence proof; no /LOG supplied to Abort processes' } else { 'optional DetailPrint and /LOG comparison; /LOG absence is informational' }
            installer_abort_process_id = $installerAbort.process_id
            installer_abort_image_path = $installerAbort.process_image_path
            installer_abort_argument_list = $installerAbort.argument_list
            installer_abort_exit_code = $installerAbort.exit_code
            installer_abort_timed_out = $installerAbort.timed_out
            installer_abort_expected_exit_code = $script:ExpectedInstallerExitCode
            uninstaller_abort_process_id = $uninstallerAbort.process_id
            uninstaller_abort_image_path = $uninstallerAbort.process_image_path
            uninstaller_abort_argument_list = $uninstallerAbort.argument_list
            uninstaller_abort_exit_code = $uninstallerAbort.exit_code
            uninstaller_abort_timed_out = $uninstallerAbort.timed_out
            uninstaller_abort_expected_exit_code = $script:ExpectedUninstallerExitCode
            installer_log_exists = (Test-Path -LiteralPath $installerLog -PathType Leaf)
            installer_log_sha256 = if (Test-Path -LiteralPath $installerLog -PathType Leaf) { Get-FileSha256 $installerLog } else { $null }
            installer_log_contains_failure = if (Test-Path -LiteralPath $installerLog -PathType Leaf) { (Get-Content -Raw -LiteralPath $installerLog).Contains('first_failure=EstimatedSize') } else { $false }
            uninstaller_log_exists = (Test-Path -LiteralPath $uninstallerLog -PathType Leaf)
            uninstaller_log_sha256 = if (Test-Path -LiteralPath $uninstallerLog -PathType Leaf) { Get-FileSha256 $uninstallerLog } else { $null }
            uninstaller_log_contains_failure = if (Test-Path -LiteralPath $uninstallerLog -PathType Leaf) { (Get-Content -Raw -LiteralPath $uninstallerLog).Contains('uninstaller_first_failure=EstimatedSize') } else { $false }
            direct_file_trace_exists = (Test-Path -LiteralPath $evidenceTracePath -PathType Leaf)
            direct_file_trace_lines = $directLines
        })
        Add-Assertion -Name "${traceMode}_abort_stdout_and_stderr_saved" `
            -Passed ((Test-Path -LiteralPath (Join-Path $modeEvidenceRoot 'installer-abort.stdout.txt')) -and
                (Test-Path -LiteralPath (Join-Path $modeEvidenceRoot 'installer-abort.stderr.txt')) -and
                (Test-Path -LiteralPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stdout.txt')) -and
                (Test-Path -LiteralPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stderr.txt'))) `
            -Details 'Both Abort process stdout/stderr files must be retained.'
        if ($traceMode -eq 'file') {
            Assert-PrimaryFileWriteEvidence -TracePath $evidenceTracePath
        }
    }

    $compilerOutputs = @($script:Executables | Where-Object { $_.role -like 'compiler_output_*' })
    $generatedUninstallers = @($script:Executables | Where-Object { $_.role -like 'generated_uninstaller_*' })
    Add-Assertion -Name 'all_four_compiler_outputs_and_two_uninstallers_hashed' `
        -Passed (($compilerOutputs.Count -eq 4) -and ($generatedUninstallers.Count -eq 2)) `
        -Details "compiler_outputs=$($compilerOutputs.Count) generated_uninstallers=$($generatedUninstallers.Count)"

    $preCleanupProcesses = @(Get-TaskOwnedProcessSnapshot)
    $result.cleanup.task_owned_processes = if ($preCleanupProcesses.Count -eq 0) { 'ALL_TASK_PATH_PROCESSES_EXITED' } else { $preCleanupProcesses }
    Add-Assertion -Name 'no_task_owned_processes_before_cleanup' -Passed ($preCleanupProcesses.Count -eq 0) `
        -Details "remaining=$($preCleanupProcesses | ConvertTo-Json -Compress -Depth 4)"
} catch {
    $script:Failure = $_.Exception.ToString()
    if ($script:Acquisition.sha256_verified_before_extract -and $script:Acquisition.extraction_status -eq 'NOT_STARTED') {
        $script:Acquisition.extraction_status = 'REJECTED_OR_FAILED'
        $script:Acquisition.failure = $script:Failure
    } elseif (-not $script:Acquisition.sha256_verified_before_extract -and $null -eq $script:Acquisition.failure) {
        $script:Acquisition.failure = $script:Failure
    }
} finally {
    try {
        foreach ($mode in @('file', 'detail')) {
            $modeTaskRoot = Join-Path (Join-Path $taskRoot 'compiled') $mode
            $modeEvidenceRoot = Join-Path $outputFullPath $mode
            if (Test-Path -LiteralPath $modeTaskRoot -PathType Container) {
                New-Item -ItemType Directory -Path $modeEvidenceRoot -Force -ErrorAction Stop | Out-Null
                foreach ($compiledName in @('stage.exe', 'installer-abort.exe')) {
                    $sourceExe = Join-Path $modeTaskRoot $compiledName
                    $evidenceExe = Join-Path $modeEvidenceRoot $compiledName
                    if ((Test-Path -LiteralPath $sourceExe -PathType Leaf) -and -not (Test-Path -LiteralPath $evidenceExe -PathType Leaf)) {
                        Add-ExecutableEvidence -SourcePath $sourceExe -EvidencePath $evidenceExe `
                            -Role "compiler_output_${mode}_$([System.IO.Path]::GetFileNameWithoutExtension($compiledName))" -AllowEmpty $true
                    }
                }
                $sourceUninstaller = Join-Path $modeTaskRoot 'generated-uninstaller.exe'
                $evidenceUninstaller = Join-Path $modeEvidenceRoot 'generated-uninstaller.exe'
                if ((Test-Path -LiteralPath $sourceUninstaller -PathType Leaf) -and -not (Test-Path -LiteralPath $evidenceUninstaller -PathType Leaf)) {
                    Add-ExecutableEvidence -SourcePath $sourceUninstaller -EvidencePath $evidenceUninstaller -Role "generated_uninstaller_$mode"
                }
            }
            $sourceTrace = Join-Path (Join-Path (Join-Path $taskRoot 'compiled') $mode) 'direct-evidence.log'
            if (Test-Path -LiteralPath $sourceTrace -PathType Leaf) {
                Copy-Item -LiteralPath $sourceTrace -Destination (Join-Path $modeEvidenceRoot 'direct-evidence.log') -Force
            }
        }
    } catch {
        $traceCopyFailure = $_.Exception.ToString()
        if ($null -eq $script:Failure) { $script:Failure = "Direct trace evidence preservation failed: $traceCopyFailure" }
    }

    try {
        $remainingProcesses = @(Wait-ForTaskOwnedProcessesToExit -TimeoutSeconds 2)
        if ($remainingProcesses.Count -eq 0) {
            $result.cleanup.task_owned_processes = 'ALL_TASK_PATH_PROCESSES_EXITED'
        } else {
            $result.cleanup.task_owned_processes = $remainingProcesses
            if ($null -eq $script:Failure) { $script:Failure = "Task-owned processes remain before cleanup: $($remainingProcesses | ConvertTo-Json -Compress -Depth 4)" }
        }
    } catch {
        $processCleanupFailure = $_.Exception.ToString()
        $result.cleanup.task_owned_processes = "CHECK_FAILED: $processCleanupFailure"
        if ($null -eq $script:Failure) { $script:Failure = "Task-owned process check failed: $processCleanupFailure" }
    }

    try {
        if ($script:CanDeleteSyntheticKey) {
            if ($result.cleanup.task_owned_processes -ne 'ALL_TASK_PATH_PROCESSES_EXITED') {
                $result.cleanup.registry_key = 'PRESERVED_BECAUSE_TASK_PROCESS_STATE_IS_NOT_CLEAN'
                throw "Preserving HKCU test key because task-owned process exit is not proven: $($result.cleanup.task_owned_processes | ConvertTo-Json -Compress -Depth 4)"
            }
            $currentRegistry = Get-SyntheticRegistrySnapshot
            if ($currentRegistry.exists) {
                $ownerToken = Get-SyntheticRegistryOwnerToken
                $expectedMarker = "key=HKCU\$($script:RegistrySubKey)`nowner_token=$ownerToken`n"
                if (-not (Test-Path -LiteralPath $syntheticKeyOwnerMarker -PathType Leaf) -or
                    [System.IO.File]::ReadAllText($syntheticKeyOwnerMarker) -cne $expectedMarker) {
                    $result.cleanup.registry_key = 'PRESERVED_BECAUSE_OUTPUT_OWNERSHIP_MARKER_MISMATCHED'
                    throw 'Preserving HKCU key because the run-owned Registry marker is missing or mismatched.'
                }
                $allowedValueNames = @($script:RegistryOwnerValueName, $script:RegistryValueName)
                $unexpectedValues = @($currentRegistry.value_names | Where-Object { $_ -notin $allowedValueNames })
                $valueStateIsExpected = ($currentRegistry.value_names -contains $script:RegistryOwnerValueName) -and
                    ($currentRegistry.owner_kind -eq 'String') -and
                    ($currentRegistry.owner_value -ceq $ownerToken) -and
                    ($currentRegistry.subkey_names.Count -eq 0) -and
                    ($unexpectedValues.Count -eq 0) -and
                    (($currentRegistry.kind -eq 'MISSING') -or
                        ($currentRegistry.kind -eq 'DWord' -and [int]$currentRegistry.value -eq $script:ActualValue))
                if (-not $valueStateIsExpected) {
                    $result.cleanup.registry_key = 'PRESERVED_BECAUSE_REGISTRY_STATE_CHANGED'
                    throw "Preserving HKCU key because its values differ from this run's exact synthetic state: $($currentRegistry | ConvertTo-Json -Compress -Depth 5)"
                }
            }
            $cleanupBase = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
                [Microsoft.Win32.RegistryHive]::CurrentUser,
                [Microsoft.Win32.RegistryView]::Registry64
            )
            try {
                $cleanupBase.DeleteSubKeyTree($script:RegistrySubKey, $false)
                $remaining = Get-SyntheticRegistrySnapshot
                if ($remaining.exists) { throw "Synthetic HKCU key remains after cleanup: $($script:RegistrySubKey)" }
                $result.cleanup.registry_key = 'REMOVED_AND_VERIFIED_ABSENT'
            } finally { $cleanupBase.Dispose() }
        } elseif ($script:SyntheticKeyChecked) {
            $result.cleanup.registry_key = 'PREEXISTING_KEY_PRESERVED'
        } else {
            $result.cleanup.registry_key = 'NOT_CHECKED'
        }
    } catch {
        $cleanupFailure = $_.Exception.ToString()
        $result.cleanup.registry_key = "FAILED: $cleanupFailure"
        if ($null -eq $script:Failure) { $script:Failure = "Synthetic HKCU cleanup failed: $cleanupFailure" }
    }

    try {
        if (-not (Test-Path -LiteralPath $taskRoot)) {
            $result.cleanup.task_temp = 'NOT_CREATED_OR_ALREADY_ABSENT'
        } else {
            if ($result.cleanup.task_owned_processes -ne 'ALL_TASK_PATH_PROCESSES_EXITED') {
                $result.cleanup.task_temp = 'PRESERVED_BECAUSE_TASK_PROCESS_STATE_IS_NOT_CLEAN'
                throw 'Preserving task directory because task-owned process exit is not proven.'
            }
            $null = Resolve-Issue380S1R4Path -TaskRoot $runnerTempFullPath -CandidatePath $taskRoot -Kind ExistingDirectory
            $expectedTaskMarker = "repository=$env:GITHUB_REPOSITORY`nref=$env:GITHUB_REF`nrun_id=$env:GITHUB_RUN_ID`nattempt=$env:GITHUB_RUN_ATTEMPT`nhead=$env:GITHUB_SHA`nroot=$taskRoot`n"
            if (-not (Test-Path -LiteralPath $taskMarker -PathType Leaf)) { throw 'Task directory ownership marker is missing; refusing recursive cleanup.' }
            $actualTaskMarker = [System.IO.File]::ReadAllText($taskMarker)
            if ($actualTaskMarker -cne $expectedTaskMarker) { throw 'Task directory ownership marker does not match this exact run; refusing recursive cleanup.' }
            $reparseEntries = @(Get-ChildItem -LiteralPath $taskRoot -Force -Recurse -ErrorAction Stop |
                Where-Object { ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 })
            if ($reparseEntries.Count -ne 0) { throw "Task directory contains reparse points; refusing recursive cleanup: $($reparseEntries.FullName -join ', ')" }
            Remove-Item -LiteralPath $taskRoot -Recurse -Force -ErrorAction Stop
            if (Test-Path -LiteralPath $taskRoot) { throw 'Owned task directory remains after cleanup.' }
            $result.cleanup.task_temp = 'REMOVED_AND_VERIFIED_ABSENT'
        }
    } catch {
        $taskCleanupFailure = $_.Exception.ToString()
        if ($result.cleanup.task_temp -notlike 'PRESERVED_*') { $result.cleanup.task_temp = "FAILED: $taskCleanupFailure" }
        if ($null -eq $script:Failure) { $script:Failure = "Task temp cleanup failed: $taskCleanupFailure" }
    }

    if ($null -eq $script:Failure -and $result.cleanup.registry_key -ne 'REMOVED_AND_VERIFIED_ABSENT') {
        $script:Failure = "Successful run requires verified synthetic Registry cleanup; actual=$($result.cleanup.registry_key)"
    }
    if ($null -eq $script:Failure -and $result.cleanup.task_temp -ne 'REMOVED_AND_VERIFIED_ABSENT') {
        $script:Failure = "Successful run requires verified task temp cleanup; actual=$($result.cleanup.task_temp)"
    }

    $result.acquisition = $script:Acquisition
    $result.compiler = $script:Compiler
    $result.executables = @($script:Executables.ToArray())
    $result.processes = @($script:Processes.ToArray())
    $result.registry_snapshots = @($script:RegistrySnapshots.ToArray())
    $result.log_comparison = @($script:LogComparison.ToArray())
    $result.git_invocations = @($script:GitInvocations.ToArray())
    $result.assertions = @($script:Assertions.ToArray())
    try { Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition } catch { }
    if ($null -eq $script:Failure) {
        $result.status = 'PASS'
    } else {
        $result.status = 'FAIL'
        $result.failure = $script:Failure
        try { Write-Utf8Text -Path (Join-Path $outputFullPath 'failure.txt') -Text ($script:Failure + "`n") } catch { }
    }
    try {
        Write-Utf8Json -Path (Join-Path $outputFullPath 'results.json') -Value $result
        $sumLines = [System.Collections.Generic.List[string]]::new()
        foreach ($file in (Get-ChildItem -LiteralPath $outputFullPath -Recurse -File | Sort-Object FullName)) {
            if ($file.Name -eq 'SHA256SUMS') { continue }
            $relative = $file.FullName.Substring($outputFullPath.Length).TrimStart('\', '/').Replace('\', '/')
            $sumLines.Add("$(Get-FileSha256 $file.FullName)  $relative")
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
