[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:S1R2Head = '6fe01abae7640a93f4a694616d758fd54c714960'
$script:S1R2Tree = 'af88309c65999d73564cf3876bb5bd03a15bf597'
$script:S1RHead = '45a2dfe0e51cee5708dfaa5335affbad947c30c2'
$script:S1RTree = '9bcc5a2a03a8335d2cb2e81c3d497efb275b8724'
$script:S1Head = 'bd56788aa317d808dadf930260e191e9d1b45adb'
$script:S0Head = '1f4951f3c93b2eb549d159fed2dd288992c625c8'
$script:ProbePath = 'scripts/issue380S0NsisObservabilityProbe.nsi'
$script:ExpectedBranch = 'research/issue-380-s1r3-nsis-uninstaller-trace-verification'
$script:ZipUrl = 'https://sourceforge.net/projects/nsis/files/NSIS%203/3.11/nsis-3.11.zip/download'
$script:ExpectedZipSha256 = 'c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1'
$script:LauncherRelativePath = 'nsis-3.11/makensis.exe'
$script:LauncherLength = 2560L
$script:LauncherSha256 = 'f497e92deb9f179f7b8f7553fcb3bd04f511bb2949e5e4a2aee80a10f7b20431'
$script:CompilerRelativePath = 'nsis-3.11/Bin/makensis.exe'
$script:CompilerLength = 468992L
$script:CompilerSha256 = '42850802704ecb11163f7e0329d35ee54bd288953200d4966e226d572848cfc5'
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
    archive_entry_count = $null
    archive_uncompressed_bytes = $null
    validated_archive_members = @()
    extraction_root = $null
    extracted_members = @()
    extraction_status = 'NOT_STARTED'
    failure = $null
}
$script:Compiler = $null
$script:LogComparison = [System.Collections.Generic.List[object]]::new()
$script:RegistrySnapshots = [System.Collections.Generic.List[object]]::new()

$outputFullPath = [System.IO.Path]::GetFullPath($OutputRoot)
$taskRoot = [System.IO.Path]::GetFullPath((Join-Path $env:RUNNER_TEMP 'issue380-s1r3-task'))
$script:TaskRoot = $taskRoot
$expectedEvidenceRoot = [System.IO.Path]::GetFullPath((Join-Path $env:RUNNER_TEMP 'issue380-s1r3-evidence'))
$taskMarker = Join-Path $taskRoot 's1r3-task-owned.txt'
$syntheticKeyOwnerMarker = Join-Path $outputFullPath 'synthetic-key-owned.txt'
$zipPath = Join-Path $taskRoot 'nsis-3.11.zip'
$extractionRoot = Join-Path $taskRoot 'nsis-3.11-extracted'
$expectedArchiveFiles = [ordered]@{
    'nsis-3.11/makensis.exe' = [ordered]@{ length = $script:LauncherLength; sha256 = $script:LauncherSha256; role = 'forwarding_launcher' }
    'nsis-3.11/Bin/makensis.exe' = [ordered]@{ length = $script:CompilerLength; sha256 = $script:CompilerSha256; role = 'underlying_compiler' }
}

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

function Get-TaskOwnedProcessSnapshot {
    $root = [System.IO.Path]::GetFullPath($script:TaskRoot).TrimEnd('\', '/')
    $prefix = $root + [System.IO.Path]::DirectorySeparatorChar
    $matching = [System.Collections.Generic.List[object]]::new()
    foreach ($processInfo in (Get-CimInstance -ClassName Win32_Process -Property @('ProcessId', 'ExecutablePath') -ErrorAction Stop)) {
        if ([string]::IsNullOrWhiteSpace([string]$processInfo.ExecutablePath)) { continue }
        try { $imagePath = [System.IO.Path]::GetFullPath([string]$processInfo.ExecutablePath) }
        catch { continue }
        if ($imagePath.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            $matching.Add([ordered]@{
                process_id = [int]$processInfo.ProcessId
                image_path = $imagePath
            })
        }
    }
    return @($matching.ToArray())
}

function Wait-ForTaskOwnedProcessesToExit([int]$TimeoutSeconds = 8) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $snapshot = @(Get-TaskOwnedProcessSnapshot)
        if ($snapshot.Count -eq 0) { return $snapshot }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    return @(Get-TaskOwnedProcessSnapshot)
}

function Invoke-CapturedProcess(
    [string]$FilePath,
    [string[]]$ArgumentList,
    [string]$WorkingDirectory,
    [string]$StdoutPath,
    [string]$StderrPath,
    [int]$TimeoutSeconds = 45
) {
    $fileFullPath = [System.IO.Path]::GetFullPath($FilePath)
    $workingDirectoryFullPath = [System.IO.Path]::GetFullPath($WorkingDirectory)
    $taskRootPrefix = [System.IO.Path]::GetFullPath($script:TaskRoot).TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if (-not $fileFullPath.StartsWith($taskRootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to launch a process outside the task-private temp root: $fileFullPath"
    }
    if (-not $workingDirectoryFullPath.StartsWith($taskRootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to use a working directory outside the task-private temp root: $workingDirectoryFullPath"
    }
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $fileFullPath
    $startInfo.WorkingDirectory = $workingDirectoryFullPath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.Environment['TEMP'] = $workingDirectoryFullPath
    $startInfo.Environment['TMP'] = $workingDirectoryFullPath
    foreach ($argument in $ArgumentList) { [void]$startInfo.ArgumentList.Add($argument) }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "Could not start process: $FilePath" }
    $processId = $process.Id
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $completed = $process.WaitForExit($TimeoutSeconds * 1000)
    if (-not $completed) {
        try { $process.Kill($true) } catch { }
        $process.WaitForExit()
    }
    $processExited = $process.HasExited
    $exitCode = if ($processExited) { [int]$process.ExitCode } else { $null }
    $stdoutComplete = $stdoutTask.Wait(5000)
    $stderrComplete = $stderrTask.Wait(5000)
    if (-not $stdoutComplete -or -not $stderrComplete) {
        try { $process.StandardOutput.Close() } catch { }
        try { $process.StandardError.Close() } catch { }
        try { [void]$stdoutTask.Wait(1000) } catch { }
        try { [void]$stderrTask.Wait(1000) } catch { }
    }
    $stdout = if ($stdoutComplete) { $stdoutTask.GetAwaiter().GetResult() } else { '[stdout stream did not close within 5 seconds after process wait]' }
    $stderr = if ($stderrComplete) { $stderrTask.GetAwaiter().GetResult() } else { '[stderr stream did not close within 5 seconds after process wait]' }
    Write-Utf8Text -Path $StdoutPath -Text $stdout
    Write-Utf8Text -Path $StderrPath -Text $stderr

    $remainingTaskProcesses = @()
    $processCheckFailure = $null
    try { $remainingTaskProcesses = @(Wait-ForTaskOwnedProcessesToExit) }
    catch { $processCheckFailure = $_.Exception.ToString() }
    $record = [ordered]@{
        file = $fileFullPath
        process_image_path = $fileFullPath
        file_sha256 = if (Test-Path -LiteralPath $fileFullPath -PathType Leaf) { Get-FileSha256 $fileFullPath } else { $null }
        process_id = $processId
        argument_list = @($ArgumentList)
        arguments = @($ArgumentList)
        working_directory = $workingDirectoryFullPath
        child_temp_directory = $workingDirectoryFullPath
        timed_out = (-not $completed)
        timeout_state = if ($completed) { 'COMPLETED_BEFORE_TIMEOUT' } else { 'TIMED_OUT_OWNED_PROCESS_TREE_TERMINATED' }
        process_exited = $processExited
        exit_code = $exitCode
        stdout_stream_completed = $stdoutComplete
        stderr_stream_completed = $stderrComplete
        process_check_failure = $processCheckFailure
        task_owned_processes_remaining = $remainingTaskProcesses
        all_task_owned_processes_exited = ($remainingTaskProcesses.Count -eq 0)
        stdout_path = $StdoutPath
        stderr_path = $StderrPath
        stdout_utf8 = $stdout
        stderr_utf8 = $stderr
    }
    $script:Processes.Add($record)
    $process.Dispose()
    Add-Assertion -Name "task_owned_processes_exited_after_pid_${processId}" `
        -Passed (($processExited) -and $stdoutComplete -and $stderrComplete -and ($null -eq $processCheckFailure) -and ($remainingTaskProcesses.Count -eq 0)) `
        -Details "process_id=$processId process_exited=$processExited stdout_complete=$stdoutComplete stderr_complete=$stderrComplete process_check_failure=$processCheckFailure remaining=$($remainingTaskProcesses | ConvertTo-Json -Compress -Depth 4)"
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
        -Details "expected=$ExpectedCode actual=$($Result.exit_code) timed_out=$($Result.timed_out) pid=$($Result.process_id) path=$($Result.process_image_path) args=$($Result.argument_list -join ' | ') stdout=$($Result.stdout_path) stderr=$($Result.stderr_path)"
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

function Get-ZipEntrySha256([System.IO.Compression.ZipArchiveEntry]$Entry) {
    $stream = $Entry.Open()
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try { return ([Convert]::ToHexString($sha.ComputeHash($stream))).ToLowerInvariant() }
    finally { $sha.Dispose(); $stream.Dispose() }
}

function Assert-ResolvedPathUnderRoot([string]$Root, [string]$Path, [string]$Label) {
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
    $pathFull = [System.IO.Path]::GetFullPath($Path)
    $prefix = $rootFull + [System.IO.Path]::DirectorySeparatorChar
    if (-not $pathFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label escaped the approved root: root=$rootFull path=$pathFull"
    }
    return $pathFull
}

function Expand-VerifiedZipSafely {
    if (-not $script:Acquisition.sha256_verified_before_extract) { throw 'Refusing to inspect or extract an NSIS ZIP without a verified whole-archive SHA-256.' }
    if (Test-Path -LiteralPath $extractionRoot) { throw "Extraction directory already exists: $extractionRoot" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
    $validatedMembers = [System.Collections.Generic.List[object]]::new()
    try {
        if ($archive.Entries.Count -gt 2000) { throw "NSIS ZIP has too many entries: $($archive.Entries.Count)" }
        $script:Acquisition.archive_entry_count = $archive.Entries.Count
        $uncompressedTotal = [long]0
        $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
        $matched = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        foreach ($entry in $archive.Entries) {
            $normalized = $entry.FullName.Replace('\', '/')
            $isDirectory = $normalized.EndsWith('/')
            $relative = $normalized.TrimEnd('/')
            if ([string]::IsNullOrWhiteSpace($relative) -or $relative.StartsWith('/') -or $relative -match '^[A-Za-z]:' -or $relative.Contains(':') -or $relative.Contains([char]0)) {
                throw "Unsafe ZIP entry path: $($entry.FullName)"
            }
            $segments = $relative.Split('/')
            if (($segments -contains '..') -or ($segments -contains '.') -or ($segments -contains '')) { throw "Unsafe ZIP traversal or empty path segment: $($entry.FullName)" }
            if ($relative -ne 'nsis-3.11' -and -not $relative.StartsWith('nsis-3.11/', [System.StringComparison]::Ordinal)) {
                throw "ZIP entry is outside the pinned nsis-3.11 root: $($entry.FullName)"
            }
            if (-not $seen.Add($relative)) { throw "Duplicate or case-colliding ZIP entry path: $relative" }
            $mode = ($entry.ExternalAttributes -shr 16) -band 0xF000
            if ($mode -eq 0xA000) { throw "Symbolic link is not allowed in NSIS ZIP: $relative" }
            if ($isDirectory -and $entry.Length -ne 0) { throw "Directory entry has unexpected payload bytes: $relative" }
            $uncompressedTotal += [long]$entry.Length
            if ($uncompressedTotal -gt 536870912) { throw "NSIS ZIP expands beyond 512 MiB: $uncompressedTotal" }

            if ($expectedArchiveFiles.Contains($relative)) {
                if ($isDirectory) { throw "Expected executable is encoded as a directory: $relative" }
                $expected = $expectedArchiveFiles[$relative]
                if ([long]$entry.Length -ne [long]$expected.length) {
                    throw "Pinned archive member length mismatch: path=$relative expected=$($expected.length) actual=$($entry.Length)"
                }
                $memberHash = Get-ZipEntrySha256 -Entry $entry
                if ($memberHash -ne $expected.sha256) {
                    throw "Pinned archive member SHA-256 mismatch: path=$relative expected=$($expected.sha256) actual=$memberHash"
                }
                [void]$matched.Add($relative)
                $validatedMembers.Add([ordered]@{
                    path = $relative
                    role = $expected.role
                    length_bytes = [long]$entry.Length
                    sha256 = $memberHash
                    verification = 'ZIP_MEMBER_PRE_EXTRACTION'
                })
            }
            if ($relative -match '(?i)(^|/)makensis\.exe$' -and -not $expectedArchiveFiles.Contains($relative)) {
                throw "Unapproved makensis.exe member path: $relative"
            }
        }
        foreach ($memberPath in $expectedArchiveFiles.Keys) {
            if (-not $matched.Contains($memberPath)) { throw "Required pinned archive member is missing: $memberPath" }
        }
        $script:Acquisition.archive_uncompressed_bytes = $uncompressedTotal
        $script:Acquisition.validated_archive_members = @($validatedMembers.ToArray())
    } finally { $archive.Dispose() }

    New-Item -ItemType Directory -Path $extractionRoot -ErrorAction Stop | Out-Null
    $taskRootFull = [System.IO.Path]::GetFullPath($taskRoot)
    $extractFull = Assert-ResolvedPathUnderRoot -Root $taskRootFull -Path $extractionRoot -Label 'Extraction directory'
    $script:Acquisition.extraction_root = $extractFull
    try {
        [System.IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $extractFull)
        $extractedMembers = [System.Collections.Generic.List[object]]::new()
        foreach ($memberPath in $expectedArchiveFiles.Keys) {
            $relativeFs = $memberPath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
            $memberFull = Assert-ResolvedPathUnderRoot -Root $extractFull -Path (Join-Path $extractFull $relativeFs) -Label "Extracted member $memberPath"
            if (-not (Test-Path -LiteralPath $memberFull -PathType Leaf)) { throw "Pinned extracted member is missing: $memberPath" }
            $item = Get-Item -LiteralPath $memberFull -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Pinned extracted member is a reparse point: $memberPath" }
            $expected = $expectedArchiveFiles[$memberPath]
            $memberHash = Get-FileSha256 $memberFull
            if ([long]$item.Length -ne [long]$expected.length -or $memberHash -ne $expected.sha256) {
                throw "Extracted member identity mismatch: path=$memberPath expected_length=$($expected.length) actual_length=$($item.Length) expected_sha256=$($expected.sha256) actual_sha256=$memberHash"
            }
            $extractedMembers.Add([ordered]@{
                path = $memberPath
                absolute_path = $memberFull
                role = $expected.role
                length_bytes = [long]$item.Length
                sha256 = $memberHash
                verification = 'EXTRACTED_FILE_PRE_EXECUTION'
            })
        }
        $script:Acquisition.extracted_members = @($extractedMembers.ToArray())
        $script:Acquisition.extraction_status = 'EXTRACTED_AND_MEMBER_IDENTITIES_VERIFIED'
        Write-Utf8Json -Path (Join-Path $outputFullPath 'tool-acquisition.json') -Value $script:Acquisition
    } catch {
        $script:Acquisition.extraction_status = 'FAILED_OR_REJECTED'
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
    phase = '#380-S1R3 Synthetic NSIS Runtime Trace Persistence'
    status = 'IN_PROGRESS'
    repository = $env:GITHUB_REPOSITORY
    ref = $env:GITHUB_REF
    workflow_sha = $env:GITHUB_SHA
    run_id = $env:GITHUB_RUN_ID
    run_attempt = $env:GITHUB_RUN_ATTEMPT
    job = $env:GITHUB_JOB
    frozen_s1r2_head = $script:S1R2Head
    frozen_s1r2_tree = $script:S1R2Tree
    frozen_s1r_head = $script:S1RHead
    frozen_s1r_tree = $script:S1RTree
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
    compile_options = 'nsis-3.11/makensis.exe (pinned forwarding launcher) -NOCD -V2 -DTRACE_MODE=<file|detail> -DRUN_MODE=<stage|abort> -DOUTPUT_NAME=<name> <frozen S0 probe>; TRACE_MODE=file executes first.'
    executables = @()
    processes = @()
    registry_snapshots = @()
    log_comparison = @()
    assertions = @()
    cleanup = [ordered]@{ registry_key = 'NOT_CHECKED'; task_temp = 'WORKFLOW_CLEANUP_PENDING'; task_owned_processes = 'NOT_CHECKED' }
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
        $frozenS1R2Tree = (& git rev-parse "$($script:S1R2Head)^{tree}").Trim()
        if ($LASTEXITCODE -ne 0 -or $frozenS1R2Tree -ne $script:S1R2Tree) { throw "Frozen S1R2 identity mismatch: expected_tree=$($script:S1R2Tree) actual_tree=$frozenS1R2Tree" }
        & git merge-base --is-ancestor $script:S1R2Head HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Frozen S1R2 final HEAD is not an ancestor of this S1R3 candidate.' }
        $frozenS1RTree = (& git rev-parse "$($script:S1RHead)^{tree}").Trim()
        if ($LASTEXITCODE -ne 0 -or $frozenS1RTree -ne $script:S1RTree) { throw "Frozen S1R identity mismatch: expected_tree=$($script:S1RTree) actual_tree=$frozenS1RTree" }
        & git merge-base --is-ancestor $script:S1RHead HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Frozen S1R final HEAD is not an ancestor of this S1R3 candidate.' }
        & git merge-base --is-ancestor $script:S1Head HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Frozen S1 HEAD is not an ancestor of this S1R3 candidate.' }
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
    Add-Assertion -Name 'two_pinned_archive_members_verified_before_extract' `
        -Passed (($script:Acquisition.validated_archive_members.Count -eq 2) -and
            (@($script:Acquisition.validated_archive_members | Where-Object { $_.path -eq $script:LauncherRelativePath -and $_.length_bytes -eq $script:LauncherLength -and $_.sha256 -eq $script:LauncherSha256 }).Count -eq 1) -and
            (@($script:Acquisition.validated_archive_members | Where-Object { $_.path -eq $script:CompilerRelativePath -and $_.length_bytes -eq $script:CompilerLength -and $_.sha256 -eq $script:CompilerSha256 }).Count -eq 1)) `
        -Details "members=$($script:Acquisition.validated_archive_members | ConvertTo-Json -Compress -Depth 5)"
    Add-Assertion -Name 'two_pinned_extracted_members_verified_under_task_root' `
        -Passed (($script:Acquisition.extracted_members.Count -eq 2) -and
            (@($script:Acquisition.extracted_members | Where-Object { $_.path -eq $script:LauncherRelativePath -and $_.length_bytes -eq $script:LauncherLength -and $_.sha256 -eq $script:LauncherSha256 }).Count -eq 1) -and
            (@($script:Acquisition.extracted_members | Where-Object { $_.path -eq $script:CompilerRelativePath -and $_.length_bytes -eq $script:CompilerLength -and $_.sha256 -eq $script:CompilerSha256 }).Count -eq 1)) `
        -Details "members=$($script:Acquisition.extracted_members | ConvertTo-Json -Compress -Depth 5)"

    $relativeLauncher = $script:LauncherRelativePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
    $relativeCompiler = $script:CompilerRelativePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
    $makensis = Assert-ResolvedPathUnderRoot -Root $extractionRoot -Path (Join-Path $extractionRoot $relativeLauncher) -Label 'Approved top-level makensis launcher'
    $binCompiler = Assert-ResolvedPathUnderRoot -Root $extractionRoot -Path (Join-Path $extractionRoot $relativeCompiler) -Label 'Approved Bin makensis compiler'
    $makensisHash = Get-FileSha256 $makensis
    $binCompilerHash = Get-FileSha256 $binCompiler
    if ($makensisHash -ne $script:LauncherSha256 -or (Get-Item -LiteralPath $makensis).Length -ne $script:LauncherLength) {
        throw "Top-level launcher identity changed before execution: path=$makensis sha256=$makensisHash"
    }
    if ($binCompilerHash -ne $script:CompilerSha256 -or (Get-Item -LiteralPath $binCompiler).Length -ne $script:CompilerLength) {
        throw "Bin compiler identity changed before execution: path=$binCompiler sha256=$binCompilerHash"
    }
    $identityRoot = Join-Path $outputFullPath 'compiler'
    New-Item -ItemType Directory -Path $identityRoot -ErrorAction Stop | Out-Null
    Add-ExecutableEvidence -SourcePath $makensis -EvidencePath (Join-Path $identityRoot 'makensis-launcher.exe') -Role 'pinned_forwarding_launcher'
    Add-ExecutableEvidence -SourcePath $binCompiler -EvidencePath (Join-Path $identityRoot 'Bin-makensis.exe') -Role 'pinned_underlying_compiler'
    $script:Compiler = [ordered]@{
        executed_path = $makensis
        executed_role = 'top-level forwarding launcher; the Bin compiler is not invoked directly'
        launcher = [ordered]@{ path = $makensis; relative_path = $script:LauncherRelativePath; length_bytes = $script:LauncherLength; sha256 = $makensisHash }
        underlying_compiler = [ordered]@{ path = $binCompiler; relative_path = $script:CompilerRelativePath; length_bytes = $script:CompilerLength; sha256 = $binCompilerHash }
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
    foreach ($traceMode in @('file', 'detail')) {
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
        $installerArguments = @('/S')
        if ($traceMode -eq 'detail') { $installerArguments += "/LOG=$installerLog" }
        $installerAbort = Invoke-CapturedProcess -FilePath $abortExe -ArgumentList $installerArguments `
            -WorkingDirectory $modeTaskRoot -StdoutPath (Join-Path $modeEvidenceRoot 'installer-abort.stdout.txt') `
            -StderrPath (Join-Path $modeEvidenceRoot 'installer-abort.stderr.txt')
        if ($traceMode -eq 'file') {
            Assert-ProcessExit -Name 'file_installer_abort_expected_exit_37' -Result $installerAbort -ExpectedCode $script:ExpectedInstallerExitCode
        }

        $uninstallerLog = Join-Path $modeEvidenceRoot 'uninstaller-runtime.log'
        $absoluteInstallDirectory = [System.IO.Path]::GetFullPath($modeTaskRoot)
        $absoluteUninstallerPath = [System.IO.Path]::GetFullPath($generatedUninstaller)
        if (-not $absoluteUninstallerPath.StartsWith($absoluteInstallDirectory.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Generated uninstaller escaped the controlled task directory: $absoluteUninstallerPath"
        }
        $uninstallerArguments = [System.Collections.Generic.List[string]]::new()
        $uninstallerArguments.Add('/S')
        if ($traceMode -eq 'detail') { $uninstallerArguments.Add("/LOG=$uninstallerLog") }
        $uninstallerArguments.Add("_?=$absoluteInstallDirectory")
        Add-Assertion -Name "${traceMode}_uninstaller_no_copy_argument_is_final" `
            -Passed (($uninstallerArguments.Count -ge 2) -and $uninstallerArguments[$uninstallerArguments.Count - 1].Equals("_?=$absoluteInstallDirectory", [System.StringComparison]::OrdinalIgnoreCase)) `
            -Details "process_image=$absoluteUninstallerPath controlled_absolute_directory=$absoluteInstallDirectory argument_list=$($uninstallerArguments -join ' | ')"
        $uninstallerAbort = Invoke-CapturedProcess -FilePath $absoluteUninstallerPath -ArgumentList @($uninstallerArguments.ToArray()) `
            -WorkingDirectory $modeTaskRoot -StdoutPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stdout.txt') `
            -StderrPath (Join-Path $modeEvidenceRoot 'uninstaller-abort.stderr.txt')
        if ($traceMode -eq 'file') {
            Assert-ProcessExit -Name 'file_uninstaller_abort_expected_exit_42' -Result $uninstallerAbort -ExpectedCode $script:ExpectedUninstallerExitCode
        }

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
            role = if ($traceMode -eq 'file') { 'primary direct FileWrite persistence proof; no /LOG supplied to Abort processes' } else { 'optional DetailPrint and /LOG comparison; /LOG absence is informational' }
            installer_abort_process_id = $installerAbort.process_id
            installer_abort_image_path = $installerAbort.process_image_path
            installer_abort_argument_list = $installerAbort.argument_list
            installer_abort_exit_code = $installerAbort.exit_code
            installer_abort_timed_out = $installerAbort.timed_out
            installer_abort_expected_exit_code = $script:ExpectedInstallerExitCode
            uninstaller_abort_process_id = $uninstallerAbort.process_id
            uninstaller_abort_image_path = $uninstallerAbort.process_image_path
            uninstaller_abort_argument_list = $uninstallerAbort.argument_list
            uninstaller_abort_exit_code = $uninstallerAbort.exit_code
            uninstaller_abort_timed_out = $uninstallerAbort.timed_out
            uninstaller_abort_expected_exit_code = $script:ExpectedUninstallerExitCode
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

    $preCleanupProcesses = @(Get-TaskOwnedProcessSnapshot)
    $result.cleanup.task_owned_processes = if ($preCleanupProcesses.Count -eq 0) { 'ALL_TASK_PATH_PROCESSES_EXITED' } else { $preCleanupProcesses }
    Add-Assertion -Name 'no_task_owned_processes_before_cleanup' -Passed ($preCleanupProcesses.Count -eq 0) `
        -Details "remaining=$($preCleanupProcesses | ConvertTo-Json -Compress -Depth 4)"
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
        foreach ($mode in @('file', 'detail')) {
            $modeTaskRoot = Join-Path (Join-Path $taskRoot 'compiled') $mode
            $modeEvidenceRoot = Join-Path $outputFullPath $mode
            if (Test-Path -LiteralPath $modeTaskRoot -PathType Container) {
                New-Item -ItemType Directory -Path $modeEvidenceRoot -Force -ErrorAction Stop | Out-Null
                foreach ($compiledName in @('stage.exe', 'installer-abort.exe')) {
                    $sourceExe = Join-Path $modeTaskRoot $compiledName
                    $evidenceExe = Join-Path $modeEvidenceRoot $compiledName
                    if ((Test-Path -LiteralPath $sourceExe -PathType Leaf) -and -not (Test-Path -LiteralPath $evidenceExe -PathType Leaf)) {
                        Add-ExecutableEvidence -SourcePath $sourceExe -EvidencePath $evidenceExe `
                            -Role "compiler_output_${mode}_$([System.IO.Path]::GetFileNameWithoutExtension($compiledName))" -AllowEmpty $true
                    }
                }
                $sourceUninstaller = Join-Path $modeTaskRoot 'generated-uninstaller.exe'
                $evidenceUninstaller = Join-Path $modeEvidenceRoot 'generated-uninstaller.exe'
                if ((Test-Path -LiteralPath $sourceUninstaller -PathType Leaf) -and -not (Test-Path -LiteralPath $evidenceUninstaller -PathType Leaf)) {
                    Add-ExecutableEvidence -SourcePath $sourceUninstaller -EvidencePath $evidenceUninstaller -Role "generated_uninstaller_$mode"
                }
            }
            $sourceTrace = Join-Path (Join-Path (Join-Path $taskRoot 'compiled') $mode) 'direct-evidence.log'
            if (Test-Path -LiteralPath $sourceTrace -PathType Leaf) {
                Copy-Item -LiteralPath $sourceTrace -Destination (Join-Path $modeEvidenceRoot 'direct-evidence.log') -Force
            }
        }
    } catch {
        $traceCopyFailure = $_.Exception.ToString()
        if ($null -eq $script:Failure) { $script:Failure = "Direct trace evidence preservation failed: $traceCopyFailure" }
    }

    try {
        $remainingProcesses = @(Wait-ForTaskOwnedProcessesToExit -TimeoutSeconds 2)
        if ($remainingProcesses.Count -eq 0) {
            $result.cleanup.task_owned_processes = 'ALL_TASK_PATH_PROCESSES_EXITED'
        } else {
            $result.cleanup.task_owned_processes = $remainingProcesses
            if ($null -eq $script:Failure) { $script:Failure = "Task-owned processes remain before cleanup: $($remainingProcesses | ConvertTo-Json -Compress -Depth 4)" }
        }
    } catch {
        $processCleanupFailure = $_.Exception.ToString()
        $result.cleanup.task_owned_processes = "CHECK_FAILED: $processCleanupFailure"
        if ($null -eq $script:Failure) { $script:Failure = "Task-owned process check failed: $processCleanupFailure" }
    }

    try {
        if ($script:CanDeleteSyntheticKey) {
            if ($result.cleanup.task_owned_processes -ne 'ALL_TASK_PATH_PROCESSES_EXITED') {
                $result.cleanup.registry_key = 'PRESERVED_BECAUSE_TASK_PROCESS_STATE_IS_NOT_CLEAN'
                throw "Preserving HKCU test key because task-owned process exit is not proven: $($result.cleanup.task_owned_processes | ConvertTo-Json -Compress -Depth 4)"
            }
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
