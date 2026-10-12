[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('DelayedOutput', 'HoldPipe', 'BlockForTimeout', 'HoldPipeChild')]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string]$RecordPath,

    [Parameter(Mandatory = $true)]
    [string]$Nonce,

    [int]$DelayMilliseconds = 1500,

    [int]$HoldSeconds = 15
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:ChildStartedUtc = [DateTime]::UtcNow

function Write-ChildRecord([string]$Path, [object]$Value) {
    $json = ConvertTo-Json -InputObject $Value -Depth 8
    [System.IO.File]::WriteAllText(
        $Path,
        $json + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Start-PipeHoldingChild([string]$ChildMode) {
    $childRecordPath = "$RecordPath.child"
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = Join-Path $PSHOME 'pwsh.exe'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $false
    foreach ($argument in @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-File', $PSCommandPath,
        '-Mode', $ChildMode, '-RecordPath', $childRecordPath, '-Nonce', $Nonce,
        '-DelayMilliseconds', [string]$DelayMilliseconds,
        '-HoldSeconds', [string]$HoldSeconds
    )) {
        [void]$startInfo.ArgumentList.Add([string]$argument)
    }

    $child = [System.Diagnostics.Process]::Start($startInfo)
    if ($null -eq $child) { throw 'Could not start controlled pipe-holder child.' }
    $childInfo = [ordered]@{
        process_id = $child.Id
        image_path = $startInfo.FileName
        arguments = @($startInfo.ArgumentList | ForEach-Object { [string]$_ })
        started_utc = $script:ChildStartedUtc.ToString('o')
    }
    $child.Dispose()
    return [pscustomobject]$childInfo
}

switch ($Mode) {
    'DelayedOutput' {
        Start-Sleep -Milliseconds $DelayMilliseconds
        [Console]::Out.Write("S1R4_STDOUT_DELAYED|$Nonce`n")
        [Console]::Out.Flush()
        [Console]::Error.Write("S1R4_STDERR_DELAYED|$Nonce`n")
        [Console]::Error.Flush()
        Write-ChildRecord -Path $RecordPath -Value ([ordered]@{
            mode = $Mode
            nonce = $Nonce
            pid = $PID
            image_path = Join-Path $PSHOME 'pwsh.exe'
            arguments = @('-File', $PSCommandPath, '-Mode', $Mode, '-Nonce', $Nonce, '-DelayMilliseconds', [string]$DelayMilliseconds)
            started_utc = $script:ChildStartedUtc.ToString('o')
            delay_ms = $DelayMilliseconds
            exit_code = 0
        })
        exit 0
    }

    'HoldPipe' {
        $child = Start-PipeHoldingChild -ChildMode 'HoldPipeChild'
        Write-ChildRecord -Path $RecordPath -Value ([ordered]@{
            mode = $Mode
            nonce = $Nonce
            parent_pid = $PID
            parent_image_path = Join-Path $PSHOME 'pwsh.exe'
            parent_arguments = @('-File', $PSCommandPath, '-Mode', $Mode, '-Nonce', $Nonce)
            child_pid = $child.process_id
            child_image_path = $child.image_path
            child_arguments = $child.arguments
            started_utc = $script:ChildStartedUtc.ToString('o')
            hold_seconds = $HoldSeconds
            parent_exit_code = 0
        })
        exit 0
    }

    'BlockForTimeout' {
        $child = Start-PipeHoldingChild -ChildMode 'HoldPipeChild'
        Write-ChildRecord -Path $RecordPath -Value ([ordered]@{
            mode = $Mode
            nonce = $Nonce
            parent_pid = $PID
            parent_image_path = Join-Path $PSHOME 'pwsh.exe'
            parent_arguments = @('-File', $PSCommandPath, '-Mode', $Mode, '-Nonce', $Nonce)
            child_pid = $child.process_id
            child_image_path = $child.image_path
            child_arguments = $child.arguments
            started_utc = $script:ChildStartedUtc.ToString('o')
            hold_seconds = $HoldSeconds
        })
        Start-Sleep -Seconds 120
        exit 0
    }

    'HoldPipeChild' {
        Write-ChildRecord -Path $RecordPath -Value ([ordered]@{
            mode = $Mode
            nonce = $Nonce
            pid = $PID
            image_path = Join-Path $PSHOME 'pwsh.exe'
            arguments = @('-File', $PSCommandPath, '-Mode', $Mode, '-Nonce', $Nonce, '-HoldSeconds', [string]$HoldSeconds)
            started_utc = $script:ChildStartedUtc.ToString('o')
            hold_seconds = $HoldSeconds
        })
        Start-Sleep -Seconds $HoldSeconds
        exit 0
    }
}

throw "Unsupported test child mode: $Mode"
