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
$worker = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'verifyWindowsNsisLifecycle.ps1'))
$pythonPath = Get-Command python -CommandType Application -ErrorAction Stop |
    Select-Object -First 1 -ExpandProperty Source
$runAttempt = if ($env:GITHUB_RUN_ATTEMPT) { $env:GITHUB_RUN_ATTEMPT } else { '1' }
$sharedRoot = Join-Path $env:ProgramData "FyluraNsisLifecycle_$($env:GITHUB_RUN_ID)_$runAttempt"
$sharedWorker = Join-Path $sharedRoot 'verifyWindowsNsisLifecycle.ps1'
$sharedInstaller = Join-Path $sharedRoot 'Fylura-Setup.exe'
$workerEvidence = Join-Path $sharedRoot 'lifecycle-evidence.json'
$workerLog = Join-Path $sharedRoot 'worker-error.log'
Assert-Condition (Test-Path -LiteralPath $installer -PathType Leaf) 'The packaged NSIS installer is missing.'
Assert-Condition (Test-Path -LiteralPath $worker -PathType Leaf) 'The isolated Windows lifecycle worker is missing.'
Assert-Condition (-not (Test-Path -LiteralPath $evidence)) "Refusing to overwrite lifecycle evidence: $evidence"
Assert-Condition (-not (Test-Path -LiteralPath $sharedRoot)) "Refusing pre-existing isolated lifecycle staging path: $sharedRoot"
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

    New-Item -ItemType Directory -Path $sharedRoot | Out-Null
    $sharedAcl = Get-Acl -LiteralPath $sharedRoot
    $inheritance = [System.Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [System.Security.AccessControl.InheritanceFlags]::ObjectInherit
    $workerAccess = [System.Security.AccessControl.FileSystemAccessRule]::new(
        $account,
        [System.Security.AccessControl.FileSystemRights]::Modify,
        $inheritance,
        [System.Security.AccessControl.PropagationFlags]::None,
        [System.Security.AccessControl.AccessControlType]::Allow
    )
    $sharedAcl.AddAccessRule($workerAccess)
    Set-Acl -LiteralPath $sharedRoot -AclObject $sharedAcl
    Copy-Item -LiteralPath $worker -Destination $sharedWorker
    Copy-Item -LiteralPath $installer -Destination $sharedInstaller

    $literalWorker = ConvertTo-PSLiteral $sharedWorker
    $literalInstaller = ConvertTo-PSLiteral $sharedInstaller
    $literalEvidence = ConvertTo-PSLiteral $workerEvidence
    $literalRunnerTemp = ConvertTo-PSLiteral $sharedRoot
    $literalRunId = ConvertTo-PSLiteral $env:GITHUB_RUN_ID
    $literalSourceSha = ConvertTo-PSLiteral $env:GITHUB_SHA
    $literalPythonPath = ConvertTo-PSLiteral $pythonPath
    $literalWorkerLog = ConvertTo-PSLiteral $workerLog
    $workerCommand = @"
try {
    & $literalWorker -InstallerPath $literalInstaller -EvidencePath $literalEvidence -TaskWorker -RunnerTemp $literalRunnerTemp -RunId $literalRunId -SourceSha $literalSourceSha -PythonPath $literalPythonPath
    exit 0
} catch {
    `$message = `$_.Exception.Message
    try { [System.IO.File]::AppendAllText($literalWorkerLog, `$message + [Environment]::NewLine) } catch { }
    [Console]::Error.WriteLine(`$message)
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
    if (Test-Path -LiteralPath $workerEvidence -PathType Leaf) {
        Copy-Item -LiteralPath $workerEvidence -Destination $evidence -ErrorAction Stop
    }
    if ($taskInfo.LastTaskResult -ne 0) {
        $evidenceSummary = ''
        if (Test-Path -LiteralPath $evidence -PathType Leaf) {
            $document = Get-Content -LiteralPath $evidence -Raw | ConvertFrom-Json
            $failure = @($document.phases | Where-Object { $_.phase -eq 'failed' } | Select-Object -Last 1)
            if ($failure.Count -gt 0 -and $failure[0].error) {
                $evidenceSummary = ": $($failure[0].error)"
            }
        } elseif (Test-Path -LiteralPath $workerLog -PathType Leaf) {
            $workerError = ([System.IO.File]::ReadAllText($workerLog) -replace '\s+', ' ').Trim()
            if ($workerError.Length -gt 500) { $workerError = $workerError.Substring(0, 500) }
            if ($workerError) { $evidenceSummary = ": $workerError" }
        }
        throw "Isolated elevated NSIS lifecycle task failed with result $($taskInfo.LastTaskResult)$evidenceSummary"
    }
    Assert-Condition (Test-Path -LiteralPath $evidence -PathType Leaf) 'The isolated NSIS lifecycle task did not write evidence.'
    $document = Get-Content -LiteralPath $evidence -Raw | ConvertFrom-Json
    Assert-Condition ($document.status -eq 'PASS') 'The isolated NSIS lifecycle task did not report PASS evidence.'
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
        if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
            $cleanupFailures.Add('The disposable lifecycle scheduled task remained after cleanup.')
        }
    }

    if (-not (Test-Path -LiteralPath $evidence -PathType Leaf) -and (Test-Path -LiteralPath $workerEvidence -PathType Leaf)) {
        try {
            Copy-Item -LiteralPath $workerEvidence -Destination $evidence -ErrorAction Stop
        } catch {
            $cleanupFailures.Add('Could not copy the isolated lifecycle evidence to the runner output path.')
        }
    }
    if (-not (Test-Path -LiteralPath $evidence -PathType Leaf)) {
        $failureDetail = $workerFailure
        if (-not $failureDetail -and (Test-Path -LiteralPath $workerLog -PathType Leaf)) {
            $failureDetail = [System.IO.File]::ReadAllText($workerLog).Trim()
        }
        if (-not $failureDetail) {
            $failureDetail = 'The isolated lifecycle worker ended without writing evidence or an error log.'
        }
        $failureDocument = [ordered]@{
            product = 'Fylura'
            sourceSha = $env:GITHUB_SHA
            runId = $env:GITHUB_RUN_ID
            status = 'FAIL'
            phases = @(
                [ordered]@{
                    phase = 'isolated-worker'
                    at = [DateTime]::UtcNow.ToString('o')
                    error = $failureDetail
                }
            )
        }
        try {
            $failureDocument | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $evidence -Encoding utf8 -ErrorAction Stop
        } catch {
            $cleanupFailures.Add('Could not write failure evidence for the isolated lifecycle worker.')
        }
    }

    if ($userCreated) {
        if (-not $userSid) {
            $createdUser = Get-LocalUser -Name $userName -ErrorAction SilentlyContinue
            if ($createdUser) { $userSid = $createdUser.SID.Value }
        }
        if ($userSid) {
            for ($attempt = 0; $attempt -lt 30; $attempt++) {
                $userProfileRecord = Get-CimInstance -ClassName Win32_UserProfile -ErrorAction SilentlyContinue |
                    Where-Object { $_.SID -eq $userSid } | Select-Object -First 1
                if (-not $userProfileRecord -or -not $userProfileRecord.Loaded) { break }
                Start-Sleep -Milliseconds 500
            }
            if ($userProfileRecord -and $userProfileRecord.Loaded) {
                # The hosted scheduled-task logon can leave the disposable
                # account's registry hive mounted after its process exits.
                # This SID belongs only to the just-created test account.
                $hiveUnloadOutput = & "$env:WINDIR\System32\reg.exe" unload "HKU\$userSid" 2>&1
                $hiveUnloadExitCode = $LASTEXITCODE
                for ($attempt = 0; $attempt -lt 30; $attempt++) {
                    $userProfileRecord = Get-CimInstance -ClassName Win32_UserProfile -ErrorAction SilentlyContinue |
                        Where-Object { $_.SID -eq $userSid } | Select-Object -First 1
                    if (-not $userProfileRecord -or -not $userProfileRecord.Loaded) { break }
                    Start-Sleep -Milliseconds 500
                }
                if ($userProfileRecord -and $userProfileRecord.Loaded) {
                    $unloadDetail = ($hiveUnloadOutput | Out-String).Trim()
                    if (-not $unloadDetail) { $unloadDetail = "reg.exe exit code $hiveUnloadExitCode" }
                    $cleanupFailures.Add("The disposable lifecycle user profile hive remained loaded after cleanup: $unloadDetail")
                }
            }
            if ($userProfileRecord -and -not $userProfileRecord.Loaded) {
                try {
                    $userProfileRecord | Remove-CimInstance -ErrorAction Stop
                } catch {
                    $cleanupFailures.Add("Could not remove the disposable lifecycle user profile record: $($_.Exception.Message)")
                }
            }
        }
        Remove-LocalUser -Name $userName -ErrorAction SilentlyContinue
        $remainingProfile = if ($userSid) {
            Get-CimInstance -ClassName Win32_UserProfile -ErrorAction SilentlyContinue |
                Where-Object { $_.SID -eq $userSid } | Select-Object -First 1
        }
        if ($remainingProfile) {
            $cleanupFailures.Add("The disposable lifecycle user profile remained (Loaded=$($remainingProfile.Loaded), Special=$($remainingProfile.Special), LocalPath=$($remainingProfile.LocalPath)).")
        }
        if (Get-LocalUser -Name $userName -ErrorAction SilentlyContinue) {
            $cleanupFailures.Add('The disposable lifecycle user account remained after cleanup.')
        }
    }

    if (Test-Path -LiteralPath $sharedRoot) {
        try {
            Remove-Item -LiteralPath $sharedRoot -Recurse -Force -ErrorAction Stop
        } catch {
            $cleanupFailures.Add('The isolated lifecycle staging directory could not be removed.')
        }
        if (Test-Path -LiteralPath $sharedRoot) {
            $cleanupFailures.Add('The isolated lifecycle staging directory remained after cleanup.')
        }
    }
}

if ($workerFailure) {
    throw $workerFailure
}
if ($cleanupFailures.Count -gt 0) {
    if (Test-Path -LiteralPath $evidence -PathType Leaf) {
        try {
            $document = Get-Content -LiteralPath $evidence -Raw | ConvertFrom-Json
            $document.status = 'FAIL'
            $document.phases = @($document.phases) + @(
                [pscustomobject]@{
                    phase = 'runner-cleanup-failed'
                    at = [DateTime]::UtcNow.ToString('o')
                    error = ($cleanupFailures -join ' ')
                }
            )
            $document | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $evidence -Encoding utf8
        }
        catch {
            Write-Error 'Could not update the NSIS lifecycle evidence after test-account cleanup failed.'
        }
    }
    throw ($cleanupFailures -join ' ')
}
Write-Output 'PASS: NSIS install, ordinary uninstall, reinstall, explicit data cleanup, credential cleanup, and other-user preservation ran in a disposable elevated profile.'
