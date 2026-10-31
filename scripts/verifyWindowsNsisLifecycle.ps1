param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [Parameter(Mandatory = $true)]
    [string]$EvidencePath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$installer = [System.IO.Path]::GetFullPath($InstallerPath)
$runnerTemp = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')
$taskRoot = [System.IO.Path]::GetFullPath((Join-Path $runnerTemp "fylura-nsis-runtime-$env:GITHUB_RUN_ID"))
$installDir = Join-Path $taskRoot 'install'
$mainBinary = Join-Path $installDir 'fylura.exe'
$profileRoot = Join-Path $taskRoot 'profile'
$legacyService = 'ZenCanvasGlobalIndex'
$currentService = 'FyluraGlobalIndex'
$legacyServiceKey = "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$legacyService"
$currentServiceKey = "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$currentService"
$uninstallRoot = 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
$evidence = [System.Collections.Generic.List[object]]::new()
$legacyServiceCreated = $false
$originalAppData = $env:APPDATA
$script:overallStatus = 'IN_PROGRESS'

function Save-Evidence {
    $parent = Split-Path -Parent $EvidencePath
    if ($parent) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $document = [ordered]@{
        product = 'Fylura'
        sourceSha = $env:GITHUB_SHA
        runId = $env:GITHUB_RUN_ID
        status = $overallStatus
        phases = @($evidence.ToArray())
    }
    $document | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
}

function Record-Phase([string]$Phase, [hashtable]$Details = @{}) {
    $evidence.Add([ordered]@{
        phase = $Phase
        at = [DateTime]::UtcNow.ToString('o')
        details = $Details
    })
    Save-Evidence
}

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function Invoke-Sc([string[]]$Arguments) {
    $output = & "$env:WINDIR\System32\sc.exe" @Arguments 2>&1
    $exitCode = $LASTEXITCODE
    return [pscustomobject]@{
        exitCode = $exitCode
        output = ($output | Out-String).Trim()
    }
}

function Invoke-Installer([string]$Path) {
    $process = Start-Process -FilePath $Path -ArgumentList @('/S', "/D=$installDir") -PassThru -NoNewWindow
    if (-not $process.WaitForExit(120000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "Timed out waiting for task-owned installer process $($process.Id)."
    }
    $process.Refresh()
    return [int]$process.ExitCode
}

function Stop-TaskOwnedMainWindowProcess {
    $expectedBinary = [System.IO.Path]::GetFullPath($mainBinary)
    $ownedProcesses = @(
        Get-CimInstance -ClassName Win32_Process -Filter "Name = 'fylura.exe'" | Where-Object {
            $_.ExecutablePath -and
            [System.IO.Path]::GetFullPath($_.ExecutablePath).Equals($expectedBinary, [StringComparison]::OrdinalIgnoreCase) -and
            $_.CommandLine -notmatch '--index-service'
        }
    )
    foreach ($ownedProcess in $ownedProcesses) {
        Stop-Process -Id $ownedProcess.ProcessId -Force -ErrorAction Stop
    }
    return $ownedProcesses.Count
}

function Get-FyluraUninstallEntries {
    if (-not (Test-Path -LiteralPath $uninstallRoot)) {
        return @()
    }
    $matches = @(
        Get-ChildItem -LiteralPath $uninstallRoot -ErrorAction Stop | ForEach-Object {
            $properties = Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue
            $displayNameProperty = if ($null -ne $properties) { $properties.PSObject.Properties['DisplayName'] }
            if ($null -ne $displayNameProperty -and $displayNameProperty.Value -eq 'Fylura') {
                $installLocationProperty = $properties.PSObject.Properties['InstallLocation']
                [pscustomobject]@{
                    key = $_.PSChildName
                    installLocation = if ($null -ne $installLocationProperty) { $installLocationProperty.Value } else { $null }
                }
            }
        } | Where-Object { $_ }
    )
    return $matches
}

function Remove-TaskOwnedLegacyService {
    if (-not $legacyServiceCreated) {
        return
    }
    $null = Invoke-Sc @('stop', $legacyService)
    $result = Invoke-Sc @('delete', $legacyService)
    Assert-Condition ($result.exitCode -eq 0) "Could not delete task-owned legacy service fixture: $($result.output)"
    for ($attempt = 0; $attempt -lt 30 -and (Test-Path -LiteralPath $legacyServiceKey); $attempt++) {
        Start-Sleep -Milliseconds 500
    }
    Assert-Condition (-not (Test-Path -LiteralPath $legacyServiceKey)) 'Task-owned legacy service fixture remained registered after deletion.'
    $script:legacyServiceCreated = $false
}

try {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    Assert-Condition $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) 'UNVERIFIED: NSIS lifecycle validation requires the disposable runner to be elevated.'
    Assert-Condition (Test-Path -LiteralPath $installer -PathType Leaf) "NSIS installer was not found: $installer"
    Assert-Condition (-not (Test-Path -LiteralPath $taskRoot)) "Refusing pre-existing task directory: $taskRoot"
    Assert-Condition (-not (Test-Path -LiteralPath $legacyServiceKey)) "Refusing pre-existing legacy service: $legacyService"
    Assert-Condition (-not (Test-Path -LiteralPath $currentServiceKey)) "Refusing pre-existing Fylura service: $currentService"
    $existingProducts = @(Get-FyluraUninstallEntries)
    Assert-Condition ($existingProducts.Count -eq 0) 'Refusing to run against a pre-existing Fylura installation.'

    New-Item -ItemType Directory -Path $taskRoot | Out-Null
    New-Item -ItemType Directory -Path $profileRoot | Out-Null
    $env:APPDATA = $profileRoot
    Record-Phase 'preflight' @{
        installer = $installer
        taskRoot = $taskRoot
        installDir = $installDir
        appDataRoot = $profileRoot
        administrator = $true
        existingFyluraProducts = 0
        existingLegacyService = $false
        existingFyluraService = $false
    }

    $legacyBinPath = "`"$env:WINDIR\System32\cmd.exe`" /c exit 0"
    $createLegacy = Invoke-Sc @('create', $legacyService, 'binPath=', $legacyBinPath, 'start=', 'demand', 'DisplayName=', 'Fylura task-owned legacy conflict fixture')
    Assert-Condition ($createLegacy.exitCode -eq 0) "Could not create task-owned legacy service fixture: $($createLegacy.output)"
    $legacyServiceCreated = $true
    $legacyImagePath = (Get-ItemProperty -LiteralPath $legacyServiceKey -Name ImagePath).ImagePath
    Assert-Condition ($legacyImagePath -like '*cmd.exe* /c exit 0*') "Legacy service fixture has an unexpected ImagePath: $legacyImagePath"
    $rollbackExitCode = Invoke-Installer $installer
    $legacyImagePathAfter = (Get-ItemProperty -LiteralPath $legacyServiceKey -Name ImagePath).ImagePath
    Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $installDir 'fylura.exe'))) 'Legacy-service conflict did not abort before installing the Fylura executable.'
    Assert-Condition (-not (Test-Path -LiteralPath $currentServiceKey)) 'Legacy-service conflict created the new Fylura service.'
    Assert-Condition (@(Get-FyluraUninstallEntries).Count -eq 0) 'Legacy-service conflict left a Fylura uninstall registration behind.'
    Assert-Condition ($legacyImagePathAfter -eq $legacyImagePath) 'Installer rollback modified the task-owned legacy service registration.'
    Record-Phase 'legacy-service-conflict-rollback' @{
        installerExitCode = $rollbackExitCode
        legacyServicePreserved = $true
        fyluraExecutableAbsent = $true
        fyluraServiceAbsent = $true
        uninstallRegistrationAbsent = $true
    }

    Remove-TaskOwnedLegacyService
    if (Test-Path -LiteralPath $installDir) {
        $resolvedInstallDir = [System.IO.Path]::GetFullPath($installDir)
        Assert-Condition $resolvedInstallDir.StartsWith($taskRoot, [StringComparison]::OrdinalIgnoreCase) 'Refusing to clear an install path outside the task-owned directory.'
        Remove-Item -LiteralPath $resolvedInstallDir -Recurse -Force
    }

    $freshExitCode = Invoke-Installer $installer
    Assert-Condition ($freshExitCode -eq 0) "Fresh NSIS install failed with exit code $freshExitCode."
    $freshMainProcessCount = Stop-TaskOwnedMainWindowProcess
    $uninstaller = Join-Path $installDir 'uninstall.exe'
    Assert-Condition (Test-Path -LiteralPath $mainBinary -PathType Leaf) 'Fresh install did not publish fylura.exe.'
    Assert-Condition (Test-Path -LiteralPath $uninstaller -PathType Leaf) 'Fresh install did not publish uninstall.exe.'
    $freshEntries = @(Get-FyluraUninstallEntries)
    Assert-Condition ($freshEntries.Count -eq 1) "Fresh install created $($freshEntries.Count) Fylura uninstall entries; expected exactly one."
    Record-Phase 'fresh-install' @{
        exitCode = $freshExitCode
        mainBinary = $mainBinary
        uninstaller = $uninstaller
        stoppedOwnedMainWindowProcesses = $freshMainProcessCount
        uninstallEntry = $freshEntries[0]
    }

    $currentDataDir = Join-Path $profileRoot 'com.startlan.fylura'
    $legacyDataDir = Join-Path $profileRoot 'com.startlan.zencanvas'
    $safeTrashSource = Join-Path $taskRoot 'safe-trash-source'
    $trashDir = Join-Path $safeTrashSource '.zen-canvas-trash\items\task-owned-fixture'
    New-Item -ItemType Directory -Path $currentDataDir, $legacyDataDir, $trashDir -Force | Out-Null
    $sentinels = [ordered]@{
        currentDatabase = Join-Path $currentDataDir 'fylura.sqlite3'
        currentWal = Join-Path $currentDataDir 'fylura.sqlite3-wal'
        legacyDatabase = Join-Path $legacyDataDir 'zen-canvas.sqlite3'
        safeTrash = Join-Path $trashDir 'user-data-sentinel.txt'
    }
    foreach ($entry in $sentinels.GetEnumerator()) {
        [System.IO.File]::WriteAllText($entry.Value, "task-owned $($entry.Key) sentinel")
    }

    $repeatExitCode = Invoke-Installer $installer
    Assert-Condition ($repeatExitCode -eq 0) "Repeated NSIS install failed with exit code $repeatExitCode."
    $repeatMainProcessCount = Stop-TaskOwnedMainWindowProcess
    Assert-Condition (Test-Path -LiteralPath $mainBinary -PathType Leaf) 'Repeated install removed fylura.exe.'
    Assert-Condition (@(Get-FyluraUninstallEntries).Count -eq 1) 'Repeated install duplicated or removed the Fylura uninstall registration.'
    foreach ($entry in $sentinels.GetEnumerator()) {
        Assert-Condition (Test-Path -LiteralPath $entry.Value -PathType Leaf) "Repeated install removed task-owned user data: $($entry.Key)."
    }
    Record-Phase 'repeat-install' @{
        exitCode = $repeatExitCode
        stoppedOwnedMainWindowProcesses = $repeatMainProcessCount
        mainBinaryPreserved = $true
        uninstallRegistrationCount = 1
        userDataPreserved = $true
    }

    $uninstallExitCode = (Start-Process -FilePath $uninstaller -ArgumentList @('/S') -PassThru -NoNewWindow -Wait).ExitCode
    Assert-Condition ($uninstallExitCode -eq 0) "NSIS uninstall failed with exit code $uninstallExitCode."
    for ($attempt = 0; $attempt -lt 30 -and (Test-Path -LiteralPath $mainBinary); $attempt++) {
        Start-Sleep -Milliseconds 500
    }
    Assert-Condition (-not (Test-Path -LiteralPath $mainBinary)) 'NSIS uninstall left the Fylura executable installed.'
    Assert-Condition (-not (Test-Path -LiteralPath $currentServiceKey)) 'NSIS uninstall left FyluraGlobalIndex registered.'
    Assert-Condition (@(Get-FyluraUninstallEntries).Count -eq 0) 'NSIS uninstall left Fylura in Add/Remove Programs.'
    foreach ($entry in $sentinels.GetEnumerator()) {
        Assert-Condition (Test-Path -LiteralPath $entry.Value -PathType Leaf) "NSIS uninstall deleted task-owned user data: $($entry.Key)."
        $contents = [System.IO.File]::ReadAllText($entry.Value)
        Assert-Condition ($contents -eq "task-owned $($entry.Key) sentinel") "NSIS uninstall changed task-owned user data: $($entry.Key)."
    }
    Record-Phase 'uninstall' @{
        exitCode = $uninstallExitCode
        executableRemoved = $true
        fyluraServiceRemoved = $true
        uninstallRegistrationRemoved = $true
        currentDatabaseWalLegacyDatabaseAndSafeTrashPreserved = $true
    }
    $script:overallStatus = 'PASS'
    Save-Evidence
}
catch {
    $script:overallStatus = 'FAIL'
    $evidence.Add([ordered]@{
        phase = 'failed'
        at = [DateTime]::UtcNow.ToString('o')
        error = $_.Exception.Message
    })
    Save-Evidence
    throw
}
finally {
    $env:APPDATA = $originalAppData
    Remove-TaskOwnedLegacyService
}
