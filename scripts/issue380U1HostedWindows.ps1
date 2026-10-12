param(
    [Parameter(Mandatory = $true)][string]$EvidenceRoot,
    [ValidateSet("U1", "U1R")][string]$Mode = "U1"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$oldSha = $env:U1_OLD_SHA
$oldTree = $env:U1_OLD_TREE
$d1aSha = $env:U1_D1A_SHA
$d1aTree = $env:U1_D1A_TREE
$u1Head = (& git rev-parse HEAD).Trim()
$u1Tree = (& git rev-parse 'HEAD^{tree}').Trim()
$repoRoot = [IO.Path]::GetFullPath((Get-Location).Path)
$EvidenceRoot = [IO.Path]::GetFullPath($EvidenceRoot)
$worktreeRoot = Join-Path $env:RUNNER_TEMP "issue-380-u1-worktrees-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT"
$cargoTarget = Join-Path $repoRoot "src-tauri/target"
$taskRoot = Join-Path $env:RUNNER_TEMP "issue-380-u1-runtime-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT"
$global:U1OwnedInstallDirectory = $null
$global:U1RVolumes = [Collections.Generic.List[object]]::new()
$global:U1Evidence = [ordered]@{
    contract = if ($Mode -eq "U1R") { "U1R focused installer diagnostics and same-path generation test; test-only; not a production fix" } else { "U1 frozen-candidate compatibility; test-only; not a production fix" }
    mode = $Mode
    u1Head = $u1Head
    u1Tree = $u1Tree
    old = [ordered]@{ commit = $oldSha; tree = $oldTree; sourceStatus = "UNBUILT" }
    d1a = [ordered]@{ commit = $d1aSha; tree = $d1aTree; sourceStatus = "UNBUILT" }
    runner = $null
    candidates = [ordered]@{}
    matchingPairs = [ordered]@{}
    mixedPairs = [ordered]@{}
    directFallback = [ordered]@{}
    installer = [ordered]@{ status = "UNVERIFIED"; samples = @(); transitions = @(); interruption = [ordered]@{ status = "UNVERIFIED" }; repair = [ordered]@{ status = "UNVERIFIED" } }
    u1rDisposition = [ordered]@{ status = "BLOCKED"; blockers = @("U1R evidence not yet completed"); productionFixImplemented = $false }
    cleanup = [ordered]@{ details = @(); failures = @() }
    failures = @()
}

function Save-Evidence {
    New-Item -ItemType Directory -Path $EvidenceRoot -Force | Out-Null
    $global:U1Evidence | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $EvidenceRoot "u1-evidence.json") -Encoding utf8NoBOM
}

function Add-Failure([string]$Scenario, [string]$Message) {
    $global:U1Evidence.failures += [ordered]@{ scenario = $Scenario; message = $Message }
    Save-Evidence
}

function Invoke-LoggedCommand([string]$Name, [string]$Executable, [string[]]$Arguments, [string]$WorkingDirectory) {
    $log = Join-Path $EvidenceRoot "$Name.log"
    Push-Location $WorkingDirectory
    try {
        & $Executable @Arguments *> $log
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    if ($exitCode -ne 0) {
        $tail = if (Test-Path -LiteralPath $log) { (Get-Content -LiteralPath $log -Tail 100 | Out-String).Trim() } else { "no command log" }
        throw "$Name failed with exit code $exitCode. Log: $log`n$tail"
    }
    return $log
}

function Get-Hash([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
}

function Get-GitValue([string]$Directory, [string]$Argument) {
    Push-Location $Directory
    try {
        $value = (& git rev-parse $Argument).Trim()
        if ($LASTEXITCODE -ne 0) { throw "git rev-parse $Argument failed in $Directory" }
        return $value
    } finally {
        Pop-Location
    }
}

function Invoke-ProcessCaptured([string]$Executable, [string[]]$Arguments, [string]$WorkingDirectory, [hashtable]$Environment = @{}, [int]$TimeoutSeconds = 60) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $startInfo.ArgumentList.Add($argument) }
    foreach ($name in $Environment.Keys) { $startInfo.Environment[$name] = [string]$Environment[$name] }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to start $Executable" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill($true)
        $process.WaitForExit(10000) | Out-Null
        throw "$Executable did not finish within $TimeoutSeconds seconds"
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $exitCode = $process.ExitCode
    $process.Dispose()
    return [pscustomobject]@{ ExitCode = $exitCode; Stdout = $stdout; Stderr = $stderr }
}

function Invoke-ProfilePrepare([string]$ProbeExe, [string]$ProfileRoot, [string]$Drive) {
    $prepared = Invoke-ProcessCaptured $ProbeExe @("--prepare-profile", $ProfileRoot, "$Drive`:\") $repoRoot @{} 60
    if ($prepared.ExitCode -ne 0) { throw "profile preparation failed: $($prepared.Stderr) $($prepared.Stdout)" }
    $payload = $prepared.Stdout.Trim() | ConvertFrom-Json
    if (-not $payload.prepared -or $payload.enabledSourceCount -ne 1) { throw "profile preparation did not isolate a single source: $($prepared.Stdout)" }
    return $payload
}

function Invoke-ProfileQuery([string]$ProbeExe, [string]$ProfileRoot, [string]$Query) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $ProbeExe
    $startInfo.WorkingDirectory = $repoRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.Environment["ZC_NATIVE_QA_PROFILE_ROOT"] = $ProfileRoot
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to start isolated QA probe" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.StandardInput.WriteLine((@{ query = $Query } | ConvertTo-Json -Compress))
    $process.StandardInput.Close()
    if (-not $process.WaitForExit(15000)) {
        $process.Kill($true)
        $process.WaitForExit(5000) | Out-Null
        throw "isolated QA probe query timed out"
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult().Trim()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $exitCode = $process.ExitCode
    $process.Dispose()
    if ($exitCode -ne 0) { throw "isolated QA probe failed (exit=$exitCode): $stderr $stdout" }
    return $stdout | ConvertFrom-Json
}

function Invoke-ProfileSqlite([string]$Mode, [string]$ProfileRoot, [string]$SourceId, [string]$OldMarker, [string]$NextMarker) {
    $python = Get-Command "python.exe" -ErrorAction Stop | Select-Object -First 1
    $database = Join-Path $ProfileRoot "zen-canvas.sqlite3"
    $helper = Join-Path $repoRoot "scripts/issue380U1ResetProfile.py"
    $result = Invoke-ProcessCaptured $python.Source @($helper, $Mode, $database, $SourceId, $OldMarker, $NextMarker) $repoRoot @{} 30
    if ($result.ExitCode -ne 0) { throw "isolated SQLite $Mode failed: $($result.Stderr) $($result.Stdout)" }
    return $result.Stdout.Trim() | ConvertFrom-Json
}

function Wait-ForProfileMarker([string]$ProfileRoot, [string]$SourceId, [string]$OldMarker, [string]$NextMarker, [int]$TimeoutSeconds = 240) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $snapshot = $null
    $lastReadError = $null
    do {
        try {
            $snapshot = Invoke-ProfileSqlite "snapshot" $ProfileRoot $SourceId $OldMarker $NextMarker
            if ([long]$snapshot.nextMarkerActiveRows -gt 0) { return $snapshot }
        } catch {
            $lastReadError = $_.Exception.Message
        }
        Start-Sleep -Seconds 5
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $snapshot -and $lastReadError) { throw "SQLite snapshot never became readable: $lastReadError" }
    return $snapshot
}

function Get-FreeDriveLetter {
    $used = @([IO.DriveInfo]::GetDrives() | ForEach-Object { $_.Name.Substring(0, 1).ToUpperInvariant() })
    foreach ($candidate in @("Z", "Y", "X", "W", "V", "U", "T", "S", "R", "Q", "P", "O", "N", "M", "L", "K", "J", "I", "H", "G", "F", "E")) {
        if ($used -notcontains $candidate) { return $candidate }
    }
    throw "no unused drive letter for task-owned VHD"
}

function Invoke-DiskPart([string]$Name, [string[]]$Commands) {
    $path = Join-Path $taskRoot "diskpart-$Name.txt"
    Set-Content -LiteralPath $path -Value $Commands -Encoding ascii
    $output = (& diskpart.exe /s $path 2>&1 | Out-String).Trim()
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0 -or $output -match "(?im)(DiskPart has encountered an error|Virtual Disk Service error|The system failed to|could not be)") {
        throw "DiskPart $Name failed (exit=$exitCode): $output"
    }
    return $output
}

function New-U1Volume([string]$Name) {
    $drive = Get-FreeDriveLetter
    $vhd = Join-Path $taskRoot "$Name.vhdx"
    if (Test-Path -LiteralPath $vhd) { throw "refusing to overwrite VHD $vhd" }
    Invoke-DiskPart "$Name-create" @("create vdisk file=`"$vhd`" maximum=512 type=expandable", "exit") | Out-Null
    Invoke-DiskPart "$Name-format" @("select vdisk file=`"$vhd`"", "attach vdisk", "create partition primary", "format fs=ntfs quick label=ZC380U1", "assign letter=$drive", "exit") | Out-Null
    $volume = "$drive`:\"
    $disk = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='$drive`:'" -ErrorAction SilentlyContinue
    if (-not $disk -or $disk.FileSystem -ine "NTFS" -or $disk.DriveType -ne 3) { throw "task volume is not fixed NTFS: $volume" }
    $journal = (& fsutil.exe usn queryjournal "$drive`:" 2>&1 | Out-String).Trim()
    $journalExit = $LASTEXITCODE
    if ($journalExit -ne 0) {
        $create = (& fsutil.exe usn createjournal m=16777216 a=2097152 "$drive`:" 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw "could not create isolated USN journal: $create" }
        $journal = (& fsutil.exe usn queryjournal "$drive`:" 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw "isolated USN journal unavailable after create: $journal" }
    }
    $result = [ordered]@{ name = $Name; vhd = $vhd; drive = $drive; root = $volume; filesystem = $disk.FileSystem; usnJournal = $journal }
    $global:U1Evidence.cleanup.details += "created task-owned isolated volume $Name at $volume"
    return $result
}

function Remove-U1Volume($Volume) {
    if ($null -eq $Volume) { return }
    try {
        Invoke-DiskPart "$($Volume.name)-detach" @("select vdisk file=`"$($Volume.vhd)`"", "detach vdisk", "exit") | Out-Null
        if (Test-Path -LiteralPath $Volume.vhd) { Remove-Item -LiteralPath $Volume.vhd -Force }
        $global:U1Evidence.cleanup.details += "detached and removed task-owned isolated volume $($Volume.name)"
    } catch {
        $global:U1Evidence.cleanup.failures += "volume cleanup $($Volume.name): $($_.Exception.Message)"
    }
}

function Set-ChildProfileEnvironment([string]$ProfileRoot, [string]$TracePath, [string]$WatcherRoot = "") {
    $local = Join-Path $ProfileRoot "local"
    $roaming = Join-Path $ProfileRoot "roaming"
    $temp = Join-Path $ProfileRoot "temp"
    foreach ($path in @($ProfileRoot, $local, $roaming, $temp)) { New-Item -ItemType Directory -Path $path -Force | Out-Null }
    $environment = @{ ZC_NATIVE_QA_PROFILE_ROOT = $ProfileRoot; ZC_GLOBAL_INDEX_QA_TRACE = $TracePath; APPDATA = $roaming; LOCALAPPDATA = $local; TEMP = $temp; TMP = $temp }
    if (-not [string]::IsNullOrWhiteSpace($WatcherRoot)) {
        $environment.ZC_NATIVE_QA_WATCHER_TRACE = "1"
        $environment.ZC_NATIVE_QA_WATCHER_TRACE_ROOT = $WatcherRoot
    }
    return $environment
}

function Start-U1App([string]$Exe, [string]$Scenario, [string]$ProfileRoot, [string]$TracePath, [string]$WatcherRoot = "") {
    $stdout = Join-Path $taskRoot "$Scenario-app.stdout.log"
    $stderr = Join-Path $taskRoot "$Scenario-app.stderr.log"
    $environment = Set-ChildProfileEnvironment $ProfileRoot $TracePath $WatcherRoot
    $previous = @{}
    foreach ($name in $environment.Keys) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, "Process") }
    foreach ($name in $environment.Keys) { [Environment]::SetEnvironmentVariable($name, [string]$environment[$name], "Process") }
    try {
        return Start-Process -FilePath $Exe -ArgumentList @("--background") -WorkingDirectory (Split-Path -Parent $Exe) -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    } finally {
        foreach ($name in $environment.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name], "Process") }
    }
}

function Wait-U1AppReady($Process, [string]$Scenario, [int]$TimeoutSeconds = 90) {
    $stderr = Join-Path $taskRoot "$Scenario-app.stderr.log"
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $Process.Refresh()
        if ($Process.HasExited) { throw "$Scenario desktop exited before its isolated watcher was ready (exit=$($Process.ExitCode))" }
        if (Test-Path -LiteralPath $stderr) {
            $content = Get-Content -LiteralPath $stderr -Raw -ErrorAction SilentlyContinue
            if ($content -match "native_qa startup_checkpoint=setup_complete") { return }
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "$Scenario desktop did not reach setup_complete within $TimeoutSeconds seconds"
}

function Wait-ForWatcherPath([string]$Scenario, [string]$PathFragment, [int]$TimeoutSeconds = 45) {
    $stderr = Join-Path $taskRoot "$Scenario-app.stderr.log"
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-Path -LiteralPath $stderr) {
            $content = Get-Content -LiteralPath $stderr -Raw -ErrorAction SilentlyContinue
            if ($content -and $content.Contains($PathFragment, [StringComparison]::OrdinalIgnoreCase)) { return $true }
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Get-U1TraceCount([string]$TracePath, [string]$Event) {
    if (-not (Test-Path -LiteralPath $TracePath)) { return 0 }
    $lines = @(Get-Content -LiteralPath $TracePath -ErrorAction SilentlyContinue)
    return @($lines | Where-Object { $_ -ceq $Event }).Count
}

function Wait-U1TraceCount([string]$TracePath, [string]$Event, [int]$MinimumCount, [int]$TimeoutSeconds = 60) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if ((Get-U1TraceCount $TracePath $Event) -ge $MinimumCount) { return $true }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Wait-U1TraceContains([string]$TracePath, [string]$Text, [int]$TimeoutSeconds = 60) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-Path -LiteralPath $TracePath) {
            $content = Get-Content -LiteralPath $TracePath -Raw -ErrorAction SilentlyContinue
            if ($content -and $content.Contains($Text, [StringComparison]::Ordinal)) { return $true }
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Start-U1ConsoleService([string]$Exe, [string]$Scenario) {
    $stdout = Join-Path $taskRoot "$Scenario-service.stdout.log"
    $stderr = Join-Path $taskRoot "$Scenario-service.stderr.log"
    $process = Start-Process -FilePath $Exe -ArgumentList @("--index-service") -WorkingDirectory (Split-Path -Parent $Exe) -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        if ($process.HasExited) { throw "console service exited early (exit=$($process.ExitCode)); $(& Get-Content -LiteralPath $stderr -Raw -ErrorAction SilentlyContinue)" }
        $pipe = @(Get-ChildItem -LiteralPath "\\.\pipe\" -ErrorAction SilentlyContinue | Where-Object { $_.Name -ieq "ZenCanvas.GlobalIndex.v3" })
        if ($pipe.Count -gt 0) { return $process }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "console service process did not expose the v3 named pipe within 30 seconds"
}

function Stop-U1Process($Process) {
    if ($null -eq $Process) { return }
    try {
        $Process.Refresh()
        if (-not $Process.HasExited) { $Process.Kill($true); $Process.WaitForExit(10000) | Out-Null }
        $Process.Dispose()
    } catch { $global:U1Evidence.cleanup.failures += "process cleanup: $($_.Exception.Message)" }
}

function Wait-ForFixture([string]$ProbeExe, [string]$ProfileRoot, [string]$MarkerName, [int]$TimeoutSeconds = 180) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $snapshot = $null
    do {
        $snapshot = Invoke-ProfileQuery $ProbeExe $ProfileRoot $MarkerName
        $hit = @($snapshot.results | Where-Object { $_.name -ceq $MarkerName }) | Select-Object -First 1
        if ($hit) { return [pscustomobject]@{ Snapshot = $snapshot; Hit = $hit } }
        Start-Sleep -Seconds 5
    } while ([DateTime]::UtcNow -lt $deadline)
    return [pscustomobject]@{ Snapshot = $snapshot; Hit = $null }
}

function Wait-PipeAbsent([int]$TimeoutSeconds = 15) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $pipe = @(Get-ChildItem -LiteralPath "\\.\pipe\" -ErrorAction SilentlyContinue | Where-Object { $_.Name -ieq "ZenCanvas.GlobalIndex.v3" })
        if ($pipe.Count -eq 0) { return }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "v3 pipe remained visible after the task-owned console Service stopped"
}

function Invoke-BackgroundPairScenario([string]$Name, [string]$ClientExe, [string]$ClientProbe, [string]$ClientSha, [string]$ServiceExe, [string]$BaselineServiceExe, [bool]$ExpectDirect, [bool]$UseRecoveryQa = $false) {
    $volume = $null
    $service = $null
    $client = $null
    $profile = Join-Path $taskRoot "$Name-profile"
    $trace = Join-Path $taskRoot "$Name-trace.log"
    $safeName = $Name -replace "[^A-Za-z0-9]", ""
    $markerName = "u1${safeName}$([Guid]::NewGuid().ToString('N')).txt"
    $nextMarkerName = "u1next${safeName}$([Guid]::NewGuid().ToString('N')).txt"
    $result = [ordered]@{ status = "RUNNING"; clientSha256 = $ClientSha; serviceExe = $ServiceExe; useRecoveryQa = $UseRecoveryQa; pipeObserved = $false; markerName = $markerName; nextMarkerName = $nextMarkerName; baselineIndexed = $false; markerFound = $false; oldMarkerPreserved = $false; sourceState = $null; baselineSourceState = $null; entryCount = $null; baselineEntryCount = $null; resultPath = $null; clientExitCode = $null; recoveryStdout = $null; recoveryStderr = $null; recoveryReport = $null; trace = $trace; logs = @() }
    try {
        $volume = New-U1Volume $Name
        $fixtureRoot = Join-Path $volume.root "fixture"
        New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
        $markerPath = Join-Path $fixtureRoot $markerName
        Set-Content -LiteralPath $markerPath -Value "isolated U1 old active marker" -NoNewline
        $prepared = Invoke-ProfilePrepare $ClientProbe $profile $volume.drive
        $result.sourceId = $prepared.targetSourceId

        # First establish a real, matching old-index baseline. The second file
        # and metadata-only re-arm make coordinator startup exercise a genuine
        # initial index request while preserving the old active entry and FTS.
        if (-not $BaselineServiceExe) { throw "a matching Service is required to establish the baseline" }
        $service = Start-U1ConsoleService $BaselineServiceExe "$Name-baseline"
        $client = Start-U1App $ClientExe "$Name-baseline" $profile "$trace-baseline" $fixtureRoot
        $result.baselineClientPid = $client.Id
        $baseline = Wait-ForFixture $ClientProbe $profile $markerName 180
        if (-not $baseline.Hit) { throw "matching baseline did not index the original fixture marker" }
        $baselineSource = @($baseline.Snapshot.volumes | Where-Object { $_.id -ceq $prepared.targetSourceId } | Select-Object -First 1)
        $result.baselineIndexed = $true
        if ($baselineSource.Count -gt 0) { $result.baselineSourceState = $baselineSource[0].status; $result.baselineEntryCount = [long]$baselineSource[0].entryCount }
        Stop-U1Process $client
        $client = $null
        Stop-U1Process $service
        $service = $null
        Wait-PipeAbsent

        $nextMarkerPath = Join-Path $fixtureRoot $nextMarkerName
        Set-Content -LiteralPath $nextMarkerPath -Value "isolated U1 post-upgrade marker" -NoNewline
        $result.reset = Invoke-ProfileSqlite "reset" $profile $prepared.targetSourceId $markerName $nextMarkerName
        $result.preTestSnapshot = $result.reset.after
        $before = $result.reset.after

        if ($ServiceExe) {
            $service = Start-U1ConsoleService $ServiceExe $Name
            $result.pipeObserved = $true
            $result.servicePid = $service.Id
            $serviceProcess = Get-CimInstance Win32_Process -Filter "ProcessId=$($service.Id)"
            $result.serviceImagePath = $serviceProcess.ExecutablePath
            $result.serviceSha256 = Get-Hash $ServiceExe
        }

        if ($UseRecoveryQa) {
            $qaReport = Join-Path $EvidenceRoot "$Name-recovery-qa.json"
            $captured = Invoke-ProcessCaptured $ClientExe @("--global-index-recovery-qa", $profile, $prepared.targetSourceId, $qaReport) (Split-Path -Parent $ClientExe) @{} 180
            $result.clientExitCode = $captured.ExitCode
            $result.recoveryStdout = $captured.Stdout
            $result.recoveryStderr = $captured.Stderr
            if (Test-Path -LiteralPath $qaReport) { $result.recoveryReport = $qaReport }
            if ($captured.ExitCode -eq 0) { throw "D1-A recovery QA unexpectedly succeeded against the reachable old Service at a different image path" }
            $result.coordinatorCycleObserved = $true
        } else {
            $client = Start-U1App $ClientExe $Name $profile $trace $fixtureRoot
            $result.clientPid = $client.Id
            Wait-U1AppReady $client $Name
            $result.coordinatorCycleObserved = Wait-U1TraceCount $trace "coordinator_cycle" 1 90
            if (-not $result.coordinatorCycleObserved) { throw "tested client did not execute its real startup coordinator cycle" }
            if ($ExpectDirect) {
                $after = Wait-ForProfileMarker $profile $prepared.targetSourceId $markerName $nextMarkerName 240
            } else {
                Start-Sleep -Seconds 30
                $after = Invoke-ProfileSqlite "snapshot" $profile $prepared.targetSourceId $markerName $nextMarkerName
            }
        }

        if ($UseRecoveryQa) { $after = Invoke-ProfileSqlite "snapshot" $profile $prepared.targetSourceId $markerName $nextMarkerName }
        $result.postTestSnapshot = $after
        $result.markerFound = [long]$after.nextMarkerActiveRows -gt 0
        $result.nextMarkerFtsHits = [long]$after.nextMarkerFtsHits
        $result.oldMarkerPreserved = [long]$after.oldMarkerActiveRows -eq 1 -and [long]$after.oldMarkerFtsHits -gt 0
        $result.sourceState = $after.source.index_status
        $result.lastError = $after.source.last_error
        $result.entryCount = [long]$after.source.entry_count
        $result.baselineEntryCount = [long]$before.source.entry_count
        $result.entryCountMatchesActiveRows = [long]$after.source.entry_count -eq [long]$after.activeEntries
        $result.expectedDirect = $ExpectDirect
        if (-not $result.oldMarkerPreserved) { throw "old active Global Entry or its FTS row changed during the tested generation pairing" }

        if ($ExpectDirect) {
            if (-not $result.markerFound -or $result.nextMarkerFtsHits -lt 1) { throw "old client with absent Service did not perform the expected legacy Direct index; source=$($result.sourceState), error=$($result.lastError)" }
            if ([long]$after.activeEntries -le [long]$before.activeEntries) { throw "Direct fallback did not increase the active Global Entry set" }
            if ($result.sourceState -ine "ready") { throw "Direct fallback completed without READY source state: $($result.sourceState)" }
            $result.status = "OBSERVED_LEGACY_DIRECT_FALLBACK"
            $result.securityFinding = "Legacy client mutated the isolated Global Index while the Service was absent. Stop-the-world must prevent this state; v3 client behavior alone does not."
        } else {
            if ($result.markerFound -or $result.nextMarkerFtsHits -ne 0) { throw "mixed-generation pairing indexed the post-baseline marker without an admitted Service request" }
            if ([long]$after.activeEntries -ne [long]$before.activeEntries -or [long]$after.staleEntries -ne [long]$before.staleEntries) { throw "fail-closed mixed pairing changed active/stale Global Entry rows" }
            if ([long]$after.source.entry_count -ne [long]$before.source.entry_count) { throw "fail-closed mixed pairing changed source entry_count" }
            foreach ($field in @("index_status", "last_error", "journal_id", "journal_cursor", "last_full_index_at", "last_incremental_sync_at", "provider")) {
                $beforeValue = $before.source.PSObject.Properties[$field].Value
                $afterValue = $after.source.PSObject.Properties[$field].Value
                if ($afterValue -cne $beforeValue) { throw "fail-closed mixed pairing changed source $field from '$beforeValue' to '$afterValue'" }
            }
            if (-not $result.pipeObserved) { throw "mixed-pair test did not prove a reachable Service pipe" }
            if ($UseRecoveryQa) { $result.status = "PASS_NEW_CLIENT_FAIL_CLOSED_OLD_SERVICE" }
            else { $result.status = "PASS_OLD_CLIENT_NO_DIRECT_ON_PATH_REJECTION" }
            $result.scope = "real candidate binaries from separate image paths; exact same-path mixed generations remain separately classified"
        }
    } catch {
        $result.status = "HARNESS_FAILURE"
        $result.failure = $_.Exception.ToString()
        Add-Failure $Name $result.failure
    } finally {
        Stop-U1Process $client
        Stop-U1Process $service
        foreach ($suffix in @("$Name-app.stdout.log", "$Name-app.stderr.log", "$Name-baseline-app.stdout.log", "$Name-baseline-app.stderr.log")) {
            $path = Join-Path $taskRoot $suffix
            if (Test-Path -LiteralPath $path) {
                $destination = Join-Path $EvidenceRoot $suffix
                Copy-Item -LiteralPath $path -Destination $destination -Force
                $result.logs += $destination
            }
        }
        foreach ($suffix in @("$Name-service.stdout.log", "$Name-service.stderr.log", "$Name-baseline-service.stdout.log", "$Name-baseline-service.stderr.log")) {
            $path = Join-Path $taskRoot $suffix
            if (Test-Path -LiteralPath $path) {
                $destination = Join-Path $EvidenceRoot $suffix
                Copy-Item -LiteralPath $path -Destination $destination -Force
                $result.logs += $destination
            }
        }
        if ($UseRecoveryQa -and $result.recoveryStdout) { $result.recoveryStdout | Set-Content -LiteralPath (Join-Path $EvidenceRoot "$Name-recovery-qa.stdout.log") -Encoding utf8NoBOM }
        if ($UseRecoveryQa -and $result.recoveryStderr) { $result.recoveryStderr | Set-Content -LiteralPath (Join-Path $EvidenceRoot "$Name-recovery-qa.stderr.log") -Encoding utf8NoBOM }
        if (Test-Path -LiteralPath $trace) {
            $traceCopy = Join-Path $EvidenceRoot "$Name-trace.log"
            Copy-Item -LiteralPath $trace -Destination $traceCopy -Force
            $result.trace = $traceCopy
        }
        $databasePath = Join-Path $profile "zen-canvas.sqlite3"
        if (Test-Path -LiteralPath $databasePath) {
            $profileArtifact = Join-Path $EvidenceRoot "$Name-profile"
            New-Item -ItemType Directory -Path $profileArtifact -Force | Out-Null
            foreach ($suffix in @("zen-canvas.sqlite3", "zen-canvas.sqlite3-wal", "zen-canvas.sqlite3-shm")) {
                $sourcePath = Join-Path $profile $suffix
                if (Test-Path -LiteralPath $sourcePath) { Copy-Item -LiteralPath $sourcePath -Destination (Join-Path $profileArtifact $suffix) -Force }
            }
            $result.databaseSha256 = Get-Hash (Join-Path $profileArtifact "zen-canvas.sqlite3")
            $result.profileFileHashes = @(Get-ChildItem -LiteralPath $profileArtifact -File | ForEach-Object { [ordered]@{ name = $_.Name; sha256 = Get-Hash $_.FullName; bytes = $_.Length } })
            $result.databaseArtifact = $profileArtifact
        }
        Remove-U1Volume $volume
        if (Test-Path -LiteralPath $profile) { Remove-Item -LiteralPath $profile -Recurse -Force -ErrorAction SilentlyContinue }
    }
    return $result
}

function Get-ServiceSnapshot {
    $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
    if (-not $service) { return $null }
    return [ordered]@{ state = $service.State; processId = [int]$service.ProcessId; imagePath = $service.PathName; exitCode = $service.ExitCode; serviceSpecificExitCode = $service.ServiceSpecificExitCode; process = if ($service.ProcessId) { (Get-CimInstance Win32_Process -Filter "ProcessId=$($service.ProcessId)" -ErrorAction SilentlyContinue | Select-Object -First 1 | ForEach-Object { [ordered]@{ path = $_.ExecutablePath; created = $_.CreationDate } }) } else { $null } }
}

function Wait-ServiceState([string]$State, [int]$TimeoutSeconds = 90) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $snapshot = Get-ServiceSnapshot
        if ($snapshot -and $snapshot.state -ieq $State) { return $snapshot }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "ZenCanvasGlobalIndex did not reach $State; last state=$($snapshot | ConvertTo-Json -Compress)"
}

function Get-AppProcessSnapshots([string]$ExePath) {
    $canonical = [IO.Path]::GetFullPath($ExePath)
    $imageName = [IO.Path]::GetFileName($canonical)
    return @(Get-CimInstance Win32_Process -Filter "Name='$imageName'" -ErrorAction SilentlyContinue | Where-Object { $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).Equals($canonical, [StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { [ordered]@{ processId = [int]$_.ProcessId; path = $_.ExecutablePath; commandLine = $_.CommandLine; created = $_.CreationDate } })
}

function Get-U1RUninstallerProcessSnapshots([string]$InstallDirectory) {
    $canonical = [IO.Path]::GetFullPath((Join-Path $InstallDirectory "uninstall.exe"))
    return @(Get-CimInstance Win32_Process -Filter "Name='uninstall.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).Equals($canonical, [StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { [ordered]@{ processId = [int]$_.ProcessId; path = $_.ExecutablePath; commandLine = $_.CommandLine; created = $_.CreationDate } })
}

function Get-InstallerProductRegistrationSnapshot {
    $rootPath = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"
    $results = [Collections.Generic.List[object]]::new()
    if (Test-Path -LiteralPath $rootPath) {
        foreach ($key in Get-ChildItem -LiteralPath $rootPath -ErrorAction SilentlyContinue) {
            try {
                $properties = Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction Stop
                if ($properties.DisplayName -eq "Zen Canvas" -or $properties.MainBinaryName -eq "zen-canvas.exe") {
                    $values = [ordered]@{}
                    foreach ($name in @("DisplayName", "MainBinaryName", "DisplayVersion", "Publisher", "InstallLocation", "UninstallString", "EstimatedSize", "NoModify", "NoRepair")) {
                        $values[$name] = $properties.$name
                    }
                    $results.Add([ordered]@{ key = $key.Name; values = $values })
                }
            } catch { }
        }
    }
    return @($results)
}

function Install-NSIS([string]$Installer, [string]$InstallDirectory, [string]$LogPath, [int]$TimeoutSeconds = 600) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Installer
    $startInfo.WorkingDirectory = Split-Path -Parent $Installer
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    # The frozen per-machine package intentionally uses its canonical
    # Program Files install location on this disposable Hosted Windows runner.
    # Avoid relying on NSIS /D parsing for a path containing spaces; the caller
    # first verifies that this exact location is absent and task-owned.
    foreach ($argument in @("/S", "/LOG=$LogPath")) { $startInfo.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to start installer $Installer" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill($true)
        $process.WaitForExit(10000) | Out-Null
        $process.Dispose()
        throw "installer timed out after $TimeoutSeconds seconds: $Installer"
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $exitCode = $process.ExitCode
    $process.Dispose()
    $log = if (Test-Path -LiteralPath $LogPath) { [ordered]@{ exists = $true; bytes = (Get-Item -LiteralPath $LogPath).Length; sha256 = Get-Hash $LogPath; text = (Get-Content -LiteralPath $LogPath -Raw -ErrorAction SilentlyContinue) } } else { [ordered]@{ exists = $false; bytes = 0; sha256 = $null; text = $null } }
    return [pscustomobject]@{ ExitCode = $exitCode; Stdout = $stdout; Stderr = $stderr; Log = $log }
}

function Wait-ForInstaller([string]$Installer, [string]$InstallDirectory, [string]$LogPath, [string]$ExpectedExeHash, [string]$OriginalExeHash, [int]$TimeoutSeconds, [bool]$InterruptOnReplacement, [string[]]$ExtraArguments = @()) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Installer
    $startInfo.WorkingDirectory = Split-Path -Parent $Installer
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in (@("/S", "/LOG=$LogPath") + $ExtraArguments)) { $startInfo.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to start installer $Installer" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $exe = Join-Path $InstallDirectory "zen-canvas.exe"
    $samples = [Collections.Generic.List[object]]::new()
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $interrupted = $false
    $fileStamp = $null
    $fileHash = $null
    do {
        $process.Refresh()
        $file = Get-Item -LiteralPath $exe -ErrorAction SilentlyContinue
        $nextStamp = if ($file) { "$($file.Length):$($file.LastWriteTimeUtc.Ticks)" } else { $null }
        if ($nextStamp -cne $fileStamp -or ($file -and -not $fileHash)) {
            $fileStamp = $nextStamp
            $fileHash = $null
            if ($file) {
                try { $fileHash = Get-Hash $exe } catch { $fileHashError = $_.Exception.Message }
            }
        }
        $serviceSnapshot = Get-ServiceSnapshot
        $apps = @(Get-AppProcessSnapshots $exe)
        $samples.Add([ordered]@{ atUtc = [DateTime]::UtcNow.ToString("o"); installerPid = $process.Id; installerExited = $process.HasExited; exeSha256 = $fileHash; targetExeHashObserved = [bool]($ExpectedExeHash -and $fileHash -ceq $ExpectedExeHash); service = $serviceSnapshot; appProcesses = $apps; uninstallerProcesses = @(Get-U1RUninstallerProcessSnapshots $InstallDirectory) })
        if ($InterruptOnReplacement -and -not $process.HasExited) {
            # Interrupt on the first observed payload change from the measured
            # old installed image. Package extraction is recorded separately
            # because Tauri/NSIS may patch PE resources.
            $replacementObserved = $fileHash -and $fileHash -cne $OriginalExeHash
            if ($replacementObserved) {
                $process.Kill($true)
                $process.WaitForExit(10000) | Out-Null
                $interrupted = $true
                break
            }
        }
        if ($process.HasExited) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    $process.Refresh()
    if (-not $process.HasExited) {
        $process.Kill($true)
        $process.WaitForExit(10000) | Out-Null
        $process.Dispose()
        throw "installer did not finish within $TimeoutSeconds seconds"
    }
    $exitCode = $process.ExitCode
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $process.Dispose()
    $log = if (Test-Path -LiteralPath $LogPath) { [ordered]@{ exists = $true; bytes = (Get-Item -LiteralPath $LogPath).Length; sha256 = Get-Hash $LogPath; text = (Get-Content -LiteralPath $LogPath -Raw -ErrorAction SilentlyContinue) } } else { [ordered]@{ exists = $false; bytes = 0; sha256 = $null; text = $null } }
    return [pscustomobject]@{ ExitCode = $exitCode; Interrupted = $interrupted; Samples = @($samples); LogPath = $LogPath; Log = $log; Stdout = $stdout; Stderr = $stderr }
}

function Build-Candidate([string]$Label, [string]$Sha, [string]$Tree, [string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        & git -C $repoRoot worktree add --detach $Path $Sha *> (Join-Path $EvidenceRoot "$Label-worktree.log")
        if ($LASTEXITCODE -ne 0) { throw "could not create $Label source worktree" }
    }
    $actualSha = Get-GitValue $Path "HEAD"
    $actualTree = Get-GitValue $Path "HEAD^{tree}"
    if ($actualSha -cne $Sha -or $actualTree -cne $Tree) { throw "$Label source identity mismatch: got $actualSha $actualTree expected $Sha $Tree" }
    $status = @(git -C $Path status --porcelain)
    if ($LASTEXITCODE -ne 0 -or $status.Count -gt 0) { throw "$Label frozen worktree is not clean before build: $($status -join '; ')" }

    $global:U1Evidence[$Label].sourceStatus = "PASS_EXACT_CLEAN"
    $global:U1Evidence[$Label].sourceTree = $actualTree
    $global:U1Evidence[$Label].worktree = $Path
    Save-Evidence

    $env:CARGO_TARGET_DIR = $cargoTarget
    Invoke-LoggedCommand "$Label-npm-ci" "npm.cmd" @("ci") $Path | Out-Null
    Invoke-LoggedCommand "$Label-frontend" "npm.cmd" @("run", "build:frontend") $Path | Out-Null
    $u1rInstrumentationFiles = @()
    if ($Mode -eq "U1R") {
        Add-U1RTestInstrumentation $Label $Path
        Add-U1RNsisDiagnostics $Label $Path
        $u1rInstrumentationFiles = if ($Label -eq "old") {
            @("src-tauri/src/main.rs", "src-tauri/src/global_index/windows/mod.rs", "src-tauri/windows/installer-hooks.nsh")
        } else {
            @("src-tauri/src/global_index/windows/service.rs", "src-tauri/src/global_index/windows/service_host.rs", "scripts/buildWindowsPackage.mjs", "src-tauri/windows/installer-hooks.nsh")
        }
    }
    Invoke-LoggedCommand "$Label-qa-build" "cargo.exe" @("build", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--release", "--features", "desktop-runtime native-qa", "--bin", "zen-canvas", "--bin", "zb05-global-index-qa") $Path | Out-Null
    $qaDir = Join-Path $Path "src-tauri/target/release/u1-qa"
    New-Item -ItemType Directory -Path $qaDir -Force | Out-Null
    $qaExe = Join-Path $qaDir "zen-canvas-qa.exe"
    $probeExe = Join-Path $qaDir "zb05-global-index-qa.exe"
    Copy-Item -LiteralPath (Join-Path $cargoTarget "release/zen-canvas.exe") -Destination $qaExe -Force
    Copy-Item -LiteralPath (Join-Path $cargoTarget "release/zb05-global-index-qa.exe") -Destination $probeExe -Force
    $qaHash = Get-Hash $qaExe
    $probeHash = Get-Hash $probeExe

    $priorNsisDiagnostics = $env:ZC_U1R_NSIS_DIAGNOSTICS
    try {
        if ($Mode -eq "U1R" -and $Label -eq "d1a") { $env:ZC_U1R_NSIS_DIAGNOSTICS = "1" }
        else { Remove-Item Env:ZC_U1R_NSIS_DIAGNOSTICS -ErrorAction SilentlyContinue }
        Invoke-LoggedCommand "$Label-nsis-package" "npm.cmd" @("run", "build") $Path | Out-Null
    } finally {
        if ($null -eq $priorNsisDiagnostics) { Remove-Item Env:ZC_U1R_NSIS_DIAGNOSTICS -ErrorAction SilentlyContinue }
        else { $env:ZC_U1R_NSIS_DIAGNOSTICS = $priorNsisDiagnostics }
    }
    $releaseExe = Join-Path $cargoTarget "release/zen-canvas.exe"
    $installerCandidates = @(Get-ChildItem -LiteralPath (Join-Path $cargoTarget "release/bundle/nsis") -Filter "*.exe" -File | Sort-Object LastWriteTimeUtc -Descending)
    if (-not (Test-Path -LiteralPath $releaseExe) -or $installerCandidates.Count -lt 1) { throw "$Label NSIS output or desktop executable missing after build" }
    $installer = Join-Path $qaDir "$Label-setup.exe"
    Copy-Item -LiteralPath $installerCandidates[0].FullName -Destination $installer -Force
    $releaseHash = Get-Hash $releaseExe
    $installerHash = Get-Hash $installer
    $packageExeHash = $null
    $packageExeEvidence = [ordered]@{ status = "UNVERIFIED"; extractionTool = $null; payloadFiles = @() }
    $sevenZip = Get-Command "7z.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $sevenZip) {
        $sevenZipPath = Join-Path $env:ProgramFiles "7-Zip/7z.exe"
        if (Test-Path -LiteralPath $sevenZipPath) { $sevenZip = [pscustomobject]@{ Source = $sevenZipPath } }
    }
    if ($sevenZip) {
        $payloadRoot = Join-Path $taskRoot "$Label-nsis-payload"
        New-Item -ItemType Directory -Path $payloadRoot -Force | Out-Null
        $extractLog = Join-Path $EvidenceRoot "$Label-nsis-payload-extract.log"
        $extractOutput = (& $sevenZip.Source x $installer "-o$payloadRoot" -y 2>&1 | Out-String)
        $extractExit = $LASTEXITCODE
        $extractOutput | Set-Content -LiteralPath $extractLog -Encoding utf8NoBOM
        $payloadFiles = @(Get-ChildItem -LiteralPath $payloadRoot -Recurse -File -Filter "zen-canvas.exe" -ErrorAction SilentlyContinue)
        $packageExeEvidence = [ordered]@{ status = if ($extractExit -eq 0 -and $payloadFiles.Count -gt 0) { "PASS_EXTRACTED" } else { "UNVERIFIED" }; extractionTool = $sevenZip.Source; exitCode = $extractExit; log = $extractLog; payloadFiles = @($payloadFiles | ForEach-Object { [ordered]@{ path = $_.FullName; sha256 = Get-Hash $_.FullName; bytes = $_.Length } }) }
        if ($extractExit -eq 0 -and $payloadFiles.Count -gt 0) {
            $largestPayload = $payloadFiles | Sort-Object Length -Descending | Select-Object -First 1
            $packageExeHash = Get-Hash $largestPayload.FullName
            $packageExeEvidence.selectedPayload = $largestPayload.FullName
            $packageExeEvidence.selectedSha256 = $packageExeHash
            $packageExeEvidence.selectedMatchesReleaseExe = $packageExeHash -ceq $releaseHash
        }
    }
    $cargoVersion = (Get-Content -LiteralPath (Join-Path $Path "src-tauri/Cargo.toml") -Raw | Select-String -Pattern '(?m)^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value
    $packageVersion = (Get-Content -LiteralPath (Join-Path $Path "package.json") -Raw | ConvertFrom-Json).version
    $tauriConfig = Get-Content -LiteralPath (Join-Path $Path "src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json
    $candidate = [ordered]@{ sha = $Sha; tree = $Tree; packageVersion = $packageVersion; cargoVersion = $cargoVersion; tauriVersion = $tauriConfig.version; identifier = $tauriConfig.identifier; qaExe = $qaExe; qaExeSha256 = $qaHash; probeExe = $probeExe; probeExeSha256 = $probeHash; productionExe = $releaseExe; productionExeSha256 = $releaseHash; packageExeSha256 = $packageExeHash; packageExeEvidence = $packageExeEvidence; installer = $installer; installerSha256 = $installerHash; fileVersion = (Get-Item -LiteralPath $releaseExe).VersionInfo.FileVersion; productVersion = (Get-Item -LiteralPath $releaseExe).VersionInfo.ProductVersion }
    $global:U1Evidence.candidates[$Label] = $candidate
    Save-Evidence

    if ($Mode -eq "U1R") {
        $manifestPath = Join-Path $Path "src-tauri/Cargo.toml"
        if (Test-Path -LiteralPath $manifestPath) {
            & git -C $Path checkout -- src-tauri/Cargo.toml
            if ($LASTEXITCODE -ne 0) { throw "$Label could not restore the packaging-generated Cargo.toml mutation before recording U1R instrumentation" }
        }
        $patch = (& git -C $Path diff --binary -- @u1rInstrumentationFiles | Out-String)
        $patchPath = Join-Path $EvidenceRoot "$Label-u1r-test-instrumentation.patch"
        $patch | Set-Content -LiteralPath $patchPath -Encoding utf8NoBOM
        $patchHash = Get-Hash $patchPath
        foreach ($relative in $u1rInstrumentationFiles) {
            & git -C $Path checkout -- $relative
            if ($LASTEXITCODE -ne 0) { throw "$Label could not restore test-only instrumentation in $relative" }
        }
        $global:U1Evidence["${Label}TestInstrumentation"] = [ordered]@{ patch = $patchPath; sha256 = $patchHash; files = $u1rInstrumentationFiles; restoredToFrozenSource = $true }
        if ($Label -eq "d1a") {
            $global:U1Evidence["d1aNsisInstaller"] = [ordered]@{ installerSha256 = $installerHash; diagnosticBuild = $true; diagnosticHookPatchSha256 = $patchHash }
        }
        Save-Evidence
    }

    $postBuildStatus = @(git -C $Path status --porcelain)
    $trackedChanges = @($postBuildStatus | Where-Object { $_ -notmatch '^\?\?' })
    if ($trackedChanges.Count -gt 0) {
        # The Windows Tauri packaging command rewrites Cargo.toml in this
        # frozen source tree. Retain its exact diff for the evidence record,
        # restore the frozen input bytes, then reject every other tracked
        # mutation. The candidate identity was already checked before build.
        $manifestRelativePath = "src-tauri/Cargo.toml"
        $manifestChange = @($trackedChanges | Where-Object { $_ -match '^\s*M\s+src-tauri/Cargo\.toml$' })
        $unexpectedChanges = @($trackedChanges | Where-Object { $_ -notmatch '^\s*M\s+src-tauri/Cargo\.toml$' })
        if ($manifestChange.Count -gt 0) {
            $manifestDiff = (& git -C $Path diff -- $manifestRelativePath | Out-String)
            & git -C $Path checkout -- $manifestRelativePath
            if ($LASTEXITCODE -ne 0) { throw "$Label could not restore generated Cargo.toml mutation" }
            $global:U1Evidence[$Label].packageGeneratedManifestDiff = $manifestDiff.TrimEnd()
            $global:U1Evidence[$Label].packageManifestRestoredToFrozenSource = $true
            Save-Evidence
        }
        if ($unexpectedChanges.Count -gt 0) { throw "$Label build changed unexpected tracked source files: $($unexpectedChanges -join '; ')" }
        $remainingChanges = @(git -C $Path status --porcelain | Where-Object { $_ -notmatch '^\?\?' })
        if ($remainingChanges.Count -gt 0) { throw "$Label frozen worktree remains modified after restoring package input: $($remainingChanges -join '; ')" }
    }
    return $candidate
}

function Invoke-MatchingQualification([string]$Label, $Candidate) {
    $evidencePath = Join-Path $EvidenceRoot "$Label-qualification.json"
    $worktree = $global:U1Evidence[$Label].worktree
    $global:U1Evidence[$Label].expectedSource = $Candidate.sha
    $oldExpected = $env:EXPECTED_SOURCE_SHA
    $env:EXPECTED_SOURCE_SHA = $Candidate.sha
    try {
        Push-Location $worktree
        try {
            & pwsh.exe -NoProfile -File (Join-Path $worktree "scripts/qualifyWindowsGlobalIndexService.ps1") -CandidateExe $Candidate.qaExe -ProbeExe $Candidate.probeExe -EvidencePath $evidencePath
            $exitCode = $LASTEXITCODE
        } finally { Pop-Location }
        if ($exitCode -ne 0) { throw "matching-pair exact SCM qualification failed; see $evidencePath" }
        $global:U1Evidence.matchingPairs[$Label] = [ordered]@{ status = "PASS"; clientSha256 = $Candidate.qaExeSha256; serviceSha256 = $Candidate.qaExeSha256; evidence = $evidencePath }
        Save-Evidence
    } finally {
        $env:EXPECTED_SOURCE_SHA = $oldExpected
    }
}

function Invoke-InstallerTransition($OldCandidate, $D1ACandidate) {
    $serviceName = "ZenCanvasGlobalIndex"
    $existing = Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue
    if ($existing) { throw "refusing installer test because $serviceName pre-exists: $($existing.PathName)" }
    $installDir = Join-Path $env:ProgramFiles "Zen Canvas"
    $exe = Join-Path $installDir "zen-canvas.exe"
    if (Test-Path -LiteralPath $installDir) { throw "refusing installer test because canonical per-machine path already exists: $installDir" }
    $global:U1OwnedInstallDirectory = $installDir
    $oldLog = Join-Path $EvidenceRoot "installer-old-fresh.log"
    $newUpgradeLog = Join-Path $EvidenceRoot "installer-d1a-upgrade.log"
    $oldRepairLog = Join-Path $EvidenceRoot "installer-old-reinstall.log"
    $interruptedLog = Join-Path $EvidenceRoot "installer-d1a-interrupted.log"
    $repairLog = Join-Path $EvidenceRoot "installer-d1a-repair.log"
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $oldInstall = Install-NSIS $OldCandidate.installer $installDir $oldLog
    if ($oldInstall.ExitCode -ne 0) { throw "fresh old installer returned exit code $($oldInstall.ExitCode); $oldLog" }
    $oldService = Wait-ServiceState "Running" 120
    $installedOldHash = Get-Hash $exe
    $global:U1Evidence.installer.freshOldInstall = [ordered]@{ exePath = $exe; sourceExeSha256 = $OldCandidate.productionExeSha256; packageExeSha256 = $OldCandidate.packageExeSha256; installedExeSha256 = $installedOldHash; installedMatchesPackagePayload = if ($OldCandidate.packageExeSha256) { $installedOldHash -ceq $OldCandidate.packageExeSha256 } else { $null }; fileVersion = (Get-Item -LiteralPath $exe).VersionInfo.FileVersion; productVersion = (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion; service = $oldService; log = $oldLog }
    Save-Evidence
    if ($OldCandidate.packageExeSha256 -and $installedOldHash -cne $OldCandidate.packageExeSha256) { throw "fresh old installed EXE differs from extracted NSIS payload: expected=$($OldCandidate.packageExeSha256), installed=$installedOldHash, path=$exe" }
    $expectedImage = '"' + $exe + '" --index-service'
    if ($oldService.imagePath.Trim() -ine $expectedImage) { throw "fresh old service ImagePath mismatch: $($oldService.imagePath)" }

    $app = $null
    try {
        $installedAppProfile = Join-Path $taskRoot "installer-old-app-profile"
        $app = Start-U1App $exe "installer-old-app" $installedAppProfile (Join-Path $EvidenceRoot "installer-old-app.trace.log")
        Start-Sleep -Seconds 3
        $oldApps = @(Get-AppProcessSnapshots $exe)
        if (-not ($oldApps | Where-Object { $_.processId -eq $app.Id })) { throw "old installed desktop process did not remain alive before upgrade" }
        $preUpgrade = [ordered]@{ atUtc = [DateTime]::UtcNow.ToString("o"); service = $oldService; desktopProcessId = $app.Id; installedExeSha256 = Get-Hash $exe; isolatedAppProfile = $installedAppProfile; productRegistration = @(Get-InstallerProductRegistrationSnapshot) }
        $transition = Wait-ForInstaller $D1ACandidate.installer $installDir $newUpgradeLog $D1ACandidate.packageExeSha256 $installedOldHash 600 $false
        $global:U1Evidence.installer.upgradeAttempt = [ordered]@{ exitCode = $transition.ExitCode; interrupted = $transition.Interrupted; samples = $transition.Samples; log = $transition.Log; stdout = $transition.Stdout; stderr = $transition.Stderr; finalService = Get-ServiceSnapshot; finalDesktopProcesses = @(Get-AppProcessSnapshots $exe); finalExeSha256 = if (Test-Path -LiteralPath $exe) { Get-Hash $exe } else { $null }; productRegistration = @(Get-InstallerProductRegistrationSnapshot) }
        $global:U1Evidence.installer.samples = $transition.Samples
        $global:U1Evidence.installer.transitions = @($transition.Samples | ForEach-Object {
            $serviceState = if ($_.service) { $_.service.state } else { $null }
            $servicePid = if ($_.service) { $_.service.processId } else { $null }
            [ordered]@{ atUtc = $_.atUtc; installerExited = $_.installerExited; installedExeSha256 = $_.exeSha256; serviceState = $serviceState; servicePid = $servicePid; desktopPids = @($_.appProcesses | ForEach-Object processId) }
        })
        Save-Evidence
        if ($transition.ExitCode -ne 0) { throw "D1-A in-place upgrade returned $($transition.ExitCode); logExists=$($transition.Log.exists); samples=$($transition.Samples.Count); $newUpgradeLog" }
        $firstNewImageSample = -1
        for ($sampleIndex = 0; $sampleIndex -lt $transition.Samples.Count; $sampleIndex++) {
            if ($transition.Samples[$sampleIndex].exeSha256 -and $transition.Samples[$sampleIndex].exeSha256 -cne $installedOldHash) { $firstNewImageSample = $sampleIndex; break }
        }
        if ($firstNewImageSample -lt 0) { throw "installer samples did not observe the installed EXE change from its measured old payload" }
        $beforeImageMutation = @($transition.Samples[0..$firstNewImageSample] | Where-Object { $_.exeSha256 -ceq $installedOldHash })
        $serviceStoppedBeforeMutation = @($beforeImageMutation | Where-Object { $_.service -and $_.service.state -ieq "Stopped" }).Count -gt 0
        $desktopExitedBeforeMutation = @($beforeImageMutation | Where-Object { $_.appProcesses.Count -eq 0 }).Count -gt 0
        if (-not $serviceStoppedBeforeMutation) { throw "samples did not prove Service STOPPED before executable replacement" }
        if (-not $desktopExitedBeforeMutation) { throw "samples did not prove old desktop exited before executable replacement" }
        $postService = Wait-ServiceState "Running" 120
        $app.Refresh()
        if (-not $app.HasExited) { throw "old desktop process survived D1-A in-place upgrade" }
        if ($postService.processId -eq $oldService.processId) { throw "Service PID did not change across stop/restart" }
        $installedD1AHash = Get-Hash $exe
        if ($installedD1AHash -ceq $installedOldHash) { throw "D1-A installer left the old installed EXE payload unchanged" }
        if ($D1ACandidate.packageExeSha256 -and $installedD1AHash -cne $D1ACandidate.packageExeSha256) { throw "post-upgrade installed EXE differs from extracted D1-A NSIS payload: expected=$($D1ACandidate.packageExeSha256), installed=$installedD1AHash" }
        if ($postService.imagePath.Trim() -ine $expectedImage) { throw "post-upgrade Service ImagePath mismatch: $($postService.imagePath)" }
        $newApps = @(Get-AppProcessSnapshots $exe)
        $global:U1Evidence.installer.status = "PASS_OLD_TO_D1A_IN_PLACE"
        $global:U1Evidence.installer.installDirectory = $installDir
        $global:U1Evidence.installer.oldInstallLog = $oldLog
        $global:U1Evidence.installer.upgradeLog = $newUpgradeLog
        $global:U1Evidence.installer.preUpgrade = $preUpgrade
        $global:U1Evidence.installer.postUpgrade = [ordered]@{ service = $postService; sourceExeSha256 = $D1ACandidate.productionExeSha256; packageExeSha256 = $D1ACandidate.packageExeSha256; installedExeSha256 = $installedD1AHash; installedMatchesPackagePayload = if ($D1ACandidate.packageExeSha256) { $installedD1AHash -ceq $D1ACandidate.packageExeSha256 } else { $null }; oldDesktopExited = $app.HasExited; remainingDesktopProcesses = $newApps; installerExitCode = $transition.ExitCode; serviceStoppedBeforeMutation = $serviceStoppedBeforeMutation; desktopExitedBeforeMutation = $desktopExitedBeforeMutation; newServicePidAfterMutation = (@($transition.Samples[$firstNewImageSample..($transition.Samples.Count - 1)] | Where-Object { $_.service -and $_.service.state -ieq "Running" -and $_.service.processId -ne $oldService.processId }).Count -gt 0) }
        $global:U1Evidence.installer.samples = $transition.Samples
        $global:U1Evidence.installer.transitions = @($transition.Samples | ForEach-Object {
            $serviceState = if ($_.service) { $_.service.state } else { $null }
            $servicePid = if ($_.service) { $_.service.processId } else { $null }
            [ordered]@{ atUtc = $_.atUtc; installerExited = $_.installerExited; installedExeSha256 = $_.exeSha256; serviceState = $serviceState; servicePid = $servicePid; desktopPids = @($_.appProcesses | ForEach-Object processId) }
        })
        Save-Evidence
    } finally {
        Stop-U1Process $app
    }

    # Reinstall the old frozen package in the same disposable location so the
    # next operation can interrupt an actual old-to-new file mutation.
    $oldAgain = Install-NSIS $OldCandidate.installer $installDir $oldRepairLog
    if ($oldAgain.ExitCode -ne 0) { throw "old candidate reinstallation returned $($oldAgain.ExitCode); $oldRepairLog" }
    $oldBeforeInterrupt = Wait-ServiceState "Running" 120
    $reinstalledOldHash = Get-Hash $exe
    if ($OldCandidate.packageExeSha256 -and $reinstalledOldHash -cne $OldCandidate.packageExeSha256) { throw "old package did not restore the measured old NSIS payload before interruption test" }
    if ($reinstalledOldHash -ceq $installedD1AHash) { throw "old package reinstall did not change the installed executable payload" }
    $interruption = Wait-ForInstaller $D1ACandidate.installer $installDir $interruptedLog $D1ACandidate.packageExeSha256 $reinstalledOldHash 600 $true
    $global:U1Evidence.installer.interruption = [ordered]@{ status = if ($interruption.Interrupted) { "INTERRUPTED_AFTER_EXE_REPLACEMENT" } else { "UNVERIFIED_INSTALLER_COMPLETED_BEFORE_INTERRUPT" }; installerExitCode = $interruption.ExitCode; observedOldServicePid = $oldBeforeInterrupt.processId; log = $interruptedLog; samples = $interruption.Samples }
    $stoppedOrNewPid = Get-ServiceSnapshot
    $runningApps = @(Get-AppProcessSnapshots $exe)
    $global:U1Evidence.installer.interruption.serviceAfterKill = $stoppedOrNewPid
    $global:U1Evidence.installer.interruption.desktopProcessesAfterKill = $runningApps
    Save-Evidence
    if ($interruption.Interrupted) {
        if ($runningApps.Count -gt 0) { throw "desktop process remains after interrupted upgrade: $($runningApps | ConvertTo-Json -Compress)" }
        if ($stoppedOrNewPid -and $stoppedOrNewPid.state -ieq "Running" -and $stoppedOrNewPid.processId -eq $oldBeforeInterrupt.processId) { throw "old Service process remained RUNNING across a post-replacement interruption" }
        $repairInstall = Install-NSIS $D1ACandidate.installer $installDir $repairLog
        $repairExit = $repairInstall.ExitCode
        if ($repairExit -ne 0) { throw "D1-A repair after interruption returned $repairExit; $repairLog" }
        $repairedService = Wait-ServiceState "Running" 120
        $repairedExeHash = Get-Hash $exe
        if ($D1ACandidate.packageExeSha256 -and $repairedExeHash -cne $D1ACandidate.packageExeSha256) { throw "repair did not converge installed exe to extracted D1-A NSIS payload" }
        if ($repairedExeHash -ceq $reinstalledOldHash) { throw "repair left the old installed executable payload in place" }
        if ($repairedService.imagePath.Trim() -ine $expectedImage) { throw "repaired Service ImagePath mismatch" }
        if ($repairedService.processId -eq $oldBeforeInterrupt.processId) { throw "repair did not restart Service after interrupted install" }
        $global:U1Evidence.installer.repair = [ordered]@{ status = "PASS_REPAIR_TO_D1A"; exitCode = $repairExit; service = $repairedService; installedExeSha256 = $repairedExeHash; packageExeSha256 = $D1ACandidate.packageExeSha256; installedMatchesPackagePayload = if ($D1ACandidate.packageExeSha256) { $repairedExeHash -ceq $D1ACandidate.packageExeSha256 } else { $null }; log = $repairLog }
    } else {
        $global:U1Evidence.installer.repair = [ordered]@{ status = "NOT_RUN_INTERRUPT_NOT_OBSERVED"; log = $repairLog }
    }
    Save-Evidence
}

function Replace-U1RExactText([string]$Path, [string]$Before, [string]$After, [string]$Label) {
    $source = [IO.File]::ReadAllText($Path).Replace("`r`n", "`n")
    $Before = $Before.Replace("`r`n", "`n")
    $After = $After.Replace("`r`n", "`n")
    $first = $source.IndexOf($Before, [StringComparison]::Ordinal)
    if ($first -lt 0 -or $source.IndexOf($Before, $first + $Before.Length, [StringComparison]::Ordinal) -ge 0) {
        throw "U1R test instrumentation anchor '$Label' was missing or non-unique in $Path"
    }
    $updated = $source.Substring(0, $first) + $After + $source.Substring($first + $Before.Length)
    [IO.File]::WriteAllText($Path, $updated, [Text.UTF8Encoding]::new($false))
}

function Append-U1RDetailAfterAll([string]$Path, [string]$Anchor, [string]$Detail, [string]$Label) {
    $source = [IO.File]::ReadAllText($Path).Replace([string][char]13 + [char]10, [string][char]10)
    $count = [regex]::Matches($source, [regex]::Escape($Anchor)).Count
    if ($count -lt 1) { throw "U1R test instrumentation anchor '$Label' was missing in $Path" }
    $newline = [string][char]10
    $detailLine = "  DetailPrint " + [char]34 + $Detail + [char]34
    $updated = $source.Replace($Anchor, $Anchor + $newline + $detailLine)
    [IO.File]::WriteAllText($Path, $updated, [Text.UTF8Encoding]::new($false))
}

function Convert-U1RDetailPrintsToFileTrace([string]$Path, [string]$TraceFileName) {
    $source = [IO.File]::ReadAllText($Path).Replace([string][char]13 + [char]10, [string][char]10)
    $pattern = '(?m)^(?<indent>[ \t]*)DetailPrint (?<message>"\[U1R(?:-OLD-UN)?\][^"\r\n]*")[ \t]*\r?$'
    $traceMatches = [regex]::Matches($source, $pattern)
    if ($traceMatches.Count -eq 0) { throw "U1R test diagnostic had no [U1R] DetailPrint lines to instrument in $Path" }
    $builder = [Text.StringBuilder]::new()
    $cursor = 0
    $sequence = 0
    foreach ($traceMatch in $traceMatches) {
        [void]$builder.Append($source.Substring($cursor, $traceMatch.Index - $cursor))
        $id = "u1r_file_trace_$sequence"
        $sequence++
        $indent = $traceMatch.Groups["indent"].Value
        $message = $traceMatch.Groups["message"].Value
        $writeMessage = $message.Substring(0, $message.Length - 1) + '$\r$\n"'
        $tracePath = '$TEMP\' + $TraceFileName
        $lines = @(
            ($indent + 'Push $R8'),
            ($indent + 'Push $9'),
            ($indent + "IfErrors ${id}_had_error ${id}_no_error"),
            ($indent + "${id}_had_error:"),
            ($indent + 'StrCpy $R8 1'),
            ($indent + "Goto ${id}_saved"),
            ($indent + "${id}_no_error:"),
            ($indent + 'StrCpy $R8 0'),
            ($indent + "${id}_saved:"),
            ($indent + ('FileOpen $9 "' + $tracePath + '" a')),
            ($indent + ('FileWrite $9 ' + $writeMessage)),
            ($indent + 'FileClose $9'),
            ($indent + 'Pop $9'),
            ($indent + 'ClearErrors'),
            ($indent + ('StrCmp $R8 1 ' + $id + '_restore_error')),
            ($indent + 'Pop $R8'),
            ($indent + "Goto ${id}_done"),
            ($indent + "${id}_restore_error:"),
            ($indent + 'SetErrors'),
            ($indent + 'Pop $R8'),
            ($indent + "${id}_done:"),
            ($indent + ('DetailPrint ' + $message))
        )
        [void]$builder.Append($lines -join "`n")
        $cursor = $traceMatch.Index + $traceMatch.Length
    }
    [void]$builder.Append($source.Substring($cursor))
    $updated = $builder.ToString()
    [IO.File]::WriteAllText($Path, $updated, [Text.UTF8Encoding]::new($false))
}

function Add-U1RTestInstrumentation([string]$Label, [string]$Path) {
    $mainPath = Join-Path $Path "src-tauri/src/main.rs"
    if ($Label -eq "old") {
        $gateCallBefore = @'
            let global_index_coordinator = GlobalIndexCoordinator::new(db.clone());
'@
        $gateCallAfter = @'
            #[cfg(all(target_os = "windows", feature = "native-qa"))]
            u1r_startup_gate();
            let global_index_coordinator = GlobalIndexCoordinator::new(db.clone());
'@
        Replace-U1RExactText $mainPath $gateCallBefore $gateCallAfter "old client pre-coordinator startup gate"
        $rebuildTriggerBefore = @'
            if let Err(error) = global_index_coordinator.start() {
                eprintln!("Global index startup failed (non-fatal): {error}");
            }
'@
        $rebuildTriggerAfter = @'
            if let Err(error) = global_index_coordinator.start() {
                eprintln!("Global index startup failed (non-fatal): {error}");
            }
            #[cfg(all(target_os = "windows", feature = "native-qa"))]
            if let Ok(source_id) = std::env::var("ZC_U1R_FORCE_REBUILD_SOURCE_ID") {
                let coordinator = global_index_coordinator.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    eprintln!("u1r_forced_rebuild_begin");
                    let result = coordinator.rebuild(Some(source_id));
                    eprintln!("u1r_forced_rebuild_end result={result:?}");
                });
            }
'@
        Replace-U1RExactText $mainPath $rebuildTriggerBefore $rebuildTriggerAfter "old client forced post-gate Rebuild request"
        $checkpointEnd = @'
    #[cfg(not(feature = "native-qa"))]
    let _ = name;
}
'@
        $checkpointWithGate = @'
    #[cfg(not(feature = "native-qa"))]
    let _ = name;
}

#[cfg(all(target_os = "windows", feature = "native-qa"))]
fn u1r_startup_gate() {
    use std::{fs, path::PathBuf, thread, time::Duration};
    let Ok(root) = std::env::var("ZC_U1R_STARTUP_GATE") else { return; };
    let root = PathBuf::from(root);
    if fs::create_dir_all(&root).is_err() { return; }
    let _ = fs::write(root.join("generation"), "old-master-3275ac7-test-instrumented");
    let _ = fs::write(root.join("ready"), std::process::id().to_string());
    eprintln!("u1r_process_generation=old-master-3275ac7-test-instrumented pid={}", std::process::id());
    for _ in 0..720 {
        if root.join("continue").exists() {
            eprintln!("u1r_startup_gate_released pid={}", std::process::id());
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
    eprintln!("u1r_startup_gate_timeout pid={}", std::process::id());
}
'@
        Replace-U1RExactText $mainPath $checkpointEnd $checkpointWithGate "old client startup gate helper"
        $providerPath = Join-Path $Path "src-tauri/src/global_index/windows/mod.rs"
        $directBefore = @'
        let result = mft::enumerate_volume(source, sink, cancel);
        finish_initial_mft_result(&source.volume.id, sink, result)
'@
        $directAfter = @'
        super::qa_trace::record("u1r_direct_mft_started");
        let result = mft::enumerate_volume(source, sink, cancel);
        let result = finish_initial_mft_result(&source.volume.id, sink, result);
        super::qa_trace::record("u1r_direct_mft_returned");
        result
'@
        Replace-U1RExactText $providerPath $directBefore $directAfter "old Direct MFT trace"
        $fallbackBefore = @'
            Err(error) => {
                self.service_failed();
'@
        $fallbackAfter = @'
            Err(error) => {
                super::qa_trace::record("u1r_service_transport_error");
                self.service_failed();
'@
        Replace-U1RExactText $providerPath $fallbackBefore $fallbackAfter "old transport failure trace"
        $directCallBefore = @'
                direct(&self.direct, sink)
'@
        $directCallAfter = @'
                super::qa_trace::record("u1r_direct_fallback_started");
                direct(&self.direct, sink)
'@
        Replace-U1RExactText $providerPath $directCallBefore $directCallAfter "old Direct fallback trace"
    } elseif ($Label -eq "d1a") {
        $servicePath = Join-Path $Path "src-tauri/src/global_index/windows/service.rs"
        $identityBefore = @'
        if normalize_windows_path(&client_path) != normalize_windows_path(&current_path) {
            return Err("index_service_client_executable_mismatch".to_string());
        }
        Ok(())
'@
        $identityAfter = @'
        if normalize_windows_path(&client_path) != normalize_windows_path(&current_path) {
            return Err("index_service_client_executable_mismatch".to_string());
        }
        #[cfg(feature = "native-qa")]
        crate::global_index::qa_trace::record(&format!("u1r_pipe_identity_accepted_client_pid={process_id}"));
        Ok(())
'@
        Replace-U1RExactText $servicePath $identityBefore $identityAfter "D1-A named-pipe generation identity trace"
        $requestBefore = @'
            validate_request(&request)?;
            let request_id = request.request_id.clone();
'@
        $requestAfter = @'
            validate_request(&request)?;
            #[cfg(feature = "native-qa")]
            crate::global_index::qa_trace::record(&format!("u1r_service_request_accepted request_id={} command={:?}", request.request_id, request.command));
            let request_id = request.request_id.clone();
'@
        Replace-U1RExactText $servicePath $requestBefore $requestAfter "D1-A accepted request trace"
        $hostPath = Join-Path $Path "src-tauri/src/global_index/windows/service_host.rs"
        $claimBefore = @'
        if let Err(response) = self.try_begin_index_request(&request) {
            return connection.send_response(response);
        }
'@
        $claimAfter = @'
        if let Err(response) = self.try_begin_index_request(&request) {
            return connection.send_response(response);
        }
        #[cfg(feature = "native-qa")]
        crate::global_index::qa_trace::record(&format!("u1r_service_operation_claimed request_id={}", request.request_id));
'@
        Replace-U1RExactText $hostPath $claimBefore $claimAfter "D1-A operation ownership trace"
        $sourceBefore = @'
            let source = self
                .validated_source(source_id, request.source.as_ref())
                .map_err(|message| ("index_failed", message))?;
            self.cancel.store(false, Ordering::Release);
'@
        $sourceAfter = @'
            let source = self
                .validated_source(source_id, request.source.as_ref())
                .map_err(|message| ("index_failed", message))?;
            #[cfg(feature = "native-qa")]
            crate::global_index::qa_trace::record(&format!("u1r_service_source_validated request_id={} source_id={}", request.request_id, source_id));
            self.cancel.store(false, Ordering::Release);
'@
        Replace-U1RExactText $hostPath $sourceBefore $sourceAfter "D1-A source validation trace"
    } else {
        throw "unknown U1R candidate label: $Label"
    }
}

function Add-U1RNsisDiagnostics([string]$Label, [string]$Path) {
    if ($Label -eq "d1a") {
        $buildPath = Join-Path $Path "scripts/buildWindowsPackage.mjs"
        $importBefore = @'
  cleanWindowsNsisTemplate,
  prepareWindowsNsisTemplate,
'@
        $importAfter = @'
  cleanWindowsNsisTemplate,
  generatedTemplatePath,
  prepareWindowsNsisTemplate,
'@
        Replace-U1RExactText $buildPath $importBefore $importAfter "NSIS diagnostic template import"
        $prepareBefore = @'
    prepareWindowsNsisTemplate();
    generatedNsisTemplate = true;
'@
        $prepareAfter = @'
    prepareWindowsNsisTemplate();
    if (process.env.ZC_U1R_NSIS_DIAGNOSTICS === "1") {
      let diagnostic = fs.readFileSync(generatedTemplatePath, "utf8");
      const inject = (before, after, label) => {
        if (diagnostic.indexOf(before) < 0 || diagnostic.indexOf(before, diagnostic.indexOf(before) + before.length) >= 0) {
          throw new Error(`U1R NSIS diagnostic anchor missing or duplicate: ${label}`);
        }
        diagnostic = diagnostic.replace(before, after);
      };
      inject("Function .onInit\n", "Function .onInit\n  DetailPrint \"[U1R] installer_on_init\"\n", "onInit logger");
      inject("Function PageReinstall\n", "Function PageReinstall\n  DetailPrint \"[U1R] PageReinstall passive=$PassiveMode update=$UpdateMode instdir=$INSTDIR\"\n", "maintenance page entry");
      inject("nsis_tauri_utils::SemverCompare \"${VERSION}\" $R0\n  Pop $R0\n", "nsis_tauri_utils::SemverCompare \"${VERSION}\" $R0\n  Pop $R0\n  DetailPrint \"[U1R] PageReinstall semver_compare=$R0\"\n", "maintenance page version result");
      inject("Function PageLeaveReinstall\n", "Function PageLeaveReinstall\n  DetailPrint \"[U1R] PageLeaveReinstall compare=$R0 update=$UpdateMode passive=$PassiveMode wix=$WixMode\"\n", "maintenance page leave");
      inject("  reinst_uninstall:\n    HideWindow\n", "  reinst_uninstall:\n    DetailPrint \"[U1R] maintenance_uninstall_begin compare=$R0 selection=$R1 update=$UpdateMode wix=$WixMode\"\n    HideWindow\n", "maintenance uninstall branch");
      inject("    BringToFront\n\n    ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed, set fake exit code\n", "    BringToFront\n    DetailPrint \"[U1R] maintenance_uninstaller_return exit=$0\"\n\n    ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed, set fake exit code\n", "maintenance uninstaller return");
      inject('      StrCpy $R1 "$R1 _?=$4" ; append uninstall directory\n', '      DetailPrint "[U1R] old_uninstaller_trace=$TEMP\\issue-380-u1r-old-uninstaller.log"\n      StrCpy $R1 "$R1 _?=$4" ; append uninstall directory\n', "old uninstaller diagnostic trace");
      inject("  Call ZCPrepareInstallLifecycleFinal", "  DetailPrint \"[U1R] lifecycle_prepare_final_begin\"\n  Call ZCPrepareInstallLifecycleFinal\n  DetailPrint \"[U1R] lifecycle_prepare_final_returned\"", "final lifecycle owner");
      let traceSequence = 0;
      diagnostic = diagnostic.replace(/^([ \t]*)DetailPrint ("\[U1R\][^"\r\n]*")[ \t]*\r?$/gm, (_line, indent, message) => {
        const id = `u1r_d1a_trace_${traceSequence++}`;
        const writeMessage = `${message.slice(0, -1)}$\\r$\\n\"`;
        return [
          `${indent}Push $R8`,
          `${indent}Push $9`,
          `${indent}IfErrors ${id}_had_error ${id}_no_error`,
          `${indent}${id}_had_error:`,
          `${indent}StrCpy $R8 1`,
          `${indent}Goto ${id}_saved`,
          `${indent}${id}_no_error:`,
          `${indent}StrCpy $R8 0`,
          `${indent}${id}_saved:`,
          `${indent}FileOpen $9 "$TEMP\\issue-380-u1r-d1a-installer-trace.log" a`,
          `${indent}FileWrite $9 ${writeMessage}`,
          `${indent}FileClose $9`,
          `${indent}Pop $9`,
          `${indent}ClearErrors`,
          `${indent}StrCmp $R8 1 ${id}_restore_error`,
          `${indent}Pop $R8`,
          `${indent}Goto ${id}_done`,
          `${indent}${id}_restore_error:`,
          `${indent}SetErrors`,
          `${indent}Pop $R8`,
          `${indent}${id}_done:`,
          `${indent}DetailPrint ${message}`,
        ].join("\n");
      });
      if (!diagnostic.includes("u1r_d1a_trace_")) throw new Error("U1R NSIS trace markers were not converted to file writes");
      fs.writeFileSync(generatedTemplatePath, diagnostic, "utf8");
    }
    generatedNsisTemplate = true;
'@
        Replace-U1RExactText $buildPath $prepareBefore $prepareAfter "NSIS diagnostics injection hook"
    } elseif ($Label -ne "old") {
        throw "unknown U1R NSIS diagnostic candidate: $Label"
    }

    $hooksPath = Join-Path $Path "src-tauri/windows/installer-hooks.nsh"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${PATH}" "${NAME}" "${EXPECTED}" ${ZC_REG_STRING_SZ_ONLY}' '[U1R] product_string name=${NAME} state=$ZC_REG_VALUE_STATE expected=${EXPECTED}' "ARP string comparison trace"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_DWORD_STATE ${ZC_REG_ROOT_HKLM} "${PATH}" "${NAME}" ${EXPECTED}' '[U1R] product_dword name=${NAME} state=$ZC_REG_VALUE_STATE expected=${EXPECTED}' "ARP DWORD comparison trace"
    Append-U1RDetailAfterAll $hooksPath '  StrCpy $ZC_EXPECTED_ESTIMATED_SIZE $0' '[U1R] estimated_size_expected=$ZC_EXPECTED_ESTIMATED_SIZE' "EstimatedSize calculation trace"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_KEY_STATE ${ZC_REG_ROOT_HKLM} "$ZC_UNINSTALLER_REGISTRY_KEY"' '[U1R] registry_key_state key=uninstaller state=$ZC_REG_KEY_STATE' "uninstaller registry key state trace"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_KEY_STATE ${ZC_REG_ROOT_HKLM} "$ZC_MANUFACTURER_PRODUCT_KEY"' '[U1R] registry_key_state key=manufacturer_product state=$ZC_REG_KEY_STATE' "manufacturer registry key state trace"
    $uninstallerFileBefore = @'
  IfFileExists "$INSTDIR\uninstall.exe" 0 detect_product_uninstaller_absent
  StrCpy $ZC_PREEXISTING_UNINSTALLER_PRESENT 1
detect_product_uninstaller_absent:
'@
    $uninstallerFileAfter = @'
  IfFileExists "$INSTDIR\uninstall.exe" 0 detect_product_uninstaller_absent
  StrCpy $ZC_PREEXISTING_UNINSTALLER_PRESENT 1
detect_product_uninstaller_absent:
  DetailPrint "[U1R] preexisting_uninstaller_file_present=$ZC_PREEXISTING_UNINSTALLER_PRESENT"
'@
    Replace-U1RExactText $hooksPath $uninstallerFileBefore $uninstallerFileAfter "uninstaller file existence trace"
    $validateProductBefore = @'
Function ValidateZenCanvasPreexistingProduct
  Call DetectZenCanvasPreexistingProduct
'@
    $validateProductAfter = @'
Function ValidateZenCanvasPreexistingProduct
  DetailPrint "[U1R] product_validation_begin"
  Call DetectZenCanvasPreexistingProduct
  DetailPrint "[U1R] product_validation_result product=$ZC_PREEXISTING_PRODUCT valid=$ZC_PREEXISTING_PRODUCT_VALID uninstaller=$ZC_PREEXISTING_UNINSTALLER_PRESENT"
'@
    Replace-U1RExactText $hooksPath $validateProductBefore $validateProductAfter "product validation result"
    $validatePreviewBefore = @'
Function ValidateZenCanvasPreviewCore
  !insertmacro ZC_VALIDATE_PREVIEW_CORE
'@
    $validatePreviewAfter = @'
Function ValidateZenCanvasPreviewCore
  DetailPrint "[U1R] preview_validation_begin"
  !insertmacro ZC_VALIDATE_PREVIEW_CORE
  DetailPrint "[U1R] preview_validation_result core=$ZC_PREVIEW_CORE_PRESENT clsid=$ZC_PREVIEW_CLSID_PRESENT appid=$ZC_PREVIEW_APPID_PRESENT threading=$ZC_PREVIEW_THREADING_PRESENT handler=$ZC_PREVIEW_HANDLER_PRESENT inproc=$ZC_PREVIEW_INPROC_PATH_PRESENT"
'@
    Replace-U1RExactText $hooksPath $validatePreviewBefore $validatePreviewAfter "Preview validation result"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${ZC_PREVIEW_CLSID_KEY}" "" "${ZC_PREVIEW_FRIENDLY_NAME}" ${ZC_REG_STRING_SZ_ONLY}' 'preview_value=CLSID state=$ZC_REG_VALUE_STATE' "Preview CLSID comparison trace"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${ZC_PREVIEW_CLSID_KEY}" "AppID" "${ZC_PREVIEW_PREVHOST_APP_ID}" ${ZC_REG_STRING_SZ_ONLY}' 'preview_value=AppID state=$ZC_REG_VALUE_STATE' "Preview AppID comparison trace"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${ZC_PREVIEW_INPROC_KEY}" "ThreadingModel" "${ZC_PREVIEW_THREADING_MODEL}" ${ZC_REG_STRING_SZ_ONLY}' 'preview_value=ThreadingModel state=$ZC_REG_VALUE_STATE' "Preview ThreadingModel comparison trace"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${ZC_PREVIEW_HANDLERS_KEY}" "${ZC_PREVIEW_PRODUCTION_CLSID}" "${ZC_PREVIEW_FRIENDLY_NAME}" ${ZC_REG_STRING_SZ_ONLY}' 'preview_value=PreviewHandlers state=$ZC_REG_VALUE_STATE' "PreviewHandlers comparison trace"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${ZC_PREVIEW_INPROC_KEY}" "" "${ZC_PREVIEW_INSTALLED_DLL}" ${ZC_REG_STRING_SZ_ONLY}' 'preview_value=InprocServer32 state=$ZC_REG_VALUE_STATE' "Preview InprocServer32 comparison trace"
    $imagePathBefore = @'
  !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${ZC_INDEX_SERVICE_KEY}" "ImagePath" "$ZC_INDEX_SERVICE_EXPECTED_IMAGE_PATH" ${ZC_REG_STRING_EXPAND_SZ_ONLY}
  ${If} $ZC_REG_VALUE_STATE == ${ZC_REG_VALUE_EXACT}
'@
    $imagePathAfter = @'
  !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${ZC_INDEX_SERVICE_KEY}" "ImagePath" "$ZC_INDEX_SERVICE_EXPECTED_IMAGE_PATH" ${ZC_REG_STRING_EXPAND_SZ_ONLY}
  DetailPrint "[U1R] service_image_path_state=$ZC_REG_VALUE_STATE expected=$ZC_INDEX_SERVICE_EXPECTED_IMAGE_PATH"
  ${If} $ZC_REG_VALUE_STATE == ${ZC_REG_VALUE_EXACT}
'@
    Replace-U1RExactText $hooksPath $imagePathBefore $imagePathAfter "service ImagePath typed comparison trace"
    Append-U1RDetailAfterAll $hooksPath '  !insertmacro ZC_REG_QUERY_KEY_STATE ${ZC_REG_ROOT_HKLM} "${ZC_INDEX_SERVICE_KEY}"' 'service_key_state=$ZC_REG_KEY_STATE' "service registry key state trace"

    if ($Label -eq "old") {
        $preUninstallMacroBefore = @'
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  Call un.ValidateZenCanvasPreviewCore
  Call un.ValidateZenCanvasIndexServiceOwnership
  Call un.CaptureZenCanvasOriginalServiceState
  Call un.CheckZenCanvasPreDeleteProductEvidence
  ${If} $ZC_UNINSTALL_PREDELETE_COHERENT != 1
'@
        $preUninstallMacroAfter = @'
  DetailPrint "[U1R-OLD-UN] desktop_gate_begin"
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  DetailPrint "[U1R-OLD-UN] desktop_gate_returned"
  DetailPrint "[U1R-OLD-UN] preview_validation_begin"
  Call un.ValidateZenCanvasPreviewCore
  DetailPrint "[U1R-OLD-UN] preview_validation_returned"
  DetailPrint "[U1R-OLD-UN] service_ownership_validation_begin"
  Call un.ValidateZenCanvasIndexServiceOwnership
  DetailPrint "[U1R-OLD-UN] service_ownership_validation_returned"
  Call un.CaptureZenCanvasOriginalServiceState
  DetailPrint "[U1R-OLD-UN] predelete_product_evidence_begin"
  Call un.CheckZenCanvasPreDeleteProductEvidence
  DetailPrint "[U1R-OLD-UN] predelete_product_evidence_result coherent=$ZC_UNINSTALL_PREDELETE_COHERENT"
  ${If} $ZC_UNINSTALL_PREDELETE_COHERENT != 1
'@
        Replace-U1RExactText $hooksPath $preUninstallMacroBefore $preUninstallMacroAfter "old uninstaller pre-uninstall ownership trace"
        $previewAndServiceBefore = @'
  Call un.QuiesceZenCanvasPreviewBeforeUninstall
  Call un.StopZenCanvasIndexService
'@
        $previewAndServiceAfter = @'
  DetailPrint "[U1R-OLD-UN] preview_quiesce_begin"
  Call un.QuiesceZenCanvasPreviewBeforeUninstall
  DetailPrint "[U1R-OLD-UN] preview_quiesce_returned"
  DetailPrint "[U1R-OLD-UN] service_stop_begin"
  Call un.StopZenCanvasIndexService
  DetailPrint "[U1R-OLD-UN] service_stop_returned"
'@
        Replace-U1RExactText $hooksPath $previewAndServiceBefore $previewAndServiceAfter "old uninstaller reversible lifecycle trace"
    }
    $traceFileName = if ($Label -eq "old") { "issue-380-u1r-old-uninstaller.log" } else { "issue-380-u1r-d1a-installer-trace.log" }
    Convert-U1RDetailPrintsToFileTrace $hooksPath $traceFileName
}

function Get-U1RRegistryKeySnapshot([string]$SubKey) {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64)
    try {
        $key = $base.OpenSubKey($SubKey, $false)
        if (-not $key) { return [ordered]@{ path = $SubKey; present = $false; values = [ordered]@{} } }
        try {
            $values = [ordered]@{}
            foreach ($name in $key.GetValueNames() | Sort-Object) {
                $value = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                $values[$name] = [ordered]@{ kind = $key.GetValueKind($name).ToString(); value = $value }
            }
            return [ordered]@{ path = $SubKey; present = $true; values = $values }
        } finally { $key.Dispose() }
    } finally { $base.Dispose() }
}

function Get-U1RInstallIdentitySnapshot([string]$ExePath) {
    $exe = if (Test-Path -LiteralPath $ExePath) {
        $item = Get-Item -LiteralPath $ExePath
        [ordered]@{ path = $item.FullName; exists = $true; sha256 = Get-Hash $item.FullName; bytes = $item.Length; fileVersion = $item.VersionInfo.FileVersion; productVersion = $item.VersionInfo.ProductVersion }
    } else { [ordered]@{ path = $ExePath; exists = $false } }
    $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
    return [ordered]@{
        atUtc = [DateTime]::UtcNow.ToString("o")
        exe = $exe
        arp = Get-U1RRegistryKeySnapshot "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Zen Canvas"
        manufacturerProduct = Get-U1RRegistryKeySnapshot "SOFTWARE\startlan\Zen Canvas"
        previewClsid = Get-U1RRegistryKeySnapshot "SOFTWARE\Classes\CLSID\{3D1A446C-162E-4313-A026-8ADC792C4862}"
        previewInproc = Get-U1RRegistryKeySnapshot "SOFTWARE\Classes\CLSID\{3D1A446C-162E-4313-A026-8ADC792C4862}\InprocServer32"
        previewHandlers = Get-U1RRegistryKeySnapshot "SOFTWARE\Microsoft\Windows\CurrentVersion\PreviewHandlers"
        serviceRegistry = Get-U1RRegistryKeySnapshot "SYSTEM\CurrentControlSet\Services\ZenCanvasGlobalIndex"
        service = if ($service) { Get-ServiceSnapshot } else { $null }
        desktopProcesses = @(Get-AppProcessSnapshots $ExePath)
        elevated = ([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    }
}

function Invoke-U1RProfile([string]$Mode, [string]$ProfileRoot, [string]$SourceId, [string]$Marker) {
    $python = Get-Command "python.exe" -ErrorAction Stop | Select-Object -First 1
    $database = Join-Path $ProfileRoot "zen-canvas.sqlite3"
    $helper = Join-Path $repoRoot "scripts/issue380U1RProfile.py"
    $result = Invoke-ProcessCaptured $python.Source @($helper, $Mode, $database, $SourceId, $Marker) $repoRoot @{} 45
    if ($result.ExitCode -ne 0) { throw "U1R isolated SQLite $Mode failed: $($result.Stderr) $($result.Stdout)" }
    return $result.Stdout.Trim() | ConvertFrom-Json
}

function Copy-U1RProfile([string]$SourceRoot, [string]$DestinationRoot) {
    New-Item -ItemType Directory -Path $DestinationRoot -Force | Out-Null
    $sourceDb = Join-Path $SourceRoot "zen-canvas.sqlite3"
    if (-not (Test-Path -LiteralPath $sourceDb)) { throw "cannot clone missing isolated U1R database $sourceDb" }
    Copy-Item -LiteralPath $sourceDb -Destination (Join-Path $DestinationRoot "zen-canvas.sqlite3") -Force
}

function Copy-U1RRuntimeFile([string]$SourcePath, [string]$ArtifactName) {
    if (-not (Test-Path -LiteralPath $SourcePath -PathType Leaf)) { return $null }
    $destination = Join-Path $EvidenceRoot $ArtifactName
    Copy-Item -LiteralPath $SourcePath -Destination $destination -Force
    return $destination
}

function Copy-U1RProfileEvidence([string]$ProfileRoot, [string]$Name) {
    $database = Join-Path $ProfileRoot "zen-canvas.sqlite3"
    if (-not (Test-Path -LiteralPath $database)) { return @() }
    $destination = Join-Path $EvidenceRoot "$Name-profile"
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    $copied = [Collections.Generic.List[object]]::new()
    foreach ($suffix in @("zen-canvas.sqlite3", "zen-canvas.sqlite3-wal", "zen-canvas.sqlite3-shm")) {
        $sourcePath = Join-Path $ProfileRoot $suffix
        if (Test-Path -LiteralPath $sourcePath) {
            $target = Join-Path $destination $suffix
            Copy-Item -LiteralPath $sourcePath -Destination $target -Force
            $item = Get-Item -LiteralPath $target
            $copied.Add([ordered]@{ name = $suffix; sha256 = Get-Hash $target; bytes = $item.Length })
        }
    }
    return @($copied)
}

function Compare-U1RStateSnapshots($Before, $After) {
    $sourceFields = @("enabled", "provider", "index_status", "last_error", "journal_id", "journal_cursor", "last_full_index_at", "last_incremental_sync_at", "entry_count", "updated_at")
    $sourceChecks = [ordered]@{}
    foreach ($field in $sourceFields) {
        $beforeValue = $Before.source.PSObject.Properties[$field].Value
        $afterValue = $After.source.PSObject.Properties[$field].Value
        $sourceChecks[$field] = $beforeValue -ceq $afterValue
    }
    $associationFields = @("managedScopes", "managedEntries", "aiAnalysisState", "aiJobs", "aiJobItems", "counts")
    $associationChecks = [ordered]@{}
    foreach ($field in $associationFields) {
        $beforeJson = ConvertTo-Json -InputObject $Before.$field -Depth 20 -Compress
        $afterJson = ConvertTo-Json -InputObject $After.$field -Depth 20 -Compress
        $associationChecks[$field] = $beforeJson -ceq $afterJson
    }
    $entryBefore = ConvertTo-Json -InputObject $Before.entry -Depth 20 -Compress
    $entryAfter = ConvertTo-Json -InputObject $After.entry -Depth 20 -Compress
    $checks = [System.Collections.Generic.List[bool]]::new()
    foreach ($field in $sourceFields) { $checks.Add([bool]$sourceChecks[$field]) }
    foreach ($field in $associationFields) { $checks.Add([bool]$associationChecks[$field]) }
    $checks.Add([bool]($entryBefore -ceq $entryAfter))
    $checks.Add([bool]([long]$Before.activeEntries -eq [long]$After.activeEntries))
    $checks.Add([bool]([long]$Before.staleEntries -eq [long]$After.staleEntries))
    $checks.Add([bool]([long]$Before.markerFtsHits -eq [long]$After.markerFtsHits))
    return [ordered]@{
        preserved = -not $checks.Contains($false)
        sourceFields = $sourceChecks
        associations = $associationChecks
        markerEntry = ($entryBefore -ceq $entryAfter)
        activeEntries = ([long]$Before.activeEntries -eq [long]$After.activeEntries)
        staleEntries = ([long]$Before.staleEntries -eq [long]$After.staleEntries)
        markerFtsHits = ([long]$Before.markerFtsHits -eq [long]$After.markerFtsHits)
    }
}

function Wait-U1RStartupGate([string]$GateRoot, [int]$TimeoutSeconds = 90) {
    $ready = Join-Path $GateRoot "ready"
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-Path -LiteralPath $ready) { return [int](Get-Content -LiteralPath $ready -Raw).Trim() }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "old QA client did not reach its pre-coordinator gate within $TimeoutSeconds seconds"
}

function Set-U1RServiceTraceEnvironment([string]$TracePath) {
    $serviceKey = [Microsoft.Win32.Registry]::LocalMachine.CreateSubKey("SYSTEM\CurrentControlSet\Services\ZenCanvasGlobalIndex")
    try { $serviceKey.SetValue("Environment", [string[]]@("ZC_GLOBAL_INDEX_QA_TRACE=$TracePath"), [Microsoft.Win32.RegistryValueKind]::MultiString) }
    finally { $serviceKey.Dispose() }
}

function Start-U1RScmService([string]$ExePath, [string]$TracePath, [string]$LogPrefix) {
    $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
    if ($service) { throw "refusing to create U1R SCM Service over preexisting service: $($service.PathName)" }
    $create = Invoke-ProcessCaptured "sc.exe" @("create", "ZenCanvasGlobalIndex", "binPath=", ('"' + $ExePath + '" --index-service'), "start=", "demand", "obj=", "LocalSystem", "DisplayName=", "Zen Canvas Global Index") $taskRoot @{} 30
    $create.Stdout | Set-Content -LiteralPath (Join-Path $EvidenceRoot "$LogPrefix-sc-create.log") -Encoding utf8NoBOM
    if ($create.ExitCode -ne 0) { throw "U1R task SCM create failed (exit=$($create.ExitCode)): $($create.Stdout) $($create.Stderr)" }
    Set-U1RServiceTraceEnvironment $TracePath
    $start = Invoke-ProcessCaptured "sc.exe" @("start", "ZenCanvasGlobalIndex") $taskRoot @{} 45
    $start.Stdout | Set-Content -LiteralPath (Join-Path $EvidenceRoot "$LogPrefix-sc-start.log") -Encoding utf8NoBOM
    if ($start.ExitCode -ne 0) { throw "U1R task SCM start failed (exit=$($start.ExitCode)): $($start.Stdout) $($start.Stderr)" }
    return Wait-ServiceState "Running" 60
}

function Start-U1RExistingService([string]$ExpectedImagePath, [string]$TracePath, [string]$LogPrefix) {
    $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
    if (-not $service) { throw "expected task-installed Service registration is absent" }
    if ($service.PathName.Trim() -ine $ExpectedImagePath) { throw "refusing to restart a Service with an unexpected ImagePath: $($service.PathName)" }
    Set-U1RServiceTraceEnvironment $TracePath
    $start = Invoke-ProcessCaptured "sc.exe" @("start", "ZenCanvasGlobalIndex") $taskRoot @{} 45
    $start.Stdout | Set-Content -LiteralPath (Join-Path $EvidenceRoot "$LogPrefix-sc-start.log") -Encoding utf8NoBOM
    if ($start.ExitCode -ne 0) { throw "U1R existing SCM start failed (exit=$($start.ExitCode)): $($start.Stdout) $($start.Stderr)" }
    return Wait-ServiceState "Running" 60
}

function Stop-U1RScmService([string]$LogPrefix) {
    $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
    if (-not $service) { return $null }
    if ($service.State -ine "Stopped") {
        $stop = Invoke-ProcessCaptured "sc.exe" @("stop", "ZenCanvasGlobalIndex") $taskRoot @{} 45
        $stop.Stdout | Set-Content -LiteralPath (Join-Path $EvidenceRoot "$LogPrefix-sc-stop.log") -Encoding utf8NoBOM
        if ($stop.ExitCode -ne 0 -and $stop.Stdout -notmatch "1062|not been started") { throw "U1R task SCM stop failed: $($stop.Stdout) $($stop.Stderr)" }
    }
    return Wait-ServiceState "Stopped" 60
}

function Remove-U1RScmService([string]$LogPrefix) {
    $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
    if (-not $service) { return }
    if ($service.State -ine "Stopped") { Stop-U1RScmService $LogPrefix | Out-Null }
    $delete = Invoke-ProcessCaptured "sc.exe" @("delete", "ZenCanvasGlobalIndex") $taskRoot @{} 30
    $delete.Stdout | Set-Content -LiteralPath (Join-Path $EvidenceRoot "$LogPrefix-sc-delete.log") -Encoding utf8NoBOM
    if ($delete.ExitCode -ne 0) { throw "U1R task SCM delete failed: $($delete.Stdout) $($delete.Stderr)" }
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        if (-not (Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue)) { return }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "U1R task SCM Service registration did not disappear after delete"
}

function Invoke-U1RInstallerDiagnostics($OldCandidate, $D1ACandidate, [string]$InstallProfile, [string]$Drive, [string]$SourceId, [string]$MarkerName) {
    $serviceName = "ZenCanvasGlobalIndex"
    $installDir = Join-Path $env:ProgramFiles "Zen Canvas"
    $exe = Join-Path $installDir "zen-canvas.exe"
    if (Get-CimInstance Win32_Service -Filter "Name='$serviceName'" -ErrorAction SilentlyContinue) { throw "U1R NSIS diagnostic found a preexisting Service" }
    if (Test-Path -LiteralPath $installDir) { throw "U1R NSIS diagnostic found a preexisting canonical install directory" }
    $global:U1OwnedInstallDirectory = $installDir
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $oldFreshLog = Join-Path $EvidenceRoot "u1r-old-fresh-install.log"
    $fresh = Install-NSIS $OldCandidate.installer $installDir $oldFreshLog
    $global:U1Evidence.u1rInstaller = [ordered]@{ oldFreshExitCode = $fresh.ExitCode; oldFreshLog = $oldFreshLog; oldFreshSnapshot = Get-U1RInstallIdentitySnapshot $exe }
    Save-Evidence
    if ($fresh.ExitCode -ne 0) { throw "U1R old fresh install failed with $($fresh.ExitCode)" }
    if ($OldCandidate.packageExeSha256 -and (Get-Hash $exe) -cne $OldCandidate.packageExeSha256) { throw "U1R old fresh install payload hash mismatch" }

    # Preserve the exact installed production EXE and its exact EstimatedSize
    # basis. Place a pre-indexed test-only Schema 37 DB under a temporary APPDATA
    # root before starting the old desktop process; the runner profile is never
    # opened or altered.
    $isolatedRoaming = Join-Path $InstallProfile "roaming"
    $isolatedLocal = Join-Path $InstallProfile "local"
    $isolatedTemp = Join-Path $InstallProfile "temp"
    $isolatedAppData = Join-Path $isolatedRoaming $OldCandidate.identifier
    foreach ($path in @($isolatedRoaming, $isolatedLocal, $isolatedTemp, $isolatedAppData)) { New-Item -ItemType Directory -Path $path -Force | Out-Null }
    Copy-Item -LiteralPath (Join-Path $InstallProfile "zen-canvas.sqlite3") -Destination (Join-Path $isolatedAppData "zen-canvas.sqlite3") -Force
    $isolatedProfileSnapshot = Invoke-U1RProfile "snapshot" $isolatedAppData $SourceId $MarkerName
    $appEnvironment = @{ APPDATA = $isolatedRoaming; LOCALAPPDATA = $isolatedLocal; TEMP = $isolatedTemp; TMP = $isolatedTemp }
    $databaseBefore = $null
    $priorEnvironment = @{}
    foreach ($name in $appEnvironment.Keys) { $priorEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process") }
    foreach ($name in $appEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, [string]$appEnvironment[$name], "Process") }
    $app = $null
    $appStdout = Join-Path $taskRoot "u1r-installer-old-client.stdout.log"
    $appStderr = Join-Path $taskRoot "u1r-installer-old-client.stderr.log"
    try {
        $app = Start-Process -FilePath $exe -ArgumentList @("--background") -WorkingDirectory $installDir -WindowStyle Hidden -RedirectStandardOutput $appStdout -RedirectStandardError $appStderr -PassThru
        Start-Sleep -Seconds 4
        $app.Refresh()
        if ($app.HasExited) { throw "old production desktop exited before the D1-A in-place installer launch" }
        $oldDesktop = @(Get-AppProcessSnapshots $exe | Where-Object { $_.processId -eq $app.Id }) | Select-Object -First 1
        if (-not $oldDesktop) { throw "old production desktop process is absent from the canonical install path" }
        $oldService = Get-ServiceSnapshot
        if (-not $oldService) { throw "old production installation did not register its Global Index Service" }
        $expectedOldServicePath = ('"' + $exe + '" --index-service')
        if ($oldService.imagePath.Trim() -ine $expectedOldServicePath) { throw "old installed Service ImagePath mismatch: $($oldService.imagePath)" }
        if ($oldService.state -ine "Running") {
            $startOld = Invoke-ProcessCaptured "sc.exe" @("start", "ZenCanvasGlobalIndex") $taskRoot @{} 45
            $startOld.Stdout | Set-Content -LiteralPath (Join-Path $EvidenceRoot "u1r-old-installed-service-start.log") -Encoding utf8NoBOM
            if ($startOld.ExitCode -ne 0) { throw "old installed Service failed to start: $($startOld.Stdout) $($startOld.Stderr)" }
            $oldService = Wait-ServiceState "Running" 60
        }
        $before = Get-U1RInstallIdentitySnapshot $exe
        $oldServiceProcess = if ($oldService.process) { [ordered]@{ pid = $oldService.processId; path = $oldService.process.path; created = $oldService.process.created; sha256 = Get-Hash $oldService.process.path } } else { $null }
        $databaseBefore = Invoke-U1RProfile "snapshot" $isolatedAppData $SourceId $MarkerName
        $log = Join-Path $EvidenceRoot "u1r-d1a-inplace-no-update.log"
        $uninstallerTracePath = Join-Path $env:TEMP "issue-380-u1r-old-uninstaller.log"
        $diagnosticTracePath = Join-Path $env:TEMP "issue-380-u1r-d1a-installer-trace.log"
        Remove-Item -LiteralPath $uninstallerTracePath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $diagnosticTracePath -Force -ErrorAction SilentlyContinue
        $transition = Wait-ForInstaller $D1ACandidate.installer $installDir $log $D1ACandidate.packageExeSha256 (Get-Hash $exe) 300 $false
        $after = Get-U1RInstallIdentitySnapshot $exe
        $databaseAfter = Invoke-U1RProfile "snapshot" $isolatedAppData $SourceId $MarkerName
        $diagnosticText = if (Test-Path -LiteralPath $diagnosticTracePath) { Get-Content -LiteralPath $diagnosticTracePath -Raw } else { "" }
        $taggedLines = @($diagnosticText -split "`r?`n" | Where-Object { $_ -match '\[U1R\]' })
        $uninstallerTraceText = if (Test-Path -LiteralPath $uninstallerTracePath) { Get-Content -LiteralPath $uninstallerTracePath -Raw } else { "" }
        $uninstallerTaggedLines = @($uninstallerTraceText -split "`r?`n" | Where-Object { $_ -match '\[U1R(?:-OLD-UN)?\]' })
        $uninstallerProcessSamples = @($transition.Samples | Where-Object { $_.uninstallerProcesses.Count -gt 0 } | ForEach-Object { $_.uninstallerProcesses })
        $lastTag = if ($taggedLines.Count -gt 0) { $taggedLines[-1] } else { $null }
        $firstMismatch = $null
        foreach ($line in $taggedLines) {
            if ($line -match 'preexisting_uninstaller_file_present=0') { $firstMismatch = $line; break }
            $stateMatch = [regex]::Match($line, '(?:product_(?:string|dword)|preview_value=.*?|registry_key_state|preexisting_uninstaller_file_present|service_image_path|service_key).*?state=([0-3])')
            if (-not $stateMatch.Success -or [int]$stateMatch.Groups[1].Value -eq 1) { continue }
            if ($line -match 'product_string name=(URLInfoAbout|URLUpdateInfo|HelpLink) state=0') { continue }
            if ($line -match 'service_key_state=0') { continue }
            $firstMismatch = $line
            break
        }
        $oldUninstallerFirstMismatch = $null
        foreach ($line in $uninstallerTaggedLines) {
            $stateMatch = [regex]::Match($line, '(?:product_(?:string|dword)|preview_value=.*?|registry_key_state|preexisting_uninstaller_file_present|service_image_path|service_key).*?state=([0-3])')
            if (-not $stateMatch.Success -or [int]$stateMatch.Groups[1].Value -eq 1) { continue }
            if ($line -match 'product_string name=(URLInfoAbout|URLUpdateInfo|HelpLink) state=0') { continue }
            if ($line -match 'service_key_state=0') { continue }
            $oldUninstallerFirstMismatch = $line
            break
        }
        $maintenanceReturn = $taggedLines | Where-Object { $_ -match 'maintenance_uninstaller_return exit=([0-9]+)' } | Select-Object -First 1
        $rootCause = "UNKNOWN: diagnostic trace did not identify the first failing NSIS condition"
        if ($uninstallerTaggedLines -match 'predelete_product_evidence_result coherent=0') {
            $rootCause = "Old uninstaller failed its pre-delete product evidence guard; first mismatching registry check: $oldUninstallerFirstMismatch"
        } elseif ($uninstallerTaggedLines -match 'preview_validation_begin' -and $uninstallerTaggedLines -notmatch 'preview_validation_returned') {
            $rootCause = "Old uninstaller aborted during ValidateZenCanvasPreviewCore; first mismatching registry check: $oldUninstallerFirstMismatch"
        } elseif ($uninstallerTaggedLines -match 'service_ownership_validation_begin' -and $uninstallerTaggedLines -notmatch 'service_ownership_validation_returned') {
            $rootCause = "Old uninstaller aborted during ValidateZenCanvasIndexServiceOwnership; first mismatching registry check: $oldUninstallerFirstMismatch"
        } elseif ($uninstallerTaggedLines -match 'service_stop_begin' -and $uninstallerTaggedLines -notmatch 'service_stop_returned|service_stop_result') {
            $rootCause = "Old uninstaller did not return from StopZenCanvasIndexService; inspect Service process samples and raw nested NSIS log"
        } elseif ($uninstallerTaggedLines -match 'desktop_gate_begin' -and $uninstallerTaggedLines -notmatch 'desktop_gate_returned|desktop_gate_result') {
            $rootCause = "Old uninstaller did not return from CheckIfAppIsRunning; inspect desktop PID samples and raw nested NSIS log"
        } elseif ($uninstallerTaggedLines -match 'preview_quiesce_begin' -and $uninstallerTaggedLines -notmatch 'preview_quiesce_returned|preview_quiesce_result') {
            $rootCause = "Old uninstaller did not return from QuiesceZenCanvasPreviewBeforeUninstall; inspect Preview Handler registry snapshot and raw nested NSIS log"
        } elseif ($taggedLines -match 'maintenance_uninstall_begin' -and -not $maintenanceReturn) {
            $rootCause = "PageLeaveReinstall entered the old uninstaller branch but ExecWait did not return before NSIS aborted or stalled; inspect uninstaller process and raw NSIS log"
        } elseif ($maintenanceReturn -and $maintenanceReturn -match 'exit=([0-9]+)') {
            $uninstallerExit = [int]$Matches[1]
            if ($uninstallerExit -ne 0) {
                $rootCause = "PageLeaveReinstall invoked the old UninstallString; the ExecWait '$0 != 0' guard fired with exit $uninstallerExit before ZCPrepareInstallLifecycleFinal"
            } elseif ($after.exe.exists -and $after.exe.sha256 -ceq $before.exe.sha256) {
                $rootCause = 'PageLeaveReinstall invoked the old UninstallString; after ExecWait returned 0 the generated main EXE still existed, so the ${FileExists} $INSTDIR\${MAINBINARYNAME}.exe guard fired before ZCPrepareInstallLifecycleFinal'
            }
        } elseif ($firstMismatch -match 'preexisting_uninstaller_file_present=0') {
            $rootCause = 'DetectZenCanvasPreexistingProduct failed its IfFileExists "$INSTDIR\uninstall.exe" presence check while existing product metadata was registered'
        } elseif ($firstMismatch) {
            $rootCause = "first non-exact pre-file validation condition: $firstMismatch"
        } elseif ($taggedLines -match 'product_validation_result product=2') {
            $rootCause = "UNKNOWN: DetectZenCanvasPreexistingProduct returned invalid product state, but no inner validation predicate was captured"
        } elseif ($taggedLines -match 'preview_validation_begin' -and $taggedLines -notmatch 'preview_validation_result') {
            $rootCause = "ValidateZenCanvasPreviewCore aborted during a typed Preview Handler comparison; use the last preview_value trace and registry snapshot"
        } elseif ($taggedLines -match 'service_image_path_state=' -and $taggedLines -notmatch 'lifecycle_prepare_final_returned') {
            $rootCause = "ValidateZenCanvasIndexServiceOwnership did not complete; use the ImagePath trace and 64-bit registry snapshot"
        }
        $rootLocated = $rootCause -notmatch '^UNKNOWN:'
        $status = if ($transition.ExitCode -eq 2 -and $rootLocated) { "EXIT_2_ROOT_CAUSE_LOCATED" } elseif ($transition.ExitCode -eq 0) { "PASS_NO_UPDATE" } else { "ROOT_CAUSE_UNRESOLVED" }
        $global:U1Evidence.u1rInstaller.noUpdateAttempt = [ordered]@{
            arguments = @("/S", "/LOG=$log")
            exitCode = $transition.ExitCode
            status = $status
            log = $log
            logExists = $transition.Log.exists
            logSha256 = $transition.Log.sha256
            diagnosticTrace = $diagnosticTracePath
            diagnosticTraceExists = Test-Path -LiteralPath $diagnosticTracePath
            diagnosticTraceSha256 = if (Test-Path -LiteralPath $diagnosticTracePath) { Get-Hash $diagnosticTracePath } else { $null }
            allDiagnosticLines = $taggedLines
            oldUninstallerLog = $uninstallerTracePath
            oldUninstallerLogExists = Test-Path -LiteralPath $uninstallerTracePath
            oldUninstallerLogSha256 = if (Test-Path -LiteralPath $uninstallerTracePath) { Get-Hash $uninstallerTracePath } else { $null }
            oldUninstallerDiagnosticLines = $uninstallerTaggedLines
            oldUninstallerFirstFailingValidationTrace = $oldUninstallerFirstMismatch
            oldUninstallerProcessSamples = $uninstallerProcessSamples
            lastDiagnosticLine = $lastTag
            firstFailingCondition = $rootCause
            firstFailingValidationTrace = $firstMismatch
            firstAbortMessage = @($diagnosticText -split "`r?`n" | Where-Object { $_ -match '(?i)(abort|aborted|failed|error|could not|inconsistent|foreign)' } | Select-Object -First 8)
            installerStdout = $transition.Stdout
            installerStderr = $transition.Stderr
            before = $before
            after = $after
            processSamples = $transition.Samples
            stdout = $transition.Stdout
            stderr = $transition.Stderr
            initialOldDesktop = $oldDesktop
            initialOldService = $oldService
            initialOldServiceBinary = $oldServiceProcess
            isolatedProfileDatabase = Join-Path $isolatedAppData "zen-canvas.sqlite3"
            isolatedProfileDatabaseSha256 = Get-Hash (Join-Path $isolatedAppData "zen-canvas.sqlite3")
            isolatedProfileSnapshotBeforeLaunch = $isolatedProfileSnapshot
            databaseBeforeInstaller = $databaseBefore
            databaseAfterNoUpdateAttempt = $databaseAfter
            databaseStatePreserved = Compare-U1RStateSnapshots $databaseBefore $databaseAfter
            serviceStoppedBeforeMutation = @($transition.Samples | Where-Object { $_.service -and $_.service.state -ieq "Stopped" -and $_.exeSha256 -ceq $before.exe.sha256 }).Count -gt 0
            oldDesktopStillRunning = @($after.desktopProcesses | Where-Object { $_.processId -eq $app.Id }).Count -gt 0
        }
        Save-Evidence
        Copy-U1RRuntimeFile $diagnosticTracePath "u1r-d1a-installer-nsis-trace.log" | Out-Null
        Copy-U1RRuntimeFile $uninstallerTracePath "u1r-old-uninstaller-nsis.log" | Out-Null
        if ($transition.ExitCode -eq 2 -and $taggedLines.Count -gt 0) {
            # Follow the same-instance diagnosis with Tauri's explicit update
            # path. This is an observation only; any earlier side effect is
            # captured above before the command is issued.
            $updateLog = Join-Path $EvidenceRoot "u1r-d1a-inplace-update-flag.log"
            Remove-Item -LiteralPath $diagnosticTracePath -Force -ErrorAction SilentlyContinue
            Remove-Item -LiteralPath $uninstallerTracePath -Force -ErrorAction SilentlyContinue
            $imageHashBeforeUpdateFlag = if (Test-Path -LiteralPath $exe) { Get-Hash $exe } else { "" }
            $update = Wait-ForInstaller $D1ACandidate.installer $installDir $updateLog $D1ACandidate.packageExeSha256 $imageHashBeforeUpdateFlag 300 $false @("/UPDATE")
            $updateSnapshot = Get-U1RInstallIdentitySnapshot $exe
            $updateText = if (Test-Path -LiteralPath $diagnosticTracePath) { Get-Content -LiteralPath $diagnosticTracePath -Raw } else { "" }
            $global:U1Evidence.u1rInstaller.updateFlagAttempt = [ordered]@{ arguments = @("/S", "/UPDATE", "/LOG=$updateLog"); exitCode = $update.ExitCode; log = $updateLog; logSha256 = $update.Log.sha256; diagnosticTrace = $diagnosticTracePath; diagnosticTraceExists = Test-Path -LiteralPath $diagnosticTracePath; diagnosticTraceSha256 = if (Test-Path -LiteralPath $diagnosticTracePath) { Get-Hash $diagnosticTracePath } else { $null }; diagnosticLines = @($updateText -split "`r?`n" | Where-Object { $_ -match '\[U1R\]' }); finalSnapshot = $updateSnapshot; samples = $update.Samples; stdout = $update.Stdout; stderr = $update.Stderr }
            Save-Evidence
            Copy-U1RRuntimeFile $diagnosticTracePath "u1r-d1a-update-installer-nsis-trace.log" | Out-Null
            Copy-U1RRuntimeFile $uninstallerTracePath "u1r-old-uninstaller-update-nsis-trace.log" | Out-Null
        }
    } finally {
        if ($app) { Stop-U1Process $app }
        if (Get-ServiceSnapshot) {
            try { Stop-U1RScmService "u1r-installer-service-shutdown" | Out-Null } catch { $global:U1Evidence.u1rInstaller.serviceShutdownFailure = $_.Exception.ToString() }
        }
        if (Test-Path -LiteralPath (Join-Path $isolatedAppData "zen-canvas.sqlite3")) {
            try {
                $global:U1Evidence.u1rInstaller.databaseAfterProcessShutdown = Invoke-U1RProfile "snapshot" $isolatedAppData $SourceId $MarkerName
                if ($databaseBefore) { $global:U1Evidence.u1rInstaller.databaseStateAfterShutdown = Compare-U1RStateSnapshots $databaseBefore $global:U1Evidence.u1rInstaller.databaseAfterProcessShutdown }
            } catch { $global:U1Evidence.u1rInstaller.databaseSnapshotAfterShutdownFailure = $_.Exception.ToString() }
        }
        foreach ($name in $appEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $priorEnvironment[$name], "Process") }
        Copy-U1RRuntimeFile $appStdout "u1r-installer-old-client.stdout.log" | Out-Null
        Copy-U1RRuntimeFile $appStderr "u1r-installer-old-client.stderr.log" | Out-Null
        Copy-U1RProfileEvidence $isolatedAppData "u1r-installer-profile" | Out-Null
    }
}

function Invoke-U1RSamePathAndDirectTests($OldCandidate, $D1ACandidate, [string]$PreparedRoot, [string]$SourceId, [string]$MarkerName, [string]$FixtureRoot) {
    $installDir = Join-Path $env:ProgramFiles "Zen Canvas"
    $exe = Join-Path $installDir "zen-canvas.exe"
    if (Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue) { throw "U1R same-path gate found an existing Service" }
    if (Test-Path -LiteralPath $installDir) { Remove-Item -LiteralPath $installDir -Recurse -Force }
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $global:U1OwnedInstallDirectory = $installDir
    Copy-Item -LiteralPath $OldCandidate.qaExe -Destination $exe -Force

    $samePathProfile = Join-Path $taskRoot "u1r-same-path-profile"
    $directProfile = Join-Path $taskRoot "u1r-direct-profile"
    Copy-U1RProfile $PreparedRoot $samePathProfile
    Copy-U1RProfile $PreparedRoot $directProfile
    $sameTrace = Join-Path $taskRoot "u1r-same-path-client.trace.log"
    $serviceTrace = Join-Path $taskRoot "u1r-same-path-service.trace.log"
    $gateRoot = Join-Path $taskRoot "u1r-old-client-gate"
    $oldAppTracePath = Join-Path $taskRoot "u1r-same-path-client.stderr.log"
    $oldHash = Get-Hash $exe
    $oldApp = $null
    $samePath = [ordered]@{ status = "RUNNING"; installPath = $exe; oldBinarySha256 = $oldHash; d1aBinarySha256 = Get-Hash $D1ACandidate.qaExe; sourceId = $SourceId; marker = $MarkerName }
    $samePath.preSnapshot = Invoke-U1RProfile "snapshot" $samePathProfile $SourceId $MarkerName
    $samePath.databasePath = Join-Path $samePathProfile "zen-canvas.sqlite3"
    $samePath.databaseSha256BeforeRequest = Get-Hash $samePath.databasePath
    try {
        $oldService = Start-U1RScmService $exe (Join-Path $taskRoot "u1r-old-same-path-service.trace.log") "u1r-old-same-path-service"
        $environment = Set-ChildProfileEnvironment $samePathProfile $sameTrace $FixtureRoot
        $environment.ZC_U1R_STARTUP_GATE = $gateRoot
        $environment.ZC_U1R_FORCE_REBUILD_SOURCE_ID = $SourceId
        $previous = @{}
        foreach ($name in $environment.Keys) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, "Process") }
        foreach ($name in $environment.Keys) { [Environment]::SetEnvironmentVariable($name, [string]$environment[$name], "Process") }
        try {
            $oldApp = Start-Process -FilePath $exe -ArgumentList @("--background") -WorkingDirectory $installDir -WindowStyle Hidden -RedirectStandardOutput (Join-Path $taskRoot "u1r-same-path-client.stdout.log") -RedirectStandardError $oldAppTracePath -PassThru
        } finally { foreach ($name in $environment.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name], "Process") } }
        $gatedPid = Wait-U1RStartupGate $gateRoot
        if ($gatedPid -ne $oldApp.Id) { throw "old generation gate PID does not match launched desktop PID" }
        $oldProcessBefore = @(Get-AppProcessSnapshots $exe | Where-Object { $_.processId -eq $oldApp.Id }) | Select-Object -First 1
        if (-not $oldProcessBefore) { throw "old client process is absent before same-path replacement" }
        $samePath.oldClientBeforeReplacement = $oldProcessBefore
        $samePath.oldClientBinarySha256AtStart = Get-Hash $exe
        $samePath.oldClientTrace = $oldAppTracePath
            $samePath.generationMarkerPath = Join-Path $gateRoot "generation"
        $samePath.oldServiceBeforeReplacement = $oldService
        $samePath.oldServiceBinarySha256 = $oldHash
        $samePath.oldServiceStoppedBeforeReplacement = Stop-U1RScmService "u1r-old-same-path-service"
        Remove-U1RScmService "u1r-old-same-path-service"
        $samePath.oldServiceRegistrationRemovedBeforeReplacement = -not [bool](Get-ServiceSnapshot)

        $replacement = Join-Path $installDir "zen-canvas.u1r-replacement.exe"
        $backup = Join-Path $installDir "zen-canvas.u1r-old-mapped-backup.exe"
        Copy-Item -LiteralPath $D1ACandidate.qaExe -Destination $replacement -Force
        $replacementCandidateHash = Get-Hash $replacement
        if ($replacementCandidateHash -cne $samePath.d1aBinarySha256) { throw "U1R replacement candidate hash did not match the frozen D1-A QA binary" }
        $replacementSucceeded = $false
        try {
            [IO.File]::Replace($replacement, $exe, $backup, $true)
            $samePath.fileReplace = [ordered]@{ status = "PASS_ATOMIC_REPLACE_WITH_OLD_PROCESS_ALIVE"; backup = $backup; backupSha256 = Get-Hash $backup; installedNewSha256 = Get-Hash $exe }
            $replacementSucceeded = $true
        } catch {
            $samePath.fileReplace = [ordered]@{ status = "UNVERIFIED_REPLACEMENT_REFUSED"; error = $_.Exception.ToString(); oldProcessStillAlive = -not $oldApp.HasExited; installedSha256 = if (Test-Path -LiteralPath $exe) { Get-Hash $exe } else { $null } }
        }
        if ($replacementSucceeded) {
            $oldApp.Refresh()
            $samePath.oldProcessAfterReplacement = @(Get-AppProcessSnapshots $exe | Where-Object { $_.processId -eq $oldApp.Id }) | Select-Object -First 1
            if ($oldApp.HasExited -or -not $samePath.oldProcessAfterReplacement -or $samePath.oldProcessAfterReplacement.processId -ne $gatedPid -or $samePath.oldProcessAfterReplacement.created -cne $oldProcessBefore.created) { throw "old process did not remain alive with its original PID and start time across image replacement" }
            if ((Get-Hash $backup) -cne $oldHash -or (Get-Hash $exe) -cne $samePath.d1aBinarySha256) { throw "same-path atomic replacement did not leave the verified old backup and D1-A image" }
            $newService = Start-U1RScmService $exe $serviceTrace "u1r-d1a-same-path-service"
            $samePath.newServiceAfterReplacement = $newService
            $samePath.newServiceProcess = (Get-CimInstance Win32_Process -Filter "ProcessId=$($newService.processId)" | Select-Object -First 1 | ForEach-Object { [ordered]@{ pid = [int]$_.ProcessId; path = $_.ExecutablePath; commandLine = $_.CommandLine; creationDate = $_.CreationDate; onDiskSha256 = Get-Hash $_.ExecutablePath } })
            $samePath.newServiceBinaryMatchesD1A = $samePath.newServiceProcess.onDiskSha256 -ceq (Get-Hash $D1ACandidate.qaExe)
            $samePath.oldClientLogBeforeRelease = Get-Content -LiteralPath $oldAppTracePath -Raw -ErrorAction SilentlyContinue
            $samePath.oldGenerationTracePresent = [bool]((Test-Path -LiteralPath $samePath.generationMarkerPath) -and ((Get-Content -LiteralPath $samePath.generationMarkerPath -Raw).Trim() -ceq "old-master-3275ac7-test-instrumented") -and $samePath.oldClientLogBeforeRelease -and $samePath.oldClientLogBeforeRelease.Contains("u1r_process_generation=old-master-3275ac7-test-instrumented"))
            Set-Content -LiteralPath (Join-Path $gateRoot "continue") -Value "continue" -Encoding ascii
            $identityEvent = "u1r_pipe_identity_accepted_client_pid=$gatedPid"
            $identityAccepted = Wait-U1TraceCount $serviceTrace $identityEvent 1 120
            $rebuildAccepted = Wait-U1TraceContains $serviceTrace "command=Rebuild" 120
            $operationClaimed = Wait-U1TraceContains $serviceTrace "u1r_service_operation_claimed" 120
            $sourceValidated = Wait-U1TraceContains $serviceTrace "u1r_service_source_validated" 120
            $routeComplete = Wait-U1TraceCount $sameTrace "windows_service_route_ok" 1 180
            $serviceTraceText = Get-Content -LiteralPath $serviceTrace -Raw -ErrorAction SilentlyContinue
            $acceptedLine = @($serviceTraceText -split '\r?\n' | Where-Object { $_ -match 'u1r_service_request_accepted.*command=Rebuild' } | Select-Object -First 1)
            $claimLine = @($serviceTraceText -split '\r?\n' | Where-Object { $_ -match 'u1r_service_operation_claimed' } | Select-Object -First 1)
            $validatedLine = @($serviceTraceText -split '\r?\n' | Where-Object { $_ -match 'u1r_service_source_validated' } | Select-Object -First 1)
            $acceptedRequestId = if ($acceptedLine.Count -gt 0 -and $acceptedLine[0] -match 'request_id=([^\s]+)') { $Matches[1] } else { $null }
            $claimedRequestId = if ($claimLine.Count -gt 0 -and $claimLine[0] -match 'request_id=([^\s]+)') { $Matches[1] } else { $null }
            $validatedRequestId = if ($validatedLine.Count -gt 0 -and $validatedLine[0] -match 'request_id=([^\s]+)') { $Matches[1] } else { $null }
            $validatedSourceId = if ($validatedLine.Count -gt 0 -and $validatedLine[0] -match 'source_id=([^\s]+)') { $Matches[1] } else { $null }
            $requestIdentityConsistent = [bool]($acceptedRequestId -and $acceptedRequestId -ceq $claimedRequestId -and $acceptedRequestId -ceq $validatedRequestId -and $validatedSourceId -ceq $SourceId)
            $samePath.identityAcceptedOldPid = $identityAccepted
            $samePath.rebuildRequestAccepted = $rebuildAccepted
            $samePath.serviceOperationClaimed = $operationClaimed
            $samePath.serviceSourceValidated = $sourceValidated
            $samePath.requestIdsMatchAcrossAcceptanceClaimAndValidation = $requestIdentityConsistent
            $samePath.acceptedRequestId = $acceptedRequestId
            $samePath.claimedRequestId = $claimedRequestId
            $samePath.validatedRequestId = $validatedRequestId
            $samePath.validatedSourceId = $validatedSourceId
            $samePath.oldClientServiceRouteCompleted = $routeComplete
            $samePath.serviceTrace = $serviceTrace
            $samePath.clientTrace = $sameTrace
            $samePath.oldClientTraceAfterRelease = Get-Content -LiteralPath $oldAppTracePath -Raw -ErrorAction SilentlyContinue
            $samePath.requestedRebuildTracePresent = [bool]($samePath.oldClientTraceAfterRelease -match 'u1r_forced_rebuild_begin')
            $samePath.requestedRebuildResultTrace = @($samePath.oldClientTraceAfterRelease -split '\r?\n' | Where-Object { $_ -match 'u1r_forced_rebuild_end' })
            $samePath.status = if ($samePath.oldGenerationTracePresent -and $identityAccepted -and $rebuildAccepted -and $operationClaimed -and $sourceValidated -and $requestIdentityConsistent -and $samePath.newServiceBinaryMatchesD1A) { "PROVED_OLD_GENERATION_REQUEST_ACCEPTED_BY_NEW_SAME_PATH_SERVICE" } else { "UNVERIFIED_SAME_PATH_SERVICE_OR_REQUEST" }
            if ($samePath.status -eq "PROVED_OLD_GENERATION_REQUEST_ACCEPTED_BY_NEW_SAME_PATH_SERVICE") {
                $samePath.securityFinding = "D1-A's exact image-path check accepted an old-master client PID created before file replacement; that old client submitted a Rebuild request which the new D1-A Service accepted. No binary-generation identity check was made."
            }
        } else {
            $samePath.status = "UNVERIFIED_OLD_PROCESS_ISOLATION"
            $samePath.securityFinding = "NTFS refused replacement while the old client remained alive. The harness did not terminate the process or force replacement."
        }
        $global:U1Evidence.u1rSamePath = $samePath
        Save-Evidence
    } catch {
        $samePath.status = "UNVERIFIED_SAME_PATH_TEST_HARNESS_FAILURE"
        $samePath.failure = $_.Exception.ToString()
        $global:U1Evidence.u1rSamePath = $samePath
        Save-Evidence
    } finally {
        Stop-U1Process $oldApp
        if ((Get-ServiceSnapshot) -and (Get-ServiceSnapshot).state -ine "Stopped") { Stop-U1RScmService "u1r-same-path-cleanup" | Out-Null }
        Remove-U1RScmService "u1r-same-path-cleanup"
        foreach ($candidate in @(Get-AppProcessSnapshots $exe)) { Stop-Process -Id $candidate.processId -Force -ErrorAction SilentlyContinue }
        if ($samePath.preSnapshot -and (Test-Path -LiteralPath $samePath.databasePath)) {
            try {
                $samePath.postSnapshot = Invoke-U1RProfile "snapshot" $samePathProfile $SourceId $MarkerName
                $samePath.databaseSha256AfterRequest = Get-Hash $samePath.databasePath
                $samePath.statePreservation = Compare-U1RStateSnapshots $samePath.preSnapshot $samePath.postSnapshot
            } catch { $samePath.postSnapshotFailure = $_.Exception.ToString() }
        }
        Copy-U1RRuntimeFile $oldAppTracePath "u1r-same-path-old-client.stderr.log" | Out-Null
        Copy-U1RRuntimeFile $serviceTrace "u1r-same-path-d1a-service.trace.log" | Out-Null
        Copy-U1RRuntimeFile $sameTrace "u1r-same-path-old-client.trace.log" | Out-Null
        $samePath.profileFiles = Copy-U1RProfileEvidence $samePathProfile "u1r-same-path"
        $global:U1Evidence.u1rSamePath = $samePath
        Save-Evidence
    }

    $directTrace = Join-Path $taskRoot "u1r-direct-client.trace.log"
    $directApp = $null
    $direct = [ordered]@{ status = "UNVERIFIED_DIRECT_FALLBACK"; databasePath = Join-Path $directProfile "zen-canvas.sqlite3" }
    try {
        Copy-Item -LiteralPath $OldCandidate.qaExe -Destination $exe -Force
        $directServiceTrace = Join-Path $taskRoot "u1r-direct-stopped-service.trace.log"
        $directService = Start-U1RScmService $exe $directServiceTrace "u1r-direct-service"
        $directServiceImageHash = Get-Hash $exe
        Stop-U1RScmService "u1r-direct-service" | Out-Null
        $directServiceStopped = Get-ServiceSnapshot
        Wait-PipeAbsent
        $directBefore = Invoke-U1RProfile "snapshot" $directProfile $SourceId $MarkerName
        $directDatabasePath = Join-Path $directProfile "zen-canvas.sqlite3"
        $directDatabaseSha256BeforeRequest = Get-Hash $directDatabasePath
        $pipePresent = @((Get-ChildItem -LiteralPath "\\.\pipe\" -ErrorAction SilentlyContinue | Where-Object { $_.Name -ieq "ZenCanvas.GlobalIndex.v3" })).Count -gt 0
        $pipeAbsentBefore = ($directServiceStopped -and $directServiceStopped.state -ieq "Stopped" -and -not $pipePresent)
        if (-not $pipeAbsentBefore) { throw "U1R Direct fallback test did not prove the task-owned Service was stopped and v3 pipe absent" }
        $directApp = Start-U1App $exe "u1r-direct" $directProfile $directTrace $FixtureRoot
        $directProcess = @(Get-AppProcessSnapshots $exe | Where-Object { $_.processId -eq $directApp.Id }) | Select-Object -First 1
        if (-not $directProcess) { throw "U1R Direct fallback client PID is absent from the canonical old image path" }
        $directDone = Wait-U1TraceCount $directTrace "u1r_direct_mft_returned" 1 240
        $transportError = Wait-U1TraceCount $directTrace "u1r_service_transport_error" 1 30
        $directFallback = Wait-U1TraceCount $directTrace "u1r_direct_fallback_started" 1 30
        $direct = [ordered]@{ status = if ($directDone -and $directFallback -and $transportError) { "OBSERVED_LEGACY_DIRECT_FALLBACK_WITH_RICH_BASELINE" } else { "UNVERIFIED_DIRECT_FALLBACK" }; clientPid = $directApp.Id; process = $directProcess; binarySha256 = Get-Hash $exe; clientTrace = $directTrace; serviceWasStopped = $pipeAbsentBefore; startedService = $directService; stoppedService = $directServiceStopped; stoppedServiceBinarySha256 = $directServiceImageHash; transportErrorObserved = $transportError; databasePath = $directDatabasePath; databaseSha256BeforeRequest = $directDatabaseSha256BeforeRequest; initialSnapshot = $directBefore; directFallbackObserved = $directFallback; mftReturned = $directDone }
        Stop-U1Process $directApp
        $direct.postSnapshot = Invoke-U1RProfile "snapshot" $directProfile $SourceId $MarkerName
        $direct.databaseSha256AfterRequest = Get-Hash $directDatabasePath
        $direct.statePreservation = Compare-U1RStateSnapshots $directBefore $direct.postSnapshot
        $direct.stateDelta = [ordered]@{ source = $direct.postSnapshot.source; entry = $direct.postSnapshot.entry; markerFtsHits = $direct.postSnapshot.markerFtsHits; managedEntries = $direct.postSnapshot.managedEntries; aiAnalysisState = $direct.postSnapshot.aiAnalysisState; aiJobs = $direct.postSnapshot.aiJobs; aiJobItems = $direct.postSnapshot.aiJobItems }
        Remove-U1RScmService "u1r-direct-service" | Out-Null
        $global:U1Evidence.u1rDirectFallback = $direct
        Copy-U1RRuntimeFile $directTrace "u1r-direct-client.trace.log" | Out-Null
        Copy-U1RProfileEvidence $directProfile "u1r-direct-fallback" | Out-Null
        Save-Evidence
    } catch {
        $direct.status = "UNVERIFIED_DIRECT_TEST_HARNESS_FAILURE"
        $direct.failure = $_.Exception.ToString()
        $global:U1Evidence.u1rDirectFallback = $direct
        Save-Evidence
    } finally {
        Stop-U1Process $directApp
        Remove-U1RScmService "u1r-direct-cleanup"
        Copy-U1RRuntimeFile $directTrace "u1r-direct-client.trace.log" | Out-Null
        $direct.profileFiles = Copy-U1RProfileEvidence $directProfile "u1r-direct-fallback"
        $global:U1Evidence.u1rDirectFallback = $direct
        Save-Evidence
    }
    Remove-Item -LiteralPath $installDir -Recurse -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $installDir) { throw "U1R same-path install directory could not be cleaned after isolated test" }
    $global:U1OwnedInstallDirectory = $null
}

function New-U1RRichFixture($OldCandidate) {
    $volume = New-U1Volume "u1r-rich-generation-fixture"
    $global:U1RVolumes.Add($volume)
    $fixtureRoot = Join-Path $volume.root "fixture"
    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
    $markerName = "u1r-generation-$([Guid]::NewGuid().ToString('N')).txt"
    Set-Content -LiteralPath (Join-Path $fixtureRoot $markerName) -Value "synthetic U1R managed and AI marker" -NoNewline
    $profile = Join-Path $taskRoot "u1r-baseline-profile"
    $trace = Join-Path $taskRoot "u1r-baseline-client.trace.log"
    $service = $null
    $client = $null
    try {
        $prepared = Invoke-ProfilePrepare $OldCandidate.probeExe $profile $volume.drive
        $service = Start-U1ConsoleService $OldCandidate.qaExe "u1r-matching-baseline"
        $client = Start-U1App $OldCandidate.qaExe "u1r-matching-baseline" $profile $trace $fixtureRoot
        $baseline = Wait-ForFixture $OldCandidate.probeExe $profile $markerName 180
        if (-not $baseline.Hit) { throw "old/old real binary baseline did not index the disposable marker" }
        Stop-U1Process $client
        $client = $null
        Stop-U1Process $service
        $service = $null
        Wait-PipeAbsent
        $rich = Invoke-U1RProfile "seed-rich" $profile $prepared.targetSourceId $markerName
        $installerProfile = Join-Path $taskRoot "u1r-installer-profile"
        Copy-U1RProfile $profile $installerProfile
        $installerProfileReady = Invoke-U1RProfile "set-ready" $installerProfile $prepared.targetSourceId $markerName
        $fixture = [ordered]@{
            sourceId = $prepared.targetSourceId
            volume = $volume
            fixtureRoot = $fixtureRoot
            markerName = $markerName
            baselineProfile = $profile
            installerProfile = $installerProfile
            richFixture = $rich
            installerProfileReady = $installerProfileReady
            baselineServiceSha256 = Get-Hash $OldCandidate.qaExe
            baselineClientSha256 = Get-Hash $OldCandidate.qaExe
        }
        $global:U1Evidence.u1rFixture = $fixture
        Save-Evidence
        return $fixture
    } finally {
        Stop-U1Process $client
        Stop-U1Process $service
    }
}

try {
    New-Item -ItemType Directory -Path $EvidenceRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $worktreeRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $taskRoot -Force | Out-Null
    $os = Get-CimInstance Win32_OperatingSystem
    $global:U1Evidence.runner = [ordered]@{ caption = $os.Caption; version = $os.Version; build = $os.BuildNumber; architecture = $os.OSArchitecture; runId = $env:GITHUB_RUN_ID; runAttempt = $env:GITHUB_RUN_ATTEMPT; powershell = $PSVersionTable.PSVersion.ToString(); currentDirectory = $repoRoot }
    if ($u1Tree -cne (Get-GitValue $repoRoot "HEAD^{tree}")) { throw "U1 workflow checkout Tree mismatch" }
    if ($oldSha -cne "3275ac7c72df8ae4139a30f16c7f649971324e97" -or $d1aSha -cne "d260131f9d6ec354f2361e6c7460c79e62e38391") { throw "frozen candidate constants changed" }
    if (Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue) { throw "fresh Hosted Windows runner unexpectedly has ZenCanvasGlobalIndex installed" }

    $oldPath = Join-Path $worktreeRoot "old-master"
    $d1aPath = Join-Path $worktreeRoot "d1a"
    $oldCandidate = Build-Candidate "old" $oldSha $oldTree $oldPath
    $d1aCandidate = Build-Candidate "d1a" $d1aSha $d1aTree $d1aPath
    Save-Evidence
    if ($Mode -eq "U1R") {
        $global:U1Evidence.u1rInstaller = [ordered]@{ status = "UNVERIFIED_NOT_STARTED" }
        $global:U1Evidence.u1rSamePath = [ordered]@{ status = "UNVERIFIED_NOT_STARTED" }
        $global:U1Evidence.u1rDirectFallback = [ordered]@{ status = "UNVERIFIED_NOT_STARTED" }
        $fixture = New-U1RRichFixture $oldCandidate
        try {
            Invoke-U1RSamePathAndDirectTests $oldCandidate $d1aCandidate $fixture.baselineProfile $fixture.sourceId $fixture.markerName $fixture.fixtureRoot
        } catch {
            $global:U1Evidence.u1rSamePath = [ordered]@{ status = "UNVERIFIED_TEST_HARNESS_FAILURE"; failure = $_.Exception.ToString() }
            Add-Failure "U1R-same-path-and-direct-fallback" $_.Exception.ToString()
        }
        try {
            Invoke-U1RInstallerDiagnostics $oldCandidate $d1aCandidate $fixture.installerProfile $fixture.volume.drive $fixture.sourceId $fixture.markerName
        } catch {
            $global:U1Evidence.u1rInstaller.failure = $_.Exception.ToString()
            Add-Failure "U1R-NSIS-first-failure-diagnostic" $_.Exception.ToString()
        }
        $installerAttempt = if ($global:U1Evidence.u1rInstaller["noUpdateAttempt"]) { $global:U1Evidence.u1rInstaller["noUpdateAttempt"] } else { $null }
        $samePathAttempt = $global:U1Evidence.u1rSamePath
        $directAttempt = $global:U1Evidence.u1rDirectFallback
        $u1rBlockers = [Collections.Generic.List[string]]::new()
        if (-not $installerAttempt -or $installerAttempt.status -eq "ROOT_CAUSE_UNRESOLVED") { $u1rBlockers.Add("NSIS first failing condition remains unresolved") }
        if (-not $samePathAttempt -or $samePathAttempt.status -ne "PROVED_OLD_GENERATION_REQUEST_ACCEPTED_BY_NEW_SAME_PATH_SERVICE") {
            $u1rBlockers.Add("old process isolation is not proven by a same-path generation identity gate")
        } else {
            $u1rBlockers.Add("new same-path Service accepted a Rebuild request from an old mapped client")
        }
        if (-not $directAttempt -or $directAttempt.status -ne "OBSERVED_LEGACY_DIRECT_FALLBACK_WITH_RICH_BASELINE") {
            $u1rBlockers.Add("legacy Direct fallback was not fully observed with the rich checkpoint fixture")
        } elseif (-not $directAttempt.statePreservation.preserved) {
            $u1rBlockers.Add("legacy Direct fallback changed one or more old index, checkpoint, or user association fields")
        }
        if ($installerAttempt -and $installerAttempt.databaseStatePreserved -and -not $installerAttempt.databaseStatePreserved.preserved) {
            $u1rBlockers.Add("failed in-place installation changed the old active database or associated rows")
        }
        $global:U1Evidence.u1rDisposition = [ordered]@{
            status = if ($u1rBlockers.Count -eq 0) { "READY_FOR_U2_OWNER_DECISION" } else { "BLOCKED" }
            blockers = @($u1rBlockers)
            nsisRootCauseLocated = [bool]($installerAttempt -and $installerAttempt.status -eq "EXIT_2_ROOT_CAUSE_LOCATED")
            oldSamePathClientIsolation = if ($samePathAttempt) { $samePathAttempt.status } else { "UNVERIFIED" }
            oldClientDirectFallback = if ($directAttempt) { $directAttempt.status } else { "UNVERIFIED" }
            productionFixImplemented = $false
        }
    } else {
        try { Invoke-MatchingQualification "old" $oldCandidate } catch { Add-Failure "old-client-plus-old-Service" $_.Exception.ToString() }
        try { Invoke-MatchingQualification "d1a" $d1aCandidate } catch { Add-Failure "new-client-plus-new-Service" $_.Exception.ToString() }

        $global:U1Evidence.directFallback = Invoke-BackgroundPairScenario "old-client-service-absent" $oldCandidate.qaExe $oldCandidate.probeExe $oldCandidate.qaExeSha256 $null $oldCandidate.qaExe $true
        $global:U1Evidence.mixedPairs["old-client-plus-new-Service"] = Invoke-BackgroundPairScenario "old-client-new-Service-side-by-side-path" $oldCandidate.qaExe $oldCandidate.probeExe $oldCandidate.qaExeSha256 $d1aCandidate.qaExe $oldCandidate.qaExe $false
        $global:U1Evidence.mixedPairs["new-client-plus-old-Service"] = Invoke-BackgroundPairScenario "new-client-old-Service-side-by-side-path" $d1aCandidate.qaExe $d1aCandidate.probeExe $d1aCandidate.qaExeSha256 $oldCandidate.qaExe $d1aCandidate.qaExe $false $true
        $global:U1Evidence.mixedPairs["exact-same-path-mixed-generation"] = [ordered]@{ status = "UNVERIFIED"; reason = "The Service enforces exact executable path equality, and the normal installer stops Service and desktop before replacing that path. Side-by-side mixed-path real binaries are tested, but the transient same-path old-process/new-file race is not safely created by this harness; file locking and stale process image semantics remain unverified." }
        Save-Evidence

        try { Invoke-InstallerTransition $oldCandidate $d1aCandidate } catch { $global:U1Evidence.installer.status = "FAIL_OR_UNVERIFIED"; $global:U1Evidence.installer.failure = $_.Exception.ToString(); Add-Failure "NSIS install-upgrade-interruption-repair" $_.Exception.ToString() }
    }
    Save-Evidence
} catch {
    Add-Failure "harness-fatal" $_.Exception.ToString()
} finally {
    Save-Evidence
    try {
        $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
        $installedExe = if ($global:U1OwnedInstallDirectory) { Join-Path $global:U1OwnedInstallDirectory "zen-canvas.exe" } else { $null }
        $installedImagePath = if ($installedExe) { ('"' + $installedExe + '" --index-service') } else { $null }
        if ($service) {
            if ($service.State -ne "Stopped") { & sc.exe stop "ZenCanvasGlobalIndex" *> (Join-Path $EvidenceRoot "cleanup-service-stop.log"); Start-Sleep -Seconds 3 }
            $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
            if ($service -and $service.PathName -match [regex]::Escape($taskRoot)) {
                & sc.exe delete "ZenCanvasGlobalIndex" *> (Join-Path $EvidenceRoot "cleanup-service-delete.log")
            } elseif ($service -and $installedImagePath -and $service.PathName.Trim() -ieq $installedImagePath) {
                & sc.exe stop "ZenCanvasGlobalIndex" *> (Join-Path $EvidenceRoot "cleanup-installed-service-stop.log")
                $stopDeadline = [DateTime]::UtcNow.AddSeconds(90)
                do {
                    $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
                    if (-not $service -or $service.State -ieq "Stopped") { break }
                    Start-Sleep -Milliseconds 500
                } while ([DateTime]::UtcNow -lt $stopDeadline)
                if ($service -and $service.State -ine "Stopped") {
                    $servicePid = [int]$service.ProcessId
                    $serviceImage = if ($servicePid) { (Get-CimInstance Win32_Process -Filter "ProcessId=$servicePid" -ErrorAction SilentlyContinue).ExecutablePath } else { $null }
                    if ($serviceImage -and [IO.Path]::GetFullPath($serviceImage).Equals([IO.Path]::GetFullPath($installedExe), [StringComparison]::OrdinalIgnoreCase)) {
                        Stop-Process -Id $servicePid -Force -ErrorAction SilentlyContinue
                    } else {
                        throw "refusing to terminate non-matching installer Service process: $serviceImage"
                    }
                }
                & sc.exe delete "ZenCanvasGlobalIndex" *> (Join-Path $EvidenceRoot "cleanup-installed-service-delete.log")
                $deleteDeadline = [DateTime]::UtcNow.AddSeconds(45)
                do {
                    $service = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
                    if (-not $service) { break }
                    Start-Sleep -Milliseconds 500
                } while ([DateTime]::UtcNow -lt $deleteDeadline)
            } else {
                $global:U1Evidence.cleanup.failures += "refused SCM cleanup because service ImagePath was not within the task-owned directories: $($service.PathName)"
            }
        }
        Get-CimInstance Win32_Process -Filter "Name='zen-canvas.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.ExecutablePath -and (($global:U1OwnedInstallDirectory -and [IO.Path]::GetFullPath($_.ExecutablePath).StartsWith([IO.Path]::GetFullPath($global:U1OwnedInstallDirectory), [StringComparison]::OrdinalIgnoreCase)) -or $_.ExecutablePath.StartsWith($taskRoot, [StringComparison]::OrdinalIgnoreCase) -or $_.ExecutablePath.StartsWith($worktreeRoot, [StringComparison]::OrdinalIgnoreCase)) } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
        Get-CimInstance Win32_Process -Filter "Name='zen-canvas-qa.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($worktreeRoot, [StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
        if ($global:U1OwnedInstallDirectory -and (Test-Path -LiteralPath $global:U1OwnedInstallDirectory)) {
            $uninstaller = Join-Path $global:U1OwnedInstallDirectory "uninstall.exe"
            if (Test-Path -LiteralPath $uninstaller) {
                $uninstallProcess = Start-Process -FilePath $uninstaller -ArgumentList @("/S") -PassThru -WindowStyle Hidden
                if (-not $uninstallProcess.WaitForExit(120000)) { $uninstallProcess.Kill($true); throw "task-owned test install uninstaller timed out" }
                if ($uninstallProcess.ExitCode -ne 0) { throw "task-owned test install uninstaller returned $($uninstallProcess.ExitCode)" }
                $uninstallProcess.Dispose()
            }
        }
        $remainingService = Get-CimInstance Win32_Service -Filter "Name='ZenCanvasGlobalIndex'" -ErrorAction SilentlyContinue
        if ($remainingService -and ($remainingService.PathName -match [regex]::Escape($taskRoot) -or $remainingService.PathName -match [regex]::Escape($worktreeRoot) -or ($installedImagePath -and $remainingService.PathName.Trim() -ieq $installedImagePath))) {
            $global:U1Evidence.cleanup.failures += "task-owned service registration remains after cleanup: $($remainingService.PathName)"
        }
        if ($global:U1OwnedInstallDirectory -and (Test-Path -LiteralPath $global:U1OwnedInstallDirectory)) {
            Remove-Item -LiteralPath $global:U1OwnedInstallDirectory -Recurse -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $global:U1OwnedInstallDirectory) { $global:U1Evidence.cleanup.failures += "task-owned canonical install directory remains: $global:U1OwnedInstallDirectory" }
        }
        $global:U1Evidence.cleanup.details += "cleanup inspected only the task service name and task-owned process paths"
    } catch { $global:U1Evidence.cleanup.failures += $_.Exception.ToString() }
    foreach ($volume in $global:U1RVolumes) { Remove-U1Volume $volume }
    Save-Evidence
}

if ($global:U1Evidence.failures.Count -gt 0 -or $global:U1Evidence.cleanup.failures.Count -gt 0) {
    Write-Error "U1 Hosted Windows compatibility gate has failing or unverified required scenarios; see $EvidenceRoot\u1-evidence.json"
    exit 1
}
Write-Output "$Mode Hosted Windows evidence: $EvidenceRoot\u1-evidence.json"
