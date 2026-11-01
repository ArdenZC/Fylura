[CmdletBinding()]
param(
    [switch]$Child,
    [string]$ConfigPath
)

$ErrorActionPreference = "Stop"

if ($Child) {
    if (-not $ConfigPath -or -not (Test-Path -LiteralPath $ConfigPath)) {
        throw "ordinary-user native smoke configuration is missing"
    }
    $config = Get-Content -LiteralPath $ConfigPath -Raw | ConvertFrom-Json
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "native filesystem smoke process unexpectedly has an administrator token"
    }

    $env:TEMP = $config.SourceTemp
    $env:TMP = $config.SourceTemp
    $env:ZC_NATIVE_QA_REQUIRE_STANDARD_USER = "1"
    $env:ZC_NATIVE_QA_REQUIRE_CROSS_VOLUME = "1"
    $env:ZC_NATIVE_QA_ARTIFACT_ROOT = $config.EvidenceRoot
    $env:ZEN_CANVAS_NATIVE_QA_SECONDARY_TEMP = $config.SecondaryRoot

    $tokenOutput = (& whoami.exe /all 2>&1 | Out-String)
    Set-Content -LiteralPath (Join-Path $config.EvidenceRoot "native-file-hardening-whoami.txt") -Value $tokenOutput -Encoding utf8
    $testLog = Join-Path $config.EvidenceRoot "native-file-hardening-smoke.log"
    & $config.TestExecutable --ignored --exact native_file_hardening_smoke --nocapture --test-threads=1 2>&1 |
        Tee-Object -FilePath $testLog
    $testExitCode = $LASTEXITCODE
    exit $testExitCode
}

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$manifestPath = Join-Path $repositoryRoot "src-tauri/Cargo.toml"
cargo test --manifest-path $manifestPath --features "desktop-runtime native-qa" --test native_file_hardening_smoke --no-run
if ($LASTEXITCODE -ne 0) {
    throw "could not build the native filesystem smoke test (exit $LASTEXITCODE)"
}
$testExecutable = Get-ChildItem -LiteralPath (Join-Path $repositoryRoot "src-tauri/target/debug/deps") -Filter "native_file_hardening_smoke-*.exe" |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if (-not $testExecutable) {
    throw "cargo completed without producing native_file_hardening_smoke.exe"
}
if (-not (Test-Path -LiteralPath "D:\")) {
    throw "the Windows native ordinary-user smoke requires an independent D: volume"
}

$qaName = "FyluraQa_" + [Guid]::NewGuid().ToString("N").Substring(0, 10)
$password = ConvertTo-SecureString (([Guid]::NewGuid().ToString("N")) + "aA1!") -AsPlainText -Force
$userCreated = $false
$userSid = $null
$qaRoot = Join-Path "C:\fylura-native-qa" $qaName
$sourceTemp = Join-Path $qaRoot "temp"
$secondaryRoot = Join-Path (Join-Path "D:\fylura-native-qa" $qaName) "secondary"
$evidenceRoot = Join-Path $env:RUNNER_TEMP "fylura-native-file-qa-evidence"
$configPath = Join-Path $qaRoot "fylura-native-file-qa-config.json"

try {
    New-LocalUser -Name $qaName -Password $password -PasswordNeverExpires -AccountNeverExpires -Description "Isolated standard-user Fylura filesystem QA" | Out-Null
    $userCreated = $true
    Add-LocalGroupMember -Group "Users" -Member $qaName
    $userSid = (Get-LocalUser -Name $qaName).SID.Value
    $adminSids = @(Get-LocalGroupMember -Group "Administrators" | ForEach-Object { $_.SID.Value })
    if ($adminSids -contains $userSid) {
        throw "isolated QA account was added to Administrators"
    }

    foreach ($path in @($qaRoot, $sourceTemp, $secondaryRoot, $evidenceRoot)) {
        New-Item -ItemType Directory -Path $path -Force | Out-Null
        $grant = "*${userSid}:(OI)(CI)F"
        & icacls.exe $path /grant $grant | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "could not grant the ordinary QA account access to $path"
        }
    }

    $config = [ordered]@{
        TestExecutable = $testExecutable.FullName
        SourceTemp = $sourceTemp
        SecondaryRoot = $secondaryRoot
        EvidenceRoot = $evidenceRoot
    }
    Set-Content -LiteralPath $configPath -Value ($config | ConvertTo-Json -Compress) -Encoding utf8
    & icacls.exe $configPath /grant "*${userSid}:R" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "could not grant the ordinary QA account access to its test configuration"
    }

    $credential = [Management.Automation.PSCredential]::new("$env:COMPUTERNAME\$qaName", $password)
    $powershell = (Get-Process -Id $PID).Path
    $arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Child -ConfigPath `"$configPath`""
    $process = Start-Process -FilePath $powershell -Credential $credential -LoadUserProfile -ArgumentList $arguments -WorkingDirectory $repositoryRoot -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "ordinary-user native filesystem smoke failed (exit $($process.ExitCode)); see $evidenceRoot"
    }

    $smokeManifestPath = Join-Path $evidenceRoot "native-file-hardening-smoke.json"
    if (-not (Test-Path -LiteralPath $smokeManifestPath)) {
        throw "ordinary-user native filesystem smoke did not write its manifest"
    }
    $smokeManifest = Get-Content -LiteralPath $smokeManifestPath -Raw | ConvertFrom-Json
    if (-not $smokeManifest.cross_volume_exercised -or $smokeManifest.token_is_administrator -or $smokeManifest.token_security_privilege_enabled) {
        throw "native smoke manifest does not prove distinct volumes and a non-admin token without enabled SeSecurityPrivilege"
    }
    Write-Host "Ordinary-user native filesystem evidence: $smokeManifestPath"
}
finally {
    if (Test-Path -LiteralPath $configPath) {
        Remove-Item -LiteralPath $configPath -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath $qaRoot) {
        Remove-Item -LiteralPath $qaRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    $secondaryParent = Split-Path -Parent $secondaryRoot
    if (Test-Path -LiteralPath $secondaryParent) {
        Remove-Item -LiteralPath $secondaryParent -Recurse -Force -ErrorAction SilentlyContinue
    }
    if ($userCreated) {
        Remove-LocalUser -Name $qaName -ErrorAction SilentlyContinue
        if ($userSid) {
            Get-CimInstance Win32_UserProfile -Filter "SID='$userSid'" -ErrorAction SilentlyContinue |
                Remove-CimInstance -ErrorAction SilentlyContinue
        }
    }
}
