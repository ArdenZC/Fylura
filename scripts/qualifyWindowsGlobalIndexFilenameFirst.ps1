param(
    [Parameter(Mandatory = $true)][string]$BaselineProbeExe,
    [Parameter(Mandatory = $true)][string]$CandidateProbeExe,
    [Parameter(Mandatory = $true)][string]$BaselineRoot,
    [Parameter(Mandatory = $true)][string]$CandidateRoot,
    [Parameter(Mandatory = $true)][string]$ExpectedBaselineSha,
    [Parameter(Mandatory = $true)][string]$ExpectedCandidateSha,
    [Parameter(Mandatory = $true)][string]$EvidencePath,
    [int]$FileCount = 100000,
    [int]$Repetitions = 3
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
$baselineProbeExe = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $BaselineProbeExe).Path)
$candidateProbeExe = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $CandidateProbeExe).Path)
$BaselineRoot = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $BaselineRoot).Path)
$CandidateRoot = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $CandidateRoot).Path)
$EvidencePath = [IO.Path]::GetFullPath($EvidencePath)
$evidenceDirectory = Split-Path -Parent $EvidencePath
$traceDirectory = Join-Path $evidenceDirectory 'traces'
$script:evidence = [ordered]@{
    status = 'RUNNING'
    baselineSha = $ExpectedBaselineSha
    candidateSha = $ExpectedCandidateSha
    fileCount = $FileCount
    repetitions = $Repetitions
    runner = $null
    fixtureRoot = $null
    sourceId = $null
    baseline = @()
    candidate = @()
    incremental = $null
    averages = $null
    firstSearchTargetSeconds = 10
    fullIndexTargetSeconds = 10
    firstSearchTargetMet = $null
    fullIndexTargetMet = $null
    cleanup = [ordered]@{
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
    $script:fixtureRoot = Join-Path $driveRoot 'fixtures\filename-first'
    if (Test-Path -LiteralPath $vhdPath) { throw "refusing to replace a pre-existing VHD: $vhdPath" }
    # Record ownership before DiskPart mutates anything so a partial create can
    # still be detached and removed safely by the finally block.
    Set-Content -LiteralPath $vhdOwnerMarker -Value $vhdPath -NoNewline
    [void](Invoke-DiskPart 'create-attach-format' @(
        "create vdisk file=`"$vhdPath`" maximum=4096 type=expandable",
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
    Set-Content -LiteralPath $driveMarker -Value $drive -NoNewline
    Set-Content -LiteralPath (Join-Path $taskRoot 'fixture-created-by-qualification.txt') -Value $script:fixtureRoot -NoNewline
    New-Item -ItemType Directory -Path $script:fixtureRoot -Force | Out-Null
    $mounted = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='$drive'" -ErrorAction SilentlyContinue
    if (-not $mounted -or $mounted.FileSystem -ne 'NTFS') { throw "task volume is not visible as fixed NTFS at $drive" }
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
    $startInfo.Environment['ZC_GLOBAL_INDEX_MFT_PERSIST_BATCH'] = '8192'
    $startInfo.Environment['ZC_GLOBAL_INDEX_QA_TRACE'] = $TracePath

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to start $Label" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddMinutes($TimeoutMinutes)
    $peakWorkingSet = 0L
    $peakPrivateBytes = 0L
    while (-not $process.WaitForExit(100)) {
        try {
            $process.Refresh()
            $peakWorkingSet = [Math]::Max($peakWorkingSet, $process.WorkingSet64)
            $peakPrivateBytes = [Math]::Max($peakPrivateBytes, $process.PrivateMemorySize64)
        } catch { }
        if ([DateTime]::UtcNow -ge $deadline) {
            $process.Kill($true)
            $process.WaitForExit(10000)
            throw "$Label exceeded $TimeoutMinutes minutes"
        }
    }
    $process.Refresh()
    $peakWorkingSet = [Math]::Max($peakWorkingSet, $process.WorkingSet64)
    $peakPrivateBytes = [Math]::Max($peakPrivateBytes, $process.PrivateMemorySize64)
    $stdout = $stdoutTask.Result.Trim()
    $stderr = $stderrTask.Result.Trim()
    if ($process.ExitCode -ne 0) { throw "$Label failed (exit=$($process.ExitCode)): $stderr $stdout" }
    $jsonLine = @($stdout -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Last 1)
    if ($jsonLine.Count -ne 1) { throw "$Label returned no JSON evidence: $stdout" }
    $result = $jsonLine[0] | ConvertFrom-Json
    $mftTiming = Get-TraceEvent $TracePath 'mft_initial_index'
    $writerTiming = Get-TraceEvent $TracePath 'source_operation_write'
    if (-not $mftTiming -or -not $writerTiming) { throw "$Label omitted MFT or repository stage timings" }
    return [pscustomobject]@{
        label = $Label
        result = $result
        stages = [ordered]@{ mft = $mftTiming; writer = $writerTiming }
        peakWorkingSetBytes = $peakWorkingSet
        peakPrivateBytes = $peakPrivateBytes
        tracePath = $TracePath
    }
}

function Get-TraceEvent([string]$Path, [string]$EventName) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    $line = Get-Content -LiteralPath $Path | Where-Object { $_.StartsWith("$EventName ", [StringComparison]::Ordinal) } | Select-Object -Last 1
    if (-not $line) { return $null }
    $event = [ordered]@{ event = $EventName }
    foreach ($field in ($line -split '\s+')) {
        $pair = $field -split '=', 2
        if ($pair.Count -ne 2 -or $pair[0] -eq $EventName) { continue }
        $number = 0L
        if ([long]::TryParse($pair[1], [ref]$number)) { $event[$pair[0]] = $number }
        else { $event[$pair[0]] = $pair[1] }
    }
    return [pscustomobject]$event
}

function Prepare-Profile([string]$ProbeExe, [string]$Profile) {
    $prepared = Invoke-Probe "prepare $Profile" $ProbeExe @('--prepare-profile', $Profile, "$($script:driveLetter):") '0' (Join-Path $taskRoot 'prepare-trace.log') 2
    return $prepared.result
}

function Get-Average([object[]]$Rows, [string]$Property) {
    $values = foreach ($row in $Rows) {
        if ($Property.StartsWith('result.')) {
            $resultProperty = $Property.Substring(7)
            [double]$row.result.$resultProperty
        } elseif ($Property.StartsWith('stages.')) {
            $parts = $Property.Substring(7).Split('.', 2)
            [double]$row.stages.($parts[0]).($parts[1])
        } else {
            [double]$row.$Property
        }
    }
    return [double](($values | Measure-Object -Average).Average)
}

try {
    if ($FileCount -lt 100000) { throw 'native filename-first benchmark requires at least 100,000 files' }
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
        deletedPath = $deletePath
    }
    $baselineRows = [Collections.Generic.List[object]]::new()
    $candidateRows = [Collections.Generic.List[object]]::new()
    $lastProfiles = @{}
    $lastSourceIds = @{}
    for ($repetition = 1; $repetition -le $Repetitions; $repetition++) {
        $order = if ($repetition % 2 -eq 1) { @('baseline', 'candidate') } else { @('candidate', 'baseline') }
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
            $trace = Join-Path $taskRoot "$side-$repetition-trace.log"
            $run = Invoke-Probe "$side initial run $repetition" $probe @(
                '--qualify-filename-first', $profile, $prepared.targetSourceId,
                'filename-first-item-000000.txt', $firstTargetPath, 'finalmarker', $targetPath
            ) $mode $trace 90
            if (-not $run.result.restartedSearchVisible -or -not $run.result.restartedCompleteCoverageSearchVisible -or -not $run.result.indexComplete) {
                throw "$side initial run $repetition did not persist a complete searchable index"
            }
            if ($side -eq 'candidate' -and (-not $run.result.firstSearchBeforeComplete -or [long]$run.result.firstSearchProcessedEntries -ge $FileCount)) {
                throw "candidate initial run $repetition did not expose an early filename while progress was incomplete"
            }
            if ([long]$run.result.databaseActiveEntries -lt $FileCount) {
                throw "$side initial run indexed too few active entries: $($run.result.databaseActiveEntries) < $FileCount"
            }
            $run | Add-Member -NotePropertyName repetition -NotePropertyValue $repetition
            if ($side -eq 'baseline') { $baselineRows.Add($run) } else { $candidateRows.Add($run) }
            $lastProfiles[$side] = $profile
            $lastSourceIds[$side] = $prepared.targetSourceId
            New-Item -ItemType Directory -Path $traceDirectory -Force | Out-Null
            Copy-Item -LiteralPath $trace -Destination (Join-Path $traceDirectory "$side-$repetition.log") -Force
        }
    }

    [IO.File]::WriteAllBytes($addedPath, [byte[]]::new(0))
    [IO.File]::Move($targetPath, $renamedPath)
    Remove-Item -LiteralPath $deletePath -Force
    Start-Sleep -Milliseconds 500
    $incremental = [ordered]@{}
    foreach ($side in @('baseline', 'candidate')) {
        $probe = if ($side -eq 'baseline') { $baselineProbeExe } else { $candidateProbeExe }
        $mode = if ($side -eq 'baseline') { '0' } else { '1' }
        $trace = Join-Path $taskRoot "$side-incremental-trace.log"
        $run = Invoke-Probe "$side incremental changes" $probe @(
            '--qualify-filename-first-incremental',
            $lastProfiles[$side],
            $lastSourceIds[$side],
            $addedPath,
            $targetPath,
            $renamedPath,
            $deletePath
        ) $mode $trace 30
        if (-not $run.result.restartCorrect) { throw "$side incremental changes failed restart validation" }
        $incremental[$side] = $run.result
        New-Item -ItemType Directory -Path $traceDirectory -Force | Out-Null
        Copy-Item -LiteralPath $trace -Destination (Join-Path $traceDirectory "$side-incremental.log") -Force
    }

    $baselineFirst = Get-Average $baselineRows.ToArray() 'firstSearchableUs'
    $candidateFirst = Get-Average $candidateRows.ToArray() 'firstSearchableUs'
    $baselineFull = Get-Average $baselineRows.ToArray() 'fullIndexUs'
    $candidateFull = Get-Average $candidateRows.ToArray() 'fullIndexUs'
    $script:evidence.baseline = $baselineRows.ToArray()
    $script:evidence.candidate = $candidateRows.ToArray()
    $script:evidence.incremental = $incremental
    $script:evidence.averages = [ordered]@{
        baselineFirstSearchableUs = $baselineFirst
        candidateFirstSearchableUs = $candidateFirst
        firstSearchImprovementPercent = if ($baselineFirst -gt 0) { 100.0 * ($baselineFirst - $candidateFirst) / $baselineFirst } else { 0 }
        baselineFullIndexUs = $baselineFull
        candidateFullIndexUs = $candidateFull
        fullIndexImprovementPercent = if ($baselineFull -gt 0) { 100.0 * ($baselineFull - $candidateFull) / $baselineFull } else { 0 }
        baselinePeakWorkingSetBytes = Get-Average $baselineRows.ToArray() 'peakWorkingSetBytes'
        candidatePeakWorkingSetBytes = Get-Average $candidateRows.ToArray() 'peakWorkingSetBytes'
        baselinePeakPrivateBytes = Get-Average $baselineRows.ToArray() 'peakPrivateBytes'
        candidatePeakPrivateBytes = Get-Average $candidateRows.ToArray() 'peakPrivateBytes'
        baselineDatabaseBytes = Get-Average $baselineRows.ToArray() 'result.databaseBytesWithWal'
        candidateDatabaseBytes = Get-Average $candidateRows.ToArray() 'result.databaseBytesWithWal'
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
        baselineEntryUpsertUs = Get-Average $baselineRows.ToArray() 'stages.writer.entry_upsert_us'
        candidateEntryUpsertUs = Get-Average $candidateRows.ToArray() 'stages.writer.entry_upsert_us'
        baselineCommitUs = Get-Average $baselineRows.ToArray() 'stages.writer.commit_us'
        candidateCommitUs = Get-Average $candidateRows.ToArray() 'stages.writer.commit_us'
        baselineSqliteTransactionCount = Get-Average $baselineRows.ToArray() 'stages.writer.sqlite_transactions'
        candidateSqliteTransactionCount = Get-Average $candidateRows.ToArray() 'stages.writer.sqlite_transactions'
        ftsTiming = 'FTS5 is synchronous inside entry_upsert_us; use the paired 25k trigger attribution report to isolate trigger cost.'
    }
    $script:evidence.firstSearchTargetMet = $candidateFirst -le 10000000
    $script:evidence.fullIndexTargetMet = $candidateFull -le 10000000
    $targetFailures = @()
    if (-not $script:evidence.firstSearchTargetMet) {
        $targetFailures += 'first searchable result exceeded the 10-second target'
    }
    if (-not $script:evidence.fullIndexTargetMet) {
        $targetFailures += 'complete durable filename index exceeded the 10-second target'
    }
    if ($targetFailures.Count -gt 0) {
        $script:evidence.status = 'PERFORMANCE_TARGET_MISSED'
        $failure = $targetFailures -join '; '
        $script:evidence.failure = $failure
    } else {
        $script:evidence.status = 'PASS'
    }
} catch {
    $failure = $_.Exception.ToString()
    $script:evidence.status = 'FAIL'
    $script:evidence.failure = $failure
} finally {
    try {
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
