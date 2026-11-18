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
$script:secondaryBatchLogonRightGranted = $false
$script:secondaryBatchLogonRightCleanupFailure = $null
$credentialTargets = [ordered]@{
    main = 'ai-api-key.com.startlan.fylura'
    alpha = 'ai-api-key-qa_alpha.com.startlan.fylura.ai-profile'
    beta = 'ai-api-key-qa_beta.com.startlan.fylura.ai-profile'
    orphan = 'ai-api-key-qa_orphan.com.startlan.fylura.ai-profile'
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

function Set-TaskOwnedBatchLogonRight([string]$Sid, [bool]$Grant) {
    if (-not ('Fylura596BatchLogonRight' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class Fylura596BatchLogonRight
{
    [StructLayout(LayoutKind.Sequential)]
    private struct LSA_UNICODE_STRING
    {
        public ushort Length;
        public ushort MaximumLength;
        public IntPtr Buffer;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct LSA_OBJECT_ATTRIBUTES
    {
        public int Length;
        public IntPtr RootDirectory;
        public IntPtr ObjectName;
        public uint Attributes;
        public IntPtr SecurityDescriptor;
        public IntPtr SecurityQualityOfService;
    }

    [DllImport("advapi32.dll")]
    private static extern uint LsaOpenPolicy(IntPtr SystemName, ref LSA_OBJECT_ATTRIBUTES ObjectAttributes, uint DesiredAccess, out IntPtr PolicyHandle);

    [DllImport("advapi32.dll")]
    private static extern uint LsaAddAccountRights(IntPtr PolicyHandle, IntPtr AccountSid, LSA_UNICODE_STRING[] UserRights, uint CountOfRights);

    [DllImport("advapi32.dll")]
    private static extern uint LsaRemoveAccountRights(IntPtr PolicyHandle, IntPtr AccountSid, byte AllRights, LSA_UNICODE_STRING[] UserRights, uint CountOfRights);

    [DllImport("advapi32.dll")]
    private static extern uint LsaClose(IntPtr PolicyHandle);

    [DllImport("advapi32.dll")]
    private static extern uint LsaNtStatusToWinError(uint Status);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true, EntryPoint = "ConvertStringSidToSidW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ConvertStringSidToSid(string StringSid, out IntPtr Sid);

    [DllImport("kernel32.dll")]
    private static extern IntPtr LocalFree(IntPtr Memory);

    public static void Set(string sidText, bool grant)
    {
        const uint PolicyLookupNames = 0x00000800;
        const uint PolicyCreateAccount = 0x00000010;
        IntPtr policy = IntPtr.Zero;
        IntPtr sid = IntPtr.Zero;
        IntPtr rightBuffer = IntPtr.Zero;
        try
        {
            var attributes = new LSA_OBJECT_ATTRIBUTES { Length = Marshal.SizeOf(typeof(LSA_OBJECT_ATTRIBUTES)) };
            var status = LsaOpenPolicy(IntPtr.Zero, ref attributes, PolicyLookupNames | PolicyCreateAccount, out policy);
            if (status != 0) Throw(status, "Could not open the local security policy for the isolated test account.");
            if (!ConvertStringSidToSid(sidText, out sid))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Could not resolve the isolated test account SID.");

            const string right = "SeBatchLogonRight";
            rightBuffer = Marshal.StringToHGlobalUni(right);
            var rights = new[] { new LSA_UNICODE_STRING { Length = (ushort)(right.Length * 2), MaximumLength = (ushort)((right.Length + 1) * 2), Buffer = rightBuffer } };
            status = grant
                ? LsaAddAccountRights(policy, sid, rights, 1)
                : LsaRemoveAccountRights(policy, sid, 0, rights, 1);
            if (status != 0) Throw(status, grant
                ? "Could not grant the isolated test account the scheduled-task logon right."
                : "Could not revoke the isolated test account's scheduled-task logon right.");
        }
        finally
        {
            if (rightBuffer != IntPtr.Zero) Marshal.FreeHGlobal(rightBuffer);
            if (sid != IntPtr.Zero) LocalFree(sid);
            if (policy != IntPtr.Zero) LsaClose(policy);
        }
    }

    private static void Throw(uint status, string message)
    {
        throw new Win32Exception((int)LsaNtStatusToWinError(status), message);
    }
}
'@ -ErrorAction Stop | Out-Null
    }
    [Fylura596BatchLogonRight]::Set($Sid, $Grant)
}

$script:runKeyPath = 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run'
$script:startupApprovedKeyPath = 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$script:foreignAutostartName = 'Fylura596ForeignFixture'
$script:foreignAutostartCommand = 'C:\Fylura596-test-owned\foreign-fixture.exe --keep'
$script:foreignAutostartApproval = [byte[]](0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
$script:currentAutostartSeeded = $false

function Set-TaskOwnedAutostart([string]$MainBinary) {
    New-Item -Path $script:runKeyPath -Force | Out-Null
    New-Item -Path $script:startupApprovedKeyPath -Force | Out-Null
    New-ItemProperty -LiteralPath $script:runKeyPath -Name 'Fylura' -PropertyType String `
        -Value "$MainBinary --background" -Force | Out-Null
    New-ItemProperty -LiteralPath $script:startupApprovedKeyPath -Name 'Fylura' -PropertyType Binary `
        -Value ([byte[]](0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)) -Force | Out-Null
    New-ItemProperty -LiteralPath $script:runKeyPath -Name $script:foreignAutostartName -PropertyType String `
        -Value $script:foreignAutostartCommand -Force | Out-Null
    New-ItemProperty -LiteralPath $script:startupApprovedKeyPath -Name $script:foreignAutostartName -PropertyType Binary `
        -Value $script:foreignAutostartApproval -Force | Out-Null
}

function Assert-TaskOwnedAutostartPresent([string]$MainBinary) {
    $run = Get-ItemProperty -LiteralPath $script:runKeyPath
    $approved = Get-ItemProperty -LiteralPath $script:startupApprovedKeyPath
    Assert-Condition ($run.Fylura -eq "$MainBinary --background") 'The current-user Fylura autostart command changed unexpectedly.'
    Assert-Condition ([Convert]::ToBase64String($approved.Fylura) -eq [Convert]::ToBase64String([byte[]](0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))) 'The current-user Fylura Startup Apps value changed unexpectedly.'
    Assert-Condition ($run.($script:foreignAutostartName) -eq $script:foreignAutostartCommand) 'An unrelated current-user startup command changed unexpectedly.'
    Assert-Condition ([Convert]::ToBase64String($approved.($script:foreignAutostartName)) -eq [Convert]::ToBase64String($script:foreignAutostartApproval)) 'An unrelated Startup Apps value changed unexpectedly.'
}

function Assert-TaskOwnedAutostartRemoved {
    $run = Get-ItemProperty -LiteralPath $script:runKeyPath
    $approved = Get-ItemProperty -LiteralPath $script:startupApprovedKeyPath
    Assert-Condition ($null -eq $run.PSObject.Properties['Fylura']) 'The current-user Fylura Run value remained after uninstall.'
    Assert-Condition ($null -eq $approved.PSObject.Properties['Fylura']) 'The current-user Fylura Startup Apps value remained after uninstall.'
    Assert-Condition ($run.($script:foreignAutostartName) -eq $script:foreignAutostartCommand) 'Uninstall changed an unrelated current-user startup command.'
    Assert-Condition ([Convert]::ToBase64String($approved.($script:foreignAutostartName)) -eq [Convert]::ToBase64String($script:foreignAutostartApproval)) 'Uninstall changed an unrelated Startup Apps value.'
}

function Remove-TaskOwnedAutostartFixture {
    if (-not $script:currentAutostartSeeded) { return }
    foreach ($item in @(
        @{ Path = $script:runKeyPath; Name = 'Fylura'; Expected = "$mainBinary --background" },
        @{ Path = $script:runKeyPath; Name = $script:foreignAutostartName; Expected = $script:foreignAutostartCommand }
    )) {
        $properties = Get-ItemProperty -LiteralPath $item.Path -ErrorAction SilentlyContinue
        $value = if ($null -ne $properties) { $properties.PSObject.Properties[$item.Name] } else { $null }
        if ($null -ne $value -and [string]$value.Value -eq [string]$item.Expected) {
            Remove-ItemProperty -LiteralPath $item.Path -Name $item.Name -ErrorAction SilentlyContinue
        }
    }
    foreach ($item in @(
        @{ Name = 'Fylura'; Expected = [Convert]::ToBase64String([byte[]](0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)) },
        @{ Name = $script:foreignAutostartName; Expected = [Convert]::ToBase64String($script:foreignAutostartApproval) }
    )) {
        $properties = Get-ItemProperty -LiteralPath $script:startupApprovedKeyPath -ErrorAction SilentlyContinue
        $value = if ($null -ne $properties) { $properties.PSObject.Properties[$item.Name] } else { $null }
        if ($null -ne $value -and $value.Value -is [byte[]] -and [Convert]::ToBase64String($value.Value) -eq $item.Expected) {
            Remove-ItemProperty -LiteralPath $script:startupApprovedKeyPath -Name $item.Name -ErrorAction SilentlyContinue
        }
    }
    $script:currentAutostartSeeded = $false
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

function Invoke-SecondaryUserContext([string]$Mode) {
    if (Test-Path -LiteralPath $secondaryContextResultPath) {
        Remove-Item -LiteralPath $secondaryContextResultPath -Force -ErrorAction Stop
    }
    $account = "$env:COMPUTERNAME\$secondaryUserName"
    $taskName = "FyluraSecondaryContext_$([guid]::NewGuid().ToString('N'))"
    $powershell = (Get-Process -Id $PID).Path
    $arguments = "-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$secondaryContextScriptPath`" -Mode $Mode -ExpectedSid $secondaryUserSidValue -MainBinary `"$mainBinary`" -StatePath `"$secondaryContextStatePath`" -ResultPath `"$secondaryContextResultPath`""
    $action = New-ScheduledTaskAction -Execute $powershell -Argument $arguments -WorkingDirectory $taskRoot
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 5)
    $passwordText = $secondaryUserPassword | ConvertFrom-SecureString -AsPlainText
    $taskRegistered = $false
    try {
        Register-ScheduledTask -TaskName $taskName -Action $action -Settings $settings `
            -User $account -Password $passwordText -RunLevel Limited -Force | Out-Null
        $taskRegistered = $true
        $startedAt = Get-Date
        Start-ScheduledTask -TaskName $taskName
        $taskInfo = $null
        for ($attempt = 0; $attempt -lt 360; $attempt++) {
            $task = Get-ScheduledTask -TaskName $taskName -ErrorAction Stop
            $taskInfo = Get-ScheduledTaskInfo -TaskName $taskName -ErrorAction Stop
            if ($task.State -ne 'Running' -and $taskInfo.LastRunTime -gt $startedAt.AddSeconds(-1)) {
                break
            }
            Start-Sleep -Milliseconds 500
        }
        if (-not $taskInfo -or $taskInfo.LastRunTime -le $startedAt.AddSeconds(-1)) {
            $lastResult = if ($taskInfo) { '0x{0:X8}' -f [uint32]$taskInfo.LastTaskResult } else { 'unavailable' }
            $lastRun = if ($taskInfo) { $taskInfo.LastRunTime.ToString('o') } else { 'unavailable' }
            $taskState = if ($task) { [string]$task.State } else { 'unavailable' }
            $logonType = if ($task -and $task.Principal) { [string]$task.Principal.LogonType } else { 'unavailable' }
            throw "The isolated standard-user $Mode task did not start (state=$taskState; logonType=$logonType; lastTaskResult=$lastResult; lastRunTime=$lastRun)."
        }
        if ($taskInfo.LastTaskResult -ne 0) {
            $failureDetail = ''
            if (Test-Path -LiteralPath $secondaryContextResultPath -PathType Leaf) {
                try {
                    $contextResult = Get-Content -LiteralPath $secondaryContextResultPath -Raw | ConvertFrom-Json
                    $failureDetail = [string]$contextResult.error
                } catch { }
            }
            $failureDetail = ($failureDetail -replace '\s+', ' ').Trim()
            if ($failureDetail.Length -gt 500) { $failureDetail = $failureDetail.Substring(0, 500) }
            throw "The isolated standard-user $Mode task failed with result $($taskInfo.LastTaskResult). $failureDetail"
        }
        Assert-Condition (Test-Path -LiteralPath $secondaryContextResultPath -PathType Leaf) "The isolated standard-user $Mode action did not write evidence."
        $result = Get-Content -LiteralPath $secondaryContextResultPath -Raw | ConvertFrom-Json
        Assert-Condition ($result.status -eq 'PASS') "The isolated standard-user $Mode action did not report PASS."
        return $result
    }
    finally {
        if ($taskRegistered) {
            $task = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
            if ($task -and $task.State -eq 'Running') {
                Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
            }
            Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction Stop
            Assert-Condition (-not (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) 'The isolated standard-user context task remained after cleanup.'
        }
    }
}

function Invoke-CurrentUserDataCleanup([string]$MainBinary) {
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $helperRoot = Join-Path $localProfileRoot 'Temp'
    New-Item -ItemType Directory -Path $helperRoot -Force | Out-Null
    $helper = Join-Path $helperRoot "FyluraUninstall-$([guid]::NewGuid().ToString('N')).exe"
    Copy-Item -LiteralPath $MainBinary -Destination $helper -ErrorAction Stop
    $process = Start-Process -FilePath $helper `
        -ArgumentList @('--fylura-uninstall-worker-cleanup', $sid, $MainBinary, '/S') -PassThru
    if (-not $process.WaitForExit(120000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw 'Timed out waiting for the current-user cleanup worker.'
    }
    $process.Refresh()
    Assert-Condition ($process.ExitCode -eq 0) "The current-user cleanup worker returned $($process.ExitCode)."
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
    $existingRunValues = Get-ItemProperty -LiteralPath $script:runKeyPath -ErrorAction SilentlyContinue
    $existingApprovalValues = Get-ItemProperty -LiteralPath $script:startupApprovedKeyPath -ErrorAction SilentlyContinue
    Assert-Condition ($null -eq $existingRunValues -or $null -eq $existingRunValues.PSObject.Properties['Fylura']) 'Refusing to overwrite a pre-existing Fylura startup entry.'
    Assert-Condition ($null -eq $existingRunValues -or $null -eq $existingRunValues.PSObject.Properties[$script:foreignAutostartName]) 'Refusing to overwrite the foreign startup fixture name.'
    Assert-Condition ($null -eq $existingApprovalValues -or $null -eq $existingApprovalValues.PSObject.Properties['Fylura']) 'Refusing to overwrite a pre-existing Fylura Startup Apps entry.'
    Assert-Condition ($null -eq $existingApprovalValues -or $null -eq $existingApprovalValues.PSObject.Properties[$script:foreignAutostartName]) 'Refusing to overwrite the foreign Startup Apps fixture name.'
    foreach ($target in $credentialTargets.Values) {
        Assert-Condition (-not (Test-CredentialTarget $target)) "Refusing to overwrite a pre-existing Windows credential target: $target"
    }

    New-Item -ItemType Directory -Path $taskRoot | Out-Null
    $script:currentAutostartSeeded = $true
    Set-TaskOwnedAutostart $mainBinary
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
    Set-TaskOwnedBatchLogonRight $secondaryUserSidValue $true
    $script:secondaryBatchLogonRightGranted = $true
    $secondaryAccount = "$env:COMPUTERNAME\$secondaryUserName"
    $taskAcl = Get-Acl -LiteralPath $taskRoot
    $secondaryAccess = [System.Security.AccessControl.FileSystemAccessRule]::new(
        $secondaryAccount,
        [System.Security.AccessControl.FileSystemRights]::Modify,
        [System.Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [System.Security.AccessControl.InheritanceFlags]::ObjectInherit,
        [System.Security.AccessControl.PropagationFlags]::None,
        [System.Security.AccessControl.AccessControlType]::Allow
    )
    $taskAcl.AddAccessRule($secondaryAccess)
    Set-Acl -LiteralPath $taskRoot -AclObject $taskAcl
    New-Item -ItemType Directory -Path $installDir | Out-Null
    $installAcl = Get-Acl -LiteralPath $installDir
    $secondaryReadAccess = [System.Security.AccessControl.FileSystemAccessRule]::new(
        $secondaryAccount,
        [System.Security.AccessControl.FileSystemRights]::ReadAndExecute,
        [System.Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [System.Security.AccessControl.InheritanceFlags]::ObjectInherit,
        [System.Security.AccessControl.PropagationFlags]::None,
        [System.Security.AccessControl.AccessControlType]::Allow
    )
    $installAcl.AddAccessRule($secondaryReadAccess)
    Set-Acl -LiteralPath $installDir -AclObject $installAcl
    $secondaryContextScriptPath = Join-Path $taskRoot 'secondary-user-context.ps1'
    $secondaryContextStatePath = Join-Path $taskRoot 'secondary-user-context-state.json'
    $secondaryContextResultPath = Join-Path $taskRoot 'secondary-user-context-result.json'
    @'
param(
    [Parameter(Mandatory = $true)][ValidateSet('seed', 'verify', 'verify-autostart-removed', 'cleanup')][string]$Mode,
    [Parameter(Mandatory = $true)][string]$ExpectedSid,
    [Parameter(Mandatory = $true)][string]$MainBinary,
    [Parameter(Mandatory = $true)][string]$StatePath,
    [Parameter(Mandatory = $true)][string]$ResultPath
)

$ErrorActionPreference = 'Stop'

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Test-CredentialTarget([string]$Target) {
    $output = & "$env:WINDIR\System32\cmdkey.exe" /list 2>&1
    Assert-Condition ($LASTEXITCODE -eq 0) 'Could not inspect the isolated user credential store.'
    foreach ($line in $output) {
        if ($line -match '^\s*Target:\s*(.+?)\s*$') {
            $candidate = $Matches[1].Trim()
            foreach ($prefix in @('LegacyGeneric:target=', 'Generic:target=')) {
                if ($candidate.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
                    $candidate = $candidate.Substring($prefix.Length)
                    break
                }
            }
            if ($candidate.Equals($Target, [StringComparison]::OrdinalIgnoreCase)) { return $true }
        }
    }
    return $false
}

function Add-FakeCredential([string]$Target, [string]$UserName) {
    $fakeSecret = "fylura-cross-account-qa-$([guid]::NewGuid().ToString('N'))"
    $null = & "$env:WINDIR\System32\cmdkey.exe" "/generic:$Target" "/user:$UserName" "/pass:$fakeSecret" 2>&1
    Assert-Condition ($LASTEXITCODE -eq 0 -and (Test-CredentialTarget $Target)) 'Could not seed an isolated-user credential fixture.'
}

function Remove-FakeCredential([string]$Target) {
    $null = & "$env:WINDIR\System32\cmdkey.exe" "/delete:$Target" 2>&1
    Assert-Condition (-not (Test-CredentialTarget $Target)) 'Could not remove the isolated-user foreign credential fixture.'
}

function Write-Result([hashtable]$Value) {
    $Value | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $ResultPath -Encoding utf8
}

function Set-UserAutostart([string]$Binary) {
    $runPath = 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run'
    $approvedPath = 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
    New-Item -Path $runPath, $approvedPath -Force | Out-Null
    New-ItemProperty -LiteralPath $runPath -Name 'Fylura' -PropertyType String -Value "$Binary --background" -Force | Out-Null
    New-ItemProperty -LiteralPath $approvedPath -Name 'Fylura' -PropertyType Binary `
        -Value ([byte[]](0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)) -Force | Out-Null
    New-ItemProperty -LiteralPath $runPath -Name 'Fylura596ForeignFixture' -PropertyType String `
        -Value 'C:\Fylura596-test-owned\foreign-fixture.exe --keep' -Force | Out-Null
    New-ItemProperty -LiteralPath $approvedPath -Name 'Fylura596ForeignFixture' -PropertyType Binary `
        -Value ([byte[]](0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)) -Force | Out-Null
}

function Assert-UserAutostartPresent([string]$Binary) {
    $run = Get-ItemProperty -LiteralPath 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run'
    $approved = Get-ItemProperty -LiteralPath 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
    Assert-Condition ($run.Fylura -eq "$Binary --background") 'The isolated-user Fylura Run value changed unexpectedly.'
    Assert-Condition ([Convert]::ToBase64String($approved.Fylura) -eq [Convert]::ToBase64String([byte[]](0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))) 'The isolated-user Fylura Startup Apps value changed unexpectedly.'
    Assert-Condition ($run.Fylura596ForeignFixture -eq 'C:\Fylura596-test-owned\foreign-fixture.exe --keep') 'The isolated-user unrelated Run value changed unexpectedly.'
    Assert-Condition ([Convert]::ToBase64String($approved.Fylura596ForeignFixture) -eq [Convert]::ToBase64String([byte[]](0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))) 'The isolated-user unrelated Startup Apps value changed unexpectedly.'
}

function Assert-UserAutostartRemoved {
    $run = Get-ItemProperty -LiteralPath 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run'
    $approved = Get-ItemProperty -LiteralPath 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
    Assert-Condition ($null -eq $run.PSObject.Properties['Fylura']) 'The isolated-user Fylura Run value remained after cleanup.'
    Assert-Condition ($null -eq $approved.PSObject.Properties['Fylura']) 'The isolated-user Fylura Startup Apps value remained after cleanup.'
    Assert-Condition ($run.Fylura596ForeignFixture -eq 'C:\Fylura596-test-owned\foreign-fixture.exe --keep') 'Cleanup changed the isolated-user unrelated Run value.'
    Assert-Condition ([Convert]::ToBase64String($approved.Fylura596ForeignFixture) -eq [Convert]::ToBase64String([byte[]](0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))) 'Cleanup changed the isolated-user unrelated Startup Apps value.'
}

try {
    $actualSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    Assert-Condition ($actualSid -eq $ExpectedSid) 'The isolated process did not run as the authorized target user.'
    $roamingRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
    $localRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    $documentsRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::MyDocuments)
    $dataRoot = Join-Path $roamingRoot 'com.startlan.fylura'
    $cacheRoot = Join-Path $localRoot 'com.startlan.fylura'
    $mainTarget = 'ai-api-key.com.startlan.fylura'
    $profileTarget = 'ai-api-key-qa_secondary_orphan.com.startlan.fylura.ai-profile'
    $foreignTarget = 'fylura-cleanup-fixture.com.example.editor'
    $state = $null

    if ($Mode -eq 'seed') {
        $database = Join-Path $dataRoot 'fylura.sqlite3'
        $cacheLog = Join-Path $cacheRoot 'logs\app.log'
        $document = Join-Path $documentsRoot 'fylura-cross-account-user-document.sentinel'
        $trashNames = @('safe-trash', '.fylura-trash', '.zen-canvas-trash')
        $trashSentinels = @()
        foreach ($name in $trashNames) {
            $sentinel = Join-Path (Join-Path $dataRoot $name) 'items\real-user-file.sentinel'
            New-Item -ItemType Directory -Path (Split-Path -Parent $sentinel) -Force | Out-Null
            [System.IO.File]::WriteAllText($sentinel, "real file in $name")
            $trashSentinels += [pscustomobject]@{ path = $sentinel; contents = "real file in $name" }
        }
        New-Item -ItemType Directory -Path $cacheRoot, $documentsRoot -Force | Out-Null
        [System.IO.File]::WriteAllText($database, 'corrupt database fixture; must not hide orphan credentials')
        New-Item -ItemType Directory -Path (Split-Path -Parent $cacheLog) -Force | Out-Null
        [System.IO.File]::WriteAllText($cacheLog, 'secondary user cache fixture')
        [System.IO.File]::WriteAllText($document, 'secondary user document fixture')
        Add-FakeCredential $mainTarget 'ai-api-key'
        Add-FakeCredential $profileTarget 'ai-api-key-qa_secondary_orphan'
        Add-FakeCredential $foreignTarget 'foreign-editor-account'
        Set-UserAutostart $MainBinary
        $helperRoot = Join-Path $localRoot 'Temp'
        New-Item -ItemType Directory -Path $helperRoot -Force | Out-Null
        $helper = Join-Path $helperRoot "FyluraUninstall-$([guid]::NewGuid().ToString('N')).exe"
        Copy-Item -LiteralPath $MainBinary -Destination $helper -ErrorAction Stop
        $state = [ordered]@{
            dataRoot = $dataRoot
            cacheRoot = $cacheRoot
            database = $database
            cacheLog = $cacheLog
            document = $document
            helper = $helper
            mainTarget = $mainTarget
            profileTarget = $profileTarget
            foreignTarget = $foreignTarget
            autostartCommand = "$MainBinary --background"
            trashSentinels = $trashSentinels
        }
        $state | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $StatePath -Encoding utf8
        $userSentinels = @($trashSentinels) + @([pscustomobject]@{ path = $document; contents = 'secondary user document fixture' })
        Write-Result @{ status = 'PASS'; identityVerified = $true; stateSeeded = $true; userSentinels = $userSentinels }
    }
    elseif ($Mode -eq 'verify') {
        $state = Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json
        Assert-Condition (Test-Path -LiteralPath $state.database -PathType Leaf) 'The standard-user Fylura database changed unexpectedly.'
        Assert-Condition (Test-Path -LiteralPath $state.cacheLog -PathType Leaf) 'The standard-user Fylura cache changed unexpectedly.'
        Assert-Condition ([System.IO.File]::ReadAllText($state.document) -eq 'secondary user document fixture') 'The standard-user document changed unexpectedly.'
        foreach ($sentinel in $state.trashSentinels) {
            Assert-Condition (Test-Path -LiteralPath $sentinel.path -PathType Leaf) 'A standard-user Safe Trash file changed unexpectedly.'
            Assert-Condition ([System.IO.File]::ReadAllText($sentinel.path) -eq $sentinel.contents) 'A standard-user Safe Trash file contents changed unexpectedly.'
        }
        foreach ($target in @($state.mainTarget, $state.profileTarget, $state.foreignTarget)) {
            Assert-Condition (Test-CredentialTarget $target) 'The standard-user credential set changed unexpectedly.'
        }
        Assert-UserAutostartPresent $MainBinary
        Write-Result @{ status = 'PASS'; identityVerified = $true; dataAndCredentialsPreserved = $true }
    }
    elseif ($Mode -eq 'verify-autostart-removed') {
        $state = Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json
        Assert-UserAutostartRemoved
        Write-Result @{ status = 'PASS'; identityVerified = $true; ownAutostartRemoved = $true; unrelatedStartupPreserved = $true }
    }
    else {
        $state = Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json
        $helper = [string]$state.helper
        $process = Start-Process -FilePath $helper -ArgumentList @('--fylura-uninstall-worker-cleanup', $ExpectedSid, $MainBinary, '/S') -PassThru -Wait
        Assert-Condition ($process.ExitCode -eq 0) "The current-user cleanup worker returned $($process.ExitCode)."
        Assert-Condition (-not (Test-Path -LiteralPath $state.database)) "The authorized user's Fylura database remained."
        Assert-Condition (-not (Test-Path -LiteralPath $state.cacheRoot)) "The authorized user's Fylura cache remained."
        Assert-Condition ((Test-Path -LiteralPath $state.dataRoot -PathType Container)) 'Safe Trash preservation did not retain the application data root.'
        foreach ($sentinel in $state.trashSentinels) {
            Assert-Condition (Test-Path -LiteralPath $sentinel.path -PathType Leaf) 'The cleanup worker removed a Safe Trash file.'
            Assert-Condition ([System.IO.File]::ReadAllText($sentinel.path) -eq $sentinel.contents) 'The cleanup worker changed a Safe Trash file.'
        }
        Assert-Condition ([System.IO.File]::ReadAllText($state.document) -eq 'secondary user document fixture') "The cleanup worker changed the user's document."
        Assert-Condition (-not (Test-CredentialTarget $state.mainTarget)) "The user's Fylura main credential remained."
        Assert-Condition (-not (Test-CredentialTarget $state.profileTarget)) "The user's orphan Fylura profile credential remained."
        Assert-Condition (Test-CredentialTarget $state.foreignTarget) 'The cleanup worker removed an unrelated credential.'
        Assert-UserAutostartRemoved
        Remove-FakeCredential $state.foreignTarget
        Write-Result @{ status = 'PASS'; identityVerified = $true; dataAndCredentialsRemoved = $true; safeTrashAndDocumentPreserved = $true; unrelatedCredentialPreserved = $true; ownAutostartRemoved = $true; unrelatedStartupPreserved = $true }
    }
}
catch {
    Write-Result @{ status = 'FAIL'; error = $_.Exception.Message }
    exit 1
}
'@ | Set-Content -LiteralPath $secondaryContextScriptPath -Encoding utf8
    $secondaryProfileRootCreated = $false
    Record-Phase 'secondary-user-account-created' @{
        standardUserCreated = $true
        standardUser = $true
        profileInitialized = $false
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
    $mainBinaryAcl = Get-Acl -LiteralPath $mainBinary
    $mainBinaryReadAccess = [System.Security.AccessControl.FileSystemAccessRule]::new(
        $secondaryAccount,
        [System.Security.AccessControl.FileSystemRights]::ReadAndExecute,
        [System.Security.AccessControl.AccessControlType]::Allow
    )
    $mainBinaryAcl.AddAccessRule($mainBinaryReadAccess)
    Set-Acl -LiteralPath $mainBinary -AclObject $mainBinaryAcl
    $freshEntries = @(Get-FyluraUninstallEntries)
    Assert-Condition ($freshEntries.Count -eq 1) "Fresh install created $($freshEntries.Count) Fylura uninstall entries; expected exactly one."
    $registrationPath = "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$($freshEntries[0].key)"
    $registeredUninstall = (Get-ItemProperty -LiteralPath $registrationPath -Name UninstallString).UninstallString
    $expectedUninstall = '"' + $mainBinary + '" --fylura-uninstall-launcher'
    Assert-Condition ($registeredUninstall -eq $expectedUninstall) 'The machine uninstall entry does not use the per-user Fylura launcher.'
    Record-Phase 'fresh-install' @{
        exitCode = $freshExitCode
        mainBinary = $mainBinary
        uninstaller = $uninstaller
        stoppedOwnedMainWindowProcesses = $freshMainProcessCount
        accountScopedUninstallLauncher = $true
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
        if ($entry.Key -eq 'currentDatabase') { continue }
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
    Add-TaskOwnedCredential $credentialTargets.orphan 'ai-api-key-qa_orphan'
    $secondarySeed = Invoke-SecondaryUserContext 'seed'
    $secondaryProfileRecord = Get-CimInstance -ClassName Win32_UserProfile -ErrorAction Stop |
        Where-Object { $_.SID -eq $secondaryUserSidValue } | Select-Object -First 1
    Assert-Condition ($secondaryProfileRecord -and $secondaryProfileRecord.LocalPath) 'The isolated standard user did not receive a Windows profile.'
    $secondaryProfilePath = $secondaryProfileRecord.LocalPath
    $secondaryProfileRootCreated = $true
    $secondaryUserDataSentinels = @($secondarySeed.userSentinels)
    Assert-SecondaryProfileDataPreserved
    Record-Phase 'secondary-user-profile-fixture' @{
        otherUserFyluraDataCacheAndOrphanCredentialsSeeded = $true
        standardUserContextTaskCompleted = $true
        fakeCredentialValuesRecorded = $false
    }
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

    $secondaryState = Get-Content -LiteralPath $secondaryContextStatePath -Raw | ConvertFrom-Json
    $wrongTargetHelperRoot = Join-Path $localProfileRoot 'Temp'
    New-Item -ItemType Directory -Path $wrongTargetHelperRoot -Force | Out-Null
    $wrongTargetHelper = Join-Path $wrongTargetHelperRoot "FyluraUninstall-$([guid]::NewGuid().ToString('N')).exe"
    Copy-Item -LiteralPath $mainBinary -Destination $wrongTargetHelper -ErrorAction Stop
    $wrongTargetExitCode = (Start-Process -FilePath $wrongTargetHelper `
        -ArgumentList @('--fylura-uninstall-worker-cleanup', $secondaryUserSidValue, $mainBinary, '/S') `
        -PassThru -NoNewWindow -Wait).ExitCode
    Assert-Condition ($wrongTargetExitCode -ne 0) "A helper running under the administrator account accepted another user's SID."
    Assert-Condition (Test-Path -LiteralPath $sentinels.currentDatabase -PathType Leaf) 'A rejected cross-account request changed the administrator database.'
    Assert-Condition (Test-Path -LiteralPath $currentCacheDir -PathType Container) 'A rejected cross-account request changed the administrator cache.'
    foreach ($target in $credentialTargets.Values) {
        Assert-Condition (Test-CredentialTarget $target) "A rejected cross-account request changed administrator credentials: $target"
    }
    Assert-TaskOwnedAutostartPresent $mainBinary
    Assert-Condition (Test-Path -LiteralPath $secondaryState.database -PathType Leaf) 'A rejected cross-account request changed the standard-user database.'
    Assert-Condition (Test-Path -LiteralPath $secondaryState.cacheLog -PathType Leaf) 'A rejected cross-account request changed the standard-user cache.'
    $secondaryVerify = Invoke-SecondaryUserContext 'verify'
    Assert-Condition $secondaryVerify.dataAndCredentialsPreserved 'The rejected cross-account request changed standard-user credentials.'
    Assert-SecondaryProfileDataPreserved
    Remove-Item -LiteralPath $wrongTargetHelper -Force -ErrorAction Stop
    Record-Phase 'cross-account-cleanup-target-rejected' @{
        administratorAttemptedAnotherUsersSid = $true
        cleanupRejected = $true
        administratorDataAndCredentialsPreserved = $true
        standardUserDataPreserved = $true
        noCredentialValuesRecorded = $true
    }

    $uninstallExitCode = (Start-Process -FilePath $mainBinary -ArgumentList @('--fylura-uninstall-launcher', '/S') -PassThru -NoNewWindow -Wait).ExitCode
    Assert-Condition ($uninstallExitCode -eq 0) "Ordinary Fylura uninstall launcher failed with exit code $uninstallExitCode."
    for ($attempt = 0; $attempt -lt 120 -and (Test-Path -LiteralPath $mainBinary); $attempt++) {
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
    # This task runs elevated so the isolated VM can exercise machine NSIS
    # lifecycle. The production launcher therefore cannot attribute HKCU to
    # the initiating desktop user and must preserve it.
    Assert-TaskOwnedAutostartPresent $mainBinary
    Assert-Condition (Test-Path -LiteralPath $secondaryState.database -PathType Leaf) 'Ordinary uninstall changed the standard-user Fylura database.'
    Assert-Condition (Test-Path -LiteralPath $secondaryState.cacheRoot -PathType Container) 'Ordinary uninstall changed the standard-user Fylura cache.'
    $secondaryVerify = Invoke-SecondaryUserContext 'verify'
    Assert-Condition $secondaryVerify.dataAndCredentialsPreserved 'Ordinary uninstall changed standard-user credentials.'
    Assert-SecondaryProfileDataPreserved
    Record-Phase 'uninstall' @{
        exitCode = $uninstallExitCode
        executableRemoved = $true
        fyluraServiceRemoved = $true
        uninstallRegistrationRemoved = $true
        currentDatabaseLegacyDatabaseAndSafeTrashPreserved = $true
        elevatedLauncherPreservedUnverifiedCurrentUserAutostart = $true
        unrelatedCurrentUserStartupPreserved = $true
        otherUserFyluraDataAndCachePreserved = $true
    }

    $secondaryCleanup = Invoke-SecondaryUserContext 'cleanup'
    $secondaryState = Get-Content -LiteralPath $secondaryContextStatePath -Raw | ConvertFrom-Json
    Assert-Condition ($secondaryCleanup.identityVerified -and $secondaryCleanup.dataAndCredentialsRemoved) 'The standard-user cleanup worker did not verify its own Windows identity and cleanup result.'
    Assert-Condition ($secondaryCleanup.safeTrashAndDocumentPreserved -and $secondaryCleanup.unrelatedCredentialPreserved -and $secondaryCleanup.ownAutostartRemoved -and $secondaryCleanup.unrelatedStartupPreserved) 'The standard-user cleanup worker did not preserve Safe Trash, documents, unrelated credentials, or unrelated startup entries.'
    Assert-Condition (-not (Test-Path -LiteralPath $secondaryState.database)) 'Standard-user cleanup left that account database behind.'
    Assert-Condition (-not (Test-Path -LiteralPath $secondaryState.cacheRoot)) 'Standard-user cleanup left that account cache behind.'
    Assert-Condition (Test-Path -LiteralPath $sentinels.currentDatabase -PathType Leaf) 'Standard-user cleanup changed the administrator database.'
    Assert-Condition (Test-Path -LiteralPath $currentCacheDir -PathType Container) 'Standard-user cleanup changed the administrator cache.'
    foreach ($target in $credentialTargets.Values) {
        Assert-Condition (Test-CredentialTarget $target) "Standard-user cleanup changed administrator credentials: $target"
    }
    Assert-SecondaryProfileDataPreserved
    Record-Phase 'secondary-user-context-cleanup' @{
        targetUserVerifiedByWindowsSid = $true
        onlyTargetUsersAppDataAndFyluraCredentialsRemoved = $true
        administratorDataAndCredentialsPreserved = $true
        safeTrashDocumentsAndForeignCredentialPreserved = $true
        uacAlternateAdministratorPromptExercised = $false
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
    $secondarySeedAfterReinstall = Invoke-SecondaryUserContext 'seed'
    Assert-Condition $secondarySeedAfterReinstall.stateSeeded 'The standard-user Fylura fixtures could not be restored for the administrator cleanup boundary test.'
    $secondaryState = Get-Content -LiteralPath $secondaryContextStatePath -Raw | ConvertFrom-Json
    $secondaryUserDataSentinels = @($secondarySeedAfterReinstall.userSentinels)
    Assert-SecondaryProfileDataPreserved
    Record-Phase 'reinstall-after-ordinary-uninstall' @{
        exitCode = $reinstallExitCode
        stoppedOwnedMainWindowProcesses = $reinstallMainProcessCount
        databaseReadable = $true
        userDataPreserved = $true
        otherUserFixturesReseededForFullCleanup = $true
    }

    Set-TaskOwnedAutostart $mainBinary
    Invoke-CurrentUserDataCleanup $mainBinary
    Assert-Condition (-not (Test-Path -LiteralPath $sentinels.currentDatabase)) 'The current-user cleanup helper left the Fylura database.'
    Assert-Condition (-not (Test-Path -LiteralPath $currentCacheDir)) 'The current-user cleanup helper left Fylura cache and logs.'
    Assert-TaskOwnedAutostartRemoved
    foreach ($target in $credentialTargets.Values) {
        Assert-Condition (-not (Test-CredentialTarget $target)) "The current-user cleanup helper left a Fylura credential target: $target"
    }

    # The isolated scheduled task already runs elevated, so this invocation
    # validates machine teardown only. User cleanup was exercised above in the
    # exact target user's Credential Manager and registry context.
    $fullUninstallExitCode = (Start-Process -FilePath $mainBinary -ArgumentList @('--fylura-uninstall-launcher', '/S') -PassThru -NoNewWindow -Wait).ExitCode
    Assert-Condition ($fullUninstallExitCode -eq 0) "Fylura machine uninstall failed with exit code $fullUninstallExitCode."
    for ($attempt = 0; $attempt -lt 120 -and (Test-Path -LiteralPath $mainBinary); $attempt++) {
        Start-Sleep -Milliseconds 500
    }
    Assert-Condition (-not (Test-Path -LiteralPath $mainBinary)) 'Confirmed full uninstall left the Fylura executable installed.'
    Assert-Condition (-not (Test-Path -LiteralPath $currentServiceKey)) 'Confirmed full uninstall left FyluraGlobalIndex registered.'
    Assert-Condition (-not (Test-Path -LiteralPath $sentinels.currentDatabase)) 'Current-user cleanup left the Fylura database before machine uninstall.'
    Assert-Condition (-not (Test-Path -LiteralPath $currentCacheDir)) 'Current-user cleanup left Fylura cache and logs before machine uninstall.'
    foreach ($target in $credentialTargets.Values) {
        Assert-Condition (-not (Test-CredentialTarget $target)) "Current-user cleanup left a Fylura credential target: $target"
    }
    Assert-TaskOwnedAutostartRemoved
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
    $secondaryVerify = Invoke-SecondaryUserContext 'verify'
    Assert-Condition $secondaryVerify.dataAndCredentialsPreserved "Administrator full cleanup changed another user's Fylura data or credentials."
    Assert-SecondaryProfileDataPreserved
    Assert-Condition (@(Get-ChildItem -LiteralPath $currentDataDir -Force | Where-Object { $_.Name -notin @('safe-trash', '.fylura-trash', '.zen-canvas-trash') }).Count -eq 0) 'Confirmed full uninstall left unexpected Fylura application data.'
    Record-Phase 'current-user-cleanup-plus-machine-uninstall' @{
        exitCode = $fullUninstallExitCode
        executableRemoved = $true
        currentUserDataCredentialAndAutostartHelperPassed = $true
        machineUninstallPassed = $true
        integratedInteractiveFullUninstall = 'UNVERIFIED'
        profileCredentialsEnumeratedIndependentOfDatabase = @('qa_alpha', 'qa_beta', 'qa_orphan')
        currentAndLegacySafeTrashPreserved = $true
        currentUserFyluraAutostartRemoved = $true
        unrelatedCurrentUserStartupPreserved = $true
        legacyDevelopmentDataPreserved = $true
        otherUserFyluraDataCacheCredentialsSafeTrashAndDocumentsPreserved = $true
        crossAccountUacFullFlowStatus = 'UNVERIFIED'
        unrelatedDirectoriesUntouched = $true
    }
    $script:overallStatus = 'PASS'
    Save-Evidence
}
catch {
    $errorRecord = $_
    $script:overallStatus = 'FAIL'
    $evidence.Add([ordered]@{
        phase = 'failed'
        at = [DateTime]::UtcNow.ToString('o')
        error = $errorRecord.Exception.Message
        script = if ($errorRecord.InvocationInfo.ScriptName) {
            Split-Path -Leaf $errorRecord.InvocationInfo.ScriptName
        } else {
            'verifyWindowsNsisLifecycle.ps1'
        }
        scriptLine = $errorRecord.InvocationInfo.ScriptLineNumber
    })
    Save-Evidence
    throw
}
finally {
    try {
        Remove-TaskOwnedAutostartFixture
        Remove-TaskOwnedCredentialFixtures
        Remove-TaskOwnedLegacyService
        if ($secondaryUserSidValue) {
            $secondaryProfile = Get-CimInstance -ClassName Win32_UserProfile -ErrorAction SilentlyContinue |
                Where-Object { $_.SID -eq $secondaryUserSidValue } | Select-Object -First 1
            if ($secondaryProfile -and $secondaryProfile.LocalPath) {
                $secondaryProfilePath = $secondaryProfile.LocalPath
                $secondaryProfileRootCreated = $true
            }
            if ($secondaryProfile -and -not $secondaryProfile.Loaded) {
                $secondaryProfile | Remove-CimInstance -ErrorAction SilentlyContinue
            }
        }
        if ($script:secondaryBatchLogonRightGranted -and $secondaryUserSidValue) {
            try {
                Set-TaskOwnedBatchLogonRight $secondaryUserSidValue $false
                $script:secondaryBatchLogonRightGranted = $false
            } catch {
                $script:secondaryBatchLogonRightCleanupFailure = $_.Exception.Message
            }
        }
        if ($secondaryUserCreated) {
            Remove-LocalUser -Name $secondaryUserName -ErrorAction SilentlyContinue
            if (Get-LocalUser -Name $secondaryUserName -ErrorAction SilentlyContinue) {
                throw 'The disposable standard-user test account remained after cleanup.'
            }
            $secondaryUserCreated = $false
        }
        if ($secondaryProfileRootCreated -and (Test-Path -LiteralPath $secondaryProfilePath)) {
            if ($secondaryProfile -and $secondaryProfile.Loaded) {
                Write-Warning 'The disposable standard-user profile hive is still mounted; its isolated profile directory is retained until GitHub-hosted VM teardown.'
                Record-Phase 'secondary-user-profile-retained-until-vm-teardown' @{
                    disposableVm = $true
                    profileHiveStillMounted = $true
                    testAccountRemoved = $true
                }
            } else {
                Remove-Item -LiteralPath $secondaryProfilePath -Recurse -Force -ErrorAction Stop
                if (Test-Path -LiteralPath $secondaryProfilePath) {
                    throw 'The task-owned secondary profile fixture remained after cleanup.'
                }
            }
        }
        if ($appDataFixtureCreated) {
            Remove-Item -LiteralPath $currentDataDir, $currentCacheDir, $legacyDataDir -Recurse -Force -ErrorAction SilentlyContinue
        }
        if (Test-Path -LiteralPath $taskRoot) {
            Remove-Item -LiteralPath $taskRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
        if ($script:secondaryBatchLogonRightCleanupFailure) {
            throw "Could not revoke the temporary scheduled-task logon right: $($script:secondaryBatchLogonRightCleanupFailure)"
        }
    } catch {
        $script:overallStatus = 'FAIL'
        $cleanupError = ($_.Exception.Message -replace '\s+', ' ').Trim()
        if ($cleanupError.Length -gt 800) { $cleanupError = $cleanupError.Substring(0, 800) }
        $evidence.Add([ordered]@{
            phase = 'cleanup-failed'
            at = [DateTime]::UtcNow.ToString('o')
            error = $cleanupError
            script = 'verifyWindowsNsisLifecycle.ps1'
            scriptLine = $_.InvocationInfo.ScriptLineNumber
        })
        Save-Evidence
        throw
    }
}
