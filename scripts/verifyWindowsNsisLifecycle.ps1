param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [Parameter(Mandatory = $true)]
    [string]$EvidencePath,
    [Parameter(Mandatory = $true)]
    [switch]$TaskWorker,
    [Parameter(Mandatory = $true)]
    [string]$RunnerTemp,
    [Parameter(Mandatory = $true)]
    [string]$RunId,
    [Parameter(Mandatory = $true)]
    [string]$SourceSha,
    [Parameter(Mandatory = $true)]
    [string]$PythonPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $TaskWorker) {
    throw 'Run this lifecycle worker through verifyWindowsNsisLifecycleRunner.ps1 so credentials stay isolated from the logged-in runner account.'
}
if ([Environment]::UserName -notlike 'Fylura596_*') {
    throw 'The NSIS lifecycle worker only runs as its disposable Fylura596 test account.'
}
$env:GITHUB_ACTIONS = 'true'
$env:RUNNER_ENVIRONMENT = 'github-hosted'
$env:RUNNER_TEMP = [System.IO.Path]::GetFullPath($RunnerTemp)
$env:GITHUB_RUN_ID = $RunId
$env:GITHUB_SHA = $SourceSha

$installer = [System.IO.Path]::GetFullPath($InstallerPath)
$runnerTemp = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')
$taskRoot = [System.IO.Path]::GetFullPath((Join-Path $runnerTemp "fylura-nsis-runtime-$env:GITHUB_RUN_ID"))
$installDir = Join-Path $taskRoot 'install'
$mainBinary = Join-Path $installDir 'fylura.exe'
$profileRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
$localProfileRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$currentDataDir = Join-Path $profileRoot 'com.startlan.fylura'
$currentCacheDir = Join-Path $localProfileRoot 'com.startlan.fylura'
$legacyDataDir = Join-Path $profileRoot 'com.startlan.zencanvas'
$legacyService = 'ZenCanvasGlobalIndex'
$currentService = 'FyluraGlobalIndex'
$legacyServiceKey = "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$legacyService"
$currentServiceKey = "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$currentService"
$uninstallRoot = 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
$evidence = [System.Collections.Generic.List[object]]::new()
$legacyServiceCreated = $false
$appDataFixtureCreated = $false
$secondaryUserCreated = $false
$secondaryUserSidValue = $null
$secondaryProfilePath = $null
$secondaryProfileRootCreated = $false
$secondaryUserDataSentinels = @()
$credentialTargets = [ordered]@{
    main = 'ai-api-key.com.startlan.fylura'
    alpha = 'ai-api-key-qa_alpha.com.startlan.fylura.ai-profile'
    beta = 'ai-api-key-qa_beta.com.startlan.fylura.ai-profile'
}
$script:taskOwnedCredentialTargets = [System.Collections.Generic.List[string]]::new()
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

function Test-CredentialTarget([string]$Target) {
    $output = & "$env:WINDIR\System32\cmdkey.exe" /list 2>&1
    $exitCode = $LASTEXITCODE
    Assert-Condition ($exitCode -eq 0) 'Could not inspect the current Windows Credential Manager targets.'
    foreach ($line in $output) {
        if ($line -match '^\s*Target:\s*(.+?)\s*$') {
            $candidate = $Matches[1].Trim()
            foreach ($prefix in @('LegacyGeneric:target=', 'Generic:target=')) {
                if ($candidate.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
                    $candidate = $candidate.Substring($prefix.Length)
                    break
                }
            }
            if ($candidate.Equals($Target, [StringComparison]::OrdinalIgnoreCase)) {
                return $true
            }
        }
    }
    return $false
}

function Add-TaskOwnedCredential([string]$Target, [string]$UserName) {
    $fakeSecret = "fylura-596-qa-$([guid]::NewGuid().ToString('N'))"
    $script:taskOwnedCredentialTargets.Add($Target)
    $null = & "$env:WINDIR\System32\cmdkey.exe" "/generic:$Target" "/user:$UserName" "/pass:$fakeSecret" 2>&1
    Assert-Condition ($LASTEXITCODE -eq 0) 'Could not create a task-owned Windows Credential Manager fixture.'
    Assert-Condition (Test-CredentialTarget $Target) 'Windows Credential Manager did not retain a task-owned credential fixture.'
}

function Remove-TaskOwnedCredentialFixtures {
    foreach ($target in $script:taskOwnedCredentialTargets) {
        $null = & "$env:WINDIR\System32\cmdkey.exe" "/delete:$target" 2>&1
    }
    $script:taskOwnedCredentialTargets.Clear()
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

function Assert-SecondaryProfileDataPreserved {
    foreach ($sentinel in $secondaryUserDataSentinels) {
        Assert-Condition (Test-Path -LiteralPath $sentinel.path -PathType Leaf) 'Fylura cleanup changed another Windows user profile.'
        Assert-Condition ([System.IO.File]::ReadAllText($sentinel.path) -eq $sentinel.contents) 'Fylura cleanup changed another Windows user profile.'
    }
}

try {
    Assert-Condition ($env:GITHUB_ACTIONS -eq 'true' -and $env:RUNNER_ENVIRONMENT -eq 'github-hosted') 'This destructive lifecycle verifier must run only on a disposable GitHub-hosted Windows VM.'
    Assert-Condition ([System.IO.Path]::IsPathFullyQualified($profileRoot) -and [System.IO.Path]::IsPathFullyQualified($localProfileRoot)) 'Windows returned an invalid current-user app-data path.'
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    Assert-Condition $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) 'UNVERIFIED: NSIS lifecycle validation requires the disposable runner to be elevated.'
    Assert-Condition (Test-Path -LiteralPath $installer -PathType Leaf) "NSIS installer was not found: $installer"
    Assert-Condition (-not (Test-Path -LiteralPath $taskRoot)) "Refusing pre-existing task directory: $taskRoot"
    Assert-Condition (-not (Test-Path -LiteralPath $legacyServiceKey)) "Refusing pre-existing legacy service: $legacyService"
    Assert-Condition (-not (Test-Path -LiteralPath $currentServiceKey)) "Refusing pre-existing Fylura service: $currentService"
    $existingProducts = @(Get-FyluraUninstallEntries)
    Assert-Condition ($existingProducts.Count -eq 0) 'Refusing to run against a pre-existing Fylura installation.'
    Assert-Condition (-not (Test-Path -LiteralPath $currentDataDir)) 'Refusing to alter pre-existing current-user Fylura application data.'
    Assert-Condition (-not (Test-Path -LiteralPath $currentCacheDir)) 'Refusing to alter pre-existing current-user Fylura cache or logs.'
    Assert-Condition (-not (Test-Path -LiteralPath $legacyDataDir)) 'Refusing to alter pre-existing legacy development application data.'
    foreach ($target in $credentialTargets.Values) {
        Assert-Condition (-not (Test-CredentialTarget $target)) "Refusing to overwrite a pre-existing Windows credential target: $target"
    }

    New-Item -ItemType Directory -Path $taskRoot | Out-Null
    Record-Phase 'preflight' @{
        installer = $installer
        taskRoot = $taskRoot
        installDir = $installDir
        appDataRoot = $currentDataDir
        localAppDataRoot = $currentCacheDir
        isolatedRunnerVm = $true
        administrator = $true
        existingFyluraProducts = 0
        existingLegacyService = $false
        existingFyluraService = $false
    }

    # The hosted runner VM is disposable and its current account is elevated,
    # which NSIS needs to exercise its per-machine service/registry lifecycle.
    # Create a separate standard-user profile with sentinels so full cleanup
    # also proves it leaves other Windows users' Fylura roots untouched.
    $secondaryUserName = 'FylOther_' + [guid]::NewGuid().ToString('N').Substring(0, 10)
    $secondaryUserPassword = ConvertTo-SecureString (([guid]::NewGuid().ToString('N')) + 'aA1!') -AsPlainText -Force
    New-LocalUser -Name $secondaryUserName -Password $secondaryUserPassword -PasswordNeverExpires -AccountNeverExpires `
        -Description 'Isolated Fylura other-user cleanup fixture' | Out-Null
    $secondaryUserCreated = $true
    Add-LocalGroupMember -Group 'Users' -Member $secondaryUserName
    $secondaryUserSidValue = (Get-LocalUser -Name $secondaryUserName).SID.Value
    # New-LocalUser intentionally does not initialize a profile until first
    # interactive logon. Stage the account's exact profile tree directly so
    # this test does not need a logon session or mutate Windows profile state.
    $userProfilePath = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    Assert-Condition ([System.IO.Path]::IsPathFullyQualified($userProfilePath)) 'Windows returned an invalid current-user profile path.'
    $secondaryProfilePath = Join-Path (Split-Path -Parent $userProfilePath) $secondaryUserName
    Assert-Condition (-not (Test-Path -LiteralPath $secondaryProfilePath)) 'Refusing to alter a pre-existing secondary profile path.'
    New-Item -ItemType Directory -Path $secondaryProfilePath -ErrorAction Stop | Out-Null
    $secondaryProfileRootCreated = $true
    $secondaryDataRoot = Join-Path $secondaryProfilePath 'AppData\Roaming\com.startlan.fylura'
    $secondaryCacheRoot = Join-Path $secondaryProfilePath 'AppData\Local\com.startlan.fylura'
    New-Item -ItemType Directory -Path $secondaryDataRoot, $secondaryCacheRoot -Force | Out-Null
    $secondaryUserDataSentinels = @(
        [pscustomobject]@{ path = (Join-Path $secondaryDataRoot 'other-user-data.sentinel'); contents = 'other-user-data-preserved' },
        [pscustomobject]@{ path = (Join-Path $secondaryCacheRoot 'other-user-cache.sentinel'); contents = 'other-user-cache-preserved' }
    )
    foreach ($sentinel in $secondaryUserDataSentinels) {
        [System.IO.File]::WriteAllText($sentinel.path, $sentinel.contents)
    }
    Assert-SecondaryProfileDataPreserved
    Record-Phase 'secondary-user-profile-fixture' @{
        otherUserFyluraDataAndCacheSeeded = $true
        standardUser = $true
        profileInitializedByInteractiveLogon = $false
        noCredentialValuesRecorded = $true
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

    $safeTrashSource = Join-Path $taskRoot 'safe-trash-source'
    $trashDir = Join-Path $safeTrashSource '.zen-canvas-trash\items\task-owned-fixture'
    $newSafeTrashDir = Join-Path $safeTrashSource '.fylura-trash\items\task-owned-fixture'
    $ownedSafeTrashDir = Join-Path $currentDataDir 'safe-trash\items\task-owned-fixture'
    $ownedNewDotTrashDir = Join-Path $currentDataDir '.fylura-trash\items\task-owned-fixture'
    $ownedLegacyDotTrashDir = Join-Path $currentDataDir '.zen-canvas-trash\items\task-owned-fixture'
    $cacheLogsDir = Join-Path $currentCacheDir 'logs'
    $appDataFixtureCreated = $true
    New-Item -ItemType Directory -Path $currentDataDir, $currentCacheDir, $legacyDataDir, $trashDir, $newSafeTrashDir, $ownedSafeTrashDir, $ownedNewDotTrashDir, $ownedLegacyDotTrashDir, $cacheLogsDir -Force | Out-Null
    $sentinels = [ordered]@{
        currentDatabase = Join-Path $currentDataDir 'fylura.sqlite3'
        legacyDatabase = Join-Path $legacyDataDir 'zen-canvas.sqlite3'
        safeTrash = Join-Path $trashDir 'user-data-sentinel.txt'
        newSafeTrash = Join-Path $newSafeTrashDir 'user-data-sentinel.txt'
        appSafeTrash = Join-Path $ownedSafeTrashDir 'user-data-sentinel.txt'
        appNewDotTrash = Join-Path $ownedNewDotTrashDir 'user-data-sentinel.txt'
        appLegacyDotTrash = Join-Path $ownedLegacyDotTrashDir 'user-data-sentinel.txt'
        appCache = Join-Path $cacheLogsDir 'app.log'
    }
    foreach ($entry in $sentinels.GetEnumerator()) {
        [System.IO.File]::WriteAllText($entry.Value, "task-owned $($entry.Key) sentinel")
    }
    $python = [System.IO.Path]::GetFullPath($PythonPath)
    Assert-Condition (Test-Path -LiteralPath $python -PathType Leaf) 'Python is unavailable to the isolated NSIS lifecycle worker.'
    $seedDbScript = Join-Path $taskRoot 'seed-fylura-db.py'
    @'
import json
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
connection.execute("CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
connection.execute(
    "INSERT INTO app_settings (key, value) VALUES (?, ?)",
    ("ai_settings_v1", json.dumps({"customProfiles": [{"id": "qa_alpha"}, {"id": "qa_beta"}]})),
)
connection.commit()
connection.close()
'@ | Set-Content -LiteralPath $seedDbScript -Encoding ascii
    $seedOutput = @(& $python $seedDbScript $sentinels.currentDatabase 2>&1)
    $seedExitCode = $LASTEXITCODE
    $seedError = ($seedOutput | Out-String).Trim()
    Assert-Condition ($seedExitCode -eq 0) "Could not seed the task-owned Fylura SQLite database fixture (exit $seedExitCode). $seedError"
    Add-TaskOwnedCredential $credentialTargets.main 'ai-api-key'
    Add-TaskOwnedCredential $credentialTargets.alpha 'ai-api-key-qa_alpha'
    Add-TaskOwnedCredential $credentialTargets.beta 'ai-api-key-qa_beta'
    $verifyDbScript = Join-Path $taskRoot 'verify-fylura-db.py'
    @'
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
row = connection.execute("SELECT value FROM app_settings WHERE key = ?", ("ai_settings_v1",)).fetchone()
connection.close()
if row is None or 'qa_alpha' not in row[0] or 'qa_beta' not in row[0]:
    raise SystemExit("retained Fylura settings could not be read")
'@ | Set-Content -LiteralPath $verifyDbScript -Encoding ascii

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
        if ($entry.Key -eq 'currentDatabase') { continue }
        $contents = [System.IO.File]::ReadAllText($entry.Value)
        Assert-Condition ($contents -eq "task-owned $($entry.Key) sentinel") "NSIS uninstall changed task-owned user data: $($entry.Key)."
    }
    $verifyOutput = @(& $python $verifyDbScript $sentinels.currentDatabase 2>&1)
    $verifyExitCode = $LASTEXITCODE
    $verifyError = ($verifyOutput | Out-String).Trim()
    Assert-Condition ($verifyExitCode -eq 0) "Ordinary uninstall changed the retained settings database or AI profile metadata (exit $verifyExitCode). $verifyError"
    foreach ($target in $credentialTargets.Values) {
        Assert-Condition (Test-CredentialTarget $target) "Ordinary uninstall removed task-owned Fylura credentials: $target"
    }
    Assert-SecondaryProfileDataPreserved
    Record-Phase 'uninstall' @{
        exitCode = $uninstallExitCode
        executableRemoved = $true
        fyluraServiceRemoved = $true
        uninstallRegistrationRemoved = $true
        currentDatabaseLegacyDatabaseAndSafeTrashPreserved = $true
        otherUserFyluraDataAndCachePreserved = $true
    }

    $reinstallExitCode = Invoke-Installer $installer
    Assert-Condition ($reinstallExitCode -eq 0) "Reinstall after ordinary uninstall failed with exit code $reinstallExitCode."
    $reinstallMainProcessCount = Stop-TaskOwnedMainWindowProcess
    Assert-Condition (Test-Path -LiteralPath $mainBinary -PathType Leaf) 'Reinstall after ordinary uninstall did not publish fylura.exe.'
    foreach ($entry in $sentinels.GetEnumerator()) {
        Assert-Condition (Test-Path -LiteralPath $entry.Value -PathType Leaf) "Reinstall after ordinary uninstall lost retained user data: $($entry.Key)."
    }
    $verifyOutput = @(& $python $verifyDbScript $sentinels.currentDatabase 2>&1)
    $verifyExitCode = $LASTEXITCODE
    $verifyError = ($verifyOutput | Out-String).Trim()
    Assert-Condition ($verifyExitCode -eq 0) "Reinstalled Fylura could not read the retained settings database and AI profile metadata (exit $verifyExitCode). $verifyError"
    foreach ($target in $credentialTargets.Values) {
        Assert-Condition (Test-CredentialTarget $target) "Reinstall lost task-owned Fylura credentials: $target"
    }
    Record-Phase 'reinstall-after-ordinary-uninstall' @{
        exitCode = $reinstallExitCode
        stoppedOwnedMainWindowProcesses = $reinstallMainProcessCount
        databaseReadable = $true
        userDataPreserved = $true
    }

    $uninstaller = Join-Path $installDir 'uninstall.exe'
    $fullUninstallExitCode = (Start-Process -FilePath $uninstaller -ArgumentList @('/S', '/DELETE_APP_DATA', '/CONFIRM_DELETE_APP_DATA') -PassThru -NoNewWindow -Wait).ExitCode
    Assert-Condition ($fullUninstallExitCode -eq 0) "Confirmed full NSIS uninstall failed with exit code $fullUninstallExitCode."
    for ($attempt = 0; $attempt -lt 30 -and (Test-Path -LiteralPath $mainBinary); $attempt++) {
        Start-Sleep -Milliseconds 500
    }
    Assert-Condition (-not (Test-Path -LiteralPath $mainBinary)) 'Confirmed full uninstall left the Fylura executable installed.'
    Assert-Condition (-not (Test-Path -LiteralPath $currentServiceKey)) 'Confirmed full uninstall left FyluraGlobalIndex registered.'
    Assert-Condition (-not (Test-Path -LiteralPath $sentinels.currentDatabase)) 'Confirmed full uninstall left the Fylura database.'
    Assert-Condition (-not (Test-Path -LiteralPath $currentCacheDir)) 'Confirmed full uninstall left Fylura cache and logs.'
    foreach ($target in $credentialTargets.Values) {
        Assert-Condition (-not (Test-CredentialTarget $target)) "Confirmed full uninstall left a Fylura credential target: $target"
    }
    foreach ($name in @('safe-trash', '.fylura-trash', '.zen-canvas-trash')) {
        $remainingRoot = Join-Path $currentDataDir $name
        Assert-Condition (Test-Path -LiteralPath $remainingRoot -PathType Container) "Confirmed full uninstall removed Safe Trash root: $name."
    }
    foreach ($entry in $sentinels.GetEnumerator()) {
        if ($entry.Key -in @('currentDatabase', 'appCache')) { continue }
        Assert-Condition (Test-Path -LiteralPath $entry.Value -PathType Leaf) "Confirmed full uninstall deleted user or legacy data: $($entry.Key)."
        $contents = [System.IO.File]::ReadAllText($entry.Value)
        Assert-Condition ($contents -eq "task-owned $($entry.Key) sentinel") "Confirmed full uninstall changed user or legacy data: $($entry.Key)."
    }
    Assert-SecondaryProfileDataPreserved
    Assert-Condition (@(Get-ChildItem -LiteralPath $currentDataDir -Force | Where-Object { $_.Name -notin @('safe-trash', '.fylura-trash', '.zen-canvas-trash') }).Count -eq 0) 'Confirmed full uninstall left unexpected Fylura application data.'
    Record-Phase 'confirmed-full-uninstall' @{
        exitCode = $fullUninstallExitCode
        executableRemoved = $true
        currentDatabaseAndCacheRemoved = $true
        mainAndMultipleProfileCredentialsRemoved = $true
        profileIdsReadBeforeDatabaseRemoval = @('qa_alpha', 'qa_beta')
        currentAndLegacySafeTrashPreserved = $true
        legacyDevelopmentDataPreserved = $true
        otherUserFyluraDataAndCachePreserved = $true
        unrelatedDirectoriesUntouched = $true
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
    Remove-TaskOwnedCredentialFixtures
    Remove-TaskOwnedLegacyService
    if ($secondaryUserSidValue) {
        $secondaryProfile = Get-CimInstance -ClassName Win32_UserProfile -ErrorAction SilentlyContinue |
            Where-Object { $_.SID -eq $secondaryUserSidValue } | Select-Object -First 1
        if ($secondaryProfile -and -not $secondaryProfile.Loaded) {
            $secondaryProfile | Remove-CimInstance -ErrorAction SilentlyContinue
        }
    }
    if ($secondaryUserCreated) {
        Remove-LocalUser -Name $secondaryUserName -ErrorAction SilentlyContinue
    }
    if ($secondaryProfileRootCreated -and (Test-Path -LiteralPath $secondaryProfilePath)) {
        Remove-Item -LiteralPath $secondaryProfilePath -Recurse -Force -ErrorAction Stop
        if (Test-Path -LiteralPath $secondaryProfilePath) {
            throw 'The task-owned secondary profile fixture remained after cleanup.'
        }
    }
    if ($appDataFixtureCreated) {
        Remove-Item -LiteralPath $currentDataDir, $currentCacheDir, $legacyDataDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath $taskRoot) {
        Remove-Item -LiteralPath $taskRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
