[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:S1Head = 'bd56788aa317d808dadf930260e191e9d1b45adb'
$script:S0Head = '1f4951f3c93b2eb549d159fed2dd288992c625c8'
$script:ProbePath = 'scripts/issue380S0NsisObservabilityProbe.nsi'
$script:ExpectedBranch = 'research/issue-380-s1r-nsis-zip-trace-persistence'
$script:ZipUrl = 'https://sourceforge.net/projects/nsis/files/NSIS%203/3.11/nsis-3.11.zip/download'
$script:ExpectedZipSha256 = 'c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1'
$script:RegistrySubKey = 'Software\ZenCanvasS0Harness'
$script:RegistryValueName = 'EstimatedSize'
$script:ExpectedValue = 35150
$script:ActualValue = 35149
$script:ExpectedInstallerExitCode = 37
$script:ExpectedUninstallerExitCode = 42
$script:Failure = $null
$script:CanDeleteSyntheticKey = $false
$script:SyntheticKeyChecked = $false
$script:Assertions = [System.Collections.Generic.List[object]]::new()
$script:Executables = [System.Collections.Generic.List[object]]::new()
$script:Processes = [System.Collections.Generic.List[object]]::new()
$script:Acquisition = [ordered]@{
    source_url = $script:ZipUrl
    expected_sha256 = $script:ExpectedZipSha256
    final_url = $null
    http_status = $null
    content_length_header = $null
    downloaded_bytes = 0
    downloaded_sha256 = $null
    content_type = $null
    error_body_path = $null
    error_body_truncated = $false
    sha256_verified_before_extract = $false
    extraction_status = 'NOT_STARTED'
    failure = $null
}
$script:Compiler = $null
$script:LogComparison = [System.Collections.Generic.List[object]]::new()
$script:RegistrySnapshots = [System.Collections.Generic.List[object]]::new()

$outputFullPath = [System.IO.Path]::GetFullPath($OutputRoot)
$taskRoot = Join-Path $env:RUNNER_TEMP 'issue380-s1r-task'
$expectedEvidenceRoot = [System.IO.Path]::GetFullPath((Join-Path $env:RUNNER_TEMP 'issue380-s1r-evidence'))
$taskMarker = Join-Path $taskRoot 's1r-task-owned.txt'
$syntheticKeyOwnerMarker = Join-Path $outputFullPath 'synthetic-key-owned.txt'
$zipPath = Join-Path $taskRoot 'nsis-3.11.zip'
$extractionRoot = Join-Path $taskRoot 'nsis-3.11-extracted'

if (-not $outputFullPath.Equals($expectedEvidenceRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Evidence output must be inside the designated task temp directory: expected=$expectedEvidenceRoot actual=$outputFullPath"
}

if (Test-Path -LiteralPath $outputFullPath) {
    $existing = @(Get-ChildItem -LiteralPath $outputFullPath -Force -ErrorAction Stop)
    $unexpectedExisting = @($existing | Where-Object { $_.Name -ne 'preflight.json' })
    if ($unexpectedExisting.Count -ne 0) { throw "Evidence directory contains unexpected files: $($unexpectedExisting.Name -join ', ')" }
} else {
    New-Item -ItemType Directory -Path $outputFullPath -ErrorAction Stop | Out-Null
}

function Write-Utf8Text([string]$Path, [string]$Text) {
    $parent = Split-Path -Parent $Path
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force -ErrorAction Stop | Out-Null
    }
    [System.IO.File]::WriteAllText($Path, $Text, [System.Text.UTF8Encoding]::new($false))
}

function Write-Utf8Json([string]$Path, [object]$Value) {
    Write-Utf8Text -Path $Path -Text ((ConvertTo-Json -InputObject $Value -Depth 30) + "`n")
}

function Add-Assertion([string]$Name, [bool]$Passed, [string]$Details) {
    $script:Assertions.Add([ordered]@{ name = $Name; passed = $Passed; details = $Details })
    if (-not $Passed) { throw "Assertion failed: $Name — $Details" }
}

function Invoke-CapturedProcess(
    [string]$FilePath,
    [string[]]$ArgumentList,
    [string]$WorkingDirectory,
    [string]$StdoutPath,
    [string]$StderrPath,
    [int]$TimeoutSeconds = 45
) {
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $ArgumentList) { [void]$startInfo.ArgumentList.Add($argument) }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "Could not start process: $FilePath" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $completed = $process.WaitForExit($TimeoutSeconds * 1000)
    if (-not $completed) {
        try { $process.Kill($true) } catch { }
        $process.WaitForExit()
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    Write-Utf8Text -Path $StdoutPath -Text $stdout
    Write-Utf8Text -Path $StderrPath -Text $stderr

    $record = [ordered]@{
        file = $FilePath
        arguments = @($ArgumentList)
        working_directory = $WorkingDirectory
        timed_out = (-not $completed)
        exit_code = [int]$process.ExitCode
        stdout_path = $StdoutPath
        stderr_path = $StderrPath
        stdout_utf8 = $stdout
        stderr_utf8 = $stderr
    }
    $script:Processes.Add($record)
    $process.Dispose()
    return $record
}

function Get-SyntheticRegistrySnapshot {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::CurrentUser,
        [Microsoft.Win32.RegistryView]::Registry64
    )
    try {
        $key = $base.OpenSubKey($script:RegistrySubKey, $false)
        if ($null -eq $key) {
            return [ordered]@{ exists = $false; value_name = $script:RegistryValueName; kind = $null; value = $null }
        }
        try {
            if ($key.GetValueNames() -notcontains $script:RegistryValueName) {
                return [ordered]@{ exists = $true; value_name = $script:RegistryValueName; kind = 'MISSING'; value = $null }
            }
            return [ordered]@{
                exists = $true
                value_name = $script:RegistryValueName
                kind = $key.GetValueKind($script:RegistryValueName).ToString()
                value = $key.GetValue($script:RegistryValueName, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            }
        } finally { $key.Dispose() }
    } finally { $base.Dispose() }
}

function Add-ExecutableEvidence([string]$SourcePath, [string]$EvidencePath, [string]$Role, [bool]$AllowEmpty = $false) {
    if (-not (Test-Path -LiteralPath $SourcePath -PathType Leaf)) { throw "Expected executable is missing: $SourcePath" }
    $sourceItem = Get-Item -LiteralPath $SourcePath
    if (-not $AllowEmpty -and $sourceItem.Length -le 0) { throw "Executable is empty: $SourcePath" }
    Copy-Item -LiteralPath $SourcePath -Destination $EvidencePath -Force
    $item = Get-Item -LiteralPath $EvidencePath
    $hash = (Get-FileHash -LiteralPath $EvidencePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Executables.Add([ordered]@{
        role = $Role
        evidence_path = $EvidencePath.Substring($outputFullPath.Length).TrimStart('\', '/').Replace('\', '/')
        source_path = $SourcePath
        length_bytes = [long]$item.Length
        sha256 = $hash
    })
}

function Assert-ProcessExit([string]$Name, [object]$Result, [int]$ExpectedCode) {
    Add-Assertion -Name $Name -Passed ((-not $Result.timed_out) -and ($Result.exit_code -eq $ExpectedCode)) `
        -Details "expected=$ExpectedCode actual=$($Result.exit_code) timed_out=$($Result.timed_out) stdout=$($Result.stdout_path) stderr=$($Result.stderr_path)"
}

function Get-FileSha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Download-And-VerifyPinnedZip {
    if (Test-Path -LiteralPath $taskRoot) { throw "Task temp directory already exists: $taskRoot" }
    New-Item -ItemType Directory -Path $taskRoot -ErrorAction Stop | Out-Null
    Write-Utf8Text -Path $taskMarker -Text "Created by run $env:GITHUB_RUN_ID attempt $env:GITHUB_RUN_ATTEMPT.`n"
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $true
    $handler.MaxAutomaticRedirections = 5
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(120)
    $client.DefaultRequestHeaders.UserAgent.ParseAdd('Zen-Canvas-Issue-380-S1R/1.0')
    $response = $null
    $inputStream = $null
    $outputStream = $null
    try {
        $response = $client.GetAsync($script:ZipUrl, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
        $script:Acquisition.http_status = [int]$response.StatusCode
        if ($null -ne $response.RequestMessage -and $null -ne $response.RequestMessage.RequestUri) {
            $script:Acquisition.final_url = $response.RequestMessage.RequestUri.AbsoluteUri
        }
        $script:Acquisition.content_length_header = $response.Content.Headers.ContentLength
        if ($null -ne $response.Content.Headers.ContentType) { $script:Acquisition.content_type = $response.Content.Headers.ContentType.ToString() }
        if (-not $response.IsSuccessStatusCode) {
            $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
            $bodyMemory = [System.IO.MemoryStream]::new()
            $errorBuffer = [byte[]]::new(8192)
            $bodyBytes = 0
            while ($bodyBytes -lt 65536 -and ($read = $inputStream.Read($errorBuffer, 0, [Math]::Min($errorBuffer.Length, 65536 - $bodyBytes))) -gt 0) {
                $bodyMemory.Write($errorBuffer, 0, $read)
                $bodyBytes += $read
            }
            if ($bodyBytes -eq 65536) { $script:Acquisition.error_body_truncated = $true }
            $bodyPath = Join-Path $outputFullPath 'download-http-error-body.txt'
            Write-Utf8Text -Path $bodyPath -Text ([System.Text.Encoding]::UTF8.GetString($bodyMemory.ToArray()))
            $script:Acquisition.error_body_path = 'download-http-error-body.txt'
            $bodyMemory.Dispose()
            throw "NSIS ZIP download returned HTTP $([int]$response.StatusCode)"
        }
        if ($null -ne $response.Content.Headers.ContentLength -and $response.Content.Headers.ContentLength -gt 33554432) {
            throw "NSIS ZIP Content-Length exceeds 32 MiB: $($response.Content.Headers.ContentLength)"
        }

        $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
        $outputStream = [System.IO.FileStream]::new($zipPath, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
        $buffer = [byte[]]::new(65536)
        $total = [long]0
        while (($read = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $total += $read
            if ($total -gt 33554432) { throw "NSIS ZIP download exceeds 32 MiB: $total" }
            $outputStream.Write($buffer, 0, $read)
        }
        $outputStream.Flush($true)
        $script:Acquisition.downloaded_bytes = $total
    } catch {
        $script:Acquisition.failure = $_.Exception.ToString()
        throw
    } finally {
        if ($null -ne $outputStream) { $outputStream.Dispose() }
        if ($null -ne $inputStream) { $inputStream.Dispose() }
        if ($null -ne $response) { $response.Dispose() }
        $client.Dispose()
        $handler.Dispose()
        if (Test-Path -LiteralPath $zipPath -PathType Leaf) {
            $script:Acquisition.downloaded_bytes = (Get-Item -LiteralPath $zipPath).Length
            $script:Acquisition.downloaded_sha256 = Get-FileSha256 $zipPath
            Copy-Item -LiteralPath $zipPath -Destination (Join-Path $outputFullPath 'nsis-3.11-download.zip') -Force
        }
        Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
    }

    if (-not (Test-Path -LiteralPath $zipPath -PathType Leaf)) { throw 'NSIS ZIP download produced no file.' }
    $actualHash = Get-FileSha256 $zipPath
    $script:Acquisition.downloaded_sha256 = $actualHash
    if ($actualHash -ne $script:ExpectedZipSha256) {
        $script:Acquisition.failure = "SHA256 mismatch: expected=$($script:ExpectedZipSha256) actual=$actualHash"
        Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
        throw $script:Acquisition.failure
    }
    if ((Get-Item -LiteralPath $zipPath).Length -le 0) { throw 'NSIS ZIP is empty.' }
    $script:Acquisition.sha256_verified_before_extract = $true
    $script:Acquisition.failure = $null
    Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
}

function Expand-VerifiedZipSafely {
    if (-not $script:Acquisition.sha256_verified_before_extract) { throw 'Refusing to extract an NSIS ZIP without a verified SHA-256.' }
    if (Test-Path -LiteralPath $extractionRoot) { throw "Extraction directory already exists: $extractionRoot" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        if ($archive.Entries.Count -gt 2000) { throw "NSIS ZIP has too many entries: $($archive.Entries.Count)" }
        $uncompressedTotal = [long]0
        $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $archive.Entries) {
            $normalized = $entry.FullName.Replace('\', '/')
            if ([string]::IsNullOrWhiteSpace($normalized) -or $normalized.StartsWith('/') -or $normalized -match '^[A-Za-z]:' ) {
                throw "Unsafe ZIP entry path: $($entry.FullName)"
            }
            $segments = $normalized.Split('/')
            if ($segments -contains '..' -or $segments -contains '.') { throw "Unsafe ZIP traversal entry: $($entry.FullName)" }
            if (-not $seen.Add($normalized)) { throw "Duplicate ZIP entry path: $normalized" }
            $mode = ($entry.ExternalAttributes -shr 16) -band 0xF000
            if ($mode -eq 0xA000) { throw "Symbolic link is not allowed in NSIS ZIP: $normalized" }
            $uncompressedTotal += [long]$entry.Length
            if ($uncompressedTotal -gt 536870912) { throw "NSIS ZIP expands beyond 512 MiB: $uncompressedTotal" }
        }
    } finally { $archive.Dispose() }

    New-Item -ItemType Directory -Path $extractionRoot -ErrorAction Stop | Out-Null
    try {
        [System.IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $extractionRoot)
        $script:Acquisition.extraction_status = 'EXTRACTED_AFTER_SHA256_MATCH'
        Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
    } catch {
        $script:Acquisition.extraction_status = 'FAILED'
        $script:Acquisition.failure = $_.Exception.ToString()
        Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
        throw
    }
}

function Compile-Probe([string]$Makensis, [string]$Probe, [string]$TraceMode, [string]$RunMode, [string]$OutputName, [string]$WorkingDirectory, [string]$EvidenceLogStem) {
    $args = @('-NOCD', '-V2', "-DTRACE_MODE=$TraceMode", "-DRUN_MODE=$RunMode", "-DOUTPUT_NAME=$OutputName", $Probe)
    $compile = Invoke-CapturedProcess -FilePath $Makensis -ArgumentList $args -WorkingDirectory $WorkingDirectory `
        -StdoutPath (Join-Path $outputFullPath "$EvidenceLogStem.stdout.txt") `
        -StderrPath (Join-Path $outputFullPath "$EvidenceLogStem.stderr.txt") -TimeoutSeconds 60
    $outputPath = Join-Path $WorkingDirectory $OutputName
    $evidencePath = Join-Path (Join-Path $outputFullPath $TraceMode) $OutputName
    $outputExists = Test-Path -LiteralPath $outputPath -PathType Leaf
    $outputLength = 0
    if ($outputExists) {
        Add-ExecutableEvidence -SourcePath $outputPath -EvidencePath $evidencePath `
            -Role "compiler_output_${TraceMode}_${RunMode}" -AllowEmpty $true
        $outputLength = (Get-Item -LiteralPath $outputPath).Length
    }
    Add-Assertion -Name "compile_${TraceMode}_${RunMode}_exit_zero_and_output_present" `
        -Passed ((-not $compile.timed_out) -and ($compile.exit_code -eq 0) -and $outputExists -and ($outputLength -gt 0)) `
        -Details "exit=$($compile.exit_code) timed_out=$($compile.timed_out) output_exists=$outputExists output_bytes=$outputLength stdout=$EvidenceLogStem.stdout.txt stderr=$EvidenceLogStem.stderr.txt"
    return $outputPath
}

$result = [ordered]@{
    phase = '#380-S1R Synthetic NSIS Runtime Trace Persistence'
    status = 'IN_PROGRESS'
    repository = $env:GITHUB_REPOSITORY
    ref = $env:GITHUB_REF
    workflow_sha = $env:GITHUB_SHA
    run_id = $env:GITHUB_RUN_ID
    run_attempt = $env:GITHUB_RUN_ATTEMPT
    job = $env:GITHUB_JOB
    frozen_s1_head = $script:S1Head
    frozen_s0_head = $script:S0Head
    tested_head = $null
    tested_tree = $null
    frozen_probe_blob = $null
    environment = [ordered]@{
        runner_os = $env:RUNNER_OS
        runner_arch = $env:RUNNER_ARCH
        image_os = $env:ImageOS
        image_version = $env:ImageVersion
        powershell = $PSVersionTable.PSVersion.ToString()
        windows_version = [System.Environment]::OSVersion.VersionString
    }
    acquisition = $script:Acquisition
    compiler = $null
    compile_options = 'makensis.exe -NOCD -V2 -DTRACE_MODE=<detail|file> -DRUN_MODE=<stage|abort> -DOUTPUT_NAME=<name> <frozen S0 probe>'
    executables = @()
    processes = @()
    registry_snapshots = @()
    log_comparison = @()
    assertions = @()
    cleanup = [ordered]@{ registry_key = 'NOT_CHECKED'; task_temp = 'WORKFLOW_CLEANUP_PENDING' }
    failure = $null
}

try {
    if ($env:GITHUB_REPOSITORY -ne 'ArdenZC/Zen-Canvas') { throw "Unexpected repository identity: $env:GITHUB_REPOSITORY" }
    if ($env:GITHUB_REF -ne "refs/heads/$($script:ExpectedBranch)") { throw "Unexpected workflow ref: $env:GITHUB_REF" }
    if ($env:GITHUB_RUN_ATTEMPT -ne '1') { throw "One-shot gate refuses run attempt $env:GITHUB_RUN_ATTEMPT" }
    if ($env:PREFLIGHT_OUTCOME -ne 'success') { throw "Static Hosted preflight did not pass: $env:PREFLIGHT_OUTCOME" }

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
    Push-Location $repoRoot
    try {
        $head = (& git rev-parse HEAD).Trim()
        $tree = (& git rev-parse 'HEAD^{tree}').Trim()
        if ($LASTEXITCODE -ne 0) { throw 'git could not resolve the checked out source identity.' }
        if ($head -ne $env:GITHUB_SHA) { throw "Checkout HEAD $head differs from workflow SHA $env:GITHUB_SHA" }
        & git merge-base --is-ancestor $script:S1Head HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Frozen S1 HEAD is not an ancestor of this S1R candidate.' }
        $probeBlob = (& git rev-parse "$($script:S0Head):$($script:ProbePath)").Trim()
        $actualProbeBlob = (& git hash-object $script:ProbePath).Trim()
        if ($LASTEXITCODE -ne 0 -or $probeBlob -ne $actualProbeBlob) { throw 'Frozen S0 Probe blob changed.' }
        $result.tested_head = $head
        $result.tested_tree = $tree
        $result.frozen_probe_blob = $probeBlob
    } finally { Pop-Location }

    $initialRegistry = Get-SyntheticRegistrySnapshot
    $script:SyntheticKeyChecked = $true
    $script:CanDeleteSyntheticKey = (-not $initialRegistry.exists)
    $script:RegistrySnapshots.Add([ordered]@{ stage = 'initial'; snapshot = $initialRegistry })
    Add-Assertion -Name 'synthetic_hkcu_key_initially_absent' -Passed $script:CanDeleteSyntheticKey `
        -Details "key=HKCU\$($script:RegistrySubKey) exists=$($initialRegistry.exists); existing state is never overwritten or deleted."
    Write-Utf8Text -Path $syntheticKeyOwnerMarker -Text "Created only after proving HKCU\$($script:RegistrySubKey) was absent.`n"

    Download-And-VerifyPinnedZip
    Expand-VerifiedZipSafely

    $makensisMatches = @(Get-ChildItem -LiteralPath $extractionRoot -Filter 'makensis.exe' -File -Recurse -ErrorAction Stop)
    $script:Compiler = [ordered]@{
        candidate_paths = @($makensisMatches | ForEach-Object { $_.FullName })
        candidate_count = $makensisMatches.Count
    }
    if ($makensisMatches.Count -ne 1) { throw "Expected exactly one makensis.exe in the verified archive; found $($makensisMatches.Count)." }
    $makensis = $makensisMatches[0].FullName
    $extractPrefix = [System.IO.Path]::GetFullPath($extractionRoot) + [System.IO.Path]::DirectorySeparatorChar
    if (-not ([System.IO.Path]::GetFullPath($makensis).StartsWith($extractPrefix, [System.StringComparison]::OrdinalIgnoreCase))) {
        throw "makensis.exe escaped the task extraction directory: $makensis"
    }
    $makensisHash = Get-FileSha256 $makensis
    $identityRoot = Join-Path $outputFullPath 'compiler'
    New-Item -ItemType Directory -Path $identityRoot -ErrorAction Stop | Out-Null
    $script:Compiler = [ordered]@{
        path = $makensis
        sha256 = $makensisHash
        version_output = $null
        hdrinfo_output = $null
        version_exit = $null
        hdrinfo_exit = $null
        x86 = $false
        unicode = $false
        source_zip_sha256 = $script:Acquisition.downloaded_sha256
    }
    $versionResult = Invoke-CapturedProcess -FilePath $makensis -ArgumentList @('-VERSION') -WorkingDirectory $taskRoot `
        -StdoutPath (Join-Path $identityRoot 'makensis-version.stdout.txt') `
        -StderrPath (Join-Path $identityRoot 'makensis-version.stderr.txt')
    $versionText = ($versionResult.stdout_utf8 + $versionResult.stderr_utf8).Trim()
    $script:Compiler.version_output = $versionText
    $script:Compiler.version_exit = $versionResult.exit_code
    Add-Assertion -Name 'makensis_version_is_exactly_3_11' `
        -Passed ((-not $versionResult.timed_out) -and ($versionResult.exit_code -eq 0) -and ($versionText -match '(?m)^\s*v?3\.11(?:\s|$)')) `
        -Details "path=$makensis version=$versionText sha256=$makensisHash"

    $headerResult = Invoke-CapturedProcess -FilePath $makensis -ArgumentList @('-HDRINFO') -WorkingDirectory $taskRoot `
        -StdoutPath (Join-Path $identityRoot 'makensis-hdrinfo.stdout.txt') `
        -StderrPath (Join-Path $identityRoot 'makensis-hdrinfo.stderr.txt')
    $headerText = $headerResult.stdout_utf8 + $headerResult.stderr_utf8
    $script:Compiler.hdrinfo_output = $headerText
    $script:Compiler.hdrinfo_exit = $headerResult.exit_code
    $script:Compiler.x86 = ($headerText -match 'NSIS_CPU=x86')
    $script:Compiler.unicode = ($headerText -match 'NSIS_UNICODE')
    Add-Assertion -Name 'makensis_is_x86_unicode' `
        -Passed ((-not $headerResult.timed_out) -and ($headerResult.exit_code -eq 0) -and ($headerText -match 'NSIS_CPU=x86') -and ($headerText -match 'NSIS_UNICODE')) `
        -Details 'NSIS -HDRINFO must identify NSIS_CPU=x86 and NSIS_UNICODE.'

    $probePath = Join-Path $repoRoot $script:ProbePath
    foreach ($traceMode in @('detail', 'file')) {
        $modeTaskRoot = Join-Path (Join-Path $taskRoot 'compiled') $traceMode
        $modeEvidenceRoot = Join-Path $outputFullPath $traceMode
        New-Item -ItemType Directory -Path $modeTaskRoot -Force -ErrorAction Stop | Out-Null
        New-Item -ItemType Directory -Path $modeEvidenceRoot -Force -ErrorAction Stop | Out-Null

        $stageExe = Compile-Probe -Makensis $makensis -Probe $probePath -TraceMode $traceMode -RunMode 'stage' `
            -OutputName 'stage.exe' -WorkingDirectory $modeTaskRoot -EvidenceLogStem "$traceMode-compile-stage"
        $abortExe = Compile-Probe -Makensis $makensis -Probe $probePath -TraceMode $traceMode -RunMode 'abort' `
            -OutputName 'installer-abort.exe' -WorkingDirectory $modeTaskRoot -EvidenceLogStem "$traceMode-compile-installer-abort"

        $stageProcess = Invoke-CapturedProcess -FilePath $stageExe -ArgumentList @('/S') -WorkingDirectory $modeTaskRoot `
            -StdoutPath (Join-Path $modeEvidenceRoot 'stage.stdout.txt') -StderrPath (Join-Path $modeEvidenceRoot 'stage.stderr.txt')
        Assert-ProcessExit -Name "${traceMode}_stage_exit_zero" -Result $stageProcess -ExpectedCode 0
        $generatedUninstaller = Join-Path $modeTaskRoot 'generated-uninstaller.exe'
        Add-ExecutableEvidence -SourcePath $generatedUninstaller `
            -EvidencePath (Join-Path $modeEvidenceRoot 'generated-uninstaller.exe') -Role "generated_uninstaller_$traceMode"

        $afterStage = Get-SyntheticRegistrySnapshot
        $script:RegistrySnapshots.Add([ordered]@{ stage = "${traceMode}_after_stage"; snapshot = $afterStage })
        Add-Assertion -Name "${traceMode}_stage_wrote_synthetic_dword" `
            -Passed (($afterStage.exists) -and ($afterStage.kind -eq 'DWord') -and ([int]$afterStage.value -eq $script:ActualValue)) `
            -Details "expected_kind=DWord expected_value=$($script:ActualValue) actual_kind=$($afterStage.kind) actual_value=$($afterStage.value)"

        $installerLog = Join-Path $modeEvidenceRoot 'installer-runtime.log'
        $installerAbort = Invoke-CapturedProcess -FilePath $abortExe -ArgumentList @('/S', "/LOG=$installerLog") `
            -WorkingDirectory $modeTaskRoot -StdoutPath (Join-Path $modeEvidenceRoot 'installer-abort.stdout.txt') `
            -StderrPath (Join-Path $modeEvidenceRoot 'installer-abort.stderr.txt')
        Assert-ProcessExit -Name "${traceMode}_installer_abort_expected_exit" -Result $installerAbort -ExpectedCode $script:ExpectedInstallerExitCode

        $uninstallerLog = Join-Path $modeEvidenceRoot 'uninstaller-runtime.log'
        $uninstallerAbort = Invoke-CapturedProcess -FilePath $generatedUninstaller -ArgumentList @('/S', "/LOG=$uninstallerLog") `
            -WorkingDirectory $modeTaskRoot -StdoutPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stdout.txt') `
            -StderrPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stderr.txt')
        Assert-ProcessExit -Name "${traceMode}_uninstaller_abort_expected_exit" -Result $uninstallerAbort -ExpectedCode $script:ExpectedUninstallerExitCode

        $afterAborts = Get-SyntheticRegistrySnapshot
        $script:RegistrySnapshots.Add([ordered]@{ stage = "${traceMode}_after_aborts"; snapshot = $afterAborts })
        Add-Assertion -Name "${traceMode}_abort_paths_preserved_synthetic_dword" `
            -Passed (($afterAborts.exists) -and ($afterAborts.kind -eq 'DWord') -and ([int]$afterAborts.value -eq $script:ActualValue)) `
            -Details "expected_kind=DWord expected_value=$($script:ActualValue) actual_kind=$($afterAborts.kind) actual_value=$($afterAborts.value)"

        $detailTracePath = Join-Path $modeTaskRoot 'direct-evidence.log'
        $evidenceTracePath = Join-Path $modeEvidenceRoot 'direct-evidence.log'
        if (Test-Path -LiteralPath $detailTracePath -PathType Leaf) {
            Copy-Item -LiteralPath $detailTracePath -Destination $evidenceTracePath -Force
        }
        $directLines = @()
        if (Test-Path -LiteralPath $evidenceTracePath -PathType Leaf) { $directLines = @(Get-Content -LiteralPath $evidenceTracePath) }
        $script:LogComparison.Add([ordered]@{
            mode = $traceMode
            installer_log_exists = (Test-Path -LiteralPath $installerLog -PathType Leaf)
            installer_log_sha256 = if (Test-Path -LiteralPath $installerLog -PathType Leaf) { Get-FileSha256 $installerLog } else { $null }
            installer_log_contains_failure = if (Test-Path -LiteralPath $installerLog -PathType Leaf) { (Get-Content -Raw -LiteralPath $installerLog).Contains('first_failure=EstimatedSize') } else { $false }
            uninstaller_log_exists = (Test-Path -LiteralPath $uninstallerLog -PathType Leaf)
            uninstaller_log_sha256 = if (Test-Path -LiteralPath $uninstallerLog -PathType Leaf) { Get-FileSha256 $uninstallerLog } else { $null }
            uninstaller_log_contains_failure = if (Test-Path -LiteralPath $uninstallerLog -PathType Leaf) { (Get-Content -Raw -LiteralPath $uninstallerLog).Contains('uninstaller_first_failure=EstimatedSize') } else { $false }
            direct_file_trace_exists = (Test-Path -LiteralPath $evidenceTracePath -PathType Leaf)
            direct_file_trace_lines = $directLines
        })
        Add-Assertion -Name "${traceMode}_abort_stdout_and_stderr_saved" `
            -Passed ((Test-Path -LiteralPath (Join-Path $modeEvidenceRoot 'installer-abort.stdout.txt')) -and
                (Test-Path -LiteralPath (Join-Path $modeEvidenceRoot 'installer-abort.stderr.txt')) -and
                (Test-Path -LiteralPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stdout.txt')) -and
                (Test-Path -LiteralPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stderr.txt'))) `
            -Details 'Both Abort process stdout/stderr files must be retained.'
    }

    $fileTracePath = Join-Path (Join-Path $outputFullPath 'file') 'direct-evidence.log'
    Add-Assertion -Name 'direct_file_trace_exists_after_both_abort_processes_exit' `
        -Passed ((Test-Path -LiteralPath $fileTracePath -PathType Leaf) -and ((Get-Item -LiteralPath $fileTracePath).Length -gt 0)) `
        -Details "trace_path=$fileTracePath; both abort processes have exited before this check."
    $traceLines = @(Get-Content -LiteralPath $fileTracePath)
    $requiredFields = @(
        'predicate=HKCU\Software\ZenCanvasS0Harness\EstimatedSize',
        'type=REG_DWORD',
        'expected=35150',
        'actual=35149',
        'result=FAIL'
    )
    foreach ($context in @('installer', 'uninstaller')) {
        $contextLines = @($traceLines | Where-Object { $_ -match "(^|\s)context=$context(\s|$)" })
        $complete = ($contextLines.Count -eq 1)
        if ($complete) {
            foreach ($field in $requiredFields) { if (-not $contextLines[0].Contains($field)) { $complete = $false } }
        }
        Add-Assertion -Name "file_trace_${context}_record_complete" -Passed $complete `
            -Details "exactly one complete context=$context record required; matching_lines=$($contextLines -join ' | ')"
    }
    Add-Assertion -Name 'file_trace_has_exactly_two_abort_records' -Passed ($traceLines.Count -eq 2) `
        -Details "expected=2 actual=$($traceLines.Count)"
    $compilerOutputs = @($script:Executables | Where-Object { $_.role -like 'compiler_output_*' })
    $generatedUninstallers = @($script:Executables | Where-Object { $_.role -like 'generated_uninstaller_*' })
    Add-Assertion -Name 'all_four_compiler_outputs_and_two_uninstallers_hashed' `
        -Passed (($compilerOutputs.Count -eq 4) -and ($generatedUninstallers.Count -eq 2)) `
        -Details "compiler_outputs=$($compilerOutputs.Count) generated_uninstallers=$($generatedUninstallers.Count)"
} catch {
    $script:Failure = $_.Exception.ToString()
    if ($script:Acquisition.sha256_verified_before_extract -and $script:Acquisition.extraction_status -eq 'NOT_STARTED') {
        $script:Acquisition.extraction_status = 'REJECTED_OR_FAILED'
        $script:Acquisition.failure = $script:Failure
    } elseif (-not $script:Acquisition.sha256_verified_before_extract -and $null -eq $script:Acquisition.failure) {
        $script:Acquisition.failure = $script:Failure
    }
} finally {
    try {
        if ($script:CanDeleteSyntheticKey) {
            $cleanupBase = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
                [Microsoft.Win32.RegistryHive]::CurrentUser,
                [Microsoft.Win32.RegistryView]::Registry64
            )
            try {
                $cleanupBase.DeleteSubKeyTree($script:RegistrySubKey, $false)
                $remaining = Get-SyntheticRegistrySnapshot
                if ($remaining.exists) { throw "Synthetic HKCU key remains after cleanup: $($script:RegistrySubKey)" }
                $result.cleanup.registry_key = 'REMOVED_AND_VERIFIED_ABSENT'
            } finally { $cleanupBase.Dispose() }
        } elseif ($script:SyntheticKeyChecked) {
            $result.cleanup.registry_key = 'PREEXISTING_KEY_PRESERVED'
        } else {
            $result.cleanup.registry_key = 'NOT_CHECKED'
        }
    } catch {
        $cleanupFailure = $_.Exception.ToString()
        $result.cleanup.registry_key = "FAILED: $cleanupFailure"
        if ($null -eq $script:Failure) { $script:Failure = "Synthetic HKCU cleanup failed: $cleanupFailure" }
    }

    $result.acquisition = $script:Acquisition
    $result.compiler = $script:Compiler
    $result.executables = @($script:Executables.ToArray())
    $result.processes = @($script:Processes.ToArray())
    $result.registry_snapshots = @($script:RegistrySnapshots.ToArray())
    $result.log_comparison = @($script:LogComparison.ToArray())
    $result.assertions = @($script:Assertions.ToArray())
    try { Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition } catch { }
    if ($null -eq $script:Failure) {
        $result.status = 'PASS'
    } else {
        $result.status = 'FAIL'
        $result.failure = $script:Failure
        try { Write-Utf8Text -Path (Join-Path $outputFullPath 'failure.txt') -Text ($script:Failure + "`n") } catch { }
    }
    try {
        Write-Utf8Json -Path (Join-Path $outputFullPath 'results.json') -Value $result
        $sumLines = [System.Collections.Generic.List[string]]::new()
        foreach ($file in (Get-ChildItem -LiteralPath $outputFullPath -Recurse -File | Sort-Object FullName)) {
            if ($file.Name -eq 'SHA256SUMS') { continue }
            $relative = $file.FullName.Substring($outputFullPath.Length).TrimStart('\', '/').Replace('\', '/')
            $sumLines.Add("$(Get-FileSha256 $file.FullName)  $relative")
        }
        Write-Utf8Text -Path (Join-Path $outputFullPath 'SHA256SUMS') -Text (($sumLines -join "`n") + "`n")
    } catch {
        if ($null -eq $script:Failure) { $script:Failure = "Evidence finalization failed: $($_.Exception.ToString())" }
    }
}

if ($null -ne $script:Failure) {
    Write-Error -Message $script:Failure -ErrorAction Continue
    exit 1
}
exit 0
