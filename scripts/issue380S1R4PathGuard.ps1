Set-StrictMode -Version Latest

function ConvertTo-Issue380S1R4LocalPath {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if ([string]::IsNullOrWhiteSpace($Path) -or $Path.IndexOf([char]0) -ge 0) {
        throw "$Label path is empty or contains a NUL character."
    }

    $windowsForm = $Path.Replace('/', '\')
    foreach ($devicePrefix in @('\\?\', '\\.\', '\??\', '\Device\')) {
        if ($windowsForm.StartsWith($devicePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "$Label path uses a Windows device namespace, which is not allowed: $Path"
        }
    }
    if ($windowsForm.StartsWith('\\', [System.StringComparison]::Ordinal)) {
        throw "$Label path uses UNC syntax; only local drive paths are allowed: $Path"
    }
    if ($windowsForm -match '(^|\\)\.\.(\\|$)') {
        throw "$Label path contains a parent-directory traversal segment: $Path"
    }
    if (-not [System.IO.Path]::IsPathFullyQualified($Path)) {
        throw "$Label path must be fully qualified: $Path"
    }

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $fullPath = $fullPath.Replace('/', '\')
    if ($fullPath -notmatch '^[A-Za-z]:\\') {
        throw "$Label path must resolve to a local drive path: $fullPath"
    }

    $driveTail = $fullPath.Substring(2)
    if ($driveTail.Contains(':')) {
        throw "$Label path contains an alternate data stream or embedded colon: $fullPath"
    }

    $volumeRoot = [System.IO.Path]::GetPathRoot($fullPath)
    if ([string]::IsNullOrEmpty($volumeRoot) -or $volumeRoot -notmatch '^[A-Za-z]:\\$') {
        throw "$Label path has an unsupported volume root: $fullPath"
    }
    $driveInfo = [System.IO.DriveInfo]::new($volumeRoot)
    if ($driveInfo.DriveType -eq [System.IO.DriveType]::Network) {
        throw "$Label path uses a mapped network drive, which is not allowed: $fullPath"
    }

    if ($fullPath.Length -gt $volumeRoot.Length) {
        $fullPath = $fullPath.TrimEnd([char[]]@('\'))
    }
    return $fullPath
}

function Get-Issue380S1R4PathAttributes {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$FullPath,
        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    $volumeRoot = [System.IO.Path]::GetPathRoot($FullPath)
    $currentPath = $volumeRoot
    try {
        $attributes = [System.IO.File]::GetAttributes($currentPath)
    } catch {
        throw "$Label volume root is unavailable: $currentPath; $($_.Exception.Message)"
    }
    if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label path traverses a reparse point: $currentPath"
    }

    $remainder = $FullPath.Substring($volumeRoot.Length)
    if (-not [string]::IsNullOrEmpty($remainder)) {
        $segments = $remainder.Split([char]'\', [System.StringSplitOptions]::RemoveEmptyEntries)
        foreach ($segment in $segments) {
            $currentPath = [System.IO.Path]::Combine($currentPath, $segment)
            try {
                $attributes = [System.IO.File]::GetAttributes($currentPath)
            } catch {
                throw "$Label path component does not exist or cannot be inspected: $currentPath; $($_.Exception.Message)"
            }
            if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Label path traverses a reparse point: $currentPath"
            }
        }
    }

    return $attributes
}

function Resolve-Issue380S1R4Path {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$TaskRoot,
        [Parameter(Mandatory = $true)]
        [string]$CandidatePath,
        [Parameter(Mandatory = $true)]
        [ValidateSet('WorkingDirectory', 'Executable', 'ExistingFile', 'ExistingDirectory')]
        [string]$Kind
    )

    $rootFullPath = ConvertTo-Issue380S1R4LocalPath -Path $TaskRoot -Label 'TaskRoot'
    $candidateFullPath = ConvertTo-Issue380S1R4LocalPath -Path $CandidatePath -Label $Kind

    if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
        throw 'RUNNER_TEMP is required for task-private path validation.'
    }
    $runnerTempFullPath = ConvertTo-Issue380S1R4LocalPath -Path $env:RUNNER_TEMP -Label 'RUNNER_TEMP'
    $runnerTempPrefix = if ($runnerTempFullPath.EndsWith('\', [System.StringComparison]::Ordinal)) {
        $runnerTempFullPath
    } else {
        $runnerTempFullPath + '\'
    }
    $taskRootIsRunnerTemp = $rootFullPath.Equals($runnerTempFullPath, [System.StringComparison]::OrdinalIgnoreCase)
    $taskRootIsRunnerTempDescendant = $rootFullPath.StartsWith($runnerTempPrefix, [System.StringComparison]::OrdinalIgnoreCase)
    if (-not $taskRootIsRunnerTemp -and -not $taskRootIsRunnerTempDescendant) {
        throw "TaskRoot is outside RUNNER_TEMP: runner_temp=$runnerTempFullPath task_root=$rootFullPath"
    }

    $rootAttributes = Get-Issue380S1R4PathAttributes -FullPath $rootFullPath -Label 'TaskRoot'
    if (($rootAttributes -band [System.IO.FileAttributes]::Directory) -eq 0) {
        throw "TaskRoot is not a directory: $rootFullPath"
    }

    $isExactRoot = $candidateFullPath.Equals($rootFullPath, [System.StringComparison]::OrdinalIgnoreCase)
    $rootPrefix = if ($rootFullPath.EndsWith('\', [System.StringComparison]::Ordinal)) {
        $rootFullPath
    } else {
        $rootFullPath + '\'
    }
    $isDescendant = $candidateFullPath.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)

    if ($Kind -eq 'WorkingDirectory') {
        if (-not $isExactRoot -and -not $isDescendant) {
            throw "Working directory is outside TaskRoot: root=$rootFullPath candidate=$candidateFullPath"
        }
    } elseif (-not $isDescendant) {
        throw "$Kind must be a strict descendant of TaskRoot: root=$rootFullPath candidate=$candidateFullPath"
    }

    $candidateAttributes = Get-Issue380S1R4PathAttributes -FullPath $candidateFullPath -Label $Kind
    if ($Kind -eq 'WorkingDirectory' -or $Kind -eq 'ExistingDirectory') {
        if (($candidateAttributes -band [System.IO.FileAttributes]::Directory) -eq 0) {
            throw "$Kind is not a directory: $candidateFullPath"
        }
    } else {
        if (($candidateAttributes -band [System.IO.FileAttributes]::Directory) -ne 0 -or
            ($candidateAttributes -band [System.IO.FileAttributes]::Device) -ne 0) {
            throw "$Kind is not an ordinary file: $candidateFullPath"
        }
        if ($Kind -eq 'Executable' -and
            -not [System.IO.Path]::GetExtension($candidateFullPath).Equals('.exe', [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Executable path must name an .exe file: $candidateFullPath"
        }
    }

    return $candidateFullPath
}
