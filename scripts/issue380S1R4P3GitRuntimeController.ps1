[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$HarnessPath,

    [Parameter(Mandatory = $true)]
    [string]$EvidenceRoot,

    [Parameter(Mandatory = $true)]
    [string]$TaskRoot,

    [ValidateRange(1, 540000)]
    [int]$WorkerDeadlineMilliseconds = 420000
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$evidence = [System.IO.Path]::GetFullPath($EvidenceRoot)
$task = [System.IO.Path]::GetFullPath($TaskRoot)
$recordPath = Join-Path $evidence 'controller.json'
$stdoutPath = Join-Path $evidence 'controller.stdout.raw.txt'
$stderrPath = Join-Path $evidence 'controller.stderr.raw.txt'
$stdoutTask = $null
$stderrTask = $null
$script:OutputPersisted = $false
$process = $null
$stdoutText = [string]::Empty
$stderrText = [string]::Empty
$record = [ordered]@{
    status = 'NOT_STARTED'
    worker_path = [System.IO.Path]::GetFullPath($HarnessPath)
    worker_image_path = Join-Path $PSHOME 'pwsh.exe'
    worker_arguments = @()
    worker_process_id = $null
    worker_exit_code = $null
    worker_timed_out = $false
    worker_kill_attempted = $false
    worker_kill_succeeded = $false
    worker_exit_confirmed = $false
    worker_wait_ms = $WorkerDeadlineMilliseconds
    worker_termination_wait_ms = 15000
    stdout_task_status = 'NOT_STARTED'
    stderr_task_status = 'NOT_STARTED'
    stdout_complete = $false
    stderr_complete = $false
    output_wait_ms = 15000
    post_close_wait_ms = 1000
    output_wait_returned = $null
    stream_close_attempted = $false
    stream_close_error = $null
    cleanup_completed = $false
    started_utc = [DateTime]::UtcNow.ToString('o')
    ended_utc = $null
    elapsed_ms = $null
    kill_elapsed_ms = $null
    output_elapsed_ms = $null
    owned_process_cleanup_status = 'NOT_STARTED'
    owned_process_cleanup = @()
    owned_processes_remaining = @()
    errors = [System.Collections.Generic.List[string]]::new()
}
$wholeTimer = [System.Diagnostics.Stopwatch]::StartNew()

function Write-ControllerJson([string]$Path, [object]$Value) {
    [System.IO.File]::WriteAllText(
        $Path,
        (ConvertTo-Json -InputObject $Value -Depth 30) + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Complete-ControllerOutput([bool]$ForceClose) {
    $tasks = [System.Collections.Generic.List[System.Threading.Tasks.Task]]::new()
    if ($null -ne $stdoutTask) { $tasks.Add($stdoutTask) }
    if ($null -ne $stderrTask) { $tasks.Add($stderrTask) }
    if ($tasks.Count -gt 0) {
        try {
            $record.output_wait_returned = [System.Threading.Tasks.Task]::WaitAll($tasks.ToArray(), $record.output_wait_ms)
        } catch { $record.errors.Add("Controller output wait failed: $($_.Exception.ToString())") }
        $record.stdout_task_status = if ($null -ne $stdoutTask) { $stdoutTask.Status.ToString() } else { 'NOT_STARTED' }
        $record.stderr_task_status = if ($null -ne $stderrTask) { $stderrTask.Status.ToString() } else { 'NOT_STARTED' }
        $record.stdout_complete = ($null -ne $stdoutTask -and $stdoutTask.IsCompletedSuccessfully)
        $record.stderr_complete = ($null -ne $stderrTask -and $stderrTask.IsCompletedSuccessfully)
        if ($ForceClose -or -not $record.stdout_complete -or -not $record.stderr_complete) {
            $record.stream_close_attempted = $true
            if ($null -ne $process) {
                try { $process.StandardOutput.Close() } catch { $record.stream_close_error = "stdout close: $($_.Exception.ToString())" }
                try { $process.StandardError.Close() } catch {
                    if ($null -eq $record.stream_close_error) { $record.stream_close_error = "stderr close: $($_.Exception.ToString())" }
                    else { $record.stream_close_error += "`nstderr close: $($_.Exception.ToString())" }
                }
            }
            try { [void][System.Threading.Tasks.Task]::WaitAll($tasks.ToArray(), $record.post_close_wait_ms) }
            catch { $record.errors.Add("Controller post-close output wait failed: $($_.Exception.ToString())") }
            $record.stdout_task_status = if ($null -ne $stdoutTask) { $stdoutTask.Status.ToString() } else { 'NOT_STARTED' }
            $record.stderr_task_status = if ($null -ne $stderrTask) { $stderrTask.Status.ToString() } else { 'NOT_STARTED' }
            $record.stdout_complete = ($null -ne $stdoutTask -and $stdoutTask.IsCompletedSuccessfully)
            $record.stderr_complete = ($null -ne $stderrTask -and $stderrTask.IsCompletedSuccessfully)
        }
    }

    foreach ($task in @($stdoutTask, $stderrTask)) {
        if ($null -eq $task) { continue }
        if ($task.IsCompletedSuccessfully) {
            if ([object]::ReferenceEquals($task, $stdoutTask)) { $stdoutText = [string]$task.GetAwaiter().GetResult() }
            else { $stderrText = [string]$task.GetAwaiter().GetResult() }
        } elseif ($task.IsFaulted -and $null -ne $task.Exception) {
            $record.errors.Add("Controller stream task faulted: $($task.Exception.ToString())")
        } elseif ($task.IsCanceled) {
            $record.errors.Add('Controller stream task was cancelled.')
        }
    }
    [System.IO.File]::WriteAllText($stdoutPath, $stdoutText, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($stderrPath, $stderrText, [System.Text.UTF8Encoding]::new($false))
    $script:OutputPersisted = $true
}

function Complete-OwnedRuntimeProcessCleanup {
    $startPath = Join-Path $evidence 'harness-start.json'
    if (-not (Test-Path -LiteralPath $startPath -PathType Leaf)) {
        $record.owned_process_cleanup_status = 'NO_HARNESS_START_RECORD'
        return
    }
    $preflightPath = Join-Path $evidence 'preflight.json'
    $workflowRunPath = Join-Path $evidence 'workflow-run.json'
    if (-not (Test-Path -LiteralPath $preflightPath -PathType Leaf) -or -not (Test-Path -LiteralPath $workflowRunPath -PathType Leaf)) {
        throw 'Cannot prove cleanup executable identities without preflight and workflow records.'
    }
    $start = Get-Content -LiteralPath $startPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    if ([string]$start.run_id -ne $env:GITHUB_RUN_ID -or [string]$start.run_attempt -ne $env:GITHUB_RUN_ATTEMPT -or [string]$start.triggering_sha -ne $env:GITHUB_SHA) {
        throw 'Harness start record does not belong to the current run.'
    }
    $preflight = Get-Content -LiteralPath $preflightPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    $workflowRun = Get-Content -LiteralPath $workflowRunPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    $allowedImages = @([System.IO.Path]::GetFullPath([string]$preflight.git_path)) +
        @($preflight.git_shell_paths | ForEach-Object { [System.IO.Path]::GetFullPath([string]$_) }) +
        @([System.IO.Path]::GetFullPath([string]$workflowRun.powershell_path))
    $nonce = [string]$start.nonce
    if ([string]::IsNullOrWhiteSpace($nonce) -or $nonce -notmatch '^[0-9a-f]{32}$') { throw 'Harness nonce is missing or malformed.' }
    $records = [System.Collections.Generic.List[object]]::new()
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $matching = @(Get-CimInstance -ClassName Win32_Process -ErrorAction Stop | Where-Object {
            [string]$_.CommandLine -like "*$nonce*" -and
            [string]$_.CommandLine -like '*issue380S1R4P3GitRuntimeChild.ps1*'
        })
        if ($matching.Count -eq 0) { break }
        foreach ($item in $matching) {
            $image = if ([string]::IsNullOrWhiteSpace([string]$item.ExecutablePath)) { '' } else { [System.IO.Path]::GetFullPath([string]$item.ExecutablePath) }
            $itemRecord = [ordered]@{
                process_id = [int]$item.ProcessId
                image_path = $image
                command_line = [string]$item.CommandLine
                creation_date = [string]$item.CreationDate
                kill_attempted = $false
                kill_succeeded = $false
                exit_confirmed = $false
                elapsed_ms = $null
                error = $null
            }
            if ($image -notin $allowedImages) {
                $itemRecord.error = 'Image path is not in the run-pinned executable allowlist; process was not touched.'
                $records.Add($itemRecord)
                continue
            }
            $current = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId=$($itemRecord.process_id)" -ErrorAction Stop
            if ($null -eq $current) {
                $itemRecord.exit_confirmed = $true
                $records.Add($itemRecord)
                continue
            }
            if ([string]$current.CommandLine -notlike "*$nonce*" -or
                [System.IO.Path]::GetFullPath([string]$current.ExecutablePath) -cne $image) {
                $itemRecord.error = 'Process identity changed before cleanup; PID was not used.'
                $records.Add($itemRecord)
                continue
            }
            $timer = [System.Diagnostics.Stopwatch]::StartNew()
            $itemRecord.kill_attempted = $true
            try {
                $ownedProcess = [System.Diagnostics.Process]::GetProcessById($itemRecord.process_id)
                try {
                    if (-not $ownedProcess.HasExited) {
                        $ownedProcess.Kill($true)
                        $itemRecord.kill_succeeded = $true
                        $itemRecord.exit_confirmed = $ownedProcess.WaitForExit(5000)
                    } else { $itemRecord.exit_confirmed = $true }
                } finally { $ownedProcess.Dispose() }
            } catch { $itemRecord.error = $_.Exception.ToString() }
            $timer.Stop()
            $itemRecord.elapsed_ms = [Math]::Round($timer.Elapsed.TotalMilliseconds, 3)
            $records.Add($itemRecord)
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)

    $remaining = @(Get-CimInstance -ClassName Win32_Process -ErrorAction Stop | Where-Object {
        [string]$_.CommandLine -like "*$nonce*" -and
        [string]$_.CommandLine -like '*issue380S1R4P3GitRuntimeChild.ps1*'
    } | ForEach-Object { [ordered]@{ process_id = [int]$_.ProcessId; image_path = [string]$_.ExecutablePath; command_line = [string]$_.CommandLine } })
    $record.owned_process_cleanup = @($records.ToArray())
    $record.owned_processes_remaining = $remaining
    if ($remaining.Count -eq 0 -and @($records | Where-Object { -not $_.exit_confirmed -or $null -ne $_.error }).Count -eq 0) {
        $record.owned_process_cleanup_status = 'ALL_NONCE_MATCHING_TEST_PROCESSES_EXITED'
    } else {
        $record.owned_process_cleanup_status = 'CLEANUP_UNCONFIRMED'
        $record.errors.Add('Owned test process cleanup could not be confirmed; ambiguous processes were not killed.')
    }
}

try {
    if ($PSVersionTable.PSVersion.Major -ne 7) { throw "PowerShell 7 is required; actual=$($PSVersionTable.PSVersion)" }
    if ($env:GITHUB_ACTIONS -ne 'true' -or $env:GITHUB_RUN_ATTEMPT -ne '1') { throw 'Controller is restricted to the first authorized Actions attempt.' }
    if (-not (Test-Path -LiteralPath $evidence -PathType Container) -or -not (Test-Path -LiteralPath $task -PathType Container)) {
        throw 'EvidenceRoot and TaskRoot must already exist.'
    }
    $ownerPath = Join-Path $evidence 'runtime-owner.json'
    $owner = Get-Content -LiteralPath $ownerPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    if ([string]$owner.run_id -ne $env:GITHUB_RUN_ID -or
        [string]$owner.run_attempt -ne $env:GITHUB_RUN_ATTEMPT -or
        [string]$owner.triggering_sha -ne $env:GITHUB_SHA) {
        throw 'Controller ownership marker does not match the current GitHub run.'
    }

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = [string]$record.worker_image_path
    $startInfo.WorkingDirectory = (Split-Path -Parent (Split-Path -Parent $record.worker_path))
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $arguments = @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-File', $record.worker_path,
        '-EvidenceRoot', $evidence, '-TaskRoot', $task
    )
    foreach ($argument in $arguments) { [void]$startInfo.ArgumentList.Add([string]$argument) }
    $record.worker_arguments = $arguments

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw 'Worker Process.Start returned false.' }
    $record.worker_process_id = $process.Id
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()

    $workerTimer = [System.Diagnostics.Stopwatch]::StartNew()
    $exited = $false
    try { $exited = $process.WaitForExit($WorkerDeadlineMilliseconds) }
    catch { $record.errors.Add("Worker process wait failed: $($_.Exception.ToString())") }
    $workerTimer.Stop()
    $record.worker_wait_elapsed_ms = [Math]::Round($workerTimer.Elapsed.TotalMilliseconds, 3)
    $record.worker_exit_confirmed = $exited
    if (-not $exited) {
        $record.worker_timed_out = $true
        $record.worker_kill_attempted = $true
        $killTimer = [System.Diagnostics.Stopwatch]::StartNew()
        try {
            $process.Kill($true)
            $record.worker_kill_succeeded = $true
        } catch {
            $record.errors.Add("Worker process-tree kill failed: $($_.Exception.ToString())")
        }
        try { $record.worker_exit_confirmed = $process.WaitForExit($record.worker_termination_wait_ms) }
        catch { $record.errors.Add("Worker termination wait failed: $($_.Exception.ToString())") }
        $killTimer.Stop()
        $record.kill_elapsed_ms = [Math]::Round($killTimer.Elapsed.TotalMilliseconds, 3)
    }
    try {
        if ($process.HasExited) { $record.worker_exit_code = [int]$process.ExitCode }
        else { $record.errors.Add('Worker is still running after the configured termination wait.') }
    } catch { $record.errors.Add("Worker exit status lookup failed: $($_.Exception.ToString())") }

    $outputTimer = [System.Diagnostics.Stopwatch]::StartNew()
    Complete-ControllerOutput -ForceClose $false
    $outputTimer.Stop()
    $record.output_elapsed_ms = [Math]::Round($outputTimer.Elapsed.TotalMilliseconds, 3)

    $record.cleanup_completed = $record.worker_exit_confirmed -and $record.stdout_complete -and $record.stderr_complete -and $record.errors.Count -eq 0
    if ($record.worker_timed_out) { $record.status = 'WORKER_TIMEOUT' }
    elseif ($null -eq $record.worker_exit_code) { $record.status = 'WORKER_EXIT_UNKNOWN' }
    elseif ($record.worker_exit_code -ne 0) { $record.status = 'WORKER_NONZERO' }
    elseif (-not $record.cleanup_completed) { $record.status = 'CONTROLLER_CLEANUP_INCOMPLETE' }
    else { $record.status = 'WORKER_EXIT_ZERO' }
} catch {
    $record.errors.Add($_.Exception.ToString())
    if ($record.status -eq 'NOT_STARTED') { $record.status = 'CONTROLLER_FAILED' }
} finally {
    if ($null -ne $process) {
        try {
            if (-not $process.HasExited) {
                $record.worker_kill_attempted = $true
                $process.Kill($true)
                $record.worker_kill_succeeded = $true
                $record.worker_exit_confirmed = $process.WaitForExit($record.worker_termination_wait_ms)
            }
        } catch { $record.errors.Add("Final worker cleanup failed: $($_.Exception.ToString())") }
        if (-not $script:OutputPersisted) {
            try { Complete-ControllerOutput -ForceClose $true }
            catch { $record.errors.Add("Final controller output cleanup failed: $($_.Exception.ToString())") }
        }
        try { $process.Dispose() } catch { $record.errors.Add("Worker Process.Dispose failed: $($_.Exception.ToString())") }
    } elseif (-not $script:OutputPersisted) {
        try { Complete-ControllerOutput -ForceClose $true }
        catch { $record.errors.Add("Final controller output cleanup failed: $($_.Exception.ToString())") }
    }
    $wholeTimer.Stop()
    $record.ended_utc = [DateTime]::UtcNow.ToString('o')
    $record.elapsed_ms = [Math]::Round($wholeTimer.Elapsed.TotalMilliseconds, 3)
    if ($record.errors.Count -gt 0 -and $record.status -eq 'WORKER_EXIT_ZERO') {
        $record.status = 'CONTROLLER_CLEANUP_INCOMPLETE'
        $record.cleanup_completed = $false
    }
    try { Complete-OwnedRuntimeProcessCleanup }
    catch {
        $record.owned_process_cleanup_status = 'CLEANUP_CHECK_FAILED'
        $record.errors.Add("Owned runtime process cleanup failed: $($_.Exception.ToString())")
    }
    if ($record.owned_process_cleanup_status -notin @('ALL_NONCE_MATCHING_TEST_PROCESSES_EXITED', 'NO_HARNESS_START_RECORD')) {
        if ($record.status -eq 'WORKER_EXIT_ZERO') { $record.status = 'CONTROLLER_CLEANUP_INCOMPLETE' }
        $record.cleanup_completed = $false
    }
    if ($record.errors.Count -gt 0 -and $record.status -eq 'WORKER_EXIT_ZERO') {
        $record.status = 'CONTROLLER_CLEANUP_INCOMPLETE'
        $record.cleanup_completed = $false
    }
    try { Write-ControllerJson -Path $recordPath -Value $record }
    catch { [Console]::Error.WriteLine("Could not persist controller record: $($_.Exception.ToString())") }
}

if ($record.status -ne 'WORKER_EXIT_ZERO' -or -not $record.cleanup_completed) { exit 1 }
exit 0
