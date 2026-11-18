param(
    [Parameter(Mandatory = $true)][string]$CandidateExe,
    [Parameter(Mandatory = $true)][string]$ProbeExe,
    [Parameter(Mandatory = $true)][string]$EvidencePath,
    [switch]$ResidentQualification,
    [switch]$DirectOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$serviceName = "FyluraGlobalIndex"
$runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
$taskId = "zb05-global-index-service-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT"
$taskRoot = [IO.Path]::GetFullPath((Join-Path $runnerTemp $taskId))
$profileRoot = Join-Path $taskRoot "profile"
$fixtureRootMarker = Join-Path $taskRoot "fixture-root-created-by-qualification.txt"
$qualificationVhdPath = Join-Path $taskRoot "qualification-volume.vhdx"
$qualificationVhdOwnerMarker = Join-Path $taskRoot "qualification-volume-created-by-task.txt"
$qualificationVhdLetterMarker = Join-Path $taskRoot "qualification-volume-drive-letter.txt"
$fixtureRoot = $null
$fixtureCleanupBoundary = $null
$fixtureVolume = $null
$fixtureDriveLetter = $null
$tracePath = Join-Path $taskRoot "global-index-trace.log"
$CandidateExe = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $CandidateExe).Path)
$ProbeExe = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $ProbeExe).Path)
$EvidencePath = [IO.Path]::GetFullPath($EvidencePath)

if (-not $taskRoot.StartsWith($runnerTemp, [StringComparison]::OrdinalIgnoreCase)) {
    throw "refusing to create qualification data outside RUNNER_TEMP: $taskRoot"
}
if (-not $CandidateExe.StartsWith([IO.Path]::GetFullPath($PWD.Path), [StringComparison]::OrdinalIgnoreCase)) {
    throw "candidate executable must be inside the exact checked-out source workspace: $CandidateExe"
}

$script:evidence = [ordered]@{
    recovery = $null
    sourceHead = $null
    sourceTree = (& git rev-parse 'HEAD^{tree}').Trim()
    profileRoot = $profileRoot
    rawTrace = @()
    residentSamples = @()
    serviceSamples = @()
    nativeMftTrace = @()
    qaHarnessOverlay = [Environment]::GetEnvironmentVariable("ZC_GLOBAL_INDEX_QA_HARNESS_OVERLAY")
    residentStartup = $null
    residentClassification = "UNVERIFIED"
    expectedSourceHead = $env:EXPECTED_SOURCE_SHA
    candidateExe = $CandidateExe
    candidateSha256 = $null
    runner = $null
    serviceName = $serviceName
    serviceImagePath = $null
    serviceCurrentExe = $null
    clientCurrentExe = $null
    sameImage = $false
    qualificationVolumeKind = "task-owned-vhdx"
    qualificationVolumeLetter = $null
    expectedQualificationSourceId = $null
    enabledSourceCountBeforeIndex = $null
    enabledSourceCountAfterDiscovery = $null
    qualificationVolumeJournalCreated = $false
    qualificationVolumeDetached = $false
    qualificationVhdRemoved = $false
    diskpartOperations = @()
    fixedNtfsSource = $null
    fixedNtfsMountPath = $null
    provider = $null
    sourceStatus = $null
    sourceLastError = $null
    baselineEntryCount = $null
    databaseTotalEntryCount = $null
    databaseActiveEntryCount = $null
    baselineWaitMs = $null
    directMftQualificationElapsedMs = $null
    fixtureRoot = $null
    fixtureVolume = $null
    usnJournalProbes = @()
    baselineFixturePath = $null
    baselineFixtureSearchFound = $false
    ipcBatchFixtureFileCount = 0
    serviceBatchingQualification = $null
    filenameFirstServiceModeMatrix = @()
    serviceModeAfterMatrix = $null
    independentManifest = $null
    crossDirectoryMove = $null
    createLatencyMs = $null
    renameLatencyMs = $null
    crossDirectoryMoveLatencyMs = $null
    deleteLatencyMs = $null
    settledIdleWindowMs = $null
    settledCoordinatorCycleDelta = $null
    settledCoordinatorWaitDelta = $null
    serviceRouteObserved = $false
    serviceControl = @()
    failure = $null
    cleanup = [ordered]@{
        serviceStopped = $false
        serviceDeleted = $false
        candidateProcessesTerminated = $false
        fixtureRootRemoved = $false
        qualificationVolumeDetached = $false
        taskProfileRemoved = $false
        details = @()
    }
}

$script:probeProcess = $null
$script:clientProcessId = $null
$script:serviceProcessId = $null
$script:serviceCreatedByTask = $false
$script:taskRootCreatedByTask = $false
$script:qualificationVhdCreated = $false
$script:qualificationVhdAttached = $false
$script:failureMessage = $null
$script:cleanupFailure = $false
$script:baselineStartedAt = $null
$script:serviceModeProbeSequence = 0

function Set-TaskServiceFilenameFirstMode([string]$Mode) {
    $registryPath = "HKLM:\SYSTEM\CurrentControlSet\Services\$serviceName"
    if ($Mode -eq 'unset') {
        Remove-ItemProperty -LiteralPath $registryPath -Name Environment -ErrorAction SilentlyContinue
        return
    }
    if ($Mode -notin @('enabled', 'disabled')) { throw "unsupported task service mode: $Mode" }
    $value = if ($Mode -eq 'enabled') { '1' } else { '0' }
    New-ItemProperty -LiteralPath $registryPath -Name Environment -PropertyType MultiString -Value @(
        "ZC_GLOBAL_INDEX_FILENAME_FIRST=$value"
    ) -Force | Out-Null
}

function Get-ExpectedFilenameFirstMode([string]$Mode) {
    switch ($Mode) {
        'enabled' { return $true }
        'disabled' { return $false }
        # This qualification runs on Windows, whose unset default is enabled.
        'unset' { return $true }
        default { throw "unsupported filename-first mode: $Mode" }
    }
}

function Start-TaskService {
    $start = Invoke-ServiceControl @('start', $serviceName)
    if ($start.ExitCode -ne 0) {
        throw "StartService failed; exact SCM output (exit=$($start.ExitCode)): $($start.Output)"
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    $service = $null
    do {
        $service = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
        if ($service -and $service.State -eq 'Running' -and $service.ProcessId -gt 0) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $service -or $service.State -ne 'Running' -or $service.ProcessId -le 0) {
        $state = if ($service) { "state=$($service.State) win32ExitCode=$($service.ExitCode) serviceSpecificExitCode=$($service.ServiceSpecificExitCode)" } else { 'service record unavailable' }
        throw "task-owned service did not reach Running within 45 seconds; $state"
    }
    $expectedImagePath = ('"' + $CandidateExe + '" --index-service')
    if ($service.PathName.Trim() -ine $expectedImagePath) {
        throw "task-owned service ImagePath mismatch: expected=$expectedImagePath actual=$($service.PathName)"
    }
    $serviceImage = Get-ProcessImage ([int]$service.ProcessId)
    if (-not $serviceImage.Equals($CandidateExe, [StringComparison]::OrdinalIgnoreCase)) {
        throw "task-owned service current_exe mismatch: expected=$CandidateExe actual=$serviceImage"
    }
    $script:serviceProcessId = [int]$service.ProcessId
    return $service
}

function Stop-TaskService {
    $service = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
    if (-not $service -or $service.State -eq 'Stopped') { return }
    $stop = Invoke-ServiceControl @('stop', $serviceName)
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    do {
        Start-Sleep -Milliseconds 250
        $service = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
    } while ($service -and $service.State -ne 'Stopped' -and [DateTime]::UtcNow -lt $deadline)
    if ($service -and $service.State -ne 'Stopped') {
        throw "task-owned service did not stop for mode change: scExit=$($stop.ExitCode) output=$($stop.Output)"
    }
}

function Invoke-ServiceModeProbe([string]$DesktopMode, [string]$Label, [string]$Command = 'status') {
    $script:serviceModeProbeSequence++
    $reportName = 'filename-first-service-mode-{0:D2}-{1}-{2}.json' -f $script:serviceModeProbeSequence, $Label, $Command
    $reportPath = Join-Path $taskRoot $reportName
    if ($DesktopMode -notin @('enabled', 'disabled', 'unset')) { throw "unsupported desktop service mode: $DesktopMode" }
    $lastFailure = $null
    for ($attempt = 1; $attempt -le 20; $attempt++) {
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $CandidateExe
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.ArgumentList.Add('--global-index-filename-first-service-mode-qa')
        $startInfo.ArgumentList.Add($reportPath)
        $startInfo.ArgumentList.Add($profileRoot)
        $startInfo.ArgumentList.Add($Command)
        $startInfo.Environment['RUNNER_TEMP'] = $runnerTemp.TrimEnd('\')
        if ($DesktopMode -eq 'enabled') {
            $startInfo.Environment['ZC_GLOBAL_INDEX_FILENAME_FIRST'] = '1'
        } elseif ($DesktopMode -eq 'disabled') {
            $startInfo.Environment['ZC_GLOBAL_INDEX_FILENAME_FIRST'] = '0'
        } else {
            $startInfo.Environment.Remove('ZC_GLOBAL_INDEX_FILENAME_FIRST') | Out-Null
        }
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) { throw "failed to start installed-service mode probe: $Label" }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            $process.Kill($true)
            throw "installed-service mode probe timed out: $Label"
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -eq 0 -and (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
            $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
            $report | Add-Member -NotePropertyName desktopProcessId -NotePropertyValue ([long]$process.Id)
            return $report
        }
        $lastFailure = "installed-service mode probe failed: label=$Label attempt=$attempt exit=$($process.ExitCode) stdout=$stdout stderr=$stderr"
        Start-Sleep -Milliseconds 250
    }
    throw $lastFailure
}

function Test-InstalledServiceFilenameFirstModeMatrix {
    $machineSetting = [Environment]::GetEnvironmentVariable('ZC_GLOBAL_INDEX_FILENAME_FIRST', 'Machine')
    if (-not [string]::IsNullOrEmpty($machineSetting)) {
        throw "host machine environment sets ZC_GLOBAL_INDEX_FILENAME_FIRST; cannot prove an unset SCM service: $machineSetting"
    }
    $cases = @(
        [pscustomobject]@{ name = 'both_unset'; service = 'unset'; desktop = 'unset'; expectedOk = $true },
        [pscustomobject]@{ name = 'both_enabled'; service = 'enabled'; desktop = 'enabled'; expectedOk = $true },
        [pscustomobject]@{ name = 'both_disabled'; service = 'disabled'; desktop = 'disabled'; expectedOk = $true },
        [pscustomobject]@{ name = 'desktop_enabled_service_disabled'; service = 'disabled'; desktop = 'enabled'; expectedOk = $true },
        [pscustomobject]@{ name = 'desktop_disabled_service_enabled'; service = 'enabled'; desktop = 'disabled'; expectedOk = $true },
        [pscustomobject]@{ name = 'desktop_unset_service_disabled'; service = 'disabled'; desktop = 'unset'; expectedOk = $true },
        [pscustomobject]@{ name = 'desktop_disabled_service_unset'; service = 'unset'; desktop = 'disabled'; expectedOk = $true }
    )
    $results = [Collections.Generic.List[object]]::new()
    Stop-TaskService
    foreach ($case in $cases) {
        Set-TaskServiceFilenameFirstMode $case.service
        $caseService = Start-TaskService
        $probe = Invoke-ServiceModeProbe $case.desktop $case.name
        $expectedDesktopMode = Get-ExpectedFilenameFirstMode $case.desktop
        if ([bool]$probe.desktopFilenameFirstEnabled -ne [bool]$expectedDesktopMode) {
            throw "desktop filename-first mode was not captured as expected for $($case.name): $($probe | ConvertTo-Json -Compress)"
        }
        if ([bool]$probe.responseOk -ne [bool]$case.expectedOk) {
            throw "installed service mode response mismatch for $($case.name): $($probe | ConvertTo-Json -Compress)"
        }
        if ($case.expectedOk -and $probe.errorCode) {
            throw "matching installed service mode returned an error for $($case.name): $($probe | ConvertTo-Json -Compress)"
        }
        if (-not $case.expectedOk -and $probe.errorCode -ne 'index_filename_first_config_mismatch') {
            throw "mismatched installed service mode was not rejected explicitly for $($case.name): $($probe | ConvertTo-Json -Compress)"
        }
        $results.Add([ordered]@{
            name = $case.name
            serviceMode = $case.service
            desktopMode = $case.desktop
            desktopFilenameFirstEnabled = [bool]$probe.desktopFilenameFirstEnabled
            responseOk = [bool]$probe.responseOk
            errorCode = $probe.errorCode
            serviceStatus = $probe.serviceStatus
            desktopProcessId = [long]$probe.desktopProcessId
            serviceProcessId = [long]$script:serviceProcessId
            serviceImagePath = $caseService.PathName
            pauseResponseOk = $null
            pauseStatus = $null
            indexCommandRejections = @()
        })
        $caseEvidence = $results[$results.Count - 1]
        if ($case.service -ne $case.desktop) {
            $pauseProbe = Invoke-ServiceModeProbe $case.desktop $case.name 'pause'
            if (-not $pauseProbe.responseOk -or $pauseProbe.errorCode -or $pauseProbe.serviceStatus -ne 'paused') {
                throw "Pause failed despite the filename-first mode mismatch for $($case.name): $($pauseProbe | ConvertTo-Json -Compress)"
            }
            $caseEvidence.pauseResponseOk = [bool]$pauseProbe.responseOk
            $caseEvidence.pauseStatus = $pauseProbe.serviceStatus

            $indexResults = [Collections.Generic.List[object]]::new()
            foreach ($command in @('start_initial_index', 'rebuild', 'resume_incremental_sync')) {
                $beforeStatus = Invoke-ServiceModeProbe $case.desktop $case.name 'status'
                if (-not $beforeStatus.responseOk -or $beforeStatus.serviceStatus -ne 'paused') {
                    throw "mismatched Status query failed or service state changed before $command ($($case.name)): $($beforeStatus | ConvertTo-Json -Compress)"
                }

                $indexProbe = Invoke-ServiceModeProbe $case.desktop $case.name $command
                if ($indexProbe.responseOk -or $indexProbe.errorCode -ne 'index_filename_first_config_mismatch') {
                    throw "index command was not rejected for the filename-first mode mismatch ($($case.name), $command): $($indexProbe | ConvertTo-Json -Compress)"
                }
                if (-not $indexProbe.sideEffectFree) {
                    throw "rejected index command changed task-owned source/index data ($($case.name), $command): $($indexProbe | ConvertTo-Json -Compress)"
                }

                $afterStatus = Invoke-ServiceModeProbe $case.desktop $case.name 'status'
                if (-not $afterStatus.responseOk -or $afterStatus.serviceStatus -ne 'paused') {
                    throw "rejected index command changed service state ($($case.name), $command): $($afterStatus | ConvertTo-Json -Compress)"
                }
                $indexResults.Add([ordered]@{
                    command = $command
                    responseOk = [bool]$indexProbe.responseOk
                    errorCode = $indexProbe.errorCode
                    sideEffectFree = [bool]$indexProbe.sideEffectFree
                    entryCountBefore = [long]$indexProbe.entryCountBefore
                    entryCountAfter = [long]$indexProbe.entryCountAfter
                    activeEntryCountBefore = [long]$indexProbe.activeEntryCountBefore
                    activeEntryCountAfter = [long]$indexProbe.activeEntryCountAfter
                    sourceSnapshotUnchanged = [bool]$indexProbe.sourceSnapshotUnchanged
                    serviceStatusBefore = $beforeStatus.serviceStatus
                    serviceStatusAfter = $afterStatus.serviceStatus
                })
            }
            $caseEvidence.indexCommandRejections = @($indexResults.ToArray())
        }
        Stop-TaskService
    }
    Set-TaskServiceFilenameFirstMode 'unset'
    $service = Start-TaskService
    return [pscustomobject]@{ results = @($results.ToArray()); service = $service }
}

function Invoke-ServiceControl([string[]]$Arguments) {
    $output = (& sc.exe @Arguments 2>&1 | Out-String).Trim()
    $exitCode = $LASTEXITCODE
    $script:evidence.serviceControl += [ordered]@{
        arguments = @($Arguments)
        exitCode = $exitCode
        output = $output
    }
    return [pscustomobject]@{ ExitCode = $exitCode; Output = $output }
}

function Invoke-DiskPartScript([string]$Name, [string[]]$Commands) {
    $scriptPath = Join-Path $taskRoot "diskpart-$Name.txt"
    Set-Content -LiteralPath $scriptPath -Value $Commands -Encoding ascii
    $output = (& diskpart.exe /s $scriptPath 2>&1 | Out-String).Trim()
    $exitCode = $LASTEXITCODE
    $script:evidence.diskpartOperations += [ordered]@{
        name = $Name
        exitCode = $exitCode
        output = $output
    }
    if ($exitCode -ne 0 -or $output -match '(?im)(DiskPart has encountered an error|Virtual Disk Service error|The system failed to|could not be)') {
        throw "DiskPart $Name failed (exit=$exitCode): $output"
    }
    return $output
}

function Test-QualificationVhdAttached {
    if (-not (Test-Path -LiteralPath $qualificationVhdPath)) { return $false }
    if (-not (Get-Command Get-DiskImage -ErrorAction SilentlyContinue)) {
        throw "Get-DiskImage is unavailable; refusing to infer task VHD attachment state"
    }
    $image = Get-DiskImage -ImagePath $qualificationVhdPath -ErrorAction Stop
    return [bool]$image.Attached
}

function Get-FreeQualificationDriveLetter {
    $usedLetters = @([IO.DriveInfo]::GetDrives() | ForEach-Object {
        $_.Name.Substring(0, 1).ToUpperInvariant()
    })
    foreach ($candidate in @('Z', 'Y', 'X', 'W', 'V', 'U', 'T', 'S', 'R', 'Q', 'P', 'O', 'N', 'M', 'L', 'K', 'J', 'I', 'H', 'G', 'F', 'E')) {
        if ($usedLetters -notcontains $candidate) { return $candidate }
    }
    throw "no unused drive letter is available for the task-owned qualification VHD"
}

function New-QualificationVolume {
    if (Test-Path -LiteralPath $qualificationVhdPath) {
        throw "refusing to replace a pre-existing qualification VHD: $qualificationVhdPath"
    }
    $script:fixtureDriveLetter = Get-FreeQualificationDriveLetter
    $script:evidence.qualificationVolumeLetter = "$($script:fixtureDriveLetter):"
    $script:evidence.fixtureVolume = $script:evidence.qualificationVolumeLetter

    try {
        [void](Invoke-DiskPartScript "create-vhd" @(
            "create vdisk file=`"$qualificationVhdPath`" maximum=512 type=expandable",
            "exit"
        ))
    } finally {
        if (Test-Path -LiteralPath $qualificationVhdPath) {
            $script:qualificationVhdCreated = $true
            Set-Content -LiteralPath $qualificationVhdOwnerMarker -Value $qualificationVhdPath -NoNewline
        }
    }
    if (-not (Test-Path -LiteralPath $qualificationVhdPath)) {
        throw "DiskPart reported VHD creation without producing the task-owned image"
    }

    try {
        [void](Invoke-DiskPartScript "format-vhd" @(
            "select vdisk file=`"$qualificationVhdPath`"",
            "attach vdisk",
            "create partition primary",
            "format fs=ntfs quick label=ZB05QA",
            "assign letter=$($script:fixtureDriveLetter)",
            "exit"
        ))
    } finally {
        if (Test-QualificationVhdAttached) { $script:qualificationVhdAttached = $true }
    }
    if (-not $script:qualificationVhdAttached) {
        throw "task-owned qualification VHD did not remain attached after formatting"
    }

    $script:fixtureVolume = "$($script:fixtureDriveLetter):"
    $fixtureDriveRoot = "$($script:fixtureVolume)\"
    $logicalDisk = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='$($script:fixtureVolume)'" -ErrorAction SilentlyContinue
    if ($null -eq $logicalDisk -or $logicalDisk.FileSystem -ine "NTFS" -or $logicalDisk.DriveType -ne 3) {
        $filesystem = if ($logicalDisk) { $logicalDisk.FileSystem } else { "missing" }
        $driveType = if ($logicalDisk) { $logicalDisk.DriveType } else { "missing" }
        throw "qualification VHD is not a mounted fixed NTFS volume: drive=$($script:fixtureVolume) filesystem=$filesystem driveType=$driveType"
    }
    Set-Content -LiteralPath $qualificationVhdLetterMarker -Value $script:fixtureDriveLetter -NoNewline

    $journalBefore = (& fsutil.exe usn queryjournal $script:fixtureVolume 2>&1 | Out-String).Trim()
    $journalBeforeExitCode = $LASTEXITCODE
    $journalCreateOutput = $null
    $journalCreateExitCode = $null
    if ($journalBeforeExitCode -ne 0) {
        $journalCreateOutput = (& fsutil.exe usn createjournal m=16777216 a=2097152 $script:fixtureVolume 2>&1 | Out-String).Trim()
        $journalCreateExitCode = $LASTEXITCODE
        if ($journalCreateExitCode -ne 0) {
            throw "could not create a USN journal on the task-owned qualification volume: $journalCreateOutput"
        }
        $script:evidence.qualificationVolumeJournalCreated = $true
    }
    $journalAfter = (& fsutil.exe usn queryjournal $script:fixtureVolume 2>&1 | Out-String).Trim()
    $journalAfterExitCode = $LASTEXITCODE
    $script:evidence.usnJournalProbes += [ordered]@{
        volume = $script:fixtureVolume
        filesystem = $logicalDisk.FileSystem
        driveType = $logicalDisk.DriveType
        queryBeforeExitCode = $journalBeforeExitCode
        queryBeforeOutput = $journalBefore
        createExitCode = $journalCreateExitCode
        createOutput = $journalCreateOutput
        queryAfterExitCode = $journalAfterExitCode
        queryAfterOutput = $journalAfter
    }
    if ($journalAfterExitCode -ne 0) {
        throw "task-owned qualification volume does not have an active USN journal: $journalAfter"
    }

    $script:fixtureRoot = [IO.Path]::GetFullPath((Join-Path $fixtureDriveRoot "fixtures"))
    $script:fixtureCleanupBoundary = $fixtureDriveRoot
    if (-not $script:fixtureRoot.StartsWith($script:fixtureCleanupBoundary, [StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing to create qualification fixture outside the task-owned VHD volume: $($script:fixtureRoot)"
    }
    $script:evidence.fixtureRoot = $script:fixtureRoot
    Set-Content -LiteralPath $fixtureRootMarker -Value $script:fixtureRoot -NoNewline
    New-Item -ItemType Directory -Path $script:fixtureRoot -Force | Out-Null
}

function Initialize-QualificationProfile {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $ProbeExe
    $startInfo.WorkingDirectory = (Get-Location).Path
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.ArgumentList.Add("--prepare-profile")
    $startInfo.ArgumentList.Add($profileRoot)
    $startInfo.ArgumentList.Add($script:fixtureVolume)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to prepare the isolated qualification profile with $ProbeExe" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(30000)) {
        $process.Kill($true)
        $process.WaitForExit(5000)
        throw "qualification profile source isolation did not complete within 30 seconds"
    }
    $output = $stdoutTask.Result.Trim()
    $errorOutput = $stderrTask.Result.Trim()
    if ($process.ExitCode -ne 0) {
        throw "qualification profile source isolation failed (exit=$($process.ExitCode)): $errorOutput"
    }
    $prepared = $output | ConvertFrom-Json
    if (-not $prepared.prepared -or $prepared.enabledSourceCount -ne 1 -or -not $prepared.targetMountPath.Equals("$($script:fixtureVolume)\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "qualification profile did not isolate exactly the task-owned source: $output"
    }
    $script:evidence.enabledSourceCountBeforeIndex = [int]$prepared.enabledSourceCount
    $script:evidence.expectedQualificationSourceId = [string]$prepared.targetSourceId
}

function Get-TraceFiles {
    $files = [System.Collections.Generic.List[System.IO.FileInfo]]::new()
    if (Test-Path -LiteralPath $tracePath -PathType Leaf) {
        $files.Add((Get-Item -LiteralPath $tracePath))
    }
    foreach ($file in (Get-ChildItem -Path "${tracePath}.pid-*.log" -File -ErrorAction SilentlyContinue)) {
        $files.Add($file)
    }
    return $files.ToArray()
}

function Get-TraceLines {
    $lines = [System.Collections.Generic.List[string]]::new()
    foreach ($file in (Get-TraceFiles)) {
        foreach ($line in (Get-Content -LiteralPath $file.FullName -ErrorAction SilentlyContinue)) {
            $lines.Add($line)
        }
    }
    return @($lines.ToArray() | Sort-Object {
        if ($_ -match '^timestamp_us=(\d+)') { [long]$Matches[1] } else { 0L }
    })
}

function Clear-TraceFiles {
    foreach ($file in (Get-TraceFiles)) {
        Clear-Content -LiteralPath $file.FullName -ErrorAction SilentlyContinue
    }
}

function Get-TraceCounts {
    $lines = Get-TraceLines
    $lastCoordinatorLine = $lines | Where-Object {
        $_ -match '(?:^|\s)coordinator_cycle$' -or $_ -match '(?:^|\s)coordinator_wait$'
    } | Select-Object -Last 1
    $lastCoordinatorEvent = if ($lastCoordinatorLine -match '(?:^|\s)coordinator_wait$') {
        'coordinator_wait'
    } elseif ($lastCoordinatorLine -match '(?:^|\s)coordinator_cycle$') {
        'coordinator_cycle'
    } else {
        $null
    }
    return [pscustomobject]@{
        Lines = $lines
        Cycles = @($lines | Where-Object { $_ -match '(?:^|\s)coordinator_cycle$' }).Count
        Waits = @($lines | Where-Object { $_ -match '(?:^|\s)coordinator_wait$' }).Count
        ServiceRoutes = @($lines | Where-Object { $_ -match '(?:^|\s)windows_service_route_ok$' }).Count
        LastCoordinatorEvent = $lastCoordinatorEvent
    }
}

function Start-QualificationProbe {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $ProbeExe
    $startInfo.WorkingDirectory = (Get-Location).Path
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to start QA probe: $ProbeExe" }
    $process.BeginErrorReadLine()
    $script:probeProcess = $process
}

function Get-ProbeSnapshot([string]$Query = "", [int]$Limit = 25, [int]$Offset = 0, [bool]$IncludeIndexFacts = $false) {
    if ($null -eq $script:probeProcess -or $script:probeProcess.HasExited) {
        $exitCode = if ($null -eq $script:probeProcess) { "not-started" } else { $script:probeProcess.ExitCode }
        throw "read-only Global Index QA probe is not running (exit=$exitCode)"
    }
    $request = @{ query = $Query; limit = $Limit; offset = $Offset; includeIndexFacts = $IncludeIndexFacts } | ConvertTo-Json -Compress
    $script:probeProcess.StandardInput.WriteLine($request)
    $script:probeProcess.StandardInput.Flush()
    $responseTask = $script:probeProcess.StandardOutput.ReadLineAsync()
    if (-not $responseTask.Wait([TimeSpan]::FromSeconds(15))) {
        throw "Global Index QA probe did not respond within 15 seconds"
    }
    $line = $responseTask.Result
    if ([string]::IsNullOrWhiteSpace($line)) {
        throw "Global Index QA probe returned no response"
    }
    return $line | ConvertFrom-Json
}

function Test-SearchPath([string]$Query, [string]$Path) {
    $snapshot = Get-ProbeSnapshot $Query
    return @($snapshot.results | Where-Object {
        $_.path.Equals($Path, [StringComparison]::OrdinalIgnoreCase)
    }).Count -gt 0
}

function Wait-ForSearchPath([string]$Query, [string]$Path, [bool]$Present, [int]$TimeoutSeconds, [string]$Label, [Diagnostics.Stopwatch]$Timer = $null) {
    $timer = if ($null -eq $Timer) { [Diagnostics.Stopwatch]::StartNew() } else { $Timer }
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $found = Test-SearchPath $Query $Path
        if ($found -eq $Present) {
            $timer.Stop()
            return [long]$timer.Elapsed.TotalMilliseconds
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "$Label did not reach expected search state within $TimeoutSeconds seconds (expectedPresent=$Present path=$Path)"
}

function Get-ProcessImage([int]$ProcessId) {
    $process = Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId" -ErrorAction Stop
    if ($null -eq $process -or [string]::IsNullOrWhiteSpace($process.ExecutablePath)) {
        throw "cannot read ExecutablePath for process $ProcessId"
    }
    return [IO.Path]::GetFullPath($process.ExecutablePath)
}

function Stop-Probe {
    if ($null -eq $script:probeProcess) { return }
    try {
        if (-not $script:probeProcess.HasExited) {
            $script:probeProcess.StandardInput.Close()
            if (-not $script:probeProcess.WaitForExit(5000)) {
                $script:probeProcess.Kill($true)
                $script:probeProcess.WaitForExit(5000)
            }
        }
        $script:evidence.cleanup.details += "QA probe stopped"
    } catch {
        $script:cleanupFailure = $true
        $script:evidence.cleanup.details += "QA probe cleanup failed: $($_.Exception.Message)"
    }
}

try {
    $sourceHead = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse HEAD failed with exit code $LASTEXITCODE" }
    $script:evidence.sourceHead = $sourceHead
    if ($sourceHead -cne $env:EXPECTED_SOURCE_SHA) {
        throw "source HEAD mismatch: expected=$env:EXPECTED_SOURCE_SHA actual=$sourceHead"
    }
    $os = Get-CimInstance Win32_OperatingSystem
    $script:evidence.runner = [ordered]@{
        caption = $os.Caption
        version = $os.Version
        build = $os.BuildNumber
        architecture = $os.OSArchitecture
        runId = $env:GITHUB_RUN_ID
        runAttempt = $env:GITHUB_RUN_ATTEMPT
    }
    $script:evidence.candidateSha256 = (Get-FileHash -LiteralPath $CandidateExe -Algorithm SHA256).Hash

    $preexistingService = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
    if ($preexistingService) {
        throw "refusing to replace or manage a pre-existing $serviceName service: $($preexistingService.PathName)"
    }

    if (Test-Path -LiteralPath $taskRoot) {
        throw "refusing to reuse an existing qualification task root: $taskRoot"
    }
    New-Item -ItemType Directory -Path $profileRoot -Force | Out-Null
    $script:taskRootCreatedByTask = $true
    New-QualificationVolume

    $preexistingToken = "zb05qapreexisting$([Guid]::NewGuid().ToString('N'))"
    $preexistingName = "$preexistingToken.txt"
    $preexistingPath = Join-Path $fixtureRoot $preexistingName
    New-Item -ItemType File -Path $preexistingPath | Out-Null
    Set-Content -LiteralPath $preexistingPath -Value "created before the Global Index baseline" -NoNewline
    $script:evidence.baselineFixturePath = $preexistingPath

    # Smart Lean fixtures are independent of the existing manifest oracle.
    # Every path lives on the already-validated disposable NTFS VHD.
    $smartToken = "zcsmartlean$([Guid]::NewGuid().ToString('N'))"
    $smartProject = Join-Path $fixtureRoot "smart-lean-project"
    $smartLib = Join-Path $smartProject "node_modules\lib"
    $smartSource = Join-Path $smartProject "src"
    New-Item -ItemType Directory -Path $smartLib, $smartSource -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $smartProject "package.json") -Value '{}' -NoNewline
    Set-Content -LiteralPath (Join-Path $smartProject "node_modules\.package-lock.json") -Value '{}' -NoNewline
    $smartExcluded = Join-Path $smartLib "$smartToken-dependency.js"
    $smartIncluded = Join-Path $smartSource "$smartToken-source.ts"
    Set-Content -LiteralPath $smartExcluded -Value 'task-owned dependency' -NoNewline
    Set-Content -LiteralPath $smartIncluded -Value 'task-owned source' -NoNewline

    $manifestDirectory = Join-Path $fixtureRoot "manifest"
    $manifestFiles = @()
    $ipcBatchFixtureFileCount = 800
    $script:evidence.ipcBatchFixtureFileCount = $ipcBatchFixtureFileCount
    for ($manifestIndex = 0; $manifestIndex -lt $ipcBatchFixtureFileCount; $manifestIndex++) {
        $bucket = Join-Path $manifestDirectory ("bucket-{0:D2}" -f ($manifestIndex % 16))
        New-Item -ItemType Directory -Path $bucket -Force | Out-Null
        $manifestPath = Join-Path $bucket ("$preexistingToken-manifest-{0:D4}.txt" -f $manifestIndex)
        Set-Content -LiteralPath $manifestPath -Value "independent manifest fixture $manifestIndex" -NoNewline
        $manifestFiles += $manifestPath
    }
    $moveSourceDirectory = Join-Path $fixtureRoot "moves\source"
    $moveTargetDirectory = Join-Path $fixtureRoot "moves\destination"
    New-Item -ItemType Directory -Path $moveSourceDirectory -Force | Out-Null
    New-Item -ItemType Directory -Path $moveTargetDirectory -Force | Out-Null
    $manifestFiles += $preexistingPath
    $script:evidence.independentManifest = [ordered]@{
        expectedFileCount = $manifestFiles.Count
        expectedRelativePaths = @($manifestFiles | ForEach-Object {
            [IO.Path]::GetRelativePath($fixtureRoot, $_).Replace('/', '\')
        } | Sort-Object)
        indexedRelativePaths = @()
        missingRelativePaths = @()
        unexpectedRelativePaths = @()
        matchedAfterServiceRecovery = $false
    }

    $env:ZC_NATIVE_QA_PROFILE_ROOT = $profileRoot
    $env:ZC_GLOBAL_INDEX_QA_TRACE = $tracePath
    $env:APPDATA = Join-Path $taskRoot "appdata-roaming"
    $env:LOCALAPPDATA = Join-Path $taskRoot "appdata-local"
    $env:TEMP = Join-Path $taskRoot "temp"
    $env:TMP = $env:TEMP
    New-Item -ItemType Directory -Path $env:APPDATA -Force | Out-Null
    New-Item -ItemType Directory -Path $env:LOCALAPPDATA -Force | Out-Null
    New-Item -ItemType Directory -Path $env:TEMP -Force | Out-Null

    Initialize-QualificationProfile

    $directProfile = Join-Path $taskRoot "smart-lean-direct-profile"
    & $ProbeExe --prepare-profile $directProfile $script:fixtureVolume | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Smart Lean Direct profile preparation failed" }
    $nativeTempRoot = Join-Path $script:fixtureVolume "smart-lean-native-temp"
    New-Item -ItemType Directory -Path $nativeTempRoot -Force | Out-Null
    $previousTemp = $env:TEMP
    $previousTmp = $env:TMP
    $directTimer = [Diagnostics.Stopwatch]::StartNew()
    try {
        $env:TEMP = $nativeTempRoot
        $env:TMP = $nativeTempRoot
        $directJson = & $ProbeExe --qualify-smart-lean-direct $directProfile $script:evidence.expectedQualificationSourceId $smartIncluded $smartExcluded
    } finally {
        $directTimer.Stop()
        $script:evidence.directMftQualificationElapsedMs = [long]$directTimer.Elapsed.TotalMilliseconds
        $env:TEMP = $previousTemp
        $env:TMP = $previousTmp
    }
    if ($LASTEXITCODE -ne 0) { throw "Smart Lean Direct native qualification failed" }
    $script:evidence.smartLeanDirect = $directJson | ConvertFrom-Json
    $nativeMftTrace = @(Get-TraceLines)
    if ($nativeMftTrace.Count -gt 0) {
        $script:evidence.nativeMftTrace = $nativeMftTrace
        $retainedEvidenceDirectory = Split-Path -Parent $EvidencePath
        New-Item -ItemType Directory -Path $retainedEvidenceDirectory -Force | Out-Null
        $nativeMftTrace | Set-Content -LiteralPath (Join-Path $retainedEvidenceDirectory "mft-initial-index-timing.log")
    }
    if (-not $DirectOnly) {
    $serviceImagePath = '"' + $CandidateExe + '" --index-service'
    $create = Invoke-ServiceControl @("create", $serviceName, "binPath=", $serviceImagePath, "start=", "demand", "obj=", "LocalSystem")
    if ($create.ExitCode -ne 0) {
        throw "CreateService failed; exact SCM output (exit=$($create.ExitCode)): $($create.Output)"
    }
    $script:serviceCreatedByTask = $true
    Set-Content -LiteralPath (Join-Path $taskRoot "service-created-by-qualification.txt") -Value $CandidateExe -NoNewline

    $start = Invoke-ServiceControl @("start", $serviceName)
    if ($start.ExitCode -ne 0) {
        $failedService = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
        $status = if ($failedService) { "state=$($failedService.State) win32ExitCode=$($failedService.ExitCode) serviceSpecificExitCode=$($failedService.ServiceSpecificExitCode)" } else { "service record unavailable" }
        throw "StartService failed; exact SCM output (exit=$($start.ExitCode)); $status; output=$($start.Output)"
    }

    $serviceDeadline = [DateTime]::UtcNow.AddSeconds(45)
    $service = $null
    do {
        $service = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
        if ($service -and $service.State -eq "Running") { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $serviceDeadline)
    if (-not $service -or $service.State -ne "Running") {
        $state = if ($service) { "state=$($service.State) win32ExitCode=$($service.ExitCode) serviceSpecificExitCode=$($service.ServiceSpecificExitCode)" } else { "service record unavailable" }
        throw "service did not reach Running within 45 seconds; $state"
    }
    $script:serviceProcessId = [int]$service.ProcessId
    $script:evidence.serviceImagePath = $service.PathName
    $expectedServiceImagePath = ('"' + $CandidateExe + '" --index-service')
    if ($service.PathName.Trim() -ine $expectedServiceImagePath) {
        throw "service ImagePath mismatch: expected=$expectedServiceImagePath actual=$($service.PathName)"
    }
    $serviceCurrentExe = Get-ProcessImage $script:serviceProcessId
    $script:evidence.serviceCurrentExe = $serviceCurrentExe
    if (-not $serviceCurrentExe.Equals($CandidateExe, [StringComparison]::OrdinalIgnoreCase)) {
        throw "service current_exe mismatch: candidate=$CandidateExe service=$serviceCurrentExe"
    }

    $modeMatrix = Test-InstalledServiceFilenameFirstModeMatrix
    $service = $modeMatrix.service
    $script:evidence.filenameFirstServiceModeMatrix = $modeMatrix.results
    $script:serviceProcessId = [int]$service.ProcessId
    $script:evidence.serviceModeAfterMatrix = 'unset'

    # Same installed executable / actual v5 service pipe; no force-direct test.
    $recoveryReport = Join-Path $taskRoot "service-recovery.json"
    $recoveryClientMode = [Environment]::GetEnvironmentVariable('ZC_GLOBAL_INDEX_FILENAME_FIRST', 'Process')
    Remove-Item Env:ZC_GLOBAL_INDEX_FILENAME_FIRST -ErrorAction SilentlyContinue
    try {
        $recoveryProcess = Start-Process -FilePath $CandidateExe -ArgumentList @("--global-index-recovery-qa", $profileRoot, $script:evidence.expectedQualificationSourceId, $recoveryReport) -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $taskRoot "recovery-stderr.log") -RedirectStandardOutput (Join-Path $taskRoot "recovery-stdout.log")
    } finally {
        if ($null -eq $recoveryClientMode) { Remove-Item Env:ZC_GLOBAL_INDEX_FILENAME_FIRST -ErrorAction SilentlyContinue }
        else { $env:ZC_GLOBAL_INDEX_FILENAME_FIRST = $recoveryClientMode }
    }
    try {
        if (-not $recoveryProcess.WaitForExit(180000)) { throw "installed service recovery regression timed out" }
        if ($recoveryProcess.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $recoveryReport)) { throw "installed service recovery regression failed: $(Get-Content -LiteralPath (Join-Path $taskRoot 'recovery-stderr.log') -Raw)" }
        $script:evidence.recovery = Get-Content -LiteralPath $recoveryReport -Raw | ConvertFrom-Json
        $recoveryTrace = @(Get-TraceLines)
        # The instrumented Service exchange writes batching counters into the
        # recovery process trace. The background client's route trace above
        # only confirms routing and may be observed before the Service has
        # recorded its completed exchange counters.
        $batchingMetrics = @()
        foreach ($line in @($recoveryTrace | Where-Object { $_ -match '(?:^|\s)index_service_batching\s' })) {
            $metrics = [ordered]@{}
            $parts = $line -split '\s+'
            $eventIndex = [Array]::IndexOf($parts, 'index_service_batching')
            foreach ($pair in ($parts | Select-Object -Skip ($eventIndex + 1))) {
                $parts = $pair -split '=', 2
                if ($parts.Count -eq 2) { $metrics[$parts[0]] = [long]$parts[1] }
            }
            $batchingMetrics += [pscustomobject]$metrics
        }
        $ipcBatchEvidence = @($batchingMetrics | Where-Object {
            $_.input_entries -ge 600 -and $_.input_entries -le 1000
        } | Select-Object -Last 1)
        if ($ipcBatchEvidence.Count -ne 1) {
            throw "actual Service exchange did not report the 600-1000 entry task VHD baseline: metrics=$($batchingMetrics | ConvertTo-Json -Compress)"
        }
        $ipcBatchEvidence = $ipcBatchEvidence[0]
        if ($ipcBatchEvidence.entry_frames -le $ipcBatchEvidence.repository_batches -or
            $ipcBatchEvidence.repository_entries -ne $ipcBatchEvidence.input_entries -or
            $ipcBatchEvidence.max_repository_batch_entries -lt 512 -or
            $ipcBatchEvidence.checkpoints -lt 1 -or
            $ipcBatchEvidence.checkpoint_flushes -lt 1 -or
            $ipcBatchEvidence.checkpoint_flushed_entries -le 0) {
            throw "task VHD Service->Repository coalescing/checkpoint evidence is incomplete: $($ipcBatchEvidence | ConvertTo-Json -Compress)"
        }
        # QA trace lines have a timestamp and PID prefix, so anchor the event
        # token rather than the physical start of the line.
        $repositoryBatchPattern = "(?:^|\s)source_operation_write status=ready completed=true batches=$($ipcBatchEvidence.repository_batches)(\s|$)"
        if (@($recoveryTrace | Where-Object { $_ -match $repositoryBatchPattern }).Count -eq 0) {
            throw "Service exchange batch count was not confirmed by the completed repository write timing: expected_batches=$($ipcBatchEvidence.repository_batches)"
        }
        $script:evidence.serviceBatchingQualification = $ipcBatchEvidence
        if (@($recoveryTrace | Where-Object { $_ -match '(?:^|\s)windows_recovery_rebuild_admitted(?:\s|$)' }).Count -eq 0) { throw "recovery did not pass the existing Background admission gate" }
        $recoveryTrace | Set-Content -LiteralPath (Join-Path $taskRoot "recovery-trace.log")
        # Keep original background-client route/idle accounting independent.
        Clear-TraceFiles
        if (-not $script:evidence.recovery.pausedViaPipe -or -not $script:evidence.recovery.rebuildRequiredViaPipe -or $script:evidence.recovery.nextAdmittedRebuildCount -ne 1 -or $script:evidence.recovery.finalStatus -ne "ready") { throw "installed service recovery proof incomplete" }
        # Retain only the bounded recovery evidence outside the disposable task root.
        $retainedEvidenceDirectory = Split-Path -Parent $EvidencePath
        New-Item -ItemType Directory -Path $retainedEvidenceDirectory -Force | Out-Null
        Copy-Item -LiteralPath $recoveryReport -Destination (Join-Path $retainedEvidenceDirectory "service-recovery.json")
        Copy-Item -LiteralPath (Join-Path $taskRoot "recovery-trace.log") -Destination (Join-Path $retainedEvidenceDirectory "recovery-trace.log")
    } finally {
        if (-not $recoveryProcess.HasExited) { $recoveryProcess.Kill($true); $recoveryProcess.WaitForExit(5000) | Out-Null }
        # Preserve partial native-QA diagnostics even when the strict recovery
        # assertion fails. This does not change the qualification result.
        $retainedEvidenceDirectory = Split-Path -Parent $EvidencePath
        New-Item -ItemType Directory -Path $retainedEvidenceDirectory -Force | Out-Null
        if (Test-Path -LiteralPath $recoveryReport -PathType Leaf) {
            Copy-Item -LiteralPath $recoveryReport -Destination (Join-Path $retainedEvidenceDirectory "service-recovery.json") -Force
        }
        $recoveryTracePath = Join-Path $taskRoot "recovery-trace.log"
        $partialRecoveryTrace = @(Get-TraceLines)
        if (-not (Test-Path -LiteralPath $recoveryTracePath -PathType Leaf) -and $partialRecoveryTrace.Count -gt 0) {
            $partialRecoveryTrace | Set-Content -LiteralPath $recoveryTracePath
        }
        if (Test-Path -LiteralPath $recoveryTracePath -PathType Leaf) {
            Copy-Item -LiteralPath $recoveryTracePath -Destination (Join-Path $retainedEvidenceDirectory "recovery-trace.log") -Force
        }
        $recoveryProcess.Dispose()
    }

    $client = Start-Process -FilePath $CandidateExe -ArgumentList @("--background") -WorkingDirectory (Get-Location).Path -WindowStyle Hidden -RedirectStandardError (Join-Path $taskRoot "resident-stderr.log") -RedirectStandardOutput (Join-Path $taskRoot "resident-stdout.log") -PassThru
    $script:clientProcessId = $client.Id
    $clientCurrentExe = Get-ProcessImage $script:clientProcessId
    $script:evidence.clientCurrentExe = $clientCurrentExe
    if (-not $clientCurrentExe.Equals($CandidateExe, [StringComparison]::OrdinalIgnoreCase)) {
        throw "client image mismatch: candidate=$CandidateExe client=$clientCurrentExe"
    }
    $script:evidence.sameImage = $serviceCurrentExe.Equals($clientCurrentExe, [StringComparison]::OrdinalIgnoreCase)
    if (-not $script:evidence.sameImage) {
        throw "service/client physical image mismatch: service=$serviceCurrentExe client=$clientCurrentExe"
    }

    $coordinatorDeadline = [DateTime]::UtcNow.AddSeconds(90)
    do {
        if ($client.HasExited) { throw "background client exited during startup with code $($client.ExitCode)" }
        $trace = Get-TraceLines
        if (@($trace | Where-Object { $_ -match '(?:^|\s)coordinator_cycle$' }).Count -gt 0) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $coordinatorDeadline)
    if (@($trace | Where-Object { $_ -match '(?:^|\s)coordinator_cycle$' }).Count -eq 0) {
        throw "candidate coordinator did not start; no coordinator_cycle trace was recorded"
    }

    Start-QualificationProbe
    # The pre-seeded task profile enables only the disposable NTFS VHD source,
    # so this bounds an actual provider baseline without scanning runner C:.
    $baselineTimeoutMinutes = 5
    $script:baselineStartedAt = [DateTime]::UtcNow
    $baselineDeadline = $script:baselineStartedAt.AddMinutes($baselineTimeoutMinutes)
    $nextBaselineProgressSeconds = 60
    $baselineSource = $null
    $baselineSnapshot = $null
    do {
        if ($client.HasExited) { throw "background client exited before baseline completion with code $($client.ExitCode)" }
        $baselineSnapshot = Get-ProbeSnapshot
        if ($baselineSnapshot.volumes.Count -gt 0) {
            $enabledSources = @($baselineSnapshot.volumes | Where-Object { $_.enabled })
            $script:evidence.enabledSourceCountAfterDiscovery = $enabledSources.Count
            if ($enabledSources.Count -ne 1) {
                throw "candidate did not preserve the isolated task-owned source selection: enabled=$($enabledSources.Count) fixture=$($script:fixtureVolume)"
            }
            if (-not $enabledSources[0].mountPath.Equals("$($script:fixtureVolume)\", [StringComparison]::OrdinalIgnoreCase) -or $enabledSources[0].id -cne $script:evidence.expectedQualificationSourceId) {
                throw "candidate source identity differs from the pre-seeded task volume: enabledMount=$($enabledSources[0].mountPath) enabledId=$($enabledSources[0].id) expectedMount=$($script:fixtureVolume) expectedId=$($script:evidence.expectedQualificationSourceId)"
            }
        }
        $eligibleSources = @($baselineSnapshot.volumes | Where-Object {
            $_.enabled -and $_.filesystemType -ieq "NTFS" -and $_.driveKind -ieq "fixed" -and $_.provider -ceq "windows_mft_usn" -and $preexistingPath.StartsWith($_.mountPath, [StringComparison]::OrdinalIgnoreCase)
        })
        if ($eligibleSources.Count -gt 0) {
            $baselineSource = $eligibleSources | Sort-Object { $_.mountPath.Length } -Descending | Select-Object -First 1
            $script:evidence.fixedNtfsSource = $baselineSource.id
            $script:evidence.fixedNtfsMountPath = $baselineSource.mountPath
            $script:evidence.provider = $baselineSource.provider
            $script:evidence.sourceStatus = $baselineSource.status
            $script:evidence.sourceLastError = $baselineSource.lastError
            $script:evidence.baselineEntryCount = [long]$baselineSource.entryCount
            if ($baselineSource.status -in @("rebuild_required", "permission_required", "error", "unavailable")) {
                throw "fixed NTFS baseline did not complete truthfully: status=$($baselineSource.status) error=$($baselineSource.lastError) entry_count=$($baselineSource.entryCount)"
            }
            if ($baselineSource.status -eq "ready" -and $null -ne $baselineSource.lastFullIndexAt) { break }
        }
        $baselineElapsedSeconds = [int]([DateTime]::UtcNow - $script:baselineStartedAt).TotalSeconds
        if ($baselineElapsedSeconds -ge $nextBaselineProgressSeconds) {
            $progressSource = if ($baselineSource) { $baselineSource.id } else { "undiscovered" }
            $progressPath = if ($baselineSource) { $baselineSource.mountPath } else { "unknown" }
            $progressStatus = if ($baselineSource) { $baselineSource.status } else { "unknown" }
            $progressEntries = if ($baselineSource) { $baselineSource.entryCount } else { 0 }
            Write-Host "MFT baseline pending: source=$progressSource mount=$progressPath status=$progressStatus entry_count=$progressEntries elapsed_seconds=$baselineElapsedSeconds"
            $nextBaselineProgressSeconds += 60
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $baselineDeadline)
    $script:evidence.baselineWaitMs = [long]([DateTime]::UtcNow - $script:baselineStartedAt).TotalMilliseconds
    if ($null -eq $baselineSource -or $baselineSource.status -ne "ready" -or $null -eq $baselineSource.lastFullIndexAt) {
        $status = if ($baselineSource) { "status=$($baselineSource.status) error=$($baselineSource.lastError) entry_count=$($baselineSource.entryCount)" } else { "enabled fixed NTFS source was not discovered" }
        throw "native MFT baseline did not become ready within $baselineTimeoutMinutes minutes: $status"
    }
    if ([long]$baselineSource.entryCount -le 0) {
        throw "native MFT baseline is incomplete: fixed NTFS source reached ready with entry_count=$($baselineSource.entryCount)"
    }
    # Keep the pre-existing fixture search off the hot baseline polling path.
    # Search the fixture once after the non-empty baseline is complete.
    $baselineSnapshot = Get-ProbeSnapshot $preexistingName 25 0 $true
    $baselineVolumeFacts = @($baselineSnapshot.volumes | Where-Object { $_.id -ceq $script:evidence.expectedQualificationSourceId })
    if ($baselineVolumeFacts.Count -ne 1 -or
        $baselineVolumeFacts[0].databaseActiveEntries -ne $baselineVolumeFacts[0].entryCount -or
        $baselineVolumeFacts[0].databaseTotalEntries -lt $baselineVolumeFacts[0].databaseActiveEntries -or
        $baselineVolumeFacts[0].databaseActiveEntries -lt $script:evidence.independentManifest.expectedFileCount) {
        throw "task-owned NTFS persisted row facts do not match the ready source count: $($baselineVolumeFacts | ConvertTo-Json -Compress)"
    }
    $script:evidence.databaseTotalEntryCount = [long]$baselineVolumeFacts[0].databaseTotalEntries
    $script:evidence.databaseActiveEntryCount = [long]$baselineVolumeFacts[0].databaseActiveEntries
    $baselineTrace = Get-TraceCounts
    $script:evidence.serviceRouteObserved = $baselineTrace.ServiceRoutes -gt 0
    if (-not $script:evidence.serviceRouteObserved) {
        throw "fixed NTFS baseline completed without a successful service-stream request"
    }
    $script:evidence.baselineFixtureSearchFound = @($baselineSnapshot.results | Where-Object {
        $_.path.Equals($preexistingPath, [StringComparison]::OrdinalIgnoreCase)
    }).Count -gt 0
    if (-not $script:evidence.baselineFixtureSearchFound) {
        throw "pre-existing baseline fixture was not searchable after a non-empty ready baseline: $preexistingPath"
    }

    $indexedManifestResults = @()
    $manifestPageSize = 100
    for ($offset = 0; $offset -lt 2048; $offset += $manifestPageSize) {
        $manifestPage = Get-ProbeSnapshot $preexistingToken $manifestPageSize $offset
        $pageResults = @($manifestPage.results | Where-Object {
            $_.name.StartsWith($preexistingToken, [StringComparison]::OrdinalIgnoreCase) -and
            $_.path.StartsWith($fixtureRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)
        })
        $indexedManifestResults += $pageResults
        if (@($manifestPage.results).Count -lt $manifestPageSize) { break }
    }
    if ($script:evidence.baselineEntryCount -lt $script:evidence.independentManifest.expectedFileCount) {
        throw "task-owned NTFS database entry total is below the expected manifest file count: database=$($script:evidence.baselineEntryCount) manifest=$($script:evidence.independentManifest.expectedFileCount)"
    }
    $indexedManifestRelativePaths = @($indexedManifestResults | ForEach-Object {
        [IO.Path]::GetRelativePath($fixtureRoot, $_.path).Replace('/', '\')
    } | Sort-Object)
    $expectedManifestRelativePaths = @($script:evidence.independentManifest.expectedRelativePaths)
    $manifestDifferences = @(Compare-Object -ReferenceObject $expectedManifestRelativePaths -DifferenceObject $indexedManifestRelativePaths)
    $script:evidence.independentManifest.indexedRelativePaths = $indexedManifestRelativePaths
    $script:evidence.independentManifest.missingRelativePaths = @($manifestDifferences | Where-Object SideIndicator -eq '<=' | ForEach-Object InputObject)
    $script:evidence.independentManifest.unexpectedRelativePaths = @($manifestDifferences | Where-Object SideIndicator -eq '=>' | ForEach-Object InputObject)
    $script:evidence.independentManifest.matchedAfterServiceRecovery =
        $script:evidence.recovery.finalStatus -eq 'ready' -and
        $script:evidence.independentManifest.missingRelativePaths.Count -eq 0 -and
        $script:evidence.independentManifest.unexpectedRelativePaths.Count -eq 0
    if (-not $script:evidence.independentManifest.matchedAfterServiceRecovery) {
        throw "independent file manifest did not match Global Search after service recovery: expected=$($expectedManifestRelativePaths.Count) indexed=$($indexedManifestRelativePaths.Count) missing=$($script:evidence.independentManifest.missingRelativePaths -join ',') unexpected=$($script:evidence.independentManifest.unexpectedRelativePaths -join ',') recovery=$($script:evidence.recovery.finalStatus)"
    }

    $smartSnapshot = Get-ProbeSnapshot $smartToken
    if ($smartSnapshot.coverage.policy.mode -ne 'smart_lean') { throw "new VHD profile did not default to Smart Lean" }
    if (-not (Test-SearchPath "$smartToken-source.ts" $smartIncluded) -or (Test-SearchPath "$smartToken-dependency.js" $smartExcluded)) { throw "Service MFT Smart Lean admission mismatch" }
    # Included -> excluded file move and excluded -> included subtree backfill.
    $smartMovedIntoExcluded = Join-Path $smartProject "node_modules\$smartToken-source.ts"
    Move-Item -LiteralPath $smartIncluded -Destination $smartMovedIntoExcluded
    [void](Wait-ForSearchPath "$smartToken-source.ts" $smartIncluded $false 60 "Smart Lean include to exclude")
    if (Test-SearchPath "$smartToken-source.ts" $smartMovedIntoExcluded) { throw "excluded move remained searchable" }
    $smartRestoredLib = Join-Path $smartSource "restored-lib"
    Move-Item -LiteralPath $smartLib -Destination $smartRestoredLib
    $smartRestoredFile = Join-Path $smartRestoredLib "$smartToken-dependency.js"
    [void](Wait-ForSearchPath "$smartToken-dependency.js" $smartRestoredFile $true 60 "Smart Lean excluded subtree backfill")
    if (Test-SearchPath "$smartToken-dependency.js" $smartExcluded) { throw "old excluded subtree path survived rename" }
    $script:evidence.smartLeanService = [ordered]@{ defaultMode = 'smart_lean'; initialAdmission = $true; includeToExclude = $true; excludedSubtreeBackfill = $true; coverage = (Get-ProbeSnapshot).coverage }

    $moveToken = "zb05qamove$([Guid]::NewGuid().ToString('N'))"
    $moveName = "$moveToken.txt"
    $moveSourcePath = Join-Path $moveSourceDirectory $moveName
    $moveTargetPath = Join-Path $moveTargetDirectory $moveName
    $moveTimer = [Diagnostics.Stopwatch]::StartNew()
    Set-Content -LiteralPath $moveSourcePath -Value "cross-directory move fixture" -NoNewline
    [void](Wait-ForSearchPath $moveName $moveSourcePath $true 60 "cross-directory move source" $moveTimer)
    Move-Item -LiteralPath $moveSourcePath -Destination $moveTargetPath
    $moveDeadline = [DateTime]::UtcNow.AddSeconds(60)
    do {
        $newMovePathFound = Test-SearchPath $moveName $moveTargetPath
        $oldMovePathFound = Test-SearchPath $moveName $moveSourcePath
        if ($newMovePathFound -and -not $oldMovePathFound) {
            $moveTimer.Stop()
            $script:evidence.crossDirectoryMoveLatencyMs = [long]$moveTimer.Elapsed.TotalMilliseconds
            break
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $moveDeadline)
    if ($null -eq $script:evidence.crossDirectoryMoveLatencyMs) {
        throw "cross-directory move did not converge within 60 seconds (newFound=$newMovePathFound oldFound=$oldMovePathFound)"
    }
    $script:evidence.crossDirectoryMove = [ordered]@{
        sourceRelativePath = [IO.Path]::GetRelativePath($fixtureRoot, $moveSourcePath).Replace('/', '\')
        destinationRelativePath = [IO.Path]::GetRelativePath($fixtureRoot, $moveTargetPath).Replace('/', '\')
        sourcePathAbsent = -not $oldMovePathFound
        destinationPathFound = $newMovePathFound
        latencyMs = $script:evidence.crossDirectoryMoveLatencyMs
    }

    $freshnessToken = "zb05qafresh$([Guid]::NewGuid().ToString('N'))"
    $createdName = "$freshnessToken-created.txt"
    $renamedName = "$freshnessToken-renamed.txt"
    $createdPath = Join-Path $fixtureRoot $createdName
    $renamedPath = Join-Path $fixtureRoot $renamedName
    $createTimer = [Diagnostics.Stopwatch]::StartNew()
    New-Item -ItemType File -Path $createdPath | Out-Null
    Set-Content -LiteralPath $createdPath -Value "created after the validated baseline" -NoNewline
    $script:evidence.createLatencyMs = Wait-ForSearchPath $createdName $createdPath $true 60 "create freshness" $createTimer

    $renameTimer = [Diagnostics.Stopwatch]::StartNew()
    Move-Item -LiteralPath $createdPath -Destination $renamedPath
    $renameDeadline = [DateTime]::UtcNow.AddSeconds(60)
    do {
        $newFound = Test-SearchPath $renamedName $renamedPath
        $oldFound = Test-SearchPath $createdName $createdPath
        if ($newFound -and -not $oldFound) {
            $renameTimer.Stop()
            $script:evidence.renameLatencyMs = [long]$renameTimer.Elapsed.TotalMilliseconds
            break
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $renameDeadline)
    if ($null -eq $script:evidence.renameLatencyMs) {
        throw "rename freshness did not converge within 60 seconds (newFound=$newFound oldFound=$oldFound)"
    }

    $deleteTimer = [Diagnostics.Stopwatch]::StartNew()
    Remove-Item -LiteralPath $renamedPath -Force
    $script:evidence.deleteLatencyMs = Wait-ForSearchPath $renamedName $renamedPath $false 60 "delete freshness" $deleteTimer

    $settleDeadline = [DateTime]::UtcNow.AddSeconds(30)
    $settleSince = $null
    $lastCycles = -1
    $lastWaits = -1
    do {
        $counts = Get-TraceCounts
        if ($counts.LastCoordinatorEvent -ceq "coordinator_wait" -and $counts.Cycles -eq $lastCycles -and $counts.Waits -eq $lastWaits) {
            if ($null -eq $settleSince) { $settleSince = [DateTime]::UtcNow }
            if (([DateTime]::UtcNow - $settleSince).TotalSeconds -ge 2) { break }
        } else {
            $settleSince = $null
            $lastCycles = $counts.Cycles
            $lastWaits = $counts.Waits
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $settleDeadline)
    if ($null -eq $settleSince) {
        $lastTrace = if ($counts.Lines.Count -gt 0) { $counts.Lines[-1] } else { "none" }
        $lastCoordinatorEvent = if ($counts.LastCoordinatorEvent) { $counts.LastCoordinatorEvent } else { "none" }
        throw "coordinator did not settle into its blocking idle wait: cycles=$($counts.Cycles) waits=$($counts.Waits) lastCoordinatorEvent=$lastCoordinatorEvent lastTrace=$lastTrace"
    }
    $idleStartedAt = [DateTime]::UtcNow
    $script:evidence.settledIdleStartedAt = $idleStartedAt.ToString('o')
    $idleBefore = Get-TraceCounts
    $script:evidence.serviceRouteObserved = $idleBefore.ServiceRoutes -gt 0
    if (-not $script:evidence.serviceRouteObserved) { throw "no successful desktop-to-Windows-Service request was traced" }
    if ($ResidentQualification) {
        . (Join-Path $PSScriptRoot "sampleWindowsResident.ps1")
        $startup = Get-Content -LiteralPath (Join-Path $taskRoot "resident-stderr.log") -Raw
        $script:evidence.residentStartup = $startup
        if ($startup -notmatch 'ui_runtime startup_mode=background webview_count=0 labels=\s') {
            throw "resident startup did not prove background with zero Main/Search/WebViews: $startup"
        }
        $appSamples = @()
        $serviceSamples = @()
        for ($sampleIndex = 0; $sampleIndex -lt 31; $sampleIndex++) {
            $appSamples += Get-ResidentProcessSample $script:clientProcessId $CandidateExe
            $serviceSamples += Get-ResidentProcessSample $script:serviceProcessId $CandidateExe
            foreach ($series in @(@{ samples = $appSamples }, @{ samples = $serviceSamples })) {
                $samples = $series.samples
                $samples[$sampleIndex]['cpuDeltaSeconds'] = if ($sampleIndex -eq 0) { 0 } else { $samples[$sampleIndex].cpuTotalSeconds - $samples[$sampleIndex - 1].cpuTotalSeconds }
            }
            $script:evidence.residentSamples = $appSamples
            $script:evidence.serviceSamples = $serviceSamples
            foreach ($sample in @($appSamples[$sampleIndex], $serviceSamples[$sampleIndex])) {
                # Process.MainWindowHandle includes non-WebView native utility
                # windows. Main/Search/WebView truth comes from Tauri's window
                # diagnostic and lifecycle trace, plus the WebView child check.
                if ($sample.webviewChildren -ne 0) { throw "unexpected resident WebView: $($sample | ConvertTo-Json -Compress)" }
            }
            if ($sampleIndex -lt 30) { Start-Sleep -Seconds 1 }
        }
        Assert-ResidentTrend $appSamples
        Assert-ResidentTrend $serviceSamples
        $finalLog = Get-Content -LiteralPath (Join-Path $taskRoot "resident-stderr.log") -Raw
        if ($finalLog -match 'main_window_created|search_window_ready') { throw "unexpected WebView lifecycle during resident window: $finalLog" }
        $script:evidence.residentClassification = "HARD PASS / OBSERVATIONAL MEMORY"
    } else {
        Start-Sleep -Milliseconds 10000
    }
    $idleAfter = Get-TraceCounts
    $idleFinishedAt = [DateTime]::UtcNow
    $script:evidence.settledIdleFinishedAt = $idleFinishedAt.ToString('o')
    $script:evidence.settledIdleWindowMs = [long]($idleFinishedAt - $idleStartedAt).TotalMilliseconds
    $script:evidence.settledCoordinatorCycleDelta = $idleAfter.Cycles - $idleBefore.Cycles
    $script:evidence.settledCoordinatorWaitDelta = $idleAfter.Waits - $idleBefore.Waits
    if ($script:evidence.settledCoordinatorCycleDelta -ne 0 -or $script:evidence.settledCoordinatorWaitDelta -ne 0) {
        throw "settled idle was not quiet: cycleDelta=$($script:evidence.settledCoordinatorCycleDelta) waitDelta=$($script:evidence.settledCoordinatorWaitDelta)"
    }
    }
} catch {
    $script:failureMessage = $_.Exception.ToString()
    $script:evidence.failure = $script:failureMessage
    if ($ResidentQualification) { $script:evidence.residentClassification = "BLOCKED" }
} finally {
    if ($null -ne $script:baselineStartedAt -and $null -eq $script:evidence.baselineWaitMs) {
        $script:evidence.baselineWaitMs = [long]([DateTime]::UtcNow - $script:baselineStartedAt).TotalMilliseconds
    }
    Stop-Probe
    if ($script:taskRootCreatedByTask) { $script:evidence.rawTrace = @(Get-TraceLines) }

    try {
        $service = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
        if ($service -and $script:serviceCreatedByTask -and $service.PathName.Trim() -ieq ('"' + $CandidateExe + '" --index-service') -and $service.State -ne "Stopped") {
            $stop = Invoke-ServiceControl @("stop", $serviceName)
            $stopDeadline = [DateTime]::UtcNow.AddSeconds(25)
            do {
                Start-Sleep -Milliseconds 250
                $service = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
            } while ($service -and $service.State -ne "Stopped" -and [DateTime]::UtcNow -lt $stopDeadline)
            if ($service -and $service.State -ne "Stopped") {
                $script:cleanupFailure = $true
                $script:evidence.cleanup.details += "service stop timeout: scExit=$($stop.ExitCode) output=$($stop.Output)"
            } else {
                $script:evidence.cleanup.serviceStopped = $true
            }
        } elseif (-not $service) {
            $script:evidence.cleanup.serviceStopped = $true
        } elseif ($script:serviceCreatedByTask -and $service.PathName.Trim() -ieq ('"' + $CandidateExe + '" --index-service')) {
            $script:evidence.cleanup.serviceStopped = $true
        } else {
            $script:evidence.cleanup.details += "left pre-existing or non-matching service untouched: $($service.PathName)"
        }

        if ($null -ne $script:clientProcessId) {
            $clientImage = $null
            try { $clientImage = Get-ProcessImage ([int]$script:clientProcessId) } catch {}
            if ($clientImage -and $clientImage.Equals($CandidateExe, [StringComparison]::OrdinalIgnoreCase)) {
                Stop-Process -Id $script:clientProcessId -Force -ErrorAction SilentlyContinue
            }
        }
        if ($null -ne $script:serviceProcessId) {
            $serviceImage = $null
            try { $serviceImage = Get-ProcessImage ([int]$script:serviceProcessId) } catch {}
            if ($serviceImage -and $serviceImage.Equals($CandidateExe, [StringComparison]::OrdinalIgnoreCase)) {
                Stop-Process -Id $script:serviceProcessId -Force -ErrorAction SilentlyContinue
            }
        }
        Start-Sleep -Milliseconds 500
        $remainingCandidateProcesses = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object {
            $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).Equals($CandidateExe, [StringComparison]::OrdinalIgnoreCase)
        })
        $script:evidence.cleanup.candidateProcessesTerminated = $remainingCandidateProcesses.Count -eq 0
        if (-not $script:evidence.cleanup.candidateProcessesTerminated) {
            $script:cleanupFailure = $true
            $script:evidence.cleanup.details += "candidate process remains: $($remainingCandidateProcesses.ProcessId -join ',')"
        }

        $service = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
        if (-not $service) {
            $script:evidence.cleanup.serviceDeleted = $true
        } elseif ($script:serviceCreatedByTask -and $service.PathName.Trim() -ieq ('"' + $CandidateExe + '" --index-service')) {
            $delete = Invoke-ServiceControl @("delete", $serviceName)
            $deleteDeadline = [DateTime]::UtcNow.AddSeconds(10)
            do {
                $serviceAfterDelete = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
                if (-not $serviceAfterDelete) { break }
                Start-Sleep -Milliseconds 250
            } while ([DateTime]::UtcNow -lt $deleteDeadline)
            $script:evidence.cleanup.serviceDeleted = $null -eq $serviceAfterDelete
            if (-not $script:evidence.cleanup.serviceDeleted) {
                $script:cleanupFailure = $true
                $script:evidence.cleanup.details += "service still registered after delete: scExit=$($delete.ExitCode) output=$($delete.Output)"
            }
        } else {
            $script:evidence.cleanup.serviceDeleted = $false
        }

        if (Test-Path -LiteralPath $taskRoot) {
            if (-not $script:taskRootCreatedByTask) { throw "left pre-existing task root untouched: $taskRoot" }
            if (-not $taskRoot.StartsWith($runnerTemp, [StringComparison]::OrdinalIgnoreCase)) {
                throw "refusing cleanup outside RUNNER_TEMP: $taskRoot"
            }
        }
        if ($fixtureRoot -and (Test-Path -LiteralPath $fixtureRoot)) {
            if (-not $fixtureCleanupBoundary -or -not $fixtureRoot.StartsWith($fixtureCleanupBoundary, [StringComparison]::OrdinalIgnoreCase)) {
                throw "refusing cleanup outside the task-owned fixture boundary: $fixtureRoot"
            }
            Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
        }
        $script:evidence.cleanup.fixtureRootRemoved = -not $fixtureRoot -or -not (Test-Path -LiteralPath $fixtureRoot)
        if (-not $script:evidence.cleanup.fixtureRootRemoved) {
            $script:cleanupFailure = $true
            $script:evidence.cleanup.details += "task fixture root remains: $fixtureRoot"
        }

        if ($script:qualificationVhdCreated) {
            $vhdOwner = if (Test-Path -LiteralPath $qualificationVhdOwnerMarker) {
                (Get-Content -LiteralPath $qualificationVhdOwnerMarker -Raw).Trim()
            } else {
                $null
            }
            if (-not $vhdOwner -or -not $vhdOwner.Equals($qualificationVhdPath, [StringComparison]::OrdinalIgnoreCase)) {
                throw "refusing to detach or remove VHD without its matching task ownership marker: $qualificationVhdPath"
            }
            if (Test-QualificationVhdAttached) {
                [void](Invoke-DiskPartScript "detach-vhd" @(
                    "select vdisk file=`"$qualificationVhdPath`"",
                    "detach vdisk",
                    "exit"
                ))
            }
            if (Test-QualificationVhdAttached) {
                throw "task-owned qualification VHD remains attached after DiskPart detach"
            }
            $detachDeadline = [DateTime]::UtcNow.AddSeconds(10)
            $mountedVolume = $null
            do {
                $mountedVolume = if ($script:fixtureDriveLetter) {
                    Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='$($script:fixtureVolume)'" -ErrorAction SilentlyContinue
                } else {
                    $null
                }
                if (-not $mountedVolume) { break }
                Start-Sleep -Milliseconds 200
            } while ([DateTime]::UtcNow -lt $detachDeadline)
            if ($mountedVolume) {
                throw "task-owned qualification drive letter remains mounted after VHD detach: $($script:fixtureVolume)"
            }
            $script:qualificationVhdAttached = $false
            $script:evidence.qualificationVolumeDetached = $true
            $script:evidence.cleanup.qualificationVolumeDetached = $true
        } else {
            if (Test-Path -LiteralPath $qualificationVhdPath) {
                throw "refusing to remove an unmarked qualification VHD: $qualificationVhdPath"
            }
            $script:evidence.qualificationVolumeDetached = $true
            $script:evidence.cleanup.qualificationVolumeDetached = $true
        }

        if (Test-Path -LiteralPath $taskRoot) {
            Remove-Item -LiteralPath $taskRoot -Recurse -Force
        }
        $script:evidence.qualificationVhdRemoved = -not (Test-Path -LiteralPath $qualificationVhdPath)
        if (-not $script:evidence.qualificationVhdRemoved) {
            $script:cleanupFailure = $true
            $script:evidence.cleanup.details += "task-owned qualification VHD remains: $qualificationVhdPath"
        }
        $script:evidence.cleanup.taskProfileRemoved = -not (Test-Path -LiteralPath $taskRoot)
        if (-not $script:evidence.cleanup.taskProfileRemoved) {
            $script:cleanupFailure = $true
            $script:evidence.cleanup.details += "task profile remains: $taskRoot"
        }
    } catch {
        $script:cleanupFailure = $true
        $script:evidence.cleanup.details += "cleanup exception: $($_.Exception.ToString())"
    }

    $evidenceDirectory = Split-Path -Parent $EvidencePath
    New-Item -ItemType Directory -Path $evidenceDirectory -Force | Out-Null
    $script:evidence | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $EvidencePath -Encoding utf8NoBOM
    if ($ResidentQualification) {
        @("Source: $($script:evidence.sourceHead)", "Candidate SHA256: $($script:evidence.candidateSha256)", "Resident: $($script:evidence.residentClassification)", "Failure: $($script:evidence.failure)", "Cleanup: $($script:evidence.cleanup | ConvertTo-Json -Compress)", "Raw samples are in the sibling JSON; application and service remain separate.") | Set-Content -LiteralPath ([IO.Path]::ChangeExtension($EvidencePath, '.md')) -Encoding utf8NoBOM
    }
}

if ($script:failureMessage) {
    Write-Error $script:failureMessage
    exit 1
}
if ($script:cleanupFailure) {
    Write-Error "disposable Windows qualification cleanup did not fully complete"
    exit 1
}
if ($DirectOnly) {
    Write-Output "Windows Direct MFT qualification passed; evidence: $EvidencePath"
} else {
    Write-Output "Windows Global Index service qualification passed; evidence: $EvidencePath"
}
