param(
    [Parameter(Mandatory = $true)][string]$EvidenceRoot
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
$global:U1Evidence = [ordered]@{
    contract = "U1 frozen-candidate compatibility; test-only; not a production fix"
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

function Invoke-BackgroundPairScenario([string]$Name, [string]$ClientExe, [string]$ClientProbe, [string]$ClientSha, [string]$ServiceExe, [string]$BaselineServiceExe, [bool]$ExpectDirect) {
    $volume = $null
    $service = $null
    $client = $null
    $profile = Join-Path $taskRoot "$Name-profile"
    $trace = Join-Path $taskRoot "$Name-trace.log"
    $safeName = $Name -replace "[^A-Za-z0-9]", ""
    $markerName = "u1${safeName}$([Guid]::NewGuid().ToString('N')).txt"
    $nextMarkerName = "u1next${safeName}$([Guid]::NewGuid().ToString('N')).txt"
    $result = [ordered]@{ status = "RUNNING"; clientSha256 = $ClientSha; serviceExe = $ServiceExe; pipeObserved = $false; markerName = $markerName; nextMarkerName = $nextMarkerName; baselineIndexed = $false; markerFound = $false; oldMarkerPreserved = $false; sourceState = $null; baselineSourceState = $null; entryCount = $null; baselineEntryCount = $null; resultPath = $null; clientExitCode = $null; trace = $trace; logs = @() }
    try {
        $volume = New-U1Volume $Name
        $fixtureRoot = Join-Path $volume.root "fixture"
        New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
        $markerPath = Join-Path $fixtureRoot $markerName
        Set-Content -LiteralPath $markerPath -Value "isolated U1 fixture" -NoNewline
        $prepared = Invoke-ProfilePrepare $ClientProbe $profile $volume.drive
        $result.sourceId = $prepared.targetSourceId
        # Establish one live, searchable row before the compatibility pairing.
        # Matching generations use their own exact executable path. If Service
        # is absent, the old client's real Direct fallback establishes baseline.
        if ($BaselineServiceExe) { $service = Start-U1ConsoleService $BaselineServiceExe "$Name-baseline" }
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
        if ($BaselineServiceExe) { Wait-PipeAbsent }

        # Start the tested client before adding the second fixture. Otherwise a
        # native watcher can miss the file creation that happened before it was
        # registered, turning this into a no-request test rather than a
        # Service/fallback test.
        if ($ServiceExe) {
            $service = Start-U1ConsoleService $ServiceExe $Name
            $result.pipeObserved = $true
            $result.servicePid = $service.Id
            $result.serviceImagePath = (Get-CimInstance Win32_Process -Filter "ProcessId=$($service.Id)").ExecutablePath
            $result.serviceSha256 = Get-Hash $ServiceExe
        }
        $client = Start-U1App $ClientExe $Name $profile $trace $fixtureRoot
        $result.clientPid = $client.Id
        Wait-U1AppReady $client $Name
        $coordinatorCyclesBeforeMarker = Get-U1TraceCount $trace "coordinator_cycle"
        $result.coordinatorCyclesBeforeMarker = $coordinatorCyclesBeforeMarker
        $nextMarkerPath = Join-Path $fixtureRoot $nextMarkerName
        Set-Content -LiteralPath $nextMarkerPath -Value "isolated post-baseline fixture" -NoNewline
        $result.watcherObservedNextMarker = Wait-ForWatcherPath $Name $nextMarkerName 45
        if (-not $result.watcherObservedNextMarker) { throw "watcher trace did not observe the post-baseline marker path" }
        $result.coordinatorCycleAfterMarker = Wait-U1TraceCount $trace "coordinator_cycle" ($coordinatorCyclesBeforeMarker + 1) 60
        if (-not $result.coordinatorCycleAfterMarker) { throw "file watcher observed the marker but did not wake an index coordinator cycle" }
        $waited = Wait-ForFixture $ClientProbe $profile $nextMarkerName 240
        $snapshot = $waited.Snapshot
        $hit = $waited.Hit
        $result.markerFound = $null -ne $hit
        if ($hit) { $result.resultPath = $hit.path }
        $oldMarkerSnapshot = Invoke-ProfileQuery $ClientProbe $profile $markerName
        $source = @($snapshot.volumes | Where-Object { $_.id -ceq $prepared.targetSourceId } | Select-Object -First 1)
        if ($source) { $result.sourceState = $source.status; $result.lastError = $source.lastError; $result.entryCount = [long]$source.entryCount; $result.provider = $source.provider; $result.lastFullIndexAt = $source.lastFullIndexAt }
        $result.oldMarkerPreserved = @($oldMarkerSnapshot.results | Where-Object { $_.name -ceq $markerName }).Count -gt 0
        $result.nextMarkerFound = $result.markerFound
        $result.expectedDirect = $ExpectDirect
        if (-not $result.oldMarkerPreserved) { throw "previously indexed global entry disappeared during the scenario" }
        if ($ExpectDirect -and -not $result.markerFound) { throw "expected in-process Direct indexing did not discover post-baseline marker; state=$($result.sourceState) error=$($result.lastError)" }
        if (-not $ExpectDirect -and $result.markerFound) { throw "D1-A mixed-generation client performed Direct indexing after a reachable but identity-mismatched Service" }
        if (-not $ExpectDirect -and [long]$result.entryCount -ne [long]$result.baselineEntryCount) { throw "fail-closed mixed pairing changed persisted entry count from $($result.baselineEntryCount) to $($result.entryCount)" }
        if (-not $ExpectDirect -and $result.pipeObserved -ne $true) { throw "mixed-pair test did not prove a reachable Service pipe" }
        $result.status = if ($ExpectDirect) { "PASS_DIRECT_EXECUTED" } else { "PASS_DIRECT_BLOCKED" }
    } catch {
        $result.status = "FAIL"
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

function Install-NSIS([string]$Installer, [string]$InstallDirectory, [string]$LogPath, [int]$TimeoutSeconds = 600) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Installer
    $startInfo.WorkingDirectory = Split-Path -Parent $Installer
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    # The frozen per-machine package intentionally uses its canonical
    # Program Files install location on this disposable Hosted Windows runner.
    # Avoid relying on NSIS /D parsing for a path containing spaces; the caller
    # first verifies that this exact location is absent and task-owned.
    foreach ($argument in @("/S", "/LOG=$LogPath")) { $startInfo.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to start installer $Installer" }
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill($true)
        $process.WaitForExit(10000) | Out-Null
        $process.Dispose()
        throw "installer timed out after $TimeoutSeconds seconds: $Installer"
    }
    $exitCode = $process.ExitCode
    $process.Dispose()
    return $exitCode
}

function Wait-ForInstaller([string]$Installer, [string]$InstallDirectory, [string]$LogPath, [string]$ExpectedExeHash, [int]$TimeoutSeconds, [bool]$InterruptOnReplacement) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Installer
    $startInfo.WorkingDirectory = Split-Path -Parent $Installer
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    foreach ($argument in @("/S", "/LOG=$LogPath")) { $startInfo.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "failed to start installer $Installer" }
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
        if ($nextStamp -cne $fileStamp) {
            $fileStamp = $nextStamp
            $fileHash = if ($file) { Get-Hash $exe } else { $null }
        }
        $serviceSnapshot = Get-ServiceSnapshot
        $apps = @(Get-AppProcessSnapshots $exe)
        $samples.Add([ordered]@{ atUtc = [DateTime]::UtcNow.ToString("o"); installerPid = $process.Id; installerExited = $process.HasExited; exeSha256 = $fileHash; service = $serviceSnapshot; appProcesses = $apps })
        if ($InterruptOnReplacement -and $fileHash -ceq $ExpectedExeHash -and -not $process.HasExited) {
            $process.Kill($true)
            $process.WaitForExit(10000) | Out-Null
            $interrupted = $true
            break
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
    $process.Dispose()
    return [pscustomobject]@{ ExitCode = $exitCode; Interrupted = $interrupted; Samples = @($samples); LogPath = $LogPath }
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
    Invoke-LoggedCommand "$Label-qa-build" "cargo.exe" @("build", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--release", "--features", "desktop-runtime native-qa", "--bin", "zen-canvas", "--bin", "zb05-global-index-qa") $Path | Out-Null
    $qaDir = Join-Path $Path "src-tauri/target/release/u1-qa"
    New-Item -ItemType Directory -Path $qaDir -Force | Out-Null
    $qaExe = Join-Path $qaDir "zen-canvas-qa.exe"
    $probeExe = Join-Path $qaDir "zb05-global-index-qa.exe"
    Copy-Item -LiteralPath (Join-Path $cargoTarget "release/zen-canvas.exe") -Destination $qaExe -Force
    Copy-Item -LiteralPath (Join-Path $cargoTarget "release/zb05-global-index-qa.exe") -Destination $probeExe -Force
    $qaHash = Get-Hash $qaExe
    $probeHash = Get-Hash $probeExe

    Invoke-LoggedCommand "$Label-nsis-package" "npm.cmd" @("run", "build") $Path | Out-Null
    $releaseExe = Join-Path $cargoTarget "release/zen-canvas.exe"
    $installerCandidates = @(Get-ChildItem -LiteralPath (Join-Path $cargoTarget "release/bundle/nsis") -Filter "*.exe" -File | Sort-Object LastWriteTimeUtc -Descending)
    if (-not (Test-Path -LiteralPath $releaseExe) -or $installerCandidates.Count -lt 1) { throw "$Label NSIS output or desktop executable missing after build" }
    $installer = Join-Path $qaDir "$Label-setup.exe"
    Copy-Item -LiteralPath $installerCandidates[0].FullName -Destination $installer -Force
    $releaseHash = Get-Hash $releaseExe
    $installerHash = Get-Hash $installer
    $cargoVersion = (Get-Content -LiteralPath (Join-Path $Path "src-tauri/Cargo.toml") -Raw | Select-String -Pattern '(?m)^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value
    $packageVersion = (Get-Content -LiteralPath (Join-Path $Path "package.json") -Raw | ConvertFrom-Json).version
    $tauriConfig = Get-Content -LiteralPath (Join-Path $Path "src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json
    $candidate = [ordered]@{ sha = $Sha; tree = $Tree; packageVersion = $packageVersion; cargoVersion = $cargoVersion; tauriVersion = $tauriConfig.version; identifier = $tauriConfig.identifier; qaExe = $qaExe; qaExeSha256 = $qaHash; probeExe = $probeExe; probeExeSha256 = $probeHash; productionExe = $releaseExe; productionExeSha256 = $releaseHash; installer = $installer; installerSha256 = $installerHash; fileVersion = (Get-Item -LiteralPath $releaseExe).VersionInfo.FileVersion; productVersion = (Get-Item -LiteralPath $releaseExe).VersionInfo.ProductVersion }
    $global:U1Evidence.candidates[$Label] = $candidate
    Save-Evidence

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
    $oldExit = Install-NSIS $OldCandidate.installer $installDir $oldLog
    if ($oldExit -ne 0) { throw "fresh old installer returned exit code $oldExit; $oldLog" }
    $oldService = Wait-ServiceState "Running" 120
    $installedOldHash = Get-Hash $exe
    $global:U1Evidence.installer.freshOldInstall = [ordered]@{ exePath = $exe; expectedExeSha256 = $OldCandidate.productionExeSha256; installedExeSha256 = $installedOldHash; fileVersion = (Get-Item -LiteralPath $exe).VersionInfo.FileVersion; productVersion = (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion; service = $oldService; log = $oldLog }
    Save-Evidence
    if ($installedOldHash -cne $OldCandidate.productionExeSha256) { throw "fresh old installation exe hash mismatch: expected=$($OldCandidate.productionExeSha256), installed=$installedOldHash, path=$exe" }
    $expectedImage = '"' + $exe + '" --index-service'
    if ($oldService.imagePath.Trim() -ine $expectedImage) { throw "fresh old service ImagePath mismatch: $($oldService.imagePath)" }

    $app = $null
    try {
        $installedAppProfile = Join-Path $taskRoot "installer-old-app-profile"
        $app = Start-U1App $exe "installer-old-app" $installedAppProfile (Join-Path $EvidenceRoot "installer-old-app.trace.log")
        Start-Sleep -Seconds 3
        $oldApps = @(Get-AppProcessSnapshots $exe)
        if (-not ($oldApps | Where-Object { $_.processId -eq $app.Id })) { throw "old installed desktop process did not remain alive before upgrade" }
        $preUpgrade = [ordered]@{ atUtc = [DateTime]::UtcNow.ToString("o"); service = $oldService; desktopProcessId = $app.Id; installedExeSha256 = Get-Hash $exe; isolatedAppProfile = $installedAppProfile }
        $transition = Wait-ForInstaller $D1ACandidate.installer $installDir $newUpgradeLog $D1ACandidate.productionExeSha256 600 $false
        if ($transition.ExitCode -ne 0) { throw "D1-A in-place upgrade returned $($transition.ExitCode); $newUpgradeLog" }
        $firstNewImageSample = -1
        for ($sampleIndex = 0; $sampleIndex -lt $transition.Samples.Count; $sampleIndex++) {
            if ($transition.Samples[$sampleIndex].exeSha256 -ceq $D1ACandidate.productionExeSha256) { $firstNewImageSample = $sampleIndex; break }
        }
        if ($firstNewImageSample -lt 0) { throw "installer samples did not observe the exact D1-A executable replacement" }
        $beforeImageMutation = @($transition.Samples[0..$firstNewImageSample] | Where-Object { $_.exeSha256 -ceq $OldCandidate.productionExeSha256 })
        $serviceStoppedBeforeMutation = @($beforeImageMutation | Where-Object { $_.service -and $_.service.state -ieq "Stopped" }).Count -gt 0
        $desktopExitedBeforeMutation = @($beforeImageMutation | Where-Object { $_.appProcesses.Count -eq 0 }).Count -gt 0
        if (-not $serviceStoppedBeforeMutation) { throw "samples did not prove Service STOPPED before executable replacement" }
        if (-not $desktopExitedBeforeMutation) { throw "samples did not prove old desktop exited before executable replacement" }
        $postService = Wait-ServiceState "Running" 120
        $app.Refresh()
        if (-not $app.HasExited) { throw "old desktop process survived D1-A in-place upgrade" }
        if ($postService.processId -eq $oldService.processId) { throw "Service PID did not change across stop/restart" }
        if ((Get-Hash $exe) -cne $D1ACandidate.productionExeSha256) { throw "post-upgrade installed exe hash differs from D1-A binary" }
        if ($postService.imagePath.Trim() -ine $expectedImage) { throw "post-upgrade Service ImagePath mismatch: $($postService.imagePath)" }
        $newApps = @(Get-AppProcessSnapshots $exe)
        $global:U1Evidence.installer.status = "PASS_OLD_TO_D1A_IN_PLACE"
        $global:U1Evidence.installer.installDirectory = $installDir
        $global:U1Evidence.installer.oldInstallLog = $oldLog
        $global:U1Evidence.installer.upgradeLog = $newUpgradeLog
        $global:U1Evidence.installer.preUpgrade = $preUpgrade
        $global:U1Evidence.installer.postUpgrade = [ordered]@{ service = $postService; installedExeSha256 = Get-Hash $exe; oldDesktopExited = $app.HasExited; remainingDesktopProcesses = $newApps; installerExitCode = $transition.ExitCode; serviceStoppedBeforeMutation = $serviceStoppedBeforeMutation; desktopExitedBeforeMutation = $desktopExitedBeforeMutation; newServicePidAfterMutation = (@($transition.Samples[$firstNewImageSample..($transition.Samples.Count - 1)] | Where-Object { $_.service -and $_.service.state -ieq "Running" -and $_.service.processId -ne $oldService.processId }).Count -gt 0) }
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
    if ($oldAgain -ne 0) { throw "old candidate reinstallation returned $oldAgain; $oldRepairLog" }
    $oldBeforeInterrupt = Wait-ServiceState "Running" 120
    if ((Get-Hash $exe) -cne $OldCandidate.productionExeSha256) { throw "old package did not restore exact old exe before interruption test" }
    $interruption = Wait-ForInstaller $D1ACandidate.installer $installDir $interruptedLog $D1ACandidate.productionExeSha256 600 $true
    $global:U1Evidence.installer.interruption = [ordered]@{ status = if ($interruption.Interrupted) { "INTERRUPTED_AFTER_EXE_REPLACEMENT" } else { "UNVERIFIED_INSTALLER_COMPLETED_BEFORE_INTERRUPT" }; installerExitCode = $interruption.ExitCode; observedOldServicePid = $oldBeforeInterrupt.processId; log = $interruptedLog; samples = $interruption.Samples }
    $stoppedOrNewPid = Get-ServiceSnapshot
    $runningApps = @(Get-AppProcessSnapshots $exe)
    $global:U1Evidence.installer.interruption.serviceAfterKill = $stoppedOrNewPid
    $global:U1Evidence.installer.interruption.desktopProcessesAfterKill = $runningApps
    Save-Evidence
    if ($interruption.Interrupted) {
        if ($runningApps.Count -gt 0) { throw "desktop process remains after interrupted upgrade: $($runningApps | ConvertTo-Json -Compress)" }
        if ($stoppedOrNewPid -and $stoppedOrNewPid.state -ieq "Running" -and $stoppedOrNewPid.processId -eq $oldBeforeInterrupt.processId) { throw "old Service process remained RUNNING across a post-replacement interruption" }
        $repairExit = Install-NSIS $D1ACandidate.installer $installDir $repairLog
        if ($repairExit -ne 0) { throw "D1-A repair after interruption returned $repairExit; $repairLog" }
        $repairedService = Wait-ServiceState "Running" 120
        if ((Get-Hash $exe) -cne $D1ACandidate.productionExeSha256) { throw "repair did not converge installed exe to D1-A candidate" }
        if ($repairedService.imagePath.Trim() -ine $expectedImage) { throw "repaired Service ImagePath mismatch" }
        if ($repairedService.processId -eq $oldBeforeInterrupt.processId) { throw "repair did not restart Service after interrupted install" }
        $global:U1Evidence.installer.repair = [ordered]@{ status = "PASS_REPAIR_TO_D1A"; exitCode = $repairExit; service = $repairedService; installedExeSha256 = Get-Hash $exe; log = $repairLog }
    } else {
        $global:U1Evidence.installer.repair = [ordered]@{ status = "NOT_RUN_INTERRUPT_NOT_OBSERVED"; log = $repairLog }
    }
    Save-Evidence
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

    try { Invoke-MatchingQualification "old" $oldCandidate } catch { Add-Failure "old-client-plus-old-Service" $_.Exception.ToString() }
    try { Invoke-MatchingQualification "d1a" $d1aCandidate } catch { Add-Failure "new-client-plus-new-Service" $_.Exception.ToString() }

    $global:U1Evidence.directFallback = Invoke-BackgroundPairScenario "old-client-service-absent" $oldCandidate.qaExe $oldCandidate.probeExe $oldCandidate.qaExeSha256 $null $oldCandidate.qaExe $true
    $global:U1Evidence.mixedPairs["old-client-plus-new-Service"] = Invoke-BackgroundPairScenario "old-client-new-Service-side-by-side-path" $oldCandidate.qaExe $oldCandidate.probeExe $oldCandidate.qaExeSha256 $d1aCandidate.qaExe $oldCandidate.qaExe $true
    $global:U1Evidence.mixedPairs["new-client-plus-old-Service"] = Invoke-BackgroundPairScenario "new-client-old-Service-side-by-side-path" $d1aCandidate.qaExe $d1aCandidate.probeExe $d1aCandidate.qaExeSha256 $oldCandidate.qaExe $d1aCandidate.qaExe $false
    $global:U1Evidence.mixedPairs["exact-same-path-mixed-generation"] = [ordered]@{ status = "UNVERIFIED"; reason = "Windows image locking and the Service exact-client-path gate prevent two different frozen PE generations from being launched at the same installed path without a servicing race. The actual installer stop/wait/replace/restart transition is tested separately." }
    Save-Evidence

    try { Invoke-InstallerTransition $oldCandidate $d1aCandidate } catch { $global:U1Evidence.installer.status = "FAIL_OR_UNVERIFIED"; $global:U1Evidence.installer.failure = $_.Exception.ToString(); Add-Failure "NSIS install-upgrade-interruption-repair" $_.Exception.ToString() }
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
    Save-Evidence
}

if ($global:U1Evidence.failures.Count -gt 0 -or $global:U1Evidence.cleanup.failures.Count -gt 0) {
    Write-Error "U1 Hosted Windows compatibility gate has failing or unverified required scenarios; see $EvidenceRoot\u1-evidence.json"
    exit 1
}
Write-Output "U1 Hosted Windows compatibility evidence: $EvidenceRoot\u1-evidence.json"
