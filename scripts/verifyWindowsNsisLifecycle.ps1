param(
    [Parameter(Mandatory = $true)][string]$InstallerPath,
    [Parameter(Mandatory = $true)][string]$EvidencePath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Save-Evidence {
    $parent = Split-Path -Parent $EvidencePath
    if ($parent) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
    [ordered]@{ product = 'Fylura'; sourceSha = $env:GITHUB_SHA; runId = $env:GITHUB_RUN_ID; status = $script:status; phases = @($script:evidence.ToArray()) } |
        ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
}
function Record-Phase([string]$Phase, [hashtable]$Details = @{}) {
    $script:evidence.Add([ordered]@{ phase = $Phase; at = [DateTime]::UtcNow.ToString('o'); details = $Details })
    Save-Evidence
}
function Credential-Targets {
    $output = & "$env:WINDIR\System32\cmdkey.exe" /list 2>&1
    Assert-Condition ($LASTEXITCODE -eq 0) 'Could not inspect Windows Credential Manager.'
    $result = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($line in $output) {
        if ($line -match '^\s*Target:\s*(.+?)\s*$') {
            $target = $Matches[1].Trim()
            foreach ($prefix in @('LegacyGeneric:target=', 'Generic:target=')) {
                if ($target.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
                    $target = $target.Substring($prefix.Length)
                    break
                }
            }
            $null = $result.Add($target)
        }
    }
    return ,$result
}
function Has-Credential([string]$Target) { return (Credential-Targets).Contains($Target) }
function Add-Credential([string]$Target) {
    $script:credentialFixtures.Add($Target)
    $secret = "fylura-596-qa-$([guid]::NewGuid().ToString('N'))"
    $null = & "$env:WINDIR\System32\cmdkey.exe" "/generic:$Target" "/user:$Target" "/pass:$secret" 2>&1
    Assert-Condition ($LASTEXITCODE -eq 0 -and (Has-Credential $Target)) "Could not seed credential fixture: $Target"
}
function Remove-Credentials {
    foreach ($target in $script:credentialFixtures) { $null = & "$env:WINDIR\System32\cmdkey.exe" "/delete:$target" 2>&1 }
    $script:credentialFixtures.Clear()
}
function Invoke-Sc([string[]]$ScArgs) {
    $output = & "$env:WINDIR\System32\sc.exe" @ScArgs 2>&1
    return [pscustomobject]@{ code = $LASTEXITCODE; output = ($output | Out-String).Trim() }
}
function Install {
    $p = Start-Process -FilePath $installer -ArgumentList @('/S', "/D=$installDir") -PassThru -NoNewWindow
    if (-not $p.WaitForExit(120000)) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; throw 'Installer timed out.' }
    $p.Refresh()
    return [int]$p.ExitCode
}
function Invoke-Uninstaller([string[]]$UninstallArgs) {
    $script:uninstallerLogIndex++
    $script:lastUninstallerLog = Join-Path $taskRoot "uninstall-$($script:uninstallerLogIndex).log"
    $arguments = @($UninstallArgs) + @("/LOG=$script:lastUninstallerLog")
    $p = Start-Process -FilePath $uninstaller -ArgumentList $arguments -PassThru -NoNewWindow
    if (-not $p.WaitForExit(120000)) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; throw 'Uninstaller timed out.' }
    $p.Refresh()
    return [int]$p.ExitCode
}
function Get-FyluraUninstallEntries {
    if (-not (Test-Path -LiteralPath $uninstallRoot)) { return @() }
    $entries = @(
        Get-ChildItem -LiteralPath $uninstallRoot | ForEach-Object {
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
    return $entries
}
function Registry-Value([string]$Subkey, [string]$Name = '') {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64)
    try {
        $key = $base.OpenSubKey($Subkey)
        if ($null -eq $key) { return $null }
        try { return $key.GetValue($Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
        finally { $key.Dispose() }
    } finally { $base.Dispose() }
}
function Assert-Preview([bool]$Present) {
    $id = '{3D1A446C-162E-4313-A026-8ADC792C4862}'
    $category = '{8895B1C6-B41F-4C1C-A562-0D564250836F}'
    $root = "SOFTWARE\Classes\CLSID\$id"
    $inproc = "$root\InprocServer32"
    if ($Present) {
        $dll = Join-Path $installDir 'native\fylura_windows_preview_handler.dll'
        Assert-Condition ((Registry-Value $root) -eq 'Fylura Preview Handler') 'Preview Handler CLSID is missing.'
        $actualDll = [string](Registry-Value $inproc)
        Assert-Condition ([IO.Path]::GetFullPath($actualDll).Equals([IO.Path]::GetFullPath($dll), [StringComparison]::OrdinalIgnoreCase)) 'Preview Handler DLL path is unexpected.'
        Assert-Condition ((Registry-Value $root 'AppID') -eq '{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}') 'Preview Handler AppID is unexpected.'
        Assert-Condition ((Registry-Value $inproc 'ThreadingModel') -eq 'Apartment') 'Preview Handler threading model is unexpected.'
        Assert-Condition ((Registry-Value 'SOFTWARE\Microsoft\Windows\CurrentVersion\PreviewHandlers' $id) -eq 'Fylura Preview Handler') 'Preview Handler list registration is missing.'
        foreach ($ext in @('.md','.markdown','.rs','.py','.js','.jsx','.ts','.tsx','.java','.c','.h','.cpp','.hpp','.ps1','.sh','.sql')) {
            Assert-Condition ((Registry-Value "SOFTWARE\Classes\SystemFileAssociations\$ext\shellex\$category") -eq $id) "Preview Handler association missing for $ext."
        }
    } else {
        Assert-Condition ($null -eq (Registry-Value $root)) 'Preview Handler CLSID remained after uninstall.'
        Assert-Condition ($null -eq (Registry-Value $inproc)) 'Preview Handler DLL registration remained after uninstall.'
        Assert-Condition ($null -eq (Registry-Value 'SOFTWARE\Microsoft\Windows\CurrentVersion\PreviewHandlers' $id)) 'Preview Handler list registration remained.'
        foreach ($ext in @('.md','.markdown','.rs','.py','.js','.jsx','.ts','.tsx','.java','.c','.h','.cpp','.hpp','.ps1','.sh','.sql')) {
            Assert-Condition ($null -eq (Registry-Value "SOFTWARE\Classes\SystemFileAssociations\$ext\shellex\$category")) "Preview Handler association remained for $ext."
        }
    }
}
function Assert-Service {
    $s = Get-Service -Name $service -ErrorAction SilentlyContinue
    try {
        Assert-Condition ($null -ne $s -and $s.Status -eq 'Running') 'FyluraGlobalIndex did not reach RUNNING.'
    } finally {
        if ($null -ne $s) { $s.Dispose() }
    }
}
function Assert-Uninstalled([string]$phase) {
    for ($i = 0; $i -lt 120 -and ((Test-Path -LiteralPath $main) -or (Test-Path -LiteralPath $uninstaller) -or (Test-Path -LiteralPath $installDir)); $i++) {
        Start-Sleep -Milliseconds 500
    }
    $mainExists = Test-Path -LiteralPath $main
    $uninstallerExists = Test-Path -LiteralPath $uninstaller
    $installDirExists = Test-Path -LiteralPath $installDir
    if ($mainExists -or $uninstallerExists -or $installDirExists) {
        $remainingInstallFiles = @()
        if ($installDirExists) {
            $remainingInstallFiles = @(Get-ChildItem -LiteralPath $installDir -Force | ForEach-Object { $_.Name })
        }
        Assert-Condition $false "Program files remained after $phase uninstall. mainExists=$mainExists; uninstallerExists=$uninstallerExists; installDirExists=$installDirExists; installFiles=$($remainingInstallFiles -join '|')"
    }
    if (Test-Path -LiteralPath $serviceKey) {
        $scm = Invoke-Sc @('query', $service)
        $configuration = Invoke-Sc @('qc', $service)
        $imagePath = Registry-Value "SYSTEM\CurrentControlSet\Services\$service" 'ImagePath'
        $expectedImagePath = '"' + $main + '" --index-service'
        $deleteProbe = [pscustomobject]@{ code = 'not-run'; output = 'ImagePath did not match the isolated Fylura fixture.' }
        if ([string]$imagePath -ceq $expectedImagePath) {
            # This test owns the exact service path in a disposable hosted VM.
            # Probe SCM once after failure to distinguish a skipped delete from
            # a service that was marked for deletion but remains open.
            $deleteProbe = Invoke-Sc @('delete', $service)
        }
        $serviceLog = @()
        $serviceLogExists = Test-Path -LiteralPath $script:lastUninstallerLog
        if ($serviceLogExists) {
            $serviceLog = @(Get-Content -LiteralPath $script:lastUninstallerLog | Where-Object { $_ -match 'Fylura (Global Index|service cleanup|uninstall)|service registration|incomplete|sc\.exe delete' } | Select-Object -Last 40)
        }
        Assert-Condition $false "FyluraGlobalIndex remained after uninstall. SCM query exit=$($scm.code), output=$($scm.output); service config exit=$($configuration.code), output=$($configuration.output); ImagePath=$imagePath; controlled delete probe exit=$($deleteProbe.code), output=$($deleteProbe.output); NSIS log exists=$serviceLogExists, path=$script:lastUninstallerLog, service log=$($serviceLog -join ' | ')"
    }
    $entries = @(Get-FyluraUninstallEntries)
    if ($entries.Count -ne 0) {
        $remainingInstallFiles = @()
        if (Test-Path -LiteralPath $installDir) {
            $remainingInstallFiles = @(Get-ChildItem -LiteralPath $installDir -Force | ForEach-Object { $_.Name })
        }
        $registrationDetails = @(
            $entries | ForEach-Object {
                $key = "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$($_.key)"
                [pscustomobject]@{
                    key = $_.key
                    displayName = Registry-Value $key 'DisplayName'
                    installLocation = Registry-Value $key 'InstallLocation'
                    uninstallString = Registry-Value $key 'UninstallString'
                }
            }
        )
        $manufacturerMarker = Registry-Value 'SOFTWARE\Startlan\Fylura'
        $previewRegistration = Registry-Value 'SOFTWARE\Classes\CLSID\{3D1A446C-162E-4313-A026-8ADC792C4862}'
        $uninstallLogExists = $script:lastUninstallerLog -and (Test-Path -LiteralPath $script:lastUninstallerLog)
        $uninstallLogLength = 0
        $uninstallLogTail = @()
        if ($uninstallLogExists) {
            $uninstallLogLength = (Get-Item -LiteralPath $script:lastUninstallerLog).Length
            $uninstallLogTail = @(Get-Content -LiteralPath $script:lastUninstallerLog | Select-Object -Last 120)
        }
        Assert-Condition $false "Fylura remained in Add/Remove Programs. Entries=$($registrationDetails | ConvertTo-Json -Compress); installDirExists=$(Test-Path -LiteralPath $installDir); installFiles=$($remainingInstallFiles -join '|'); manufacturerMarker=$manufacturerMarker; previewRegistration=$previewRegistration; NSISLogExists=$uninstallLogExists; NSISLogBytes=$uninstallLogLength; NSISLogTail=$($uninstallLogTail -join ' | ')"
    }
    Assert-Preview $false
}
function Seed-Autostart {
    New-Item -Path $runKey, $startupApproved -Force | Out-Null
    New-ItemProperty -LiteralPath $runKey -Name 'Fylura' -PropertyType String -Value "$main --background" -Force | Out-Null
    New-ItemProperty -LiteralPath $startupApproved -Name 'Fylura' -PropertyType Binary -Value ([byte[]](2,0,0,0,0,0,0,0,0,0,0,0)) -Force | Out-Null
    New-ItemProperty -LiteralPath $runKey -Name $foreignStartup -PropertyType String -Value $foreignCommand -Force | Out-Null
    New-ItemProperty -LiteralPath $startupApproved -Name $foreignStartup -PropertyType Binary -Value $foreignApproval -Force | Out-Null
    $script:autostartSeeded = $true
}
function Assert-Autostart([bool]$FyluraPresent) {
    $r = Get-ItemProperty -LiteralPath $runKey -ErrorAction SilentlyContinue
    $a = Get-ItemProperty -LiteralPath $startupApproved -ErrorAction SilentlyContinue
    if ($FyluraPresent) {
        Assert-Condition ($r.Fylura -eq "$main --background") 'Fylura startup command changed.'
        Assert-Condition ($null -ne $a.PSObject.Properties['Fylura']) 'Fylura StartupApproved entry is missing.'
    } else {
        $viewSnapshot = foreach ($view in @([Microsoft.Win32.RegistryView]::Registry64, [Microsoft.Win32.RegistryView]::Registry32)) {
            $registryRoot = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, $view)
            try {
                $runValues = $registryRoot.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Run')
                $approvedValues = $registryRoot.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run')
                try {
                    [pscustomobject]@{
                        view = $view.ToString()
                        runValue = if ($null -ne $runValues) { $runValues.GetValue('Fylura', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } else { $null }
                        startupApprovedPresent = if ($null -ne $approvedValues) { $approvedValues.GetValueNames() -contains 'Fylura' } else { $false }
                    }
                } finally {
                    if ($null -ne $runValues) { $runValues.Dispose() }
                    if ($null -ne $approvedValues) { $approvedValues.Dispose() }
                }
            } finally {
                $registryRoot.Dispose()
            }
        }
        $viewDetails = $viewSnapshot | ConvertTo-Json -Compress
        Assert-Condition ($null -eq $r.PSObject.Properties['Fylura']) "Fylura Run entry remained after uninstall. Registry views: $viewDetails"
        Assert-Condition ($null -eq $a.PSObject.Properties['Fylura']) 'Fylura StartupApproved entry remained after uninstall.'
    }
    Assert-Condition ($r.($foreignStartup) -eq $foreignCommand) 'Uninstall changed unrelated startup.'
    Assert-Condition ([Convert]::ToBase64String($a.($foreignStartup)) -eq [Convert]::ToBase64String($foreignApproval)) 'Uninstall changed unrelated StartupApproved data.'
}
function Cleanup-Autostart {
    if (-not $script:autostartSeeded) { return }
    foreach ($x in @(@{Path=$runKey;Name='Fylura';Value="$main --background"}, @{Path=$runKey;Name=$foreignStartup;Value=$foreignCommand})) {
        $p = Get-ItemProperty -LiteralPath $x.Path -ErrorAction SilentlyContinue
        $v = if ($null -ne $p) { $p.PSObject.Properties[$x.Name] } else { $null }
        if ($null -ne $v -and [string]$v.Value -eq $x.Value) { Remove-ItemProperty -LiteralPath $x.Path -Name $x.Name -ErrorAction SilentlyContinue }
    }
    foreach ($x in @(@{Name='Fylura';Value=[Convert]::ToBase64String([byte[]](2,0,0,0,0,0,0,0,0,0,0,0))}, @{Name=$foreignStartup;Value=[Convert]::ToBase64String($foreignApproval)})) {
        $p = Get-ItemProperty -LiteralPath $startupApproved -ErrorAction SilentlyContinue
        $v = if ($null -ne $p) { $p.PSObject.Properties[$x.Name] } else { $null }
        if ($null -ne $v -and $v.Value -is [byte[]] -and [Convert]::ToBase64String($v.Value) -eq $x.Value) { Remove-ItemProperty -LiteralPath $startupApproved -Name $x.Name -ErrorAction SilentlyContinue }
    }
    $script:autostartSeeded = $false
}

$installer = [IO.Path]::GetFullPath($InstallerPath)
$script:evidence = [System.Collections.Generic.List[object]]::new()
$script:status = 'IN_PROGRESS'
$script:credentialFixtures = [System.Collections.Generic.List[string]]::new()
$script:uninstallerLogIndex = 0
$script:lastUninstallerLog = $null
$runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP)
$appDataRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
$localAppDataRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$data = Join-Path $appDataRoot 'com.startlan.fylura'
$cache = Join-Path $localAppDataRoot 'com.startlan.fylura'
$legacyData = Join-Path $appDataRoot 'com.startlan.zencanvas'
$taskRoot = Join-Path $runnerTemp "fylura-nsis-runtime-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT"
$installDir = Join-Path $taskRoot 'install'
$main = Join-Path $installDir 'fylura.exe'
$uninstaller = Join-Path $installDir 'uninstall.exe'
$service = 'FyluraGlobalIndex'
$serviceKey = "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$service"
$legacyService = 'ZenCanvasGlobalIndex'
$legacyServiceKey = "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$legacyService"
$uninstallRoot = 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
$targets = [ordered]@{
    main = 'ai-api-key.com.startlan.fylura'
    alpha = 'ai-api-key-qa_alpha.com.startlan.fylura.ai-profile'
    beta = 'ai-api-key-qa_beta.com.startlan.fylura.ai-profile'
    orphan = 'ai-api-key-qa_orphan.com.startlan.fylura.ai-profile'
}
$foreignCredential = 'fylura-cleanup-fixture.com.example.editor'
$runKey = 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run'
$startupApproved = 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$foreignStartup = 'Fylura596ForeignFixture'
$foreignCommand = 'C:\Fylura596-test-owned\foreign-fixture.exe --keep'
$foreignApproval = [byte[]](2,0,0,0,0,0,0,0,0,0,0,0)
$script:autostartSeeded = $false
$script:legacyServiceCreated = $false
$script:appDataFixtureCreated = $false

function Remove-LegacyServiceFixture {
    if (-not $script:legacyServiceCreated) { return }
    $null = Invoke-Sc @('stop', $legacyService)
    $r = Invoke-Sc @('delete', $legacyService)
    Assert-Condition ($r.code -eq 0) "Could not remove test service: $($r.output)"
    for ($i = 0; $i -lt 30 -and (Test-Path -LiteralPath $legacyServiceKey); $i++) { Start-Sleep -Milliseconds 500 }
    Assert-Condition (-not (Test-Path -LiteralPath $legacyServiceKey)) 'Test legacy service remained.'
    $script:legacyServiceCreated = $false
}

try {
    Assert-Condition ($env:GITHUB_ACTIONS -eq 'true' -and $env:RUNNER_ENVIRONMENT -eq 'github-hosted') 'Lifecycle verification is restricted to a disposable GitHub-hosted Windows VM.'
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    Assert-Condition ([Security.Principal.WindowsPrincipal]::new($id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) 'Lifecycle verification requires an elevated hosted runner.'
    Assert-Condition (Test-Path -LiteralPath $installer -PathType Leaf) 'NSIS installer is missing.'
    Assert-Condition (-not (Test-Path -LiteralPath $taskRoot)) 'Refusing a pre-existing lifecycle fixture directory.'
    Assert-Condition (-not (Test-Path -LiteralPath $serviceKey) -and -not (Test-Path -LiteralPath $legacyServiceKey)) 'Refusing a pre-existing Fylura or legacy service.'
    Assert-Condition (@(Get-FyluraUninstallEntries).Count -eq 0) 'Refusing to run over an existing Fylura installation.'
    Assert-Condition (-not (Test-Path -LiteralPath $data) -and -not (Test-Path -LiteralPath $cache) -and -not (Test-Path -LiteralPath $legacyData)) 'Refusing to alter existing application data.'
    Assert-Condition ($env:GITHUB_RUN_ID -and $env:GITHUB_RUN_ATTEMPT -and $env:GITHUB_SHA) 'GitHub run identity is unavailable.'
    foreach ($target in @($targets.Values) + @($foreignCredential)) { Assert-Condition (-not (Has-Credential $target)) "Refusing to overwrite existing credential $target" }
    $r = Get-ItemProperty -LiteralPath $runKey -ErrorAction SilentlyContinue
    $a = Get-ItemProperty -LiteralPath $startupApproved -ErrorAction SilentlyContinue
    Assert-Condition ($null -eq $r -or ($null -eq $r.PSObject.Properties['Fylura'] -and $null -eq $r.PSObject.Properties[$foreignStartup])) 'Refusing to overwrite existing startup values.'
    Assert-Condition ($null -eq $a -or ($null -eq $a.PSObject.Properties['Fylura'] -and $null -eq $a.PSObject.Properties[$foreignStartup])) 'Refusing to overwrite existing StartupApproved values.'

    New-Item -ItemType Directory -Path $taskRoot | Out-Null
    Seed-Autostart
    Record-Phase 'preflight' @{ disposableHostedVm = $true; elevated = $true; userDataAbsent = $true }

    $legacyBin = '"' + $env:WINDIR + '\System32\cmd.exe" /c exit 0'
    $created = Invoke-Sc @('create', $legacyService, 'binPath=', $legacyBin, 'start=', 'demand', 'DisplayName=', 'Fylura lifecycle conflict fixture')
    Assert-Condition ($created.code -eq 0) "Could not create legacy service fixture: $($created.output)"
    $script:legacyServiceCreated = $true
    $before = (Get-ItemProperty -LiteralPath $legacyServiceKey -Name ImagePath).ImagePath
    $failedInstall = Install
    Assert-Condition ($failedInstall -ne 0) 'Installer did not reject a legacy-service conflict.'
    Assert-Condition (-not (Test-Path -LiteralPath $main) -and -not (Test-Path -LiteralPath $serviceKey)) 'Conflict rollback left Fylura installed.'
    Assert-Condition (@(Get-FyluraUninstallEntries).Count -eq 0) 'Conflict rollback left an uninstall entry.'
    Assert-Condition ((Get-ItemProperty -LiteralPath $legacyServiceKey -Name ImagePath).ImagePath -eq $before) 'Conflict rollback changed the legacy service.'
    Record-Phase 'legacy-service-conflict-rollback' @{ installerExitCode = $failedInstall; legacyServicePreserved = $true }
    Remove-LegacyServiceFixture
    if (Test-Path -LiteralPath $installDir) {
        $resolvedInstallDir = [IO.Path]::GetFullPath($installDir)
        Assert-Condition ([IO.Path]::GetDirectoryName($resolvedInstallDir).Equals([IO.Path]::GetFullPath($taskRoot), [StringComparison]::OrdinalIgnoreCase)) 'Refusing to clear an install directory outside the task-owned fixture.'
        Remove-Item -LiteralPath $resolvedInstallDir -Recurse -Force
    }

    $fresh = Install
    Assert-Condition ($fresh -eq 0) "Fresh install failed with exit code $fresh."
    Assert-Condition ((Test-Path -LiteralPath $main -PathType Leaf) -and (Test-Path -LiteralPath $uninstaller -PathType Leaf)) 'Fresh install did not publish expected program files.'
    $entries = @(Get-FyluraUninstallEntries)
    Assert-Condition ($entries.Count -eq 1) 'Fresh install did not register exactly one uninstall entry.'
    $regPath = "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$($entries[0].key)"
    Assert-Condition ((Get-ItemProperty -LiteralPath $regPath -Name UninstallString).UninstallString -eq ('"' + $uninstaller + '"')) 'Uninstall registration is not the direct NSIS uninstaller.'
    Assert-Service
    Assert-Preview $true
    Record-Phase 'fresh-install' @{ directUninstaller = $true; serviceRunning = $true; previewRegistered = $true }

    $outside = Join-Path $taskRoot 'safe-trash-source'
    $organized = Join-Path $taskRoot 'organized-files'
    $unrelated = Join-Path $taskRoot 'unrelated-folder'
    $trash = @(
        (Join-Path $data 'safe-trash\items\fixture'),
        (Join-Path $data '.fylura-trash\items\fixture'),
        (Join-Path $data '.zen-canvas-trash\items\fixture'),
        (Join-Path $outside '.fylura-trash\items\fixture'),
        (Join-Path $outside '.zen-canvas-trash\items\fixture')
    )
    $files = [ordered]@{
        legacyDatabase = Join-Path $legacyData 'zen-canvas.sqlite3'
        safeTrash = Join-Path $trash[0] 'real-file.txt'
        newSafeTrash = Join-Path $trash[1] 'real-file.txt'
        legacySafeTrash = Join-Path $trash[2] 'real-file.txt'
        externalNewSafeTrash = Join-Path $trash[3] 'real-file.txt'
        externalLegacySafeTrash = Join-Path $trash[4] 'real-file.txt'
        organizedFile = Join-Path $organized 'organized.txt'
        unrelatedFile = Join-Path $unrelated 'keep.txt'
        database = Join-Path $data 'fylura.sqlite3'
        log = Join-Path $cache 'logs\app.log'
    }
    $script:appDataFixtureCreated = $true
    New-Item -ItemType Directory -Path @($data, $cache, $legacyData, $organized, $unrelated) -Force | Out-Null
    foreach ($dir in $trash) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    New-Item -ItemType Directory -Path (Split-Path -Parent $files.log) -Force | Out-Null
    foreach ($entry in $files.GetEnumerator()) {
        if ($entry.Key -eq 'database') { continue }
        [IO.File]::WriteAllText($entry.Value, "test-owned $($entry.Key)")
    }
    $python = (Get-Command python -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
    $seed = Join-Path $taskRoot 'seed.py'
    @'
import json, sqlite3, sys
db = sqlite3.connect(sys.argv[1])
db.execute("CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
db.execute("INSERT INTO app_settings VALUES (?, ?)", ("ai_settings_v1", json.dumps({"customProfiles": [{"id": "qa_alpha"}, {"id": "qa_beta"}]})))
db.commit()
db.close()
'@ | Set-Content -LiteralPath $seed -Encoding ascii
    $seedOutput = @(& $python $seed $files.database 2>&1)
    Assert-Condition ($LASTEXITCODE -eq 0) "Could not seed settings database: $(($seedOutput | Out-String).Trim())"
    $verify = Join-Path $taskRoot 'verify.py'
    @'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1])
row = db.execute("SELECT value FROM app_settings WHERE key = ?", ("ai_settings_v1",)).fetchone()
db.close()
if row is None or "qa_alpha" not in row[0] or "qa_beta" not in row[0]: raise SystemExit("settings unreadable")
'@ | Set-Content -LiteralPath $verify -Encoding ascii
    foreach ($target in $targets.Values) { Add-Credential $target }
    Add-Credential $foreignCredential
    Record-Phase 'application-fixtures' @{ sqliteProfiles = @('qa_alpha','qa_beta'); orphanProfileCredential = $true; safeTrashAndUnrelatedFiles = $true; credentialValuesRecorded = $false }

    Seed-Autostart
    $deleteOnly = Invoke-Uninstaller @('/S', '/DELETE_APP_DATA')
    if ($deleteOnly -ne 0) {
        $scm = Invoke-Sc @('query', $service)
        $configuration = Invoke-Sc @('qc', $service)
        $imagePath = Registry-Value "SYSTEM\CurrentControlSet\Services\$service" 'ImagePath'
        throw "Ordinary uninstall with an incomplete cleanup request returned $deleteOnly. SCM query exit=$($scm.code), output=$($scm.output); service config exit=$($configuration.code), output=$($configuration.output); ImagePath=$imagePath; NSIS log path=$script:lastUninstallerLog"
    }
    Assert-Uninstalled 'ordinary'
    foreach ($entry in $files.GetEnumerator()) {
        Assert-Condition (Test-Path -LiteralPath $entry.Value -PathType Leaf) "Delete-only request removed Fylura application data: $($entry.Key)."
        if ($entry.Key -eq 'database') { continue }
        Assert-Condition ([IO.File]::ReadAllText($entry.Value) -eq "test-owned $($entry.Key)") "Delete-only request changed Fylura application data: $($entry.Key)."
    }
    Assert-Condition ((Test-Path $data) -and (Test-Path $cache)) 'Delete-only request removed Fylura application data.'
    $dbOutput = @(& $python $verify $files.database 2>&1)
    Assert-Condition ($LASTEXITCODE -eq 0) "Delete-only request left unreadable settings: $(($dbOutput | Out-String).Trim())"
    foreach ($target in $targets.Values) { Assert-Condition (Has-Credential $target) "Delete-only request removed credential $target." }
    Assert-Condition (Has-Credential $foreignCredential) 'Delete-only request changed unrelated credential.'
    Assert-Autostart $false
    Record-Phase 'delete-only-request-falls-back-to-ordinary' @{ exitCode = $deleteOnly; appDataCredentialsAndSafeTrashPreserved = $true; servicePreviewAndOwnStartupRemoved = $true; unrelatedStartupPreserved = $true }

    $reinstall = Install
    Assert-Condition ($reinstall -eq 0) "Reinstall failed with exit code $reinstall."
    Assert-Service
    Assert-Preview $true
    $dbOutput = @(& $python $verify $files.database 2>&1)
    Assert-Condition ($LASTEXITCODE -eq 0) "Reinstalled application could not read settings: $(($dbOutput | Out-String).Trim())"
    foreach ($target in $targets.Values) { Assert-Condition (Has-Credential $target) "Reinstall lost credential $target." }
    Record-Phase 'reinstall' @{ databaseReadable = $true; serviceRunning = $true; previewRegistered = $true }

    Seed-Autostart
    $full = Invoke-Uninstaller @('/S', '/DELETE_APP_DATA', '/CONFIRM_DELETE_APP_DATA')
    Assert-Condition ($full -eq 0) "Confirmed full uninstall returned $full."
    Assert-Uninstalled 'full-cleanup'
    Assert-Condition (-not (Test-Path $cache)) 'Full cleanup left Fylura cache or logs.'
    Assert-Condition (Test-Path $data -PathType Container) 'Full cleanup did not retain Safe Trash roots.'
    $remaining = @(Get-ChildItem -LiteralPath $data -Force | ForEach-Object { $_.Name } | Sort-Object)
    Assert-Condition (($remaining -join '|') -eq ('.fylura-trash|.zen-canvas-trash|safe-trash')) 'Full cleanup left unexpected app data or removed Safe Trash.'
    foreach ($target in $targets.Values) { Assert-Condition (-not (Has-Credential $target)) "Full cleanup left Fylura credential $target." }
    Assert-Condition (Has-Credential $foreignCredential) 'Full cleanup removed unrelated credential.'
    Assert-Autostart $false
    foreach ($entry in $files.GetEnumerator()) {
        if ($entry.Key -in @('database','log')) { continue }
        Assert-Condition ((Test-Path -LiteralPath $entry.Value -PathType Leaf) -and ([IO.File]::ReadAllText($entry.Value) -eq "test-owned $($entry.Key)")) "Full cleanup changed $($entry.Key)."
    }
    Record-Phase 'confirmed-full-uninstall' @{ appDataCredentialsAndIntegrationRemoved = $true; safeTrashOrganizedFilesAndLegacyDataPreserved = $true; unrelatedCredentialAndStartupPreserved = $true; explicitConsentFlags = $true }
    $script:status = 'PASS'
    $parent = Split-Path -Parent $EvidencePath
    if ($parent) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
    [ordered]@{ product = 'Fylura'; sourceSha = $env:GITHUB_SHA; runId = $env:GITHUB_RUN_ID; status = $script:status; phases = @($script:evidence.ToArray()) } |
        ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
} catch {
    $script:status = 'FAIL'
    $script:evidence.Add([ordered]@{ phase = 'failed'; at = [DateTime]::UtcNow.ToString('o'); error = $_.Exception.Message; script = 'verifyWindowsNsisLifecycle.ps1'; scriptLine = $_.InvocationInfo.ScriptLineNumber })
    $parent = Split-Path -Parent $EvidencePath
    if ($parent) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
    Save-Evidence
    throw
} finally {
    try {
        Cleanup-Autostart
        Remove-Credentials
        Remove-LegacyServiceFixture
        if ($script:appDataFixtureCreated) { Remove-Item -LiteralPath $data, $cache, $legacyData -Recurse -Force -ErrorAction SilentlyContinue }
        if (Test-Path -LiteralPath $taskRoot) {
            $resolved = [IO.Path]::GetFullPath($taskRoot)
            Assert-Condition ([IO.Path]::GetDirectoryName($resolved).Equals($runnerTemp, [StringComparison]::OrdinalIgnoreCase)) 'Refusing to remove a fixture outside RUNNER_TEMP.'
            Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
        }
    } catch {
        $script:status = 'FAIL'
        $script:evidence.Add([ordered]@{ phase = 'cleanup-failed'; at = [DateTime]::UtcNow.ToString('o'); error = $_.Exception.Message })
        Save-Evidence
        throw
    }
}
