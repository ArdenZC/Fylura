param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [Parameter(Mandatory = $true)]
    [string]$EvidencePath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function ConvertTo-PSLiteral([string]$Value) {
    return "'" + $Value.Replace("'", "''") + "'"
}

Assert-Condition ($env:GITHUB_ACTIONS -eq 'true' -and $env:RUNNER_ENVIRONMENT -eq 'github-hosted') 'The NSIS lifecycle runner is limited to a disposable GitHub-hosted Windows VM.'
$runnerIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
$runnerPrincipal = [Security.Principal.WindowsPrincipal]::new($runnerIdentity)
Assert-Condition $runnerPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) 'NSIS lifecycle validation requires the disposable runner to be elevated.'

$installer = [System.IO.Path]::GetFullPath($InstallerPath)
$evidence = [System.IO.Path]::GetFullPath($EvidencePath)
$runnerTemp = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP)
$worker = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'verifyWindowsNsisLifecycle.ps1'))
$pythonPath = Get-Command python -CommandType Application -ErrorAction Stop |
    Select-Object -First 1 -ExpandProperty Source
Assert-Condition (Test-Path -LiteralPath $installer -PathType Leaf) 'The packaged NSIS installer is missing.'
Assert-Condition (Test-Path -LiteralPath $worker -PathType Leaf) 'The isolated Windows lifecycle worker is missing.'
Assert-Condition (-not (Test-Path -LiteralPath $evidence)) "Refusing to overwrite lifecycle evidence: $evidence"
Assert-Condition ($env:GITHUB_RUN_ID -and $env:GITHUB_SHA) 'GitHub run identity is unavailable.'

# Windows Credential Manager is scoped to a logon profile. Run all destructive
# lifecycle phases in a temporary elevated account so pre-existing runner
# credentials can never be overwritten or deleted by the test.
$userName = 'Fylura596_' + [guid]::NewGuid().ToString('N').Substring(0, 10)
$taskName = 'FyluraNsis596_' + [guid]::NewGuid().ToString('N')
$account = "$env:COMPUTERNAME\$userName"
$passwordText = 'Fylura596!' + [guid]::NewGuid().ToString('N') + 'aA1'
$password = ConvertTo-SecureString $passwordText -AsPlainText -Force
$userCreated = $false
$taskRegistered = $false
$taskStarted = $false
$userSid = $null
$workerFailure = $null
$cleanupFailures = [System.Collections.Generic.List[string]]::new()

try {
    New-LocalUser -Name $userName -Password $password -PasswordNeverExpires -AccountNeverExpires `
        -Description 'Disposable Fylura NSIS lifecycle test account' | Out-Null
    $userCreated = $true
    Add-LocalGroupMember -Group 'Administrators' -Member $userName
    $userSid = (Get-LocalUser -Name $userName).SID.Value

    $literalWorker = ConvertTo-PSLiteral $worker
    $literalInstaller = ConvertTo-PSLiteral $installer
    $literalEvidence = ConvertTo-PSLiteral $evidence
    $literalRunnerTemp = ConvertTo-PSLiteral $runnerTemp
    $literalRunId = ConvertTo-PSLiteral $env:GITHUB_RUN_ID
    $literalSourceSha = ConvertTo-PSLiteral $env:GITHUB_SHA
    $literalPythonPath = ConvertTo-PSLiteral $pythonPath
    $workerCommand = @"
try {
    & $literalWorker -InstallerPath $literalInstaller -EvidencePath $literalEvidence -TaskWorker `
        -RunnerTemp $literalRunnerTemp -RunId $literalRunId -SourceSha $literalSourceSha `
        -PythonPath $literalPythonPath
    exit 0
} catch {
    [Console]::Error.WriteLine(`$_.Exception.Message)
    exit 1
}
"@
    $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($workerCommand))
    $powershell = (Get-Process -Id $PID).Path
    $action = New-ScheduledTaskAction -Execute $powershell -Argument "-NoLogo -NoProfile -NonInteractive -EncodedCommand $encodedCommand"
    $trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(2)
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 35) -StartWhenAvailable
    Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings `
        -User $account -Password $passwordText -RunLevel Highest -Force | Out-Null
    $taskRegistered = $true

    $startedAt = Get-Date
    Start-ScheduledTask -TaskName $taskName
    $taskStarted = $true
    $deadline = $startedAt.AddMinutes(30)
    $completed = $false
    while ((Get-Date) -lt $deadline) {
        $task = Get-ScheduledTask -TaskName $taskName -ErrorAction Stop
        $taskInfo = Get-ScheduledTaskInfo -TaskName $taskName -ErrorAction Stop
        if ($task.State -ne 'Running' -and $taskInfo.LastRunTime -ge $startedAt.AddSeconds(-2)) {
            $completed = $true
            break
        }
        Start-Sleep -Seconds 2
    }
    Assert-Condition $completed 'Timed out waiting for the isolated elevated NSIS lifecycle task.'

    $taskInfo = Get-ScheduledTaskInfo -TaskName $taskName -ErrorAction Stop
    if ($taskInfo.LastTaskResult -ne 0) {
        $evidenceSummary = ''
        if (Test-Path -LiteralPath $evidence -PathType Leaf) {
            $document = Get-Content -LiteralPath $evidence -Raw | ConvertFrom-Json
            $failure = @($document.phases | Where-Object { $_.phase -eq 'failed' } | Select-Object -Last 1)
            if ($failure.Count -gt 0 -and $failure[0].error) {
                $evidenceSummary = ": $($failure[0].error)"
            }
        }
        throw "Isolated elevated NSIS lifecycle task failed with result $($taskInfo.LastTaskResult)$evidenceSummary"
    }
    Assert-Condition (Test-Path -LiteralPath $evidence -PathType Leaf) 'The isolated NSIS lifecycle task did not write evidence.'
    $document = Get-Content -LiteralPath $evidence -Raw | ConvertFrom-Json
    Assert-Condition ($document.status -eq 'PASS') 'The isolated NSIS lifecycle task did not report PASS evidence.'
    Write-Output 'PASS: NSIS install, ordinary uninstall, reinstall, explicit data cleanup, credential cleanup, and other-user preservation ran in a disposable elevated profile.'
}
catch {
    $workerFailure = $_.Exception.Message
}
finally {
    if ($taskRegistered) {
        if ($taskStarted) {
            $task = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
            if ($task -and $task.State -eq 'Running') {
                Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
                for ($attempt = 0; $attempt -lt 30; $attempt++) {
                    $task = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
                    if (-not $task -or $task.State -ne 'Running') { break }
                    Start-Sleep -Milliseconds 500
                }
            }
        }
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
    }

    if ($userCreated) {
        if (-not $userSid) {
            $createdUser = Get-LocalUser -Name $userName -ErrorAction SilentlyContinue
            if ($createdUser) { $userSid = $createdUser.SID.Value }
        }
        if ($userSid) {
            for ($attempt = 0; $attempt -lt 30; $attempt++) {
                $profile = Get-CimInstance -ClassName Win32_UserProfile -ErrorAction SilentlyContinue |
                    Where-Object { $_.SID -eq $userSid } | Select-Object -First 1
                if (-not $profile -or -not $profile.Loaded) { break }
                Start-Sleep -Milliseconds 500
            }
            if ($profile -and -not $profile.Loaded) {
                $profile | Remove-CimInstance -ErrorAction SilentlyContinue
            }
        }
        Remove-LocalUser -Name $userName -ErrorAction SilentlyContinue
        $remainingProfile = if ($userSid) {
            Get-CimInstance -ClassName Win32_UserProfile -ErrorAction SilentlyContinue |
                Where-Object { $_.SID -eq $userSid } | Select-Object -First 1
        }
        if ($remainingProfile) {
            $cleanupFailures.Add('The disposable lifecycle user profile remained loaded or registered.')
        }
        if (Get-LocalUser -Name $userName -ErrorAction SilentlyContinue) {
            $cleanupFailures.Add('The disposable lifecycle user account remained after cleanup.')
        }
    }
}

if ($workerFailure) {
    throw $workerFailure
}
if ($cleanupFailures.Count -gt 0) {
    throw ($cleanupFailures -join ' ')
}
