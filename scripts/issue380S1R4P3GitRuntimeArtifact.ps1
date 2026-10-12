[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RunRoot,
    [Parameter(Mandatory = $true)][string]$ArchiveRoot
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Walk one directory at a time: reject links before recursing into them.
function Get-RuntimeArchiveEntries([string]$Root) {
    $pending = [System.Collections.Generic.Stack[string]]::new()
    $pending.Push($Root)
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        $item = Get-Item -LiteralPath $directory -Force -ErrorAction Stop
        if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse point: $directory" }
        foreach ($entry in @(Get-ChildItem -LiteralPath $directory -Force -ErrorAction Stop)) {
            if (($entry.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse point: $($entry.FullName)" }
            if ($entry.PSIsContainer) { $pending.Push($entry.FullName) }
            else { $entry }
        }
    }
}

function Assert-RuntimeArchive([string]$Root) {
    $files = @(Get-RuntimeArchiveEntries $Root)
    $manifest = Join-Path $Root 'SHA256SUMS'
    $listed = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($line in [System.IO.File]::ReadAllLines($manifest)) {
        if ($line -cnotmatch '^([0-9a-f]{64})  (.+)$') { throw 'Malformed SHA256SUMS line.' }
        $hash = $Matches[1]; $relative = $Matches[2]
        if ($relative -match '(^|/)[.]|\\|[\r\n]' -or [System.IO.Path]::IsPathRooted($relative) -or $relative -eq 'SHA256SUMS') { throw "Unsafe manifest path: $relative" }
        if (-not $listed.Add($relative)) { throw "Duplicate manifest file: $relative" }
        $path = Join-Path $Root $relative
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Manifest file missing: $relative" }
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $hash) { throw "SHA-256 mismatch: $relative" }
    }
    $payload = @($files | Where-Object { $_.FullName -cne $manifest })
    foreach ($file in $files) {
        if (($file.Attributes -band [System.IO.FileAttributes]::Hidden) -ne 0) { throw 'Hidden archive file would be omitted by upload-artifact.' }
    }
    foreach ($file in $payload) {
        $relative = [System.IO.Path]::GetRelativePath($Root, $file.FullName).Replace('\', '/')
        if (-not $listed.Contains($relative)) { throw "Unlisted archive file: $relative" }
    }
    if ($listed.Count -ne $payload.Count) { throw 'Manifest file-set mismatch.' }
    # The manifest itself is the sole self-hash exception and is uploaded too.
    if ($files.Count -gt 250) { throw 'Archive file limit exceeded.' }
    $bytes = 0L
    foreach ($file in $files) { $bytes += $file.Length }
    if ($bytes -gt 20MB) { throw 'Archive size limit exceeded.' }
    return [pscustomobject]@{ status = 'PASS'; payload_files = $payload.Count; uploaded_files = $files.Count; bytes = $bytes; manifest_sha256 = (Get-FileHash -LiteralPath $manifest -Algorithm SHA256).Hash.ToLowerInvariant() }
}

$run = [System.IO.Path]::GetFullPath($RunRoot)
$archive = [System.IO.Path]::GetFullPath($ArchiveRoot)
$temp = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd([char[]]@('\','/'))
$expectedRun = Join-Path $temp ("issue380-s1r4-p3-git-runtime-{0}" -f $env:GITHUB_RUN_ID)
if ($run -cne $expectedRun -or $archive -cne (Join-Path $temp ("issue380-s1r4-p3-git-runtime-archive-{0}" -f $env:GITHUB_RUN_ID))) { throw 'Unexpected owned archive/run path.' }
if (Test-Path -LiteralPath $archive) { throw 'Archive must not preexist.' }
$owner = Get-Content -LiteralPath (Join-Path $run 'evidence/runtime-owner.json') -Raw | ConvertFrom-Json -AsHashtable
foreach ($field in @('repository', 'ref', 'run_id', 'run_attempt', 'triggering_sha')) {
    $envName = @{ repository='GITHUB_REPOSITORY'; ref='GITHUB_REF'; run_id='GITHUB_RUN_ID'; run_attempt='GITHUB_RUN_ATTEMPT'; triggering_sha='GITHUB_SHA' }[$field]
    if (-not $owner.ContainsKey($field) -or [string]$owner[$field] -cne [Environment]::GetEnvironmentVariable($envName)) { throw "Evidence owner mismatch: $field" }
}
$task = Join-Path $run 'task'
if (Test-Path -LiteralPath $task) {
    $taskOwner = Get-Content -LiteralPath (Join-Path $task 'runtime-task-owner.json') -Raw | ConvertFrom-Json -AsHashtable
    foreach ($field in @('run_id','run_attempt','triggering_sha')) {
        if (-not $taskOwner.ContainsKey($field) -or $taskOwner[$field] -cne $owner[$field]) { throw "Task owner mismatch: $field" }
    }
}
# Keep original limits over ALL source files, including excluded Git internals.
$sourceFiles = @(Get-RuntimeArchiveEntries $run)
if ($sourceFiles.Count -gt 250) { throw 'Source file limit exceeded.' }
$bytes = 0L
foreach ($file in $sourceFiles) { $bytes += $file.Length }
if ($bytes -gt 20MB) { throw 'Source size limit exceeded.' }
$selected = @($sourceFiles | Where-Object {
    $relative = [System.IO.Path]::GetRelativePath($run, $_.FullName).Replace('\','/')
    # Explicit evidence tree and small task diagnostics; no repositories/config/source copies.
    ($relative -cmatch '^evidence/[^/]+$' -and $relative -cnotmatch '(^|/)[.]' -and $_.Name -cne 'SHA256SUMS') -or
    $relative -cmatch '^task/(runtime-task-owner[.]json|delayed-child[.]json|pipe-holder[.]json([.]child)?|timeout-child[.]json([.]child)?)$'
})
New-Item -ItemType Directory -Path $archive -ErrorAction Stop | Out-Null
$lines = [System.Collections.Generic.List[string]]::new()
foreach ($file in @($selected | Sort-Object FullName)) {
    $relative = [System.IO.Path]::GetRelativePath($run, $file.FullName).Replace('\','/')
    if ($relative -match '[\r\n]') { throw 'Unsupported archive filename.' }
    if (($file.Attributes -band [System.IO.FileAttributes]::Hidden) -ne 0) { throw "Hidden selected evidence file: $relative" }
    $destination = Join-Path $archive $relative
    [void][System.IO.Directory]::CreateDirectory((Split-Path -Parent $destination))
    [System.IO.File]::Copy($file.FullName, $destination, $false)
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant() -cne $hash) { throw "Archive copy hash mismatch: $relative" }
    $lines.Add("$hash  $relative")
}
[System.IO.File]::WriteAllText((Join-Path $archive 'SHA256SUMS'), (($lines -join "`n") + "`n"), [System.Text.UTF8Encoding]::new($false))
Assert-RuntimeArchive $archive
