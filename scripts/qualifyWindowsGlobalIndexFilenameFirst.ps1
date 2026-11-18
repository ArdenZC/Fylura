param(
    [Parameter(Mandatory = $true)][string]$BaselineProbeExe,
    [Parameter(Mandatory = $true)][string]$CandidateProbeExe,
    [Parameter(Mandatory = $true)][string]$ServiceExe,
    [Parameter(Mandatory = $true)][string]$BaselineRoot,
    [Parameter(Mandatory = $true)][string]$CandidateRoot,
    [Parameter(Mandatory = $true)][string]$ExpectedBaselineSha,
    [Parameter(Mandatory = $true)][string]$ExpectedCandidateSha,
    [Parameter(Mandatory = $true)][string]$EvidencePath,
    [int]$FileCount = 100000,
    [int]$Repetitions = 3,
    [ValidateSet('auto', '512', '1024', '2048', '4096', '8192')][string]$CandidatePersistBatch = 'auto',
    [ValidateSet('A', 'B', 'C', 'D')][string]$DiagnosticScenario = 'D',
    [switch]$DiagnosticOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $runnerTemp "fylura-filename-first-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT"))
$vhdPath = Join-Path $taskRoot 'filename-first-test.vhdx'
$vhdOwnerMarker = Join-Path $taskRoot 'qualification-volume-created-by-task.txt'
$driveMarker = Join-Path $taskRoot 'qualification-volume-drive-letter.txt'
$fixtureRoot = $null
$driveLetter = $null
$vhdCreated = $false
$vhdAttached = $false
$failure = $null
$cleanupFailure = $false
$script:serviceName = 'FyluraGlobalIndex'
$script:serviceCreatedByTask = $false
$script:serviceProcessId = $null
$baselineProbeExe = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $BaselineProbeExe).Path)
$candidateProbeExe = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $CandidateProbeExe).Path)
$serviceExe = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $ServiceExe).Path)
$BaselineRoot = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $BaselineRoot).Path)
$CandidateRoot = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $CandidateRoot).Path)
$EvidencePath = [IO.Path]::GetFullPath($EvidencePath)
if (-not $serviceExe.Equals($candidateProbeExe, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'the Index Service and IPC probe must use the same executable image'
}
$serviceRelativePath = [IO.Path]::GetRelativePath($CandidateRoot, $serviceExe)
if ([IO.Path]::IsPathRooted($serviceRelativePath) -or
    $serviceRelativePath -eq '..' -or
    $serviceRelativePath.StartsWith(('..' + [IO.Path]::DirectorySeparatorChar), [StringComparison]::OrdinalIgnoreCase)) {
    throw "service executable must be inside the exact candidate source workspace: $serviceExe"
}
$script:candidatePersistBatch = $CandidatePersistBatch
$searchReadyTargetSeconds = if ($FileCount -ge 1000000) { 20 } else { 10 }
$durableTargetSeconds = if ($FileCount -ge 1000000) { 60 } else { 30 }
$fullSourceTargetSeconds = if ($FileCount -ge 1000000) { 90 } else { 60 }
$searchReadyTargetMicroseconds = $searchReadyTargetSeconds * 1000000
$durableTargetMicroseconds = $durableTargetSeconds * 1000000
$fullSourceTargetMicroseconds = $fullSourceTargetSeconds * 1000000
$vhdMaximumMiB = if ($FileCount -ge 1000000) { 16384 } else { 4096 }
$evidenceDirectory = Split-Path -Parent $EvidencePath
$traceDirectory = Join-Path $evidenceDirectory 'traces'
$script:evidence = [ordered]@{
    status = 'RUNNING'
    comparison = "same_release_service_ipc_sqlite_8192_vs_filename_first_catalog_$($script:candidatePersistBatch)"
    baselineSha = $ExpectedBaselineSha
    candidateSha = $ExpectedCandidateSha
    fileCount = $FileCount
    repetitions = $Repetitions
    diagnosticScenario = $DiagnosticScenario
    diagnosticOnly = [bool]$DiagnosticOnly
    runner = $null
    fixtureRoot = $null
    sourceId = $null
    baseline = @()
    candidate = @()
    incremental = $null
    averages = $null
    firstSearchTargetSeconds = $searchReadyTargetSeconds
    fullIndexTargetSeconds = $fullSourceTargetSeconds
    completeFilenameSearchTargetSeconds = $searchReadyTargetSeconds
    durableFtsBackgroundTargetSeconds = $durableTargetSeconds
    vhdMaximumMiB = $vhdMaximumMiB
    firstSearchTargetMet = $null
    fullIndexTargetMet = $null
    completeFilenameSearchTargetMet = $null
    durableFilenameIndexTargetMet = $null
    fixtureFilenameIndexTargetMet = $null
    completeCoveragePerPath = $null
    cleanup = [ordered]@{
        serviceStopped = $false
        serviceDeleted = $false
        vhdCreatedByTask = $false
        vhdDetached = $false
        fixtureRemoved = $false
        taskRootRemoved = $false
        details = @()
    }
}

function Invoke-DiskPart([string]$Name, [string[]]$Commands) {
    $path = Join-Path $taskRoot "diskpart-$Name.txt"
    Set-Content -LiteralPath $path -Value $Commands -Encoding ascii
    $output = (& diskpart.exe /s $path 2>&1 | Out-String).Trim()
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0 -or $output -match '(?im)(DiskPart has encountered an error|Virtual Disk Service error|The system failed to|could not be)') {
        throw "DiskPart $Name failed (exit=$exitCode): $output"
    }
    return $output
}

function Set-TaskServiceEnvironment([string]$FilenameFirstMode, [string]$TracePath) {
    $serviceRegistryPath = "HKLM:\SYSTEM\CurrentControlSet\Services\$script:serviceName"
    $environment = @(
        "ZC_GLOBAL_INDEX_FILENAME_FIRST=$FilenameFirstMode",
        "ZC_GLOBAL_INDEX_QA_TRACE=$TracePath",
        "ZC_GLOBAL_INDEX_QA_SCENARIO=$DiagnosticScenario",
        'ZC_GLOBAL_INDEX_MFT_EMIT_BATCH=8192'
    )
    New-ItemProperty -LiteralPath $serviceRegistryPath -Name Environment -PropertyType MultiString -Value $environment -Force | Out-Null
}

function Start-TaskIndexService([string]$FilenameFirstMode, [string]$TracePath) {
    if (-not $script:serviceCreatedByTask) {
        $existingService = Get-CimInstance Win32_Service -Filter "Name='$script:serviceName'" -ErrorAction SilentlyContinue
        if ($existingService) {
            throw "refusing to replace or manage a pre-existing $script:serviceName service: $($existingService.PathName)"
        }
        $serviceImagePath = '"' + $serviceExe + '" --index-service'
        $createOutput = (& sc.exe create $script:serviceName 'binPath=' $serviceImagePath 'start=' 'demand' 'obj=' 'LocalSystem' 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw "failed to create task-owned Index Service (exit=$LASTEXITCODE): $createOutput" }
        $script:serviceCreatedByTask = $true
    }
    Set-TaskServiceEnvironment $FilenameFirstMode $TracePath
    $startOutput = (& sc.exe start $script:serviceName 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw "failed to start task-owned Index Service (exit=$LASTEXITCODE): $startOutput" }
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        $service = Get-CimInstance Win32_Service -Filter "Name='$script:serviceName'" -ErrorAction SilentlyContinue
        if ($service -and $service.State -eq 'Running' -and $service.ProcessId -gt 0) { break }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $service -or $service.State -ne 'Running' -or $service.ProcessId -le 0) {
        $state = if ($service) { "state=$($service.State) win32ExitCode=$($service.ExitCode) serviceSpecificExitCode=$($service.ServiceSpecificExitCode)" } else { 'service record unavailable' }
        throw "task-owned Index Service did not reach Running within 45 seconds: $state"
    }
    $expectedServiceImagePath = ('"' + $serviceExe + '" --index-service')
    if ($service.PathName.Trim() -ine $expectedServiceImagePath) {
        throw "task-owned service ImagePath mismatch: expected=$expectedServiceImagePath actual=$($service.PathName)"
    }
    $script:serviceProcessId = [int]$service.ProcessId
    $serviceProcess = Get-CimInstance Win32_Process -Filter "ProcessId=$($script:serviceProcessId)" -ErrorAction Stop
    if (-not $serviceProcess.ExecutablePath -or -not $serviceProcess.ExecutablePath.Equals($serviceExe, [StringComparison]::OrdinalIgnoreCase)) {
        throw "task-owned service image mismatch: expected=$serviceExe actual=$($serviceProcess.ExecutablePath)"
    }
}

function Stop-TaskIndexService {
    if (-not $script:serviceCreatedByTask) { return }
    $service = Get-CimInstance Win32_Service -Filter "Name='$script:serviceName'" -ErrorAction SilentlyContinue
    if ($service -and $service.State -ne 'Stopped') {
        $stopOutput = (& sc.exe stop $script:serviceName 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 1062) { throw "failed to stop task-owned Index Service (exit=$LASTEXITCODE): $stopOutput" }
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        do {
            $service = Get-CimInstance Win32_Service -Filter "Name='$script:serviceName'" -ErrorAction SilentlyContinue
            if (-not $service -or $service.State -eq 'Stopped') { break }
            Start-Sleep -Milliseconds 200
        } while ([DateTime]::UtcNow -lt $deadline)
        if ($service -and $service.State -ne 'Stopped') { throw "task-owned Index Service did not stop: state=$($service.State)" }
    }
    $script:serviceProcessId = $null
}

function Remove-TaskIndexService {
    if (-not $script:serviceCreatedByTask) { return }
    Stop-TaskIndexService
    $deleteOutput = (& sc.exe delete $script:serviceName 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw "failed to delete task-owned Index Service (exit=$LASTEXITCODE): $deleteOutput" }
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $service = Get-CimInstance Win32_Service -Filter "Name='$script:serviceName'" -ErrorAction SilentlyContinue
        if (-not $service) { break }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($service) { throw "task-owned Index Service remains registered after deletion: state=$($service.State)" }
    $script:serviceCreatedByTask = $false
}

function Get-FreeDriveLetter {
    $used = @([IO.DriveInfo]::GetDrives() | ForEach-Object { $_.Name.Substring(0, 1).ToUpperInvariant() })
    foreach ($candidate in @('Z', 'Y', 'X', 'W', 'V', 'U', 'T', 'S', 'R', 'Q', 'P', 'O', 'N', 'M', 'L', 'K', 'J', 'I', 'H', 'G', 'F', 'E')) {
        if ($used -notcontains $candidate) { return $candidate }
    }
    throw 'no unused drive letter is available for the task-owned NTFS VHD'
}

function Test-VhdAttached {
    if (-not (Test-Path -LiteralPath $vhdPath -PathType Leaf)) { return $false }
    if (-not (Get-Command Get-DiskImage -ErrorAction SilentlyContinue)) {
        throw 'Get-DiskImage is unavailable; refusing to infer VHD attachment state'
    }
    return [bool](Get-DiskImage -ImagePath $vhdPath -ErrorAction Stop).Attached
}

function New-TaskVolume {
    $script:driveLetter = Get-FreeDriveLetter
    $drive = "$($script:driveLetter):"
    $driveRoot = "$drive\"
    if (Test-Path -LiteralPath $vhdPath) { throw "refusing to replace a pre-existing VHD: $vhdPath" }
    # Record ownership before DiskPart mutates anything so a partial create can
    # still be detached and removed safely by the finally block.
    Set-Content -LiteralPath $vhdOwnerMarker -Value $vhdPath -NoNewline
    [void](Invoke-DiskPart 'create-attach-format' @(
        "create vdisk file=`"$vhdPath`" maximum=$vhdMaximumMiB type=expandable",
        "select vdisk file=`"$vhdPath`"",
        'attach vdisk',
        'create partition primary',
        'format fs=ntfs quick label=FYLURAPERF',
        "assign letter=$driveLetter",
        'exit'
    ))
    $script:vhdCreated = Test-Path -LiteralPath $vhdPath -PathType Leaf
    $script:vhdAttached = Test-VhdAttached
    if (-not $script:vhdCreated -or -not $script:vhdAttached) { throw 'task VHD was not created and attached' }
    $mounted = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='$drive'" -ErrorAction SilentlyContinue
    if (-not $mounted -or $mounted.FileSystem -ne 'NTFS') { throw "task volume is not visible as fixed NTFS at $drive" }
    # Join-Path resolves the root through PowerShell's FileSystem PSDrive
    # provider, so the task volume must be attached and assigned first.
    $script:fixtureRoot = Join-Path $driveRoot 'fixtures\filename-first'
    Set-Content -LiteralPath $driveMarker -Value $drive -NoNewline
    Set-Content -LiteralPath (Join-Path $taskRoot 'fixture-created-by-qualification.txt') -Value $script:fixtureRoot -NoNewline
    New-Item -ItemType Directory -Path $script:fixtureRoot -Force | Out-Null
    $script:evidence.fixtureRoot = $script:fixtureRoot
}

function Invoke-Probe(
    [string]$Label,
    [string]$ProbeExe,
    [string[]]$Arguments,
    [string]$FilenameFirstMode,
    [string]$TracePath,
    [int]$TimeoutMinutes = 60
) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $ProbeExe
    $startInfo.WorkingDirectory = (Get-Location).Path
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $startInfo.ArgumentList.Add($argument) }
    $startInfo.Environment['RUNNER_TEMP'] = $runnerTemp.TrimEnd('\')
    $startInfo.Environment['ZC_GLOBAL_INDEX_FILENAME_FIRST'] = $FilenameFirstMode
    $startInfo.Environment['ZC_GLOBAL_INDEX_QA_SCENARIO'] = $DiagnosticScenario
    if ($script:candidatePersistBatch -eq 'auto') {
        $startInfo.Environment.Remove('ZC_GLOBAL_INDEX_MFT_PERSIST_BATCH') | Out-Null
    } else {
        $startInfo.Environment['ZC_GLOBAL_INDEX_MFT_PERSIST_BATCH'] = $script:candidatePersistBatch
    }
    $startInfo.Environment['ZC_GLOBAL_INDEX_MFT_EMIT_BATCH'] = '8192'
    $startInfo.Environment['ZC_GLOBAL_INDEX_QA_TRACE'] = $TracePath

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to start $Label" }
    $runCpuStopwatch = [Diagnostics.Stopwatch]::StartNew()
    $clientCpuStartMs = 0.0
    $serviceCpuStartMs = $null
    $serviceCpuComplete = $false
    try {
        $process.Refresh()
        $clientCpuStartMs = $process.TotalProcessorTime.TotalMilliseconds
        if ($script:serviceProcessId) {
            $serviceCpuProcess = Get-Process -Id $script:serviceProcessId -ErrorAction Stop
            $serviceCpuProcess.Refresh()
            $serviceCpuStartMs = $serviceCpuProcess.TotalProcessorTime.TotalMilliseconds
            $serviceCpuComplete = $true
        }
    } catch { }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddMinutes($TimeoutMinutes)
    $peakWorkingSet = 0L
    $peakPrivateBytes = 0L
    $peakClientWorkingSet = 0L
    $peakClientPrivateBytes = 0L
    $peakServiceWorkingSet = 0L
    $peakServicePrivateBytes = 0L
    while (-not $process.WaitForExit(100)) {
        try {
            $process.Refresh()
            $clientWorkingSet = $process.WorkingSet64
            $clientPrivateBytes = $process.PrivateMemorySize64
            $peakClientWorkingSet = [Math]::Max($peakClientWorkingSet, $clientWorkingSet)
            $peakClientPrivateBytes = [Math]::Max($peakClientPrivateBytes, $clientPrivateBytes)
            $serviceWorkingSet = 0L
            $servicePrivateBytes = 0L
            if ($script:serviceProcessId) {
                $serviceProcess = Get-Process -Id $script:serviceProcessId -ErrorAction Stop
                $serviceProcess.Refresh()
                $serviceWorkingSet = $serviceProcess.WorkingSet64
                $servicePrivateBytes = $serviceProcess.PrivateMemorySize64
            }
            $peakServiceWorkingSet = [Math]::Max($peakServiceWorkingSet, $serviceWorkingSet)
            $peakServicePrivateBytes = [Math]::Max($peakServicePrivateBytes, $servicePrivateBytes)
            $peakWorkingSet = [Math]::Max($peakWorkingSet, $clientWorkingSet + $serviceWorkingSet)
            $peakPrivateBytes = [Math]::Max($peakPrivateBytes, $clientPrivateBytes + $servicePrivateBytes)
        } catch { }
        if ([DateTime]::UtcNow -ge $deadline) {
            $process.Kill($true)
            $process.WaitForExit(10000)
            throw "$Label exceeded $TimeoutMinutes minutes"
        }
    }
    $process.Refresh()
    $peakClientWorkingSet = [Math]::Max($peakClientWorkingSet, $process.WorkingSet64)
    $peakClientPrivateBytes = [Math]::Max($peakClientPrivateBytes, $process.PrivateMemorySize64)
    $serviceWorkingSet = 0L
    $servicePrivateBytes = 0L
    if ($script:serviceProcessId) {
        $serviceProcess = Get-Process -Id $script:serviceProcessId -ErrorAction SilentlyContinue
        if ($serviceProcess) {
            $serviceProcess.Refresh()
            $serviceWorkingSet = $serviceProcess.WorkingSet64
            $servicePrivateBytes = $serviceProcess.PrivateMemorySize64
        }
    }
    $peakServiceWorkingSet = [Math]::Max($peakServiceWorkingSet, $serviceWorkingSet)
    $peakServicePrivateBytes = [Math]::Max($peakServicePrivateBytes, $servicePrivateBytes)
    $peakWorkingSet = [Math]::Max($peakWorkingSet, $process.WorkingSet64 + $serviceWorkingSet)
    $peakPrivateBytes = [Math]::Max($peakPrivateBytes, $process.PrivateMemorySize64 + $servicePrivateBytes)
    $runCpuStopwatch.Stop()
    $clientCpuTimeMs = [Math]::Max(0.0, $process.TotalProcessorTime.TotalMilliseconds - $clientCpuStartMs)
    $serviceCpuTimeMs = $null
    if ($serviceCpuComplete) {
        try {
            $serviceCpuProcess = Get-Process -Id $script:serviceProcessId -ErrorAction Stop
            $serviceCpuProcess.Refresh()
            $serviceCpuTimeMs = [Math]::Max(0.0, $serviceCpuProcess.TotalProcessorTime.TotalMilliseconds - $serviceCpuStartMs)
        } catch {
            $serviceCpuComplete = $false
        }
    }
    $totalCpuTimeMs = if ($serviceCpuComplete -and $null -ne $serviceCpuTimeMs) { $clientCpuTimeMs + $serviceCpuTimeMs } else { $null }
    $logicalProcessorCount = [Environment]::ProcessorCount
    $cpuWallTimeMs = $runCpuStopwatch.Elapsed.TotalMilliseconds
    $meanCpuPercentOfOneCore = if ($null -ne $totalCpuTimeMs -and $cpuWallTimeMs -gt 0) { 100.0 * $totalCpuTimeMs / $cpuWallTimeMs } else { $null }
    $meanCpuPercentOfMachineCapacity = if ($null -ne $meanCpuPercentOfOneCore -and $logicalProcessorCount -gt 0) { $meanCpuPercentOfOneCore / $logicalProcessorCount } else { $null }
    $stdout = $stdoutTask.Result.Trim()
    $stderr = $stderrTask.Result.Trim()
    if ($process.ExitCode -ne 0) { throw "$Label failed (exit=$($process.ExitCode)): $stderr $stdout" }
    $jsonLine = @($stdout -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Last 1)
    if ($jsonLine.Count -ne 1) { throw "$Label returned no JSON evidence: $stdout" }
    $result = $jsonLine[0] | ConvertFrom-Json
    $mftTiming = Get-TraceEvent $TracePath 'mft_initial_index'
    $writerTiming = Get-TraceEvent $TracePath 'source_operation_write'
    $writerConfig = Get-TraceEvent $TracePath 'global_index_initial_writer_config'
    $walCheckpointEvents = @(Get-TraceEvents $TracePath 'global_index_wal_checkpoint')
    $walCheckpoint = if ($walCheckpointEvents.Count -gt 0) { $walCheckpointEvents[-1] } else { $null }
    $walRestartEvents = @($walCheckpointEvents | Where-Object { $_.mode -eq 'restart' })
    $writerBatches = @(Get-TraceEvents $TracePath 'global_index_initial_writer_batch')
    $serviceBatchEvents = @(Get-TraceEvents $TracePath 'index_service_batching')
    $serviceBatching = if ($serviceBatchEvents.Count -gt 0) { $serviceBatchEvents[-1] } else { $null }
    $writerDiagnostics = $null
    if ($writerBatches.Count -gt 0) {
        $walBytesBeforeCommit = @($writerBatches | ForEach-Object { [long]$_.wal_before_commit_bytes })
        $walBytes = @($writerBatches | ForEach-Object { [long]$_.wal_bytes })
        $walFrames = @($writerBatches | ForEach-Object { [long]$_.wal_frames })
        $uncheckpointedFrames = @($writerBatches | ForEach-Object {
            [Math]::Max(0, [long]$_.wal_frames - [long]$_.checkpointed_frames)
        })
        $writerDiagnostics = [ordered]@{
            batchCount = $writerBatches.Count
            walAutocheckpointPagesBefore = if ($writerConfig) { [long]$writerConfig.wal_autocheckpoint_before } else { $null }
            walAutocheckpointPages = if ($writerConfig) { [long]$writerConfig.wal_autocheckpoint_during } else { $null }
            walRestartCount = $walRestartEvents.Count
            walRestartBusyCount = @($walRestartEvents | Where-Object { [long]$_.busy -ne 0 -or [long]$_.log_frames -ne [long]$_.checkpointed_frames }).Count
            walRestartTotalUs = [long](($walRestartEvents | Measure-Object -Property elapsed_us -Sum).Sum)
            cacheSizePages = if ($writerConfig) { [long]$writerConfig.cache_size_pages } else { $null }
            cacheSpillEnabled = if ($writerConfig) { [long]$writerConfig.cache_spill } else { $null }
            sumCommitUs = [long](($writerBatches | Measure-Object -Property commit_us -Sum).Sum)
            maxCommitUs = [long](($writerBatches | Measure-Object -Property commit_us -Maximum).Maximum)
            maxWriterLockUs = [long](($writerBatches | Measure-Object -Property writer_lock_us -Maximum).Maximum)
            maxWalBytesBeforeCommit = ($walBytesBeforeCommit | Measure-Object -Maximum).Maximum
            maxWalBytesAfterCommit = ($walBytes | Measure-Object -Maximum).Maximum
            maxWalFramesAfterCommit = ($walFrames | Measure-Object -Maximum).Maximum
            maxUncheckpointedWalFrames = ($uncheckpointedFrames | Measure-Object -Maximum).Maximum
            noopBusyFieldNonzeroSamples = @($writerBatches | Where-Object { [long]$_.wal_busy -ne 0 }).Count
            totalCacheSpillCount = [long](($writerBatches | Measure-Object -Property cache_spill_count -Sum).Sum)
            totalCacheWritePages = [long](($writerBatches | Measure-Object -Property cache_write_pages -Sum).Sum)
        }
    }
    return [pscustomobject]@{
        label = $Label
        result = $result
        stages = [ordered]@{ mft = $mftTiming; writer = $writerTiming; writerDiagnostics = $writerDiagnostics; walCheckpoint = $walCheckpoint }
        peakWorkingSetBytes = $peakWorkingSet
        peakPrivateBytes = $peakPrivateBytes
        peakClientWorkingSetBytes = $peakClientWorkingSet
        peakClientPrivateBytes = $peakClientPrivateBytes
        peakServiceWorkingSetBytes = $peakServiceWorkingSet
        peakServicePrivateBytes = $peakServicePrivateBytes
        clientCpuTimeMs = $clientCpuTimeMs
        serviceCpuTimeMs = $serviceCpuTimeMs
        totalCpuTimeMs = $totalCpuTimeMs
        cpuWallTimeMs = $cpuWallTimeMs
        logicalProcessorCount = $logicalProcessorCount
        meanCpuPercentOfOneCore = $meanCpuPercentOfOneCore
        meanCpuPercentOfMachineCapacity = $meanCpuPercentOfMachineCapacity
        cpuMetricsComplete = $serviceCpuComplete
        serviceProcessId = $script:serviceProcessId
        serviceIpcBatching = $serviceBatching
        tracePath = $TracePath
    }
}

function Convert-TraceEventLine([string]$Line, [string]$EventName) {
    $event = [ordered]@{ event = $EventName }
    foreach ($field in ($Line -split '\s+')) {
        $pair = $field -split '=', 2
        if ($pair.Count -ne 2 -or $pair[0] -eq $EventName) { continue }
        $number = 0L
        if ([long]::TryParse($pair[1], [ref]$number)) { $event[$pair[0]] = $number }
        else { $event[$pair[0]] = $pair[1] }
    }
    return [pscustomobject]$event
}

function Get-TraceFiles([string]$Path) {
    $files = [System.Collections.Generic.List[System.IO.FileInfo]]::new()
    if (Test-Path -LiteralPath $Path -PathType Leaf) {
        $files.Add((Get-Item -LiteralPath $Path))
    }
    foreach ($file in (Get-ChildItem -Path "${Path}.pid-*.log" -File -ErrorAction SilentlyContinue)) {
        $files.Add($file)
    }
    return $files.ToArray()
}

function Get-TraceEvents([string]$Path, [string]$EventName) {
    $traceFiles = @(Get-TraceFiles $Path)
    if ($traceFiles.Count -eq 0) { return @() }
    $events = [System.Collections.Generic.List[object]]::new()
    foreach ($traceFile in $traceFiles) {
        foreach ($line in (Get-Content -LiteralPath $traceFile.FullName)) {
            if ($line -match "(?:^|\s)$([regex]::Escape($EventName))(?:\s|$)") {
                $events.Add((Convert-TraceEventLine $line $EventName))
            }
        }
    }
    return @($events.ToArray() | Sort-Object { [long]$_.timestamp_us })
}

function Get-TraceEvent([string]$Path, [string]$EventName) {
    $events = @(Get-TraceEvents $Path $EventName)
    if ($events.Count -eq 0) { return $null }
    return $events[-1]
}

function Prepare-Profile([string]$ProbeExe, [string]$Profile) {
    $prepared = Invoke-Probe "prepare $Profile" $ProbeExe @('--prepare-profile', $Profile, "$($script:driveLetter):") '0' (Join-Path $taskRoot 'prepare-trace.log') 2
    return $prepared.result
}

function Get-Average([object[]]$Rows, [string]$Property) {
    $values = foreach ($row in $Rows) {
        $value = $row
        foreach ($part in $Property.Split('.')) {
            if ($null -eq $value) { break }
            $value = $value.$part
        }
        if ($null -eq $value) { 0.0 } else { [double]$value }
    }
    return [double](($values | Measure-Object -Average).Average)
}

function Get-AverageWhenAvailable([object[]]$Rows, [string]$Property) {
    $values = foreach ($row in $Rows) {
        $value = $row
        foreach ($part in $Property.Split('.')) {
            if ($null -eq $value) { break }
            $value = $value.$part
        }
        if ($null -ne $value) { [double]$value }
    }
    if ($null -eq $values -or @($values).Count -eq 0) { return $null }
    return [double](($values | Measure-Object -Average).Average)
}

try {
    if ($FileCount -notin @(100000, 250000, 1000000)) { throw 'native filename-first qualification supports 100,000, 250,000, or 1,000,000 files' }
    if ($Repetitions -lt 1 -or $Repetitions -gt 5) { throw 'repetitions must be between 1 and 5' }
    if (-not $ExpectedBaselineSha -or -not $ExpectedCandidateSha) { throw 'expected source SHAs are required' }
    foreach ($exe in @($baselineProbeExe, $candidateProbeExe)) {
        if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "required executable missing: $exe" }
    }
    if (-not $taskRoot.StartsWith($runnerTemp, [StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing to create task-owned data outside RUNNER_TEMP: $taskRoot"
    }
    if (Test-Path -LiteralPath $taskRoot) { throw "refusing to reuse an existing QA task root: $taskRoot" }
    $actualBaselineSha = (& git -C $BaselineRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $actualBaselineSha -ne $ExpectedBaselineSha) {
        throw "baseline workspace SHA mismatch: expected=$ExpectedBaselineSha actual=$actualBaselineSha"
    }
    $actualCandidateSha = (& git -C $CandidateRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $actualCandidateSha -ne $ExpectedCandidateSha) {
        throw "candidate workspace SHA mismatch: expected=$ExpectedCandidateSha actual=$actualCandidateSha"
    }
    New-Item -ItemType Directory -Path $taskRoot -Force | Out-Null
    $os = Get-CimInstance Win32_OperatingSystem
    $script:evidence.runner = [ordered]@{
        caption = $os.Caption
        version = $os.Version
        build = $os.BuildNumber
        architecture = $os.OSArchitecture
        logicalProcessors = [Environment]::ProcessorCount
        runId = $env:GITHUB_RUN_ID
        runAttempt = $env:GITHUB_RUN_ATTEMPT
    }

    New-TaskVolume
    $bucketCount = 256
    for ($bucket = 0; $bucket -lt $bucketCount; $bucket++) {
        [IO.Directory]::CreateDirectory((Join-Path $fixtureRoot ("bucket-{0:D3}" -f $bucket))) | Out-Null
    }
    for ($index = 0; $index -lt $FileCount; $index++) {
        $bucket = $index % $bucketCount
        $directory = Join-Path $fixtureRoot ("bucket-{0:D3}" -f $bucket)
        $file = Join-Path $directory ("filename-first-item-{0:D6}.txt" -f $index)
        [IO.File]::WriteAllBytes($file, [byte[]]::new(0))
    }
    $deletePath = Join-Path $fixtureRoot 'bucket-000\filename-first-deleteprobe.txt'
    [IO.File]::WriteAllBytes($deletePath, [byte[]]::new(0))
    $targetPath = Join-Path $fixtureRoot 'bucket-000\zz-filename-first-finalmarker.txt'
    $renamedPath = Join-Path $fixtureRoot 'bucket-000\zz-filename-first-renamedmarker.txt'
    $firstTargetPath = Join-Path $fixtureRoot 'bucket-000\filename-first-item-000000.txt'
    $addedPath = Join-Path $fixtureRoot 'bucket-000\filename-first-addedprobe.txt'
    $movedFromPath = Join-Path $fixtureRoot 'bucket-000\filename-first-item-000256.txt'
    $movedToPath = Join-Path $fixtureRoot 'bucket-001\filename-first-moved.txt'
    [IO.File]::WriteAllBytes($targetPath, [byte[]]::new(0))

    $script:evidence.fixture = [ordered]@{
        expectedFiles = $FileCount
        bucketCount = $bucketCount
        firstTargetPath = $firstTargetPath
        firstQuery = 'filename-first-item-000000.txt'
        completeCoverageTargetPath = $targetPath
        completeCoverageQuery = 'finalmarker'
        addedPath = $addedPath
        renamedFrom = $targetPath
        renamedTo = $renamedPath
        movedFrom = $movedFromPath
        movedTo = $movedToPath
        deletedPath = $deletePath
    }
    $baselineRows = [Collections.Generic.List[object]]::new()
    $candidateRows = [Collections.Generic.List[object]]::new()
    $lastProfiles = @{}
    $lastSourceIds = @{}
    $diagnosticScenarioIndex = [Array]::IndexOf(@('A', 'B', 'C', 'D'), $DiagnosticScenario)
    for ($repetition = 1; $repetition -le $Repetitions; $repetition++) {
        $order = if (($repetition + $diagnosticScenarioIndex) % 2 -eq 1) { @('baseline', 'candidate') } else { @('candidate', 'baseline') }
        foreach ($side in $order) {
            $profile = Join-Path $taskRoot "$side-profile-$repetition"
            $probe = if ($side -eq 'baseline') { $baselineProbeExe } else { $candidateProbeExe }
            $mode = if ($side -eq 'baseline') { '0' } else { '1' }
            $prepared = Prepare-Profile $probe $profile
            if (-not $prepared.prepared -or $prepared.enabledSourceCount -ne 1) {
                throw "$side profile did not authorize exactly the task-owned VHD source"
            }
            if ($script:evidence.sourceId -and $script:evidence.sourceId -ne $prepared.targetSourceId) {
                throw "baseline/candidate source identity differs: $($script:evidence.sourceId) vs $($prepared.targetSourceId)"
            }
            $script:evidence.sourceId = $prepared.targetSourceId
            $trace = Join-Path $taskRoot "$side-$DiagnosticScenario-$repetition-trace.log"
            Start-TaskIndexService $mode $trace
            try {
                $run = Invoke-Probe "$side initial run $repetition" $probe @(
                    '--qualify-filename-first', $profile, $prepared.targetSourceId,
                    'filename-first-item-000000.txt', $firstTargetPath, 'finalmarker', $targetPath, $FileCount.ToString()
                ) $mode $trace 90
            } finally {
                Stop-TaskIndexService
            }
            $serviceRouteEvents = @(Get-TraceEvents $trace 'windows_service_route_ok')
            if ($serviceRouteEvents.Count -lt 1) {
                throw "$side initial run $repetition did not complete a Windows Service named-pipe route; Direct fallback is not accepted"
            }
            if (-not $run.cpuMetricsComplete -or $null -eq $run.totalCpuTimeMs -or $run.cpuWallTimeMs -le 0) {
                throw "$side initial run $repetition could not measure desktop and SCM Service CPU time"
            }
            $serviceBatching = $run.serviceIpcBatching
            if (-not $serviceBatching -or
                [long]$serviceBatching.input_entries -lt $FileCount -or
                [long]$serviceBatching.repository_entries -ne [long]$serviceBatching.input_entries -or
                [long]$serviceBatching.repository_batches -lt 1 -or
                [long]$serviceBatching.entry_frames -lt 1) {
                throw "$side initial run $repetition did not prove that actual IPC entry frames reached the repository: $($serviceBatching | ConvertTo-Json -Compress)"
            }
            $expectedFilenameFirstMode = $side -eq 'candidate'
            if ([bool]$run.result.filenameFirstEnabled -ne $expectedFilenameFirstMode) {
                throw "$side initial run $repetition used the wrong filename-first mode: expected=$expectedFilenameFirstMode actual=$($run.result.filenameFirstEnabled)"
            }
            if (-not $run.stages.mft -or -not $run.stages.writer) {
                throw "$side initial run $repetition omitted MFT or repository stage timings: mft=$([bool]$run.stages.mft) writer=$([bool]$run.stages.writer) traceFiles=$(@(Get-TraceFiles $trace).Count)"
            }
            if (-not $run.result.restartedSearchVisible -or -not $run.result.restartedCompleteCoverageSearchVisible) {
                throw "$side initial run $repetition did not persist the tested filenames across restart"
            }
            if (-not $run.result.ftsIntegrityVerified -or [long]$run.result.ftsTriggerCount -ne 3) {
                throw "$side initial run $repetition did not verify FTS content and restore all triggers"
            }
            if (-not $run.result.ftsBulkBuildComplete) {
                throw "$side initial run $repetition left a partial FTS bulk build"
            }
            if ($run.result.sourceIndexStatus -ne 'ready' -or $run.result.coveragePending) {
                throw "$side initial run $repetition did not finish its source operation: status=$($run.result.sourceIndexStatus) coveragePending=$($run.result.coveragePending)"
            }
            if ([long]$run.result.fixtureFileEntries -ne $FileCount) {
                throw "$side initial run $repetition persisted $($run.result.fixtureFileEntries) task-owned fixture files; expected $FileCount"
            }
            $undeterminedFixtureEntries = @($run.result.undeterminedEntries | Where-Object {
                $_.path.StartsWith($script:fixtureRoot, [StringComparison]::OrdinalIgnoreCase)
            })
            if ($undeterminedFixtureEntries.Count -gt 0) {
                throw "$side initial run $repetition left task-owned fixture entries undetermined: $($undeterminedFixtureEntries | ConvertTo-Json -Compress)"
            }
            if ($side -eq 'candidate' -and $DiagnosticScenario -in @('B', 'D') -and (-not $run.result.firstSearchBeforeComplete -or [long]$run.result.firstSearchProcessedEntries -ge $FileCount)) {
                throw "candidate initial run $repetition did not expose an early filename while progress was incomplete"
            }
            if ([long]$run.result.databaseActiveEntries -lt $FileCount) {
                throw "$side initial run indexed too few active entries: $($run.result.databaseActiveEntries) < $FileCount"
            }
            $run | Add-Member -NotePropertyName repetition -NotePropertyValue $repetition
            if ($side -eq 'baseline') {
                $baselineRows.Add($run)
                $script:evidence.baseline = $baselineRows.ToArray()
            } else {
                $candidateRows.Add($run)
                $script:evidence.candidate = $candidateRows.ToArray()
            }
            $sample = [ordered]@{
                side = $side
                repetition = $repetition
                firstSearchableUs = $run.result.firstSearchableUs
                transientCatalogReadyUs = $run.result.transientCatalogReadyUs
                durableFilenameIndexUs = $run.result.durableFilenameIndexUs
                fullIndexUs = $run.result.fullIndexUs
                peakWorkingSetBytes = $run.peakWorkingSetBytes
                peakPrivateBytes = $run.peakPrivateBytes
                peakClientWorkingSetBytes = $run.peakClientWorkingSetBytes
                peakClientPrivateBytes = $run.peakClientPrivateBytes
                peakServiceWorkingSetBytes = $run.peakServiceWorkingSetBytes
                peakServicePrivateBytes = $run.peakServicePrivateBytes
                clientCpuTimeMs = $run.clientCpuTimeMs
                serviceCpuTimeMs = $run.serviceCpuTimeMs
                totalCpuTimeMs = $run.totalCpuTimeMs
                cpuWallTimeMs = $run.cpuWallTimeMs
                logicalProcessorCount = $run.logicalProcessorCount
                meanCpuPercentOfOneCore = $run.meanCpuPercentOfOneCore
                meanCpuPercentOfMachineCapacity = $run.meanCpuPercentOfMachineCapacity
                serviceRouteObserved = $serviceRouteEvents.Count -gt 0
                serviceIpcBatching = $serviceBatching
                candidatePersistBatch = $script:candidatePersistBatch
                writerStages = $run.stages.writer
                mftStages = $run.stages.mft
                result = $run.result
            }
            Write-Output ("FILENAME_FIRST_SAMPLE " + ($sample | ConvertTo-Json -Depth 12 -Compress))
            $lastProfiles[$side] = $profile
            $lastSourceIds[$side] = $prepared.targetSourceId
            New-Item -ItemType Directory -Path $traceDirectory -Force | Out-Null
            foreach ($traceFile in (Get-TraceFiles $trace)) {
                Copy-Item -LiteralPath $traceFile.FullName -Destination (Join-Path $traceDirectory $traceFile.Name) -Force
            }
        }
    }

    [IO.File]::WriteAllBytes($addedPath, [byte[]]::new(0))
    [IO.File]::Move($targetPath, $renamedPath)
    [IO.File]::Move($movedFromPath, $movedToPath)
    Remove-Item -LiteralPath $deletePath -Force
    Start-Sleep -Milliseconds 500
    $incremental = [ordered]@{}
    foreach ($side in @('baseline', 'candidate')) {
        $probe = if ($side -eq 'baseline') { $baselineProbeExe } else { $candidateProbeExe }
        $mode = if ($side -eq 'baseline') { '0' } else { '1' }
        $trace = Join-Path $taskRoot "$side-$DiagnosticScenario-incremental-trace.log"
        Start-TaskIndexService $mode $trace
        try {
            $run = Invoke-Probe "$side incremental changes" $probe @(
                '--qualify-filename-first-incremental',
                $lastProfiles[$side],
                $lastSourceIds[$side],
                $addedPath,
                $targetPath,
                $renamedPath,
                $movedFromPath,
                $movedToPath,
                $deletePath
            ) $mode $trace 30
        } finally {
            Stop-TaskIndexService
        }
        if (@(Get-TraceEvents $trace 'windows_service_route_ok').Count -lt 1) {
            throw "$side incremental operation did not complete a Windows Service named-pipe route"
        }
        if (-not $run.result.restartCorrect -or -not $run.result.movedVisible -or -not $run.result.oldMoveHidden) { throw "$side incremental add/rename/move/delete failed restart validation" }
        $incremental[$side] = $run.result
        New-Item -ItemType Directory -Path $traceDirectory -Force | Out-Null
        foreach ($traceFile in (Get-TraceFiles $trace)) {
            Copy-Item -LiteralPath $traceFile.FullName -Destination (Join-Path $traceDirectory $traceFile.Name) -Force
        }
    }

    $baselineFirst = Get-Average $baselineRows.ToArray() 'result.firstSearchableUs'
    $candidateFirst = Get-Average $candidateRows.ToArray() 'result.firstSearchableUs'
    $baselineFull = Get-Average $baselineRows.ToArray() 'result.fullIndexUs'
    $candidateFull = Get-Average $candidateRows.ToArray() 'result.fullIndexUs'
    $baselineDurableFilenameIndex = Get-Average $baselineRows.ToArray() 'result.durableFilenameIndexUs'
    $candidateDurableFilenameIndex = Get-Average $candidateRows.ToArray() 'result.durableFilenameIndexUs'
    $script:evidence.baseline = $baselineRows.ToArray()
    $script:evidence.candidate = $candidateRows.ToArray()
    $script:evidence.incremental = $incremental
    $script:evidence.averages = [ordered]@{
        baselineFirstSearchableUs = $baselineFirst
        candidateFirstSearchableUs = $candidateFirst
        firstSearchImprovementPercent = if ($baselineFirst -gt 0) { 100.0 * ($baselineFirst - $candidateFirst) / $baselineFirst } else { 0 }
        baselineCompleteFilenameSearchableUs = $baselineDurableFilenameIndex
        candidateCompleteFilenameSearchableUs = Get-Average $candidateRows.ToArray() 'result.transientCatalogReadyUs'
        completeFilenameSearchImprovementPercent = if ($baselineDurableFilenameIndex -gt 0) { 100.0 * ($baselineDurableFilenameIndex - (Get-Average $candidateRows.ToArray() 'result.transientCatalogReadyUs')) / $baselineDurableFilenameIndex } else { 0 }
        baselineFullIndexUs = $baselineFull
        candidateFullIndexUs = $candidateFull
        fullIndexImprovementPercent = if ($baselineFull -gt 0) { 100.0 * ($baselineFull - $candidateFull) / $baselineFull } else { 0 }
        baselineDurableFilenameIndexUs = $baselineDurableFilenameIndex
        candidateDurableFilenameIndexUs = $candidateDurableFilenameIndex
        durableFilenameIndexImprovementPercent = if ($baselineDurableFilenameIndex -gt 0) { 100.0 * ($baselineDurableFilenameIndex - $candidateDurableFilenameIndex) / $baselineDurableFilenameIndex } else { 0 }
        baselinePeakWorkingSetBytes = Get-Average $baselineRows.ToArray() 'peakWorkingSetBytes'
        candidatePeakWorkingSetBytes = Get-Average $candidateRows.ToArray() 'peakWorkingSetBytes'
        baselinePeakPrivateBytes = Get-Average $baselineRows.ToArray() 'peakPrivateBytes'
        candidatePeakPrivateBytes = Get-Average $candidateRows.ToArray() 'peakPrivateBytes'
        baselineTotalCpuTimeMs = Get-Average $baselineRows.ToArray() 'totalCpuTimeMs'
        candidateTotalCpuTimeMs = Get-Average $candidateRows.ToArray() 'totalCpuTimeMs'
        baselineMeanCpuPercentOfMachineCapacity = Get-Average $baselineRows.ToArray() 'meanCpuPercentOfMachineCapacity'
        candidateMeanCpuPercentOfMachineCapacity = Get-Average $candidateRows.ToArray() 'meanCpuPercentOfMachineCapacity'
        baselineDatabaseBytes = Get-Average $baselineRows.ToArray() 'result.databaseBytesWithWal'
        candidateDatabaseBytes = Get-Average $candidateRows.ToArray() 'result.databaseBytesWithWal'
        baselinePeakDatabaseAndWalBytes = Get-Average $baselineRows.ToArray() 'result.peakDatabaseBytesWithWal'
        candidatePeakDatabaseAndWalBytes = Get-Average $candidateRows.ToArray() 'result.peakDatabaseBytesWithWal'
        baselineTransientCatalogEstimatedBytes = Get-AverageWhenAvailable $baselineRows.ToArray() 'result.transientCatalogEstimatedBytes'
        candidateTransientCatalogEstimatedBytes = Get-AverageWhenAvailable $candidateRows.ToArray() 'result.transientCatalogEstimatedBytes'
    }
    $baselineDbstatAvailable = $baselineRows.Count -gt 0 -and @($baselineRows | Where-Object { -not $_.result.databaseFootprintAfterIndex.dbstatAvailable }).Count -eq 0
    $candidateDbstatAvailable = $candidateRows.Count -gt 0 -and @($candidateRows | Where-Object { -not $_.result.databaseFootprintAfterIndex.dbstatAvailable }).Count -eq 0
    $script:evidence.databaseFootprintAverages = [ordered]@{
        baselineDbstatAvailable = $baselineDbstatAvailable
        candidateDbstatAvailable = $candidateDbstatAvailable
        baselineMainBytes = Get-Average $baselineRows.ToArray() 'result.databaseFootprintAfterIndex.mainBytes'
        candidateMainBytes = Get-Average $candidateRows.ToArray() 'result.databaseFootprintAfterIndex.mainBytes'
        baselineWalBytes = Get-Average $baselineRows.ToArray() 'result.databaseFootprintAfterIndex.walBytes'
        candidateWalBytes = Get-Average $candidateRows.ToArray() 'result.databaseFootprintAfterIndex.walBytes'
        baselineFtsBytes = Get-AverageWhenAvailable $baselineRows.ToArray() 'result.databaseFootprintAfterIndex.ftsBytes'
        candidateFtsBytes = Get-AverageWhenAvailable $candidateRows.ToArray() 'result.databaseFootprintAfterIndex.ftsBytes'
        baselineAfterCloseMainBytes = Get-Average $baselineRows.ToArray() 'result.databaseFootprintAfterClose.mainBytes'
        candidateAfterCloseMainBytes = Get-Average $candidateRows.ToArray() 'result.databaseFootprintAfterClose.mainBytes'
        baselineAfterCloseWalBytes = Get-Average $baselineRows.ToArray() 'result.databaseFootprintAfterClose.walBytes'
        candidateAfterCloseWalBytes = Get-Average $candidateRows.ToArray() 'result.databaseFootprintAfterClose.walBytes'
        baselineAfterReopenMainBytes = Get-Average $baselineRows.ToArray() 'result.databaseFootprintAfterReopen.mainBytes'
        candidateAfterReopenMainBytes = Get-Average $candidateRows.ToArray() 'result.databaseFootprintAfterReopen.mainBytes'
        baselineAfterReopenWalBytes = Get-Average $baselineRows.ToArray() 'result.databaseFootprintAfterReopen.walBytes'
        candidateAfterReopenWalBytes = Get-Average $candidateRows.ToArray() 'result.databaseFootprintAfterReopen.walBytes'
        baselineAfterCheckpointMainBytes = Get-Average $baselineRows.ToArray() 'result.databaseFootprintAfterCheckpoint.mainBytes'
        candidateAfterCheckpointMainBytes = Get-Average $candidateRows.ToArray() 'result.databaseFootprintAfterCheckpoint.mainBytes'
        baselineAfterCheckpointWalBytes = Get-Average $baselineRows.ToArray() 'result.databaseFootprintAfterCheckpoint.walBytes'
        candidateAfterCheckpointWalBytes = Get-Average $candidateRows.ToArray() 'result.databaseFootprintAfterCheckpoint.walBytes'
    }
    $script:evidence.concurrentWriterAverages = [ordered]@{
        baselineFilesP95Us = Get-Average $baselineRows.ToArray() 'result.concurrentDatabaseWrites.files.p95Us'
        candidateFilesP95Us = Get-Average $candidateRows.ToArray() 'result.concurrentDatabaseWrites.files.p95Us'
        baselineFilesP99Us = Get-Average $baselineRows.ToArray() 'result.concurrentDatabaseWrites.files.p99Us'
        candidateFilesP99Us = Get-Average $candidateRows.ToArray() 'result.concurrentDatabaseWrites.files.p99Us'
        baselineFilesMaxUs = Get-Average $baselineRows.ToArray() 'result.concurrentDatabaseWrites.files.maxUs'
        candidateFilesMaxUs = Get-Average $candidateRows.ToArray() 'result.concurrentDatabaseWrites.files.maxUs'
        baselineFilesLockRetryAttempts = Get-Average $baselineRows.ToArray() 'result.concurrentDatabaseWrites.filesLockRetryAttempts'
        candidateFilesLockRetryAttempts = Get-Average $candidateRows.ToArray() 'result.concurrentDatabaseWrites.filesLockRetryAttempts'
        baselineJournalP95Us = Get-Average $baselineRows.ToArray() 'result.concurrentDatabaseWrites.journal.p95Us'
        candidateJournalP95Us = Get-Average $candidateRows.ToArray() 'result.concurrentDatabaseWrites.journal.p95Us'
        baselineSettingsP95Us = Get-Average $baselineRows.ToArray() 'result.concurrentDatabaseWrites.settings.p95Us'
        candidateSettingsP95Us = Get-Average $candidateRows.ToArray() 'result.concurrentDatabaseWrites.settings.p95Us'
        baselinePeakWalBytes = Get-Average $baselineRows.ToArray() 'result.concurrentDatabaseWrites.peakWalBytes'
        candidatePeakWalBytes = Get-Average $candidateRows.ToArray() 'result.concurrentDatabaseWrites.peakWalBytes'
        baselineBusyErrors = Get-Average $baselineRows.ToArray() 'result.concurrentDatabaseWrites.sqliteBusyErrors'
        candidateBusyErrors = Get-Average $candidateRows.ToArray() 'result.concurrentDatabaseWrites.sqliteBusyErrors'
    }
    $normalWriteP95LimitUs = 1000000
    $candidateConcurrentWritesSucceeded = @($candidateRows | Where-Object {
        [long]$_.result.concurrentDatabaseWrites.sqliteBusyErrors -gt 0 -or
        @($_.result.concurrentDatabaseWrites.errors).Count -gt 0
    }).Count -eq 0
    $candidateConcurrentWriteLatencyMet = @($candidateRows | Where-Object {
        [double]$_.result.concurrentDatabaseWrites.files.p95Us -gt $normalWriteP95LimitUs -or
        [double]$_.result.concurrentDatabaseWrites.journal.p95Us -gt $normalWriteP95LimitUs -or
        [double]$_.result.concurrentDatabaseWrites.settings.p95Us -gt $normalWriteP95LimitUs
    }).Count -eq 0
    $script:evidence.concurrentWritesTarget = [ordered]@{
        sqliteBusyErrors = $script:evidence.concurrentWriterAverages.candidateBusyErrors
        filesLockRetryAttempts = $script:evidence.concurrentWriterAverages.candidateFilesLockRetryAttempts
        p95LatencyLimitUs = $normalWriteP95LimitUs
        writesSucceededWithoutErrors = $candidateConcurrentWritesSucceeded
        p95LatencyWithinLimit = $candidateConcurrentWriteLatencyMet
        met = $candidateConcurrentWritesSucceeded -and $candidateConcurrentWriteLatencyMet
    }
    $script:evidence.phaseAverages = [ordered]@{
        baselineMftReadParseUs = Get-Average $baselineRows.ToArray() 'stages.mft.mft_read_parse_us'
        candidateMftReadParseUs = Get-Average $candidateRows.ToArray() 'stages.mft.mft_read_parse_us'
        baselinePathResolutionUs = Get-Average $baselineRows.ToArray() 'stages.mft.path_resolution_us'
        candidatePathResolutionUs = Get-Average $candidateRows.ToArray() 'stages.mft.path_resolution_us'
        baselineMftMetadataUs = Get-Average $baselineRows.ToArray() 'stages.mft.metadata_us'
        candidateMftMetadataUs = Get-Average $candidateRows.ToArray() 'stages.mft.metadata_us'
        baselineDeferredMetadataUs = Get-Average $baselineRows.ToArray() 'stages.writer.deferred_metadata_us'
        candidateDeferredMetadataUs = Get-Average $candidateRows.ToArray() 'stages.writer.deferred_metadata_us'
        baselineMetadataUpdateUs = Get-Average $baselineRows.ToArray() 'stages.writer.metadata_update_us'
        candidateMetadataUpdateUs = Get-Average $candidateRows.ToArray() 'stages.writer.metadata_update_us'
        baselineMetadataCommitUs = Get-Average $baselineRows.ToArray() 'stages.writer.metadata_commit_us'
        candidateMetadataCommitUs = Get-Average $candidateRows.ToArray() 'stages.writer.metadata_commit_us'
        baselineTransientStagingUs = Get-Average $baselineRows.ToArray() 'stages.writer.transient_staging_us'
        candidateTransientStagingUs = Get-Average $candidateRows.ToArray() 'stages.writer.transient_staging_us'
        baselineCoveragePolicyLoadUs = Get-Average $baselineRows.ToArray() 'stages.writer.coverage_policy_load_us'
        candidateCoveragePolicyLoadUs = Get-Average $candidateRows.ToArray() 'stages.writer.coverage_policy_load_us'
        baselineCoverageEvaluationUs = Get-Average $baselineRows.ToArray() 'stages.writer.coverage_evaluation_us'
        candidateCoverageEvaluationUs = Get-Average $candidateRows.ToArray() 'stages.writer.coverage_evaluation_us'
        baselineConnectionAcquisitionUs = Get-Average $baselineRows.ToArray() 'stages.writer.connection_acquisition_us'
        candidateConnectionAcquisitionUs = Get-Average $candidateRows.ToArray() 'stages.writer.connection_acquisition_us'
        baselineTransactionAcquisitionUs = Get-Average $baselineRows.ToArray() 'stages.writer.transaction_acquisition_us'
        candidateTransactionAcquisitionUs = Get-Average $candidateRows.ToArray() 'stages.writer.transaction_acquisition_us'
        baselineTransactionSetupUs = Get-Average $baselineRows.ToArray() 'stages.writer.transaction_setup_us'
        candidateTransactionSetupUs = Get-Average $candidateRows.ToArray() 'stages.writer.transaction_setup_us'
        baselineMaxWriterLockUs = Get-Average $baselineRows.ToArray() 'stages.writer.max_writer_lock_us'
        candidateMaxWriterLockUs = Get-Average $candidateRows.ToArray() 'stages.writer.max_writer_lock_us'
        baselineEntryUpsertUs = Get-Average $baselineRows.ToArray() 'stages.writer.entry_upsert_us'
        candidateEntryUpsertUs = Get-Average $candidateRows.ToArray() 'stages.writer.entry_upsert_us'
        baselineAiTaskQualificationUs = Get-Average $baselineRows.ToArray() 'stages.writer.ai_task_qualification_us'
        candidateAiTaskQualificationUs = Get-Average $candidateRows.ToArray() 'stages.writer.ai_task_qualification_us'
        baselineRecordDecisionsUs = Get-Average $baselineRows.ToArray() 'stages.writer.record_decisions_us'
        candidateRecordDecisionsUs = Get-Average $candidateRows.ToArray() 'stages.writer.record_decisions_us'
        baselineFtsBulkBuildUs = Get-Average $baselineRows.ToArray() 'stages.writer.fts_bulk_build_us'
        candidateFtsBulkBuildUs = Get-Average $candidateRows.ToArray() 'stages.writer.fts_bulk_build_us'
        baselineCommitUs = Get-Average $baselineRows.ToArray() 'stages.writer.commit_us'
        candidateCommitUs = Get-Average $candidateRows.ToArray() 'stages.writer.commit_us'
        baselineSqliteTransactionCount = Get-Average $baselineRows.ToArray() 'stages.writer.sqlite_transactions'
        candidateSqliteTransactionCount = Get-Average $candidateRows.ToArray() 'stages.writer.sqlite_transactions'
        baselineWalCheckpointUs = Get-Average $baselineRows.ToArray() 'stages.writer.wal_checkpoint_us'
        candidateWalCheckpointUs = Get-Average $candidateRows.ToArray() 'stages.writer.wal_checkpoint_us'
        sourceStatsTiming = "entry_count and source facts remain row-triggered so concurrent file changes stay exact; their cost is included in entry_upsert_us."
        ftsTiming = "Production FTS triggers stay enabled for existing sources and incremental changes. A fresh source defers only its own FTS inserts into restartable batches of at most 8192 rows, combined with bounded entry transactions during initial persistence; no global FTS rebuild is run."
    }
    $allCandidateFirstSearchTargetsMet = @($candidateRows | Where-Object {
        [double]$_.result.firstSearchableUs -gt $searchReadyTargetMicroseconds
    }).Count -eq 0
    if ($DiagnosticScenario -in @('B', 'D')) {
        $script:evidence.firstSearchTargetMet = $candidateFirst -le $searchReadyTargetMicroseconds -and $allCandidateFirstSearchTargetsMet
    } else {
        $script:evidence.firstSearchTargetMet = $null
    }
    $allCandidateCollectionsComplete = @($candidateRows | Where-Object { -not $_.result.indexComplete }).Count -eq 0
    if ($DiagnosticScenario -in @('B', 'D')) {
        $allCandidateCompleteCatalogsReady = @($candidateRows | Where-Object {
            $null -eq $_.result.transientCatalogReadyUs -or
            [double]$_.result.transientCatalogReadyUs -gt $searchReadyTargetMicroseconds -or
            [long]$_.result.transientFixtureFileEntries -ne $FileCount -or
            -not $_.result.completeTransientSearchVisible -or
            -not $_.result.completeTransientSearchBeforeDurableFts -or
            $_.result.collectionCompleteWhenTransientReady
        }).Count -eq 0
    } else {
        $allCandidateCompleteCatalogsReady = @($candidateRows | Where-Object {
            $null -eq $_.result.transientCatalogReadyUs -or
            [double]$_.result.transientCatalogReadyUs -gt $searchReadyTargetMicroseconds -or
            [long]$_.result.transientFixtureFileEntries -ne $FileCount
        }).Count -eq 0
    }
    $allCandidateDurableFilenameTargetsMet = @($candidateRows | Where-Object {
        [double]$_.result.durableFilenameIndexUs -gt $durableTargetMicroseconds -or
        [long]$_.result.fixtureFileEntries -ne $FileCount -or
        -not $_.result.ftsIntegrityVerified -or
        -not $_.result.restartedSearchVisible -or
        -not $_.result.restartedCompleteCoverageSearchVisible
    }).Count -eq 0
    $script:evidence.completeCoveragePerPath = [ordered]@{
        baseline = @($baselineRows | ForEach-Object { $_.result.indexComplete })
        candidate = @($candidateRows | ForEach-Object { $_.result.indexComplete })
    }
    $candidateCompleteFilenameSearchable = Get-Average $candidateRows.ToArray() 'result.transientCatalogReadyUs'
    $script:evidence.durableFilenameIndexTargetMet = $candidateDurableFilenameIndex -le $durableTargetMicroseconds -and $allCandidateDurableFilenameTargetsMet
    if ($DiagnosticScenario -in @('B', 'D')) {
        $script:evidence.completeFilenameSearchTargetMet = $candidateCompleteFilenameSearchable -le $searchReadyTargetMicroseconds -and $allCandidateCompleteCatalogsReady
        $script:evidence.fixtureFilenameIndexTargetMet = $script:evidence.completeFilenameSearchTargetMet
    } else {
        $script:evidence.completeFilenameSearchTargetMet = $null
        $script:evidence.fixtureFilenameIndexTargetMet = $null
        $script:evidence.diagnosticTransientCatalogTargetMet = $candidateCompleteFilenameSearchable -le $searchReadyTargetMicroseconds -and $allCandidateCompleteCatalogsReady
    }
    $allCandidateFullSourceTargetsMet = @($candidateRows | Where-Object {
        [double]$_.result.fullIndexUs -gt $fullSourceTargetMicroseconds
    }).Count -eq 0
    $script:evidence.fullIndexTargetMet = $candidateFull -le $fullSourceTargetMicroseconds -and $allCandidateFullSourceTargetsMet -and $allCandidateCollectionsComplete
    $script:evidence.fullSourceCompletionTargetMet = $script:evidence.fullIndexTargetMet
    $workingSetTargetBytes = 768 * 1024 * 1024
    $databaseAndWalTargetBytes = 2 * 1024 * 1024 * 1024
    $candidatePeakWorkingSet = Get-Average $candidateRows.ToArray() 'peakWorkingSetBytes'
    $candidateDatabaseAndWal = Get-Average $candidateRows.ToArray() 'result.peakDatabaseBytesWithWal'
    $allCandidateWorkingSetsMet = @($candidateRows | Where-Object {
        [double]$_.peakWorkingSetBytes -gt $workingSetTargetBytes
    }).Count -eq 0
    $allCandidateDatabaseSizesMet = @($candidateRows | Where-Object {
        [double]$_.result.peakDatabaseBytesWithWal -gt $databaseAndWalTargetBytes
    }).Count -eq 0
    $script:evidence.resourceTargets = [ordered]@{
        workingSetTargetBytes = $workingSetTargetBytes
        candidatePeakWorkingSetBytesAverage = $candidatePeakWorkingSet
        candidateWorkingSetMet = $candidatePeakWorkingSet -le $workingSetTargetBytes -and $allCandidateWorkingSetsMet
        databaseAndWalTargetBytes = $databaseAndWalTargetBytes
        candidatePeakDatabaseAndWalBytesAverage = $candidateDatabaseAndWal
        candidateDatabaseAndWalMet = $candidateDatabaseAndWal -le $databaseAndWalTargetBytes -and $allCandidateDatabaseSizesMet
    }
    $targetFailures = @()
    if ($DiagnosticScenario -in @('B', 'D') -and -not $script:evidence.firstSearchTargetMet) {
        $targetFailures += "first searchable result exceeded the $searchReadyTargetSeconds-second target"
    }
    if ($DiagnosticScenario -in @('B', 'D') -and -not $script:evidence.completeFilenameSearchTargetMet) {
        $targetFailures += "complete transient filename catalogue exceeded $searchReadyTargetSeconds seconds or failed fixture-count/search/progress checks"
    }
    if (-not $script:evidence.concurrentWritesTarget.met) {
        $targetFailures += "normal Files/Journal/Settings writes returned SQLITE_BUSY or exceeded the $normalWriteP95LimitUs-us p95 latency limit during indexing"
    }
    if ($targetFailures.Count -gt 0) {
        $script:evidence.status = 'PERFORMANCE_TARGET_MISSED'
        $script:evidence.performanceTargetMisses = $targetFailures
        if (-not $DiagnosticOnly) {
            $failure = $targetFailures -join '; '
            $script:evidence.failure = $failure
        }
    } elseif (-not $script:evidence.durableFilenameIndexTargetMet) {
        $script:evidence.status = "SEARCH_READY_TARGET_MET_BACKGROUND_FTS_OVER_${durableTargetSeconds}S"
    } elseif (-not $script:evidence.fullIndexTargetMet -or -not $script:evidence.resourceTargets.candidateWorkingSetMet -or -not $script:evidence.resourceTargets.candidateDatabaseAndWalMet) {
        $script:evidence.status = 'SEARCH_READY_TARGET_MET_FULL_INDEX_OR_RESOURCE_TARGET_PARTIAL'
    } else {
        $script:evidence.status = 'PASS'
    }
} catch {
    $failure = $_.Exception.ToString()
    $script:evidence.status = 'FAIL'
    $script:evidence.failure = $failure
} finally {
    try {
        Remove-TaskIndexService
        $script:evidence.cleanup.serviceStopped = $true
        $script:evidence.cleanup.serviceDeleted = $true
    } catch {
        $cleanupFailure = $true
        $script:evidence.cleanup.details += $_.Exception.ToString()
    }
    try {
        if (Test-Path -LiteralPath $taskRoot -PathType Container) {
            $traceFiles = @(Get-ChildItem -LiteralPath $taskRoot -Filter '*trace.log*' -File -ErrorAction SilentlyContinue)
            if ($traceFiles.Count -gt 0) {
                New-Item -ItemType Directory -Path $traceDirectory -Force | Out-Null
                foreach ($traceFile in $traceFiles) {
                    Copy-Item -LiteralPath $traceFile.FullName -Destination (Join-Path $traceDirectory $traceFile.Name) -Force
                }
            }
        }
        if (Test-Path -LiteralPath $vhdPath) {
            $owner = if (Test-Path -LiteralPath $vhdOwnerMarker) { (Get-Content -LiteralPath $vhdOwnerMarker -Raw).Trim() } else { $null }
            if (-not $owner -or -not $owner.Equals($vhdPath, [StringComparison]::OrdinalIgnoreCase)) {
                throw "refusing to detach a VHD without its matching task ownership marker: $vhdPath"
            }
            $script:vhdCreated = $true
            if ($fixtureRoot -and (Test-Path -LiteralPath $fixtureRoot)) {
                $fixtureDriveRoot = "$($driveLetter):\"
                if (-not $fixtureRoot.StartsWith($fixtureDriveRoot, [StringComparison]::OrdinalIgnoreCase)) {
                    throw "refusing cleanup outside the task-owned VHD: $fixtureRoot"
                }
                Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
            }
            if (Test-VhdAttached) {
                [void](Invoke-DiskPart 'detach-vhd' @("select vdisk file=`"$vhdPath`"", 'detach vdisk', 'exit'))
            }
            if (Test-VhdAttached) { throw 'task-owned VHD remains attached after detach' }
            Remove-Item -LiteralPath $vhdPath -Force
            if (Test-Path -LiteralPath $vhdPath) { throw 'task-owned VHD file remains after removal' }
            $vhdAttached = $false
            $script:evidence.cleanup.vhdDetached = $true
        } else {
            $script:evidence.cleanup.vhdDetached = $true
        }
        $script:evidence.cleanup.fixtureRemoved = $true
        if (-not $cleanupFailure -and (Test-Path -LiteralPath $taskRoot)) {
            if (-not $taskRoot.StartsWith($runnerTemp, [StringComparison]::OrdinalIgnoreCase)) {
                throw "refusing cleanup outside RUNNER_TEMP: $taskRoot"
            }
            Remove-Item -LiteralPath $taskRoot -Recurse -Force
        }
        $script:evidence.cleanup.taskRootRemoved = -not (Test-Path -LiteralPath $taskRoot)
    } catch {
        $cleanupFailure = $true
        $script:evidence.cleanup.details += $_.Exception.ToString()
    }
    $script:evidence.cleanup.vhdCreatedByTask = $vhdCreated
    $script:evidence.cleanup.vhdDetached = $script:evidence.cleanup.vhdDetached -and -not $vhdAttached
    New-Item -ItemType Directory -Path $evidenceDirectory -Force | Out-Null
    $script:evidence | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $EvidencePath -Encoding utf8NoBOM
}

if ($failure) { Write-Error $failure; exit 1 }
if ($cleanupFailure) { Write-Error 'task-owned NTFS benchmark cleanup did not fully complete'; exit 1 }
Write-Output "Windows real NTFS filename-first pair completed; evidence: $EvidencePath"
