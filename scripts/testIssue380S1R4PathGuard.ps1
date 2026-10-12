#requires -Version 7.0
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $IsWindows) {
    throw 'This path-guard test requires Windows PowerShell/.NET path and reparse-point semantics.'
}
if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    throw 'RUNNER_TEMP is required so every fixture stays in the task-owned temporary area.'
}

. (Join-Path $PSScriptRoot 'issue380S1R4PathGuard.ps1')

$script:Cases = [System.Collections.Generic.List[object]]::new()
$script:Failures = [System.Collections.Generic.List[string]]::new()
$script:Links = [System.Collections.Generic.List[string]]::new()
$token = [Guid]::NewGuid().ToString('N')
$runnerTemp = ConvertTo-Issue380S1R4LocalPath -Path $env:RUNNER_TEMP -Label 'RUNNER_TEMP'
$runnerTemp = Resolve-Issue380S1R4Path -TaskRoot $runnerTemp -CandidatePath $runnerTemp -Kind WorkingDirectory
$volumeRoot = [System.IO.Path]::GetPathRoot($runnerTemp)
$scopeRoot = [System.IO.Path]::GetFullPath((Join-Path $runnerTemp "issue380-s1r4-path-tests-$token"))
$externalTemp = [System.IO.Path]::GetFullPath((Join-Path $runnerTemp "issue380-s1r4-external-$token"))
$scopeMarker = Join-Path $scopeRoot 's1r4-path-test-owned.txt'
$externalMarker = Join-Path $externalTemp 's1r4-path-test-external-owned.txt'
$taskRoot = Join-Path $scopeRoot 'task'
$uppercaseTaskRoot = $taskRoot.ToUpperInvariant()
$forwardSlashTaskRoot = $taskRoot.Replace('\', '/')
$trailingSlashTaskRoot = "$taskRoot\"
$taskRootSibling = "$taskRoot-sibling"
$directChild = Join-Path $taskRoot 'direct'
$nestedChild = Join-Path $directChild 'nested'
$uppercaseNestedChild = $nestedChild.ToUpperInvariant()
$forwardSlashNestedChild = $nestedChild.Replace('\', '/')
$uppercaseForwardSlashNestedChild = $uppercaseNestedChild.Replace('\', '/')
$taskExe = Join-Path $taskRoot 'synthetic-compiler.exe'
$uppercaseTaskExe = $taskExe.ToUpperInvariant()
$forwardSlashTaskExe = $taskExe.Replace('\', '/')
$outsideExe = Join-Path $externalTemp 'outside.exe'
$directoryAsExe = $nestedChild
$missingExe = Join-Path $taskRoot 'missing.exe'
$junctionPath = Join-Path $taskRoot 'junction-to-external'
$directorySymlinkPath = Join-Path $taskRoot 'symlink-to-external'
$fileSymlinkPath = Join-Path $taskRoot 'linked.exe'
$taskRootSymlinkPath = Join-Path $scopeRoot 'taskroot-symlink'
$parentSymlinkPath = Join-Path $runnerTemp "issue380-s1r4-parent-link-$token"

function Add-PathCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$TaskRoot,
        [Parameter(Mandatory = $true)][string]$CandidatePath,
        [Parameter(Mandatory = $true)][string]$Kind,
        [Parameter(Mandatory = $true)][bool]$ExpectedAllowed
    )

    $resolvedPath = $null
    $errorText = $null
    try {
        $resolvedPath = Resolve-Issue380S1R4Path -TaskRoot $TaskRoot -CandidatePath $CandidatePath -Kind $Kind
    } catch {
        $errorText = $_.Exception.Message
    }

    $actualAllowed = ($null -eq $errorText)
    $passed = ($actualAllowed -eq $ExpectedAllowed)
    $script:Cases.Add([ordered]@{
        name = $Name
        kind = $Kind
        expected = if ($ExpectedAllowed) { 'ALLOW' } else { 'REJECT' }
        actual = if ($actualAllowed) { 'ALLOW' } else { 'REJECT' }
        passed = $passed
        resolved_path = $resolvedPath
        error = $errorText
    })
    if (-not $passed) {
        $script:Failures.Add("$Name expected_allowed=$ExpectedAllowed actual_allowed=$actualAllowed error=$errorText")
    }
}

function Remove-OwnedLink([string]$LiteralPath) {
    if (Test-Path -LiteralPath $LiteralPath) {
        Remove-Item -LiteralPath $LiteralPath -Force -ErrorAction Stop
    }
    if (Test-Path -LiteralPath $LiteralPath) {
        throw "Owned test link remains after removal: $LiteralPath"
    }
}

try {
    foreach ($ownedPath in @($scopeRoot, $externalTemp, $taskRootSibling, $parentSymlinkPath)) {
        if (Test-Path -LiteralPath $ownedPath) {
            throw "Refusing to reuse a preexisting path: $ownedPath"
        }
    }

    New-Item -ItemType Directory -Path $scopeRoot -ErrorAction Stop | Out-Null
    [System.IO.File]::WriteAllText($scopeMarker, $token, [System.Text.UTF8Encoding]::new($false))
    New-Item -ItemType Directory -Path $taskRoot -ErrorAction Stop | Out-Null
    New-Item -ItemType Directory -Path $directChild -ErrorAction Stop | Out-Null
    New-Item -ItemType Directory -Path $nestedChild -ErrorAction Stop | Out-Null
    New-Item -ItemType Directory -Path $taskRootSibling -ErrorAction Stop | Out-Null
    New-Item -ItemType Directory -Path $externalTemp -ErrorAction Stop | Out-Null
    [System.IO.File]::WriteAllText($externalMarker, $token, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($taskExe, 'synthetic path fixture; never executed', [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($outsideExe, 'synthetic external path fixture; never executed', [System.Text.UTF8Encoding]::new($false))

    Add-PathCase 'taskroot_working_directory' $taskRoot $taskRoot 'WorkingDirectory' $true
    Add-PathCase 'direct_child_working_directory' $taskRoot $directChild 'WorkingDirectory' $true
    Add-PathCase 'nested_child_working_directory' $taskRoot $nestedChild 'WorkingDirectory' $true
    Add-PathCase 'shared_prefix_sibling_working_directory' $taskRoot $taskRootSibling 'WorkingDirectory' $false
    Add-PathCase 'taskroot_parent_working_directory' $taskRoot $scopeRoot 'WorkingDirectory' $false
    Add-PathCase 'other_runner_temp_working_directory' $taskRoot $externalTemp 'WorkingDirectory' $false
    Add-PathCase 'taskroot_outside_runner_temp_rejected' $volumeRoot $volumeRoot 'WorkingDirectory' $false
    Add-PathCase 'traversal_that_resolves_back_inside' $taskRoot (Join-Path $taskRoot 'direct\..\direct') 'WorkingDirectory' $false
    Add-PathCase 'traversal_to_parent' $taskRoot (Join-Path $taskRoot '..\external') 'WorkingDirectory' $false

    Add-PathCase 'uppercase_path_variant' $taskRoot $uppercaseNestedChild 'WorkingDirectory' $true
    Add-PathCase 'forward_slash_path_variant' $taskRoot $forwardSlashNestedChild 'WorkingDirectory' $true
    Add-PathCase 'uppercase_forward_slash_path_variant' $taskRoot $uppercaseForwardSlashNestedChild 'WorkingDirectory' $true
    Add-PathCase 'uppercase_taskroot_variant' $uppercaseTaskRoot $nestedChild 'WorkingDirectory' $true
    Add-PathCase 'forward_slash_taskroot_variant' $forwardSlashTaskRoot $forwardSlashNestedChild 'WorkingDirectory' $true
    Add-PathCase 'trailing_separator_taskroot_variant' $trailingSlashTaskRoot $trailingSlashTaskRoot 'WorkingDirectory' $true

    Add-PathCase 'existing_in_root_executable' $taskRoot $taskExe 'Executable' $true
    Add-PathCase 'uppercase_executable_path_variant' $taskRoot $uppercaseTaskExe 'Executable' $true
    Add-PathCase 'forward_slash_executable_path_variant' $taskRoot $forwardSlashTaskExe 'Executable' $true
    Add-PathCase 'taskroot_is_not_an_executable' $taskRoot $taskRoot 'Executable' $false
    Add-PathCase 'missing_executable' $taskRoot $missingExe 'Executable' $false
    Add-PathCase 'directory_cannot_be_executable' $taskRoot $directoryAsExe 'Executable' $false
    Add-PathCase 'outside_taskroot_executable' $taskRoot $outsideExe 'Executable' $false
    Add-PathCase 'another_runner_temp_directory_is_not_executable' $taskRoot $externalTemp 'Executable' $false

    Add-PathCase 'unc_path_rejected' $taskRoot '\\example.invalid\share\tool.exe' 'Executable' $false
    Add-PathCase 'unc_taskroot_rejected' '\\example.invalid\share' '\\example.invalid\share' 'WorkingDirectory' $false
    Add-PathCase 'extended_device_path_rejected' $taskRoot '\\?\C:\Windows\System32\tool.exe' 'Executable' $false
    Add-PathCase 'device_namespace_taskroot_rejected' '\\?\C:\isolated\task' '\\?\C:\isolated\task' 'WorkingDirectory' $false
    Add-PathCase 'win32_device_path_rejected' $taskRoot '\\.\C:\Windows\System32\tool.exe' 'Executable' $false
    Add-PathCase 'nt_device_path_rejected' $taskRoot '\??\C:\Windows\System32\tool.exe' 'Executable' $false
    Add-PathCase 'relative_candidate_rejected' $taskRoot 'relative\tool.exe' 'Executable' $false
    Add-PathCase 'drive_relative_candidate_rejected' $taskRoot 'C:relative\tool.exe' 'Executable' $false
    Add-PathCase 'alternate_data_stream_rejected' $taskRoot "${taskExe}:stream" 'Executable' $false

    $junctionItem = New-Item -ItemType Junction -Path $junctionPath -Target $externalTemp -ErrorAction Stop
    [void]$script:Links.Add($junctionPath)
    Add-PathCase 'junction_working_directory_rejected' $taskRoot $junctionPath 'WorkingDirectory' $false
    Add-PathCase 'junction_descendant_rejected' $taskRoot (Join-Path $junctionPath 'outside.exe') 'Executable' $false

    [void][System.IO.Directory]::CreateSymbolicLink($directorySymlinkPath, $externalTemp)
    [void]$script:Links.Add($directorySymlinkPath)
    Add-PathCase 'directory_symlink_working_directory_rejected' $taskRoot $directorySymlinkPath 'WorkingDirectory' $false

    [void][System.IO.File]::CreateSymbolicLink($fileSymlinkPath, $outsideExe)
    [void]$script:Links.Add($fileSymlinkPath)
    Add-PathCase 'file_symlink_executable_rejected' $taskRoot $fileSymlinkPath 'Executable' $false

    [void][System.IO.Directory]::CreateSymbolicLink($taskRootSymlinkPath, $taskRoot)
    [void]$script:Links.Add($taskRootSymlinkPath)
    Add-PathCase 'taskroot_symlink_rejected' $taskRootSymlinkPath (Join-Path $taskRootSymlinkPath 'synthetic-compiler.exe') 'Executable' $false

    [void][System.IO.Directory]::CreateSymbolicLink($parentSymlinkPath, $scopeRoot)
    [void]$script:Links.Add($parentSymlinkPath)
    Add-PathCase 'taskroot_reparse_ancestor_rejected' $parentSymlinkPath `
        (Join-Path (Join-Path $parentSymlinkPath 'task') 'synthetic-compiler.exe') 'Executable' $false

    Add-PathCase 'compiler_version_working_directory_is_taskroot' $taskRoot $taskRoot 'WorkingDirectory' $true
    Add-PathCase 'compiler_hdrinfo_working_directory_is_taskroot' $taskRoot $taskRoot 'WorkingDirectory' $true
} catch {
    $script:Failures.Add("test setup or execution failed: $($_.Exception.ToString())")
} finally {
    try {
        foreach ($linkPath in @($script:Links.ToArray())) {
            Remove-OwnedLink -LiteralPath $linkPath
        }

        if ((Test-Path -LiteralPath $scopeMarker -PathType Leaf) -and
            ([System.IO.File]::ReadAllText($scopeMarker) -eq $token)) {
            Remove-Item -LiteralPath $scopeRoot -Recurse -Force -ErrorAction Stop
            if (Test-Path -LiteralPath $scopeRoot) { throw "Owned test root remains after cleanup: $scopeRoot" }
        } else {
            throw "Scope cleanup refused because the ownership marker did not match: $scopeMarker"
        }

        if ((Test-Path -LiteralPath $externalMarker -PathType Leaf) -and
            ([System.IO.File]::ReadAllText($externalMarker) -eq $token)) {
            Remove-Item -LiteralPath $externalTemp -Recurse -Force -ErrorAction Stop
            if (Test-Path -LiteralPath $externalTemp) { throw "External test fixture remains after cleanup: $externalTemp" }
        } else {
            throw "External fixture cleanup refused because the ownership marker did not match: $externalMarker"
        }
    } catch {
        $script:Failures.Add("test cleanup failed: $($_.Exception.ToString())")
    }
}

$allCasesPassed = ($script:Cases.Count -gt 0) -and
    ($script:Failures.Count -eq 0) -and
    (@($script:Cases | Where-Object { -not $_.passed }).Count -eq 0)
$result = [ordered]@{
    status = if ($allCasesPassed) { 'WINDOWS_RUNTIME_PASS' } else { 'WINDOWS_RUNTIME_FAIL' }
    is_windows = [bool]$IsWindows
    operating_system = [System.Environment]::OSVersion.VersionString
    powershell_version = $PSVersionTable.PSVersion.ToString()
    path_guard_module = 'scripts/issue380S1R4PathGuard.ps1'
    process_launch_count = 0
    nsis_launched = $false
    unrelated_executable_launched = $false
    cases = @($script:Cases.ToArray())
    failures = @($script:Failures.ToArray())
    cleanup = [ordered]@{
        owned_scope_root_removed = -not (Test-Path -LiteralPath $scopeRoot)
        owned_external_fixture_removed = -not (Test-Path -LiteralPath $externalTemp)
        owned_links_removed = (@($script:Links | Where-Object { Test-Path -LiteralPath $_ }).Count -eq 0)
    }
}
$result | ConvertTo-Json -Depth 10
if (-not $allCasesPassed) { exit 1 }
