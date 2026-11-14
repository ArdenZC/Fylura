# Observer only. Never trims a candidate working set, changes priority, suspends
# or otherwise perturbs the sampled process tree.
param(
    [Parameter(Mandatory = $true)][int]$MainProcessId,
    [Parameter(Mandatory = $true)][string]$OutputPath,
    [string]$Label = "sample",
    [ValidateSet("before_search", "after_first_search", "after_repeat_search", "after_deep_page", "after_incremental_update", "after_revocation", "after_idle", "at_exit")]
    [string]$Phase = "before_search",
    [string]$Query = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# Process tree sampling for the installed application: the main process, its
# WebView2 children, the separate index service and any other descendants are
# reported separately and then aggregated. Both WorkingSet and PrivateUsage are
# recorded because a trimmed working set is not evidence that committed private
# bytes were released.
function Get-TreeProcesses([int]$RootId) {
    $all = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CommandLine)
    $byParent = @{}
    foreach ($process in $all) {
        $parent = [int]$process.ParentProcessId
        if (-not $byParent.ContainsKey($parent)) { $byParent[$parent] = @() }
        $byParent[$parent] += $process
    }
    $result = [System.Collections.Generic.List[object]]::new()
    $queue = [System.Collections.Generic.Queue[int]]::new()
    $queue.Enqueue($RootId)
    $seen = @{}
    while ($queue.Count -gt 0) {
        $current = $queue.Dequeue()
        if ($seen.ContainsKey($current)) { continue }
        $seen[$current] = $true
        $match = $all | Where-Object { [int]$_.ProcessId -eq $current } | Select-Object -First 1
        if ($null -ne $match) { $result.Add($match) }
        if ($byParent.ContainsKey($current)) {
            foreach ($child in $byParent[$current]) { $queue.Enqueue([int]$child.ProcessId) }
        }
    }
    return $result
}

function Get-Role([string]$Name, [string]$CommandLine, [int]$RootId, [int]$ProcessId) {
    if ($ProcessId -eq $RootId) { return "application" }
    if ($Name -ieq "msedgewebview2.exe") { return "webview" }
    if ($Name -ieq "fylura.exe" -and $CommandLine -like "*--index-service*") { return "index_service" }
    if ($Name -ieq "fylura.exe") { return "application_helper" }
    return "descendant"
}

function Read-ProcessMetrics([int]$ProcessId, [string]$Role, [string]$Name, [string]$CommandLine) {
    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) { return $null }
    $process.Refresh()
    return [ordered]@{
        pid = $ProcessId
        role = $Role
        image = $Name
        commandLine = $CommandLine
        workingSetBytes = [int64]$process.WorkingSet64
        privateUsageBytes = [int64]$process.PrivateMemorySize64
        handleCount = [int]$process.HandleCount
        cpuTotalSeconds = [double]$process.TotalProcessorTime.TotalSeconds
        startTimeUtc = try { $process.StartTime.ToUniversalTime().ToString("o") } catch { $null }
    }
}

$root = Get-CimInstance Win32_Process -Filter "ProcessId=$MainProcessId" -ErrorAction SilentlyContinue
$rootName = if ($null -eq $root) { $null } else { $root.Name }
$commandLine = if ($null -eq $root) { $null } else { $root.CommandLine }

$processes = @(Get-TreeProcesses -RootId $MainProcessId)
$rows = [System.Collections.Generic.List[object]]::new()
foreach ($process in $processes) {
    $role = Get-Role -Name $process.Name -CommandLine $process.CommandLine -RootId $MainProcessId -ProcessId ([int]$process.ProcessId)
    $metrics = Read-ProcessMetrics -ProcessId ([int]$process.ProcessId) -Role $role -Name $process.Name -CommandLine $process.CommandLine
    if ($null -ne $metrics) { $rows.Add($metrics) }
}

# The separate index service is not a child of the desktop process. It is
# reported whenever it exists so the aggregate is not mistaken for a
# single-process measurement.
$serviceRows = @(Get-CimInstance Win32_Process -Filter "Name='fylura.exe'" | Where-Object { $_.CommandLine -like "*--index-service*" })
foreach ($service in $serviceRows) {
    if ($rows | Where-Object { $_.pid -eq [int]$service.ProcessId }) { continue }
    $metrics = Read-ProcessMetrics -ProcessId ([int]$service.ProcessId) -Role "index_service" -Name $service.Name -CommandLine $service.CommandLine
    if ($null -ne $metrics) { $rows.Add($metrics) }
}

$aggregate = [ordered]@{
    processCount = $rows.Count
    applicationCount = @($rows | Where-Object { $_.role -in @("application", "application_helper") }).Count
    webviewCount = @($rows | Where-Object { $_.role -eq "webview" }).Count
    indexServiceCount = @($rows | Where-Object { $_.role -eq "index_service" }).Count
    workingSetBytes = [int64](($rows | Measure-Object -Property workingSetBytes -Sum).Sum)
    privateUsageBytes = [int64](($rows | Measure-Object -Property privateUsageBytes -Sum).Sum)
    handleCount = [int64](($rows | Measure-Object -Property handleCount -Sum).Sum)
    cpuTotalSeconds = [double](($rows | Measure-Object -Property cpuTotalSeconds -Sum).Sum)
}

$record = [ordered]@{
    timestampUtc = [DateTime]::UtcNow.ToString("o")
    label = $Label
    phase = $Phase
    query = $Query
    mainProcessId = $MainProcessId
    mainImage = $rootName
    mainCommandLine = $commandLine
    sampledBy = "sampleGlobalSearchApplicationTree.ps1"
    workingSetTrimApplied = $false
    privateUsageIsWorkingSet = $false
    aggregate = $aggregate
    processes = $rows
    availableMemoryBytes = try {
        $os = Get-CimInstance Win32_OperatingSystem
        [int64]($os.FreePhysicalMemory * 1024)
    } catch { $null }
}

$directory = Split-Path -Parent $OutputPath
if ($directory -and -not (Test-Path -LiteralPath $directory)) { New-Item -ItemType Directory -Path $directory -Force | Out-Null }
$line = $record | ConvertTo-Json -Depth 6 -Compress
Add-Content -LiteralPath $OutputPath -Value $line -Encoding UTF8
$record | ConvertTo-Json -Depth 6
