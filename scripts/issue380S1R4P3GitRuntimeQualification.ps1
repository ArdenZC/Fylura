[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$EvidenceRoot,

    [Parameter(Mandatory = $true)]
    [string]$TaskRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:FreezeHead = 'af305adb8fba017dd9284a829b5101a209a9b4e2'
$script:FreezeTree = '9d7716add5fde99c6e797065f336e0d0cf86a510'
$script:RunnerPath = 'scripts/issue380S1R4P3SyntheticNsisRunner.ps1'
$script:RunnerBlob = 'c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa'
$script:RunnerSha256 = '92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece'
$script:TestBranch = 'research/issue-380-s1r4-p3-git-runtime-once'
$script:GitInvocations = [System.Collections.Generic.List[object]]::new()
$script:CaseRecords = [System.Collections.Generic.List[object]]::new()
$script:UnprovenCases = [System.Collections.Generic.List[string]]::new()
$script:CleanupRecords = [System.Collections.Generic.List[object]]::new()
$script:Failure = $null
$script:Nonce = [Guid]::NewGuid().ToString('N')
$script:StartUtc = [DateTime]::UtcNow
$script:EvidenceRoot = [System.IO.Path]::GetFullPath($EvidenceRoot)
$script:TaskRoot = [System.IO.Path]::GetFullPath($TaskRoot)
$script:RepoRoot = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$script:RunnerFile = Join-Path $script:RepoRoot $script:RunnerPath
$script:ChildSource = Join-Path $script:RepoRoot 'scripts/issue380S1R4P3GitRuntimeChild.ps1'
$script:RepoPath = Join-Path $script:TaskRoot 'temporary-git-repository'
$script:ChildPath = Join-Path $script:TaskRoot 'issue380S1R4P3GitRuntimeChild.ps1'
$script:PowerShellPath = Join-Path $PSHOME 'pwsh.exe'
$script:GitPath = $null
$script:GitShellPath = $null
$script:FunctionNames = @(
    'Invoke-GitCapture',
    'Get-RequiredGitScalar',
    'Assert-GitNoOutputSuccess',
    'Assert-GitWorktreeClean'
)

function Write-JsonFile([string]$Path, [object]$Value) {
    $json = ConvertTo-Json -InputObject $Value -Depth 40
    [System.IO.File]::WriteAllText(
        $Path,
        $json + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-Equal([object]$Expected, [object]$Actual, [string]$Message) {
    if ($Expected -cne $Actual) { throw "$Message expected=[$Expected] actual=[$Actual]" }
}

function Get-Sha256Hex([byte[]]$Bytes) {
    return [Convert]::ToHexString([System.Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant()
}

function ConvertTo-GitBashPath([string]$Path) {
    $full = [System.IO.Path]::GetFullPath($Path)
    $match = [regex]::Match($full, '^(?<drive>[A-Za-z]):[\\/](?<rest>.*)$')
    if (-not $match.Success) { throw "Test path is not a local drive path: $full" }
    $rest = $match.Groups['rest'].Value.Replace('\', '/').TrimStart('/')
    return '/' + $match.Groups['drive'].Value.ToLowerInvariant() + '/' + $rest
}

function Get-FunctionFragment([string]$Name, [string]$SourceText) {
    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput(
        $SourceText,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if ($parseErrors.Count -gt 0) {
        throw "Frozen Runner parse failed: $($parseErrors | Out-String)"
    }

    $found = @($ast.FindAll({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
            $node.Name -ceq $Name
    }, $true))
    if ($found.Count -ne 1) { throw "Expected exactly one frozen function $Name; found $($found.Count)." }
    return [pscustomobject]@{
        text = [string]$found[0].Extent.Text
        start_offset = [int]$found[0].Extent.StartOffset
        end_offset = [int]$found[0].Extent.EndOffset
    }
}

# Accept dictionaries and PSCustomObject only; never coerce strings/numbers to booleans.
function ConvertTo-TestCaseResult([object]$Value) {
    $fields = @{}
    if ($Value -is [System.Collections.IDictionary]) {
        foreach ($key in $Value.Keys) { $fields[$key] = $Value[$key] }
    } elseif ($Value -is [pscustomobject]) {
        foreach ($property in $Value.PSObject.Properties) { $fields[$property.Name] = $property.Value }
    } else { throw 'Case result must be one dictionary or PSCustomObject.' }
    foreach ($name in @('expected', 'actual')) {
        if (-not $fields.ContainsKey($name) -or $null -eq $fields[$name]) {
            throw "Case result requires non-null $name."
        }
    }
    foreach ($name in @('passed', 'unproven')) {
        if ($fields.ContainsKey($name) -and $fields[$name] -isnot [bool]) {
            throw "Case result $name must be Boolean."
        }
    }
    $passed = $fields.ContainsKey('passed') -and $fields['passed']
    $unproven = $fields.ContainsKey('unproven') -and $fields['unproven']
    if ($passed -and $unproven) { throw 'Case result cannot be both passed and unproven.' }
    if (-not $fields.ContainsKey('passed') -and -not $unproven) { throw 'Case result has no valid state.' }
    $status = if ($unproven) { 'UNPROVEN' } elseif ($passed) { 'PASS' } else { 'FAIL' }
    if ($fields.ContainsKey('status') -and ($fields['status'] -isnot [string] -or $fields['status'] -cne $status)) {
        throw 'Case result status contradicts its Boolean state.'
    }
    return [pscustomobject]@{ status = $status; expected = $fields['expected']; actual = $fields['actual'] }
}

function Get-RuntimeMatrixStatus {
    if ($null -ne $script:Failure -or @($script:CaseRecords | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0) { return 'FAIL' }
    if ($script:UnprovenCases.Count -gt 0) { return 'PARTIAL_BLOCKED_FAULT_INJECTION_UNPROVEN' }
    if ($script:CaseRecords.Count -ne 14) { return 'FAIL' }
    return 'RUNTIME_MATRIX_PASS'
}

function Invoke-TestCase([string]$Name, [scriptblock]$Body) {
    $startIndex = $script:GitInvocations.Count
    $startedUtc = [DateTime]::UtcNow
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $caseStatus = 'FAIL'
    $normalized = $null
    $exceptionText = $null
    $errorRecord = $null
    try {
        $details = & $Body
        $normalized = ConvertTo-TestCaseResult $details
        $caseStatus = $normalized.status
        if ($caseStatus -eq 'UNPROVEN') { $script:UnprovenCases.Add($Name) }
        if ($caseStatus -eq 'FAIL' -and $null -eq $script:Failure) { $script:Failure = "${Name}: Case reported passed=false." }
    } catch {
        $exceptionText = $_.Exception.ToString()
        $errorRecord = [ordered]@{
            fully_qualified_error_id = $_.FullyQualifiedErrorId
            category = $_.CategoryInfo.ToString()
            message = $_.ToString()
            script_stack_trace = $_.ScriptStackTrace
            position_message = $_.InvocationInfo.PositionMessage
        }
        if ($null -eq $script:Failure) { $script:Failure = "${Name}: $exceptionText" }
    } finally { $timer.Stop() }

    $calls = [System.Collections.Generic.List[object]]::new()
    for ($index = $startIndex; $index -lt $script:GitInvocations.Count; $index++) { $calls.Add($script:GitInvocations[$index]) }
    $record = [ordered]@{
        name = $Name
        status = $caseStatus
        expected = 'Valid Case result contract and successful assertions'
        actual = [ordered]@{ result_contract_valid = $false; error = $exceptionText }
        started_utc = $startedUtc.ToString('o')
        ended_utc = [DateTime]::UtcNow.ToString('o')
        elapsed_ms = [Math]::Round($timer.Elapsed.TotalMilliseconds, 3)
        exception = $exceptionText
        error_record = $errorRecord
        git_invocations = @($calls.ToArray())
    }
    if ($null -ne $normalized) {
        # Direct assignment preserves empty/single-element arrays without pipeline expansion.
        $record.expected = $normalized.expected
        $record.actual = $normalized.actual
    }
    $script:CaseRecords.Add($record)
    Write-JsonFile -Path (Join-Path $script:EvidenceRoot ("case-{0:D2}-{1}.json" -f $script:CaseRecords.Count, $Name)) -Value $record
}

function Invoke-GitForTest([string[]]$Arguments, [string]$WorkingDirectory, [int]$TimeoutSeconds = 20) {
    $startedUtc = [DateTime]::UtcNow
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $capture = Invoke-GitCapture -Arguments $Arguments -WorkingDirectory $WorkingDirectory -TimeoutSeconds $TimeoutSeconds
    $timer.Stop()
    $endedUtc = [DateTime]::UtcNow
    $elapsed = [Math]::Round($timer.Elapsed.TotalMilliseconds, 3)
    $capture | Add-Member -NotePropertyName harness_started_utc -NotePropertyValue $startedUtc.ToString('o') -Force
    $capture | Add-Member -NotePropertyName harness_ended_utc -NotePropertyValue $endedUtc.ToString('o') -Force
    $capture | Add-Member -NotePropertyName harness_elapsed_ms -NotePropertyValue $elapsed -Force
    $capture | Add-Member -NotePropertyName configured_explicit_wait_budget_ms -NotePropertyValue (
        [int]$capture.process_timeout_ms + [int]$capture.termination_timeout_ms +
        [int]$capture.output_wait_timeout_ms + [int]$capture.post_close_wait_timeout_ms
    ) -Force
    $invocation = $script:GitInvocations[$script:GitInvocations.Count - 1]
    $invocation['harness_started_utc'] = $startedUtc.ToString('o')
    $invocation['harness_ended_utc'] = $endedUtc.ToString('o')
    $invocation['harness_elapsed_ms'] = $elapsed
    $invocation['configured_explicit_wait_budget_ms'] = $capture.configured_explicit_wait_budget_ms
    return $capture
}

function Assert-GitSuccess([object]$Capture, [string]$Label) {
    Assert-Equal 'EXITED_ZERO' $Capture.status "$Label status"
    Assert-True ($Capture.exit_code -eq 0) "$Label exit code was not zero."
    Assert-True $Capture.process_exit_confirmed "$Label process exit was not confirmed."
    Assert-True ($null -ne $Capture.process_id) "$Label process ID was not recorded."
    Assert-True $Capture.stdout_complete "$Label stdout was incomplete."
    Assert-True $Capture.stderr_complete "$Label stderr was incomplete."
    Assert-True (-not $Capture.timed_out -and -not $Capture.output_timed_out) "$Label timed out."
    Assert-True $Capture.cleanup_completed "$Label cleanup did not complete."
    Assert-True ($Capture.stdout_task_status -eq 'RanToCompletion' -and $Capture.stderr_task_status -eq 'RanToCompletion') "$Label output tasks did not complete successfully."
    Assert-True (-not $Capture.kill_attempted -and $Capture.stream_close_error -eq $null) "$Label has a termination or stream close error."
    Assert-True ($null -eq $Capture.launch_error -and $null -eq $Capture.capture_error -and $null -eq $Capture.process_error -and $null -eq $Capture.cleanup_error) "$Label has a recorded failure."
}

function Assert-GitBashAliasAvailable {
    $gitRoot = Split-Path -Parent (Split-Path -Parent $script:GitPath)
    $candidates = @(
        (Join-Path $gitRoot 'bin/sh.exe'),
        (Join-Path $gitRoot 'usr/bin/sh.exe')
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            $script:GitShellPath = [System.IO.Path]::GetFullPath($candidate)
            return $script:GitShellPath
        }
    }
    throw "Git for Windows sh.exe was not found in known installation paths: $($candidates -join ', ')"
}

function New-GitAliasArguments([string]$Mode, [string]$RecordPath, [int]$DelayMilliseconds, [int]$HoldSeconds) {
    $pwshBash = ConvertTo-GitBashPath $script:PowerShellPath
    $childBash = ConvertTo-GitBashPath $script:ChildPath
    $recordBash = ConvertTo-GitBashPath $RecordPath
    $command = '!"' + $pwshBash + '" -NoLogo -NoProfile -NonInteractive -File "' + $childBash +
        '" -Mode ' + $Mode + ' -RecordPath "' + $recordBash + '" -Nonce ' + $script:Nonce +
        ' -DelayMilliseconds ' + $DelayMilliseconds + ' -HoldSeconds ' + $HoldSeconds
    return @('-c', "alias.s1r4probe=$command", 's1r4probe')
}

function Read-TestChildRecord([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    $record = Get-Content -LiteralPath $Path -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    if ($record -isnot [System.Collections.IDictionary] -or
        -not $record.ContainsKey('mode') -or -not $record.ContainsKey('nonce') -or
        $record['nonce'] -cne $script:Nonce -or
        $record['mode'] -notin @('DelayedOutput', 'HoldPipe', 'BlockForTimeout', 'HoldPipeChild')) {
        throw "Invalid or foreign child record: $Path"
    }
    $pidField = if ($record['mode'] -in @('HoldPipe', 'BlockForTimeout')) { 'child_pid' } else { 'pid' }
    if (-not $record.ContainsKey($pidField) -or $record[$pidField] -isnot [long] -and $record[$pidField] -isnot [int] -or $record[$pidField] -le 0) {
        throw "Child record has no valid $pidField`: $Path"
    }
    return $record
}

function Get-OwnedChildProcess([int]$ProcessId) {
    $item = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId=$ProcessId" -ErrorAction Stop
    if ($null -eq $item) { return $null }
    $expectedImage = [System.IO.Path]::GetFullPath($script:PowerShellPath)
    $actualImage = if ([string]::IsNullOrWhiteSpace([string]$item.ExecutablePath)) { '' } else { [System.IO.Path]::GetFullPath([string]$item.ExecutablePath) }
    if (-not $actualImage.Equals($expectedImage, [System.StringComparison]::OrdinalIgnoreCase) -or
        [string]$item.CommandLine -notlike "*$($script:Nonce)*" -or
        [string]$item.CommandLine -notlike '*issue380S1R4P3GitRuntimeChild.ps1*') {
        throw "Refusing to act on PID $ProcessId because its image or command line does not prove task ownership."
    }
    return $item
}

function Stop-OwnedChild([int]$ProcessId, [string]$Reason) {
    $record = [ordered]@{
        process_id = $ProcessId
        reason = $Reason
        image_path = $null
        command_line = $null
        kill_attempted = $false
        kill_succeeded = $false
        exit_confirmed = $false
        exit_code = $null
        error = $null
    }
    try {
        $owned = Get-OwnedChildProcess -ProcessId $ProcessId
        if ($null -eq $owned) {
            $record.exit_confirmed = $true
            $script:CleanupRecords.Add($record)
            return [pscustomobject]$record
        }
        $record.image_path = [string]$owned.ExecutablePath
        $record.command_line = [string]$owned.CommandLine
        $record.kill_attempted = $true
        $process = [System.Diagnostics.Process]::GetProcessById($ProcessId)
        try {
            $process.Kill($true)
            $record.kill_succeeded = $true
            $record.exit_confirmed = $process.WaitForExit(5000)
            if ($record.exit_confirmed) { $record.exit_code = [int]$process.ExitCode }
        } finally {
            $process.Dispose()
        }
        if (-not $record.exit_confirmed) { throw "Task child $ProcessId did not exit within the configured 5000 ms wait." }
        if ($null -ne (Get-OwnedChildProcess -ProcessId $ProcessId)) { throw "Task child $ProcessId remained visible after termination." }
    } catch {
        $record.error = $_.Exception.ToString()
        if ($null -eq $script:Failure) { $script:Failure = "Child cleanup failed: $($record.error)" }
    }
    $script:CleanupRecords.Add($record)
    return [pscustomobject]$record
}

function Stop-RecordedChildren([string[]]$ParentRecordPaths, [string]$Reason) {
    foreach ($recordPath in $ParentRecordPaths) {
        $record = Read-TestChildRecord $recordPath
        if ($null -eq $record) { continue }
        $childPid = 0
        if ($record.ContainsKey('child_pid')) { $childPid = [int]$record['child_pid'] }
        elseif ($record['mode'] -eq 'HoldPipeChild') { $childPid = [int]$record['pid'] }
        if ($childPid -gt 0) { [void](Stop-OwnedChild -ProcessId $childPid -Reason $Reason) }
    }
}

function Assert-NoOwnedProcessRemains {
    $matches = @(Get-CimInstance -ClassName Win32_Process -ErrorAction Stop |
        Where-Object { [string]$_.CommandLine -like "*$($script:Nonce)*" })
    if ($matches.Count -ne 0) {
        $ids = @($matches | ForEach-Object { [int]$_.ProcessId })
        throw "Task-owned child processes remain after cleanup: $($ids -join ',')"
    }
}

function Invoke-GitFixtureCommand([string[]]$Arguments, [string]$Label) {
    $capture = Invoke-GitForTest -Arguments $Arguments -WorkingDirectory $script:RepoPath
    Assert-GitSuccess -Capture $capture -Label $Label
    return $capture
}

function Test-TaskRootBoundary {
    $runnerTemp = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd([char[]]@('\', '/'))
    $rootPrefix = $runnerTemp + [System.IO.Path]::DirectorySeparatorChar
    foreach ($path in @($script:EvidenceRoot, $script:TaskRoot)) {
        $full = [System.IO.Path]::GetFullPath($path)
        Assert-True ($full.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) "Test root is outside RUNNER_TEMP: $full"
    }
    foreach ($path in @($script:EvidenceRoot, $script:TaskRoot)) {
        if (-not (Test-Path -LiteralPath $path -PathType Container)) { throw "Owned directory is missing: $path" }
        $item = Get-Item -LiteralPath $path -Force -ErrorAction Stop
        if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Owned directory is a reparse point: $path" }
    }
    $runRoot = Join-Path $runnerTemp ("issue380-s1r4-p3-git-runtime-{0}" -f $env:GITHUB_RUN_ID)
    Assert-Equal ([System.IO.Path]::GetFullPath((Join-Path $runRoot 'evidence'))) $script:EvidenceRoot 'EvidenceRoot path'
    Assert-Equal ([System.IO.Path]::GetFullPath((Join-Path $runRoot 'task'))) $script:TaskRoot 'TaskRoot path'
    $preexisting = @(Get-ChildItem -LiteralPath $script:TaskRoot -Force -ErrorAction Stop)
    if ($preexisting.Count -ne 1 -or $preexisting[0].Name -cne 'runtime-task-owner.json') {
        throw "Refusing to run in an unexpected task directory state: $(@($preexisting | ForEach-Object { $_.Name }) -join ', ')"
    }
}

$runRecordPath = Join-Path $script:EvidenceRoot 'runtime-qualification.json'
$startRecordPath = Join-Path $script:EvidenceRoot 'harness-start.json'
    $ownerPath = Join-Path $script:EvidenceRoot 'runtime-owner.json'
$taskOwnerPath = Join-Path $script:TaskRoot 'runtime-task-owner.json'
$preflightPath = Join-Path $script:EvidenceRoot 'preflight.json'
$summaryPath = Join-Path $script:EvidenceRoot 'summary.json'
$script:Result = [ordered]@{
    status = 'NOT_STARTED'
    repository = $env:GITHUB_REPOSITORY
    ref = $env:GITHUB_REF
    triggering_sha = $env:GITHUB_SHA
    run_id = $env:GITHUB_RUN_ID
    run_attempt = $env:GITHUB_RUN_ATTEMPT
    freeze_head = $script:FreezeHead
    freeze_tree = $script:FreezeTree
    runner_blob = $script:RunnerBlob
    runner_sha256_expected = $script:RunnerSha256
    runner_sha256_actual = $null
    runner_blob_actual = $null
    extracted_functions = @()
    powershell_version = $PSVersionTable.PSVersion.ToString()
    powershell_path = $script:PowerShellPath
    git_path = $null
    git_shell_path = $null
    git_version = $null
    git_config_nosystem = $null
    git_global_config_path = $null
    git_template_path = $null
    nonce = $script:Nonce
    started_utc = $script:StartUtc.ToString('o')
    ended_utc = $null
    elapsed_ms = $null
    cases = @()
    git_invocations = @()
    cleanup = @()
    failure = $null
}

try {
    if ($PSVersionTable.PSVersion.Major -ne 7) { throw "PowerShell 7 is required; actual=$($PSVersionTable.PSVersion)" }
    if ($env:GITHUB_ACTIONS -ne 'true' -or $env:GITHUB_REPOSITORY -ne 'ArdenZC/Zen-Canvas') { throw 'Runtime harness may run only in the authorized GitHub Actions repository.' }
    if ($env:GITHUB_REF -ne "refs/heads/$($script:TestBranch)") { throw "Unexpected qualification ref: $env:GITHUB_REF" }
    if ($env:GITHUB_RUN_ATTEMPT -ne '1') { throw "One-shot qualification refuses run attempt $env:GITHUB_RUN_ATTEMPT." }
    Test-TaskRootBoundary

    $owner = Get-Content -LiteralPath $ownerPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    Assert-Equal $env:GITHUB_RUN_ID ([string]$owner.run_id) 'Owner Run ID'
    Assert-Equal $env:GITHUB_RUN_ATTEMPT ([string]$owner.run_attempt) 'Owner Attempt'
    Assert-Equal $env:GITHUB_SHA ([string]$owner.triggering_sha) 'Owner triggering SHA'
    $taskOwner = Get-Content -LiteralPath $taskOwnerPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    Assert-Equal $env:GITHUB_RUN_ID ([string]$taskOwner.run_id) 'Task owner Run ID'
    Assert-Equal $env:GITHUB_RUN_ATTEMPT ([string]$taskOwner.run_attempt) 'Task owner Attempt'
    Assert-Equal $env:GITHUB_SHA ([string]$taskOwner.triggering_sha) 'Task owner triggering SHA'
    $preflight = Get-Content -LiteralPath $preflightPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    Assert-Equal 'PASS' ([string]$preflight.status) 'Workflow preflight status'
    Assert-Equal $env:GITHUB_SHA ([string]$preflight.head) 'Workflow preflight HEAD'
    Assert-Equal $script:FreezeHead ([string]$preflight.freeze_head) 'Workflow preflight code freeze'
    Assert-Equal $script:FreezeTree ([string]$preflight.freeze_tree) 'Workflow preflight code Tree'
    Assert-Equal $script:RunnerBlob ([string]$preflight.runner_blob_actual) 'Workflow preflight Runner Blob'
    Assert-Equal $script:RunnerSha256 ([string]$preflight.runner_sha256_actual) 'Workflow preflight Runner raw SHA'
    Write-JsonFile -Path $startRecordPath -Value ([ordered]@{
        run_id = $env:GITHUB_RUN_ID
        run_attempt = $env:GITHUB_RUN_ATTEMPT
        triggering_sha = $env:GITHUB_SHA
        nonce = $script:Nonce
        harness_pid = $PID
        harness_image_path = $script:PowerShellPath
        started_utc = $script:StartUtc.ToString('o')
    })

    $runnerBytes = [System.IO.File]::ReadAllBytes($script:RunnerFile)
    $script:Result.runner_sha256_actual = Get-Sha256Hex $runnerBytes
    Assert-Equal $script:RunnerSha256 $script:Result.runner_sha256_actual 'Frozen Runner raw SHA-256'
    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    $runnerText = $utf8.GetString($runnerBytes)
    $tokens = $null
    $parseErrors = $null
    $runnerAst = [System.Management.Automation.Language.Parser]::ParseInput($runnerText, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count -gt 0) { throw "Frozen Runner parse failed: $($parseErrors | Out-String)" }

    $fragmentRoot = Join-Path $script:TaskRoot 'extracted-frozen-functions'
    New-Item -ItemType Directory -Path $fragmentRoot -ErrorAction Stop | Out-Null
    $encoding = [System.Text.UTF8Encoding]::new($false)
    foreach ($name in $script:FunctionNames) {
        $extent = Get-FunctionFragment -Name $name -SourceText $runnerText
        $fragment = [string]$extent.text
        $fragmentBytes = $encoding.GetBytes($fragment)
        $prefixBytes = $encoding.GetBytes($runnerText.Substring(0, $extent.start_offset))
        $throughExtentBytes = $encoding.GetBytes($runnerText.Substring(0, $extent.end_offset))
        $sourceByteStart = $prefixBytes.Length
        $sourceByteLength = $throughExtentBytes.Length - $sourceByteStart
        if ($sourceByteLength -ne $fragmentBytes.Length) { throw "Function extent byte length did not map to its source slice: $name" }
        $sourceSliceBytes = [byte[]]::new($sourceByteLength)
        [Array]::Copy($runnerBytes, $sourceByteStart, $sourceSliceBytes, 0, $sourceByteLength)
        $fragmentPath = Join-Path $fragmentRoot ($name + '.ps1')
        [System.IO.File]::WriteAllBytes($fragmentPath, $fragmentBytes)
        $diskBytes = [System.IO.File]::ReadAllBytes($fragmentPath)
        $fragmentHash = Get-Sha256Hex $fragmentBytes
        Assert-Equal $fragmentHash (Get-Sha256Hex $sourceSliceBytes) "$name original source byte slice"
        Assert-Equal $fragmentHash (Get-Sha256Hex $diskBytes) "$name extracted UTF-8 bytes"
        $script:Result.extracted_functions += @([ordered]@{
            name = $name
            source_character_start = $extent.start_offset
            source_character_end = $extent.end_offset
            source_byte_start = $sourceByteStart
            source_byte_length = $sourceByteLength
            source_extent_sha256 = $fragmentHash
            original_source_slice_sha256 = Get-Sha256Hex $sourceSliceBytes
            extracted_file_sha256 = Get-Sha256Hex $diskBytes
            source_extent_length_bytes = $fragmentBytes.Length
            extracted_file = $fragmentPath
            exact_extent_text_copied = $true
        })
    }

    foreach ($name in $script:FunctionNames) { . (Join-Path $fragmentRoot ($name + '.ps1')) }

    $script:GitInvocations.Clear()
    $isolatedGitConfig = Join-Path $script:TaskRoot 'empty-git-global.config'
    $isolatedGitTemplate = Join-Path $script:TaskRoot 'empty-git-template'
    [System.IO.File]::WriteAllText($isolatedGitConfig, [string]::Empty, $encoding)
    New-Item -ItemType Directory -Path $isolatedGitTemplate -ErrorAction Stop | Out-Null
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_CONFIG_GLOBAL = $isolatedGitConfig
    $env:GIT_TEMPLATE_DIR = $isolatedGitTemplate
    $env:GIT_TERMINAL_PROMPT = '0'
    $script:Result.git_config_nosystem = $env:GIT_CONFIG_NOSYSTEM
    $script:Result.git_global_config_path = $env:GIT_CONFIG_GLOBAL
    $script:Result.git_template_path = $env:GIT_TEMPLATE_DIR
    $gitCommand = Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1
    if ($null -eq $gitCommand) { throw 'Git for Windows executable was not found.' }
    $script:GitPath = [string]$gitCommand.Path
    $script:Result.git_path = $script:GitPath

    $treeCapture = Invoke-GitForTest -Arguments @('rev-parse', "$($script:FreezeHead)^{tree}") -WorkingDirectory $script:RepoRoot
    Assert-GitSuccess -Capture $treeCapture -Label 'Frozen code Tree lookup'
    Assert-Equal $script:FreezeTree $treeCapture.stdout.Trim() 'Frozen code Tree'
    $blobCapture = Invoke-GitForTest -Arguments @('rev-parse', "$($script:FreezeHead):$($script:RunnerPath)") -WorkingDirectory $script:RepoRoot
    Assert-GitSuccess -Capture $blobCapture -Label 'Frozen Runner blob lookup'
    Assert-Equal $script:RunnerBlob $blobCapture.stdout.Trim() 'Frozen Runner Git blob'
    $worktreeBlobCapture = Invoke-GitForTest -Arguments @('hash-object', '--', $script:RunnerPath) -WorkingDirectory $script:RepoRoot
    Assert-GitSuccess -Capture $worktreeBlobCapture -Label 'Checked-out Runner blob lookup'
    $script:Result.runner_blob_actual = $worktreeBlobCapture.stdout.Trim()
    Assert-Equal $script:RunnerBlob $script:Result.runner_blob_actual 'Checked-out Runner Git blob'
    $versionCapture = Invoke-GitForTest -Arguments @('--version') -WorkingDirectory $script:RepoRoot
    Assert-GitSuccess -Capture $versionCapture -Label 'Git version lookup'
    $script:Result.git_version = $versionCapture.stdout.Trim()

    New-Item -ItemType Directory -Path $script:RepoPath -ErrorAction Stop | Out-Null
    $init = Invoke-GitForTest -Arguments @('init', '-q') -WorkingDirectory $script:RepoPath
    Assert-GitSuccess -Capture $init -Label 'Temporary Git repository initialization'
    [void](Invoke-GitFixtureCommand -Arguments @('config', 'user.name', 'S1R4 runtime harness') -Label 'Temporary Git user name')
    [void](Invoke-GitFixtureCommand -Arguments @('config', 'user.email', 's1r4-runtime@example.invalid') -Label 'Temporary Git user email')
    [System.IO.File]::WriteAllText((Join-Path $script:RepoPath 'seed.txt'), "synthetic`n", $encoding)
    [void](Invoke-GitFixtureCommand -Arguments @('add', '--', 'seed.txt') -Label 'Temporary Git add')
    [void](Invoke-GitFixtureCommand -Arguments @('commit', '-q', '-m', 'synthetic runtime fixture') -Label 'Temporary Git commit')

    Invoke-TestCase -Name '01_real_git_zero_empty_streams' -Body {
        Assert-GitWorktreeClean -WorkingDirectory $script:RepoPath
        Assert-GitNoOutputSuccess -Arguments @('diff', '--quiet', '--exit-code') -WorkingDirectory $script:RepoPath -Label 'clean git diff'
        $record = $script:GitInvocations[$script:GitInvocations.Count - 1]
        Assert-Equal 0 $record.exit_code 'empty stream exit code'
        Assert-True ($record.stdout_complete -and $record.stderr_complete) 'empty stream completion'
        return @{ passed = $true; expected = 'exit=0; stdout=""; stderr=""; both complete'; actual = $record }
    }

    Invoke-TestCase -Name '02_real_git_nonempty_stdout_and_scalar_wrapper' -Body {
        $value = Get-RequiredGitScalar -Arguments @('rev-parse', 'HEAD') -WorkingDirectory $script:RepoPath -Label 'fixture HEAD'
        Assert-True (-not [string]::IsNullOrWhiteSpace($value)) 'Required scalar wrapper returned an empty value.'
        $record = $script:GitInvocations[$script:GitInvocations.Count - 1]
        Assert-True ($record.stdout_char_count -gt 0) 'Expected non-empty real Git stdout.'
        $dirtyFile = Join-Path $script:RepoPath 'untracked-synthetic.txt'
        [System.IO.File]::WriteAllText($dirtyFile, "synthetic dirty state`n", [System.Text.UTF8Encoding]::new($false))
        $dirtyError = $null
        try { Assert-GitWorktreeClean -WorkingDirectory $script:RepoPath } catch { $dirtyError = $_.Exception.Message }
        $status = Invoke-GitForTest -Arguments @('status', '--porcelain', '--untracked-files', 'all') -WorkingDirectory $script:RepoPath
        Remove-Item -LiteralPath $dirtyFile -Force -ErrorAction Stop
        Assert-True ($dirtyError -like 'Checkout is not clean before Runner starts:*') 'WorktreeClean wrapper did not reject the dirty worktree with its dirty-state error.'
        Assert-Equal 'EXITED_ZERO' $status.status 'Dirty status Git exit status'
        Assert-Equal 0 $status.exit_code 'Dirty status Git exit code'
        Assert-True ($status.stdout_complete -and $status.stderr_complete) 'Dirty status streams were incomplete.'
        Assert-True ($status.stdout_char_count -gt 0) 'Dirty Git status did not produce non-empty stdout.'
        return @{ passed = $true; expected = 'scalar wrapper returns one line; non-empty status is rejected as dirty'; actual = @{ value = $value; scalar_invocation = $record; dirty_status = $status } }
    }

    Invoke-TestCase -Name '03_real_git_nonzero_with_stderr' -Body {
        $capture = Invoke-GitForTest -Arguments @('cat-file', '-p', '0000000000000000000000000000000000000000') -WorkingDirectory $script:RepoPath
        Assert-True ($capture.exit_code -ne 0) 'cat-file -p for the absent object unexpectedly returned exit zero.'
        Assert-Equal 'EXITED_NONZERO' $capture.status 'Missing object cat-file -p Git status'
        Assert-True (-not [string]::IsNullOrWhiteSpace($capture.stderr)) 'cat-file -p did not report stderr for the absent object.'
        Assert-Equal 0 $capture.stdout_char_count 'Missing object cat-file -p stdout length'
        Assert-True ($capture.stderr_complete -and $capture.stdout_complete) 'Nonzero Git streams were not both complete.'
        Assert-True $capture.cleanup_completed 'Nonzero Git process cleanup was incomplete.'
        return @{ passed = $true; expected = 'cat-file -p absent object: nonzero exit, empty stdout, complete non-empty stderr; no error text is pinned'; actual = $capture }
    }

    Invoke-TestCase -Name '03b_real_git_nonzero_empty_stderr_is_not_success' -Body {
        $arguments = @('cat-file', '-e', '0000000000000000000000000000000000000000')
        $capture = Invoke-GitForTest -Arguments $arguments -WorkingDirectory $script:RepoPath
        Assert-True ($capture.exit_code -ne 0) 'cat-file -e for the absent object unexpectedly returned exit zero.'
        Assert-Equal 'EXITED_NONZERO' $capture.status 'Empty-stderr failure Git status'
        Assert-Equal 0 $capture.stdout_char_count 'Empty-stderr failure stdout length'
        Assert-Equal 0 $capture.stderr_char_count 'Empty-stderr failure stderr length'
        Assert-Equal '' $capture.stdout 'Empty-stderr failure stdout'
        Assert-Equal '' $capture.stderr 'Empty-stderr failure stderr'
        Assert-True ($capture.stdout_complete -and $capture.stderr_complete -and $capture.cleanup_completed) 'Empty-stderr failure capture or cleanup was incomplete.'

        $wrapperError = $null
        try {
            Assert-GitNoOutputSuccess -Arguments $arguments -WorkingDirectory $script:RepoPath -Label 'absent-object empty-stderr probe'
        } catch {
            $wrapperError = $_.Exception.Message
        }
        Assert-True (-not [string]::IsNullOrWhiteSpace($wrapperError)) 'Assert-GitNoOutputSuccess accepted a nonzero Git result with empty stderr.'
        $wrapperCapture = $script:GitInvocations[$script:GitInvocations.Count - 1]
        Assert-Equal 'EXITED_NONZERO' $wrapperCapture.status 'Wrapper rejection status'
        Assert-Equal $capture.exit_code $wrapperCapture.exit_code 'Wrapper rejection exit code'
        Assert-Equal 0 $wrapperCapture.stdout_char_count 'Wrapper rejection stdout length'
        Assert-Equal 0 $wrapperCapture.stderr_char_count 'Wrapper rejection stderr length'
        Assert-True ($wrapperCapture.stdout_complete -and $wrapperCapture.stderr_complete -and $wrapperCapture.cleanup_completed) 'Wrapper rejection capture or cleanup was incomplete.'
        Assert-True ($wrapperError.Contains("exit=$($wrapperCapture.exit_code)")) 'Wrapper rejection did not report the nonzero exit code.'
        return @{ passed = $true; expected = 'real nonzero/empty-stderr result remains EXITED_NONZERO; no-output success wrapper rejects it using exit code'; actual = @{ capture = $capture; wrapper_capture = $wrapperCapture; wrapper_error = $wrapperError } }
    }

    Invoke-TestCase -Name '04_real_git_normal_exit_and_stream_close' -Body {
        $capture = Invoke-GitForTest -Arguments @('--version') -WorkingDirectory $script:RepoPath
        Assert-GitSuccess -Capture $capture -Label 'Git version'
        Assert-True (-not [string]::IsNullOrWhiteSpace($capture.stdout)) 'Git version did not emit stdout.'
        Assert-Equal '' $capture.stderr 'Git version stderr'
        return @{ passed = $true; expected = 'normal exit; stdout and stderr complete'; actual = $capture }
    }

    $script:Result.git_shell_path = Assert-GitBashAliasAvailable
    $delayedRecordPath = Join-Path $script:TaskRoot 'delayed-child.json'
    Copy-Item -LiteralPath $script:ChildSource -Destination $script:ChildPath -ErrorAction Stop
    Invoke-TestCase -Name '05_real_git_alias_delayed_streams_under_five_seconds' -Body {
        $args = New-GitAliasArguments -Mode 'DelayedOutput' -RecordPath $delayedRecordPath -DelayMilliseconds 1200 -HoldSeconds 1
        $capture = Invoke-GitForTest -Arguments $args -WorkingDirectory $script:RepoPath -TimeoutSeconds 8
        $childRecord = Read-TestChildRecord $delayedRecordPath
        Assert-GitSuccess -Capture $capture -Label 'delayed Git alias'
        Assert-True ($null -ne $childRecord -and $null -ne $childRecord.pid) 'Delayed child process identity was not recorded.'
        Assert-Equal 0 $childRecord.exit_code 'Delayed child exit code'
        Assert-True ($capture.harness_elapsed_ms -lt 5000) 'Delayed output did not complete within the requested 5 second qualification bound.'
        Assert-True ($capture.stdout.Contains("S1R4_STDOUT_DELAYED|$($script:Nonce)")) 'Delayed stdout marker is missing.'
        Assert-True ($capture.stderr.Contains("S1R4_STDERR_DELAYED|$($script:Nonce)")) 'Delayed stderr marker is missing.'
        return @{ passed = $true; expected = 'both streams complete within 5000 ms'; actual = @{ capture = $capture; child = $childRecord } }
    }

    $pipeRecordPath = Join-Path $script:TaskRoot 'pipe-holder.json'
    Invoke-TestCase -Name '06_git_exit_with_inherited_pipe_holder' -Body {
        $args = New-GitAliasArguments -Mode 'HoldPipe' -RecordPath $pipeRecordPath -DelayMilliseconds 0 -HoldSeconds 15
        $capture = Invoke-GitForTest -Arguments $args -WorkingDirectory $script:RepoPath -TimeoutSeconds 10
        $childRecord = Read-TestChildRecord $pipeRecordPath
        Assert-True ($null -ne $childRecord -and $null -ne $childRecord.child_pid) 'Controlled pipe-holder child identity was not recorded.'
        Assert-True ($capture.output_timed_out -or $null -ne $capture.capture_error -or -not ($capture.stdout_complete -and $capture.stderr_complete)) 'Inherited pipe did not cause the capture path to report incomplete output.'
        Assert-True ($capture.status -ne 'EXITED_ZERO') 'Incomplete output was misclassified as successful Git capture.'
        $child = Get-OwnedChildProcess -ProcessId ([int]$childRecord.child_pid)
        Assert-True ($null -ne $child) 'Pipe-holder child exited before inherited-handle behavior was observed.'
        $cleanup = Stop-OwnedChild -ProcessId ([int]$childRecord.child_pid) -Reason 'after inherited pipe observation'
        Assert-True $cleanup.exit_confirmed 'Pipe-holder cleanup was not confirmed.'
        return @{ passed = $true; expected = 'Git exit plus incomplete output is fail-closed; owned child then reaped'; actual = @{ capture = $capture; child = $childRecord; cleanup = $cleanup } }
    }

    $timeoutRecordPath = Join-Path $script:TaskRoot 'timeout-child.json'
    Invoke-TestCase -Name '07_git_process_timeout_and_tree_termination' -Body {
        $args = New-GitAliasArguments -Mode 'BlockForTimeout' -RecordPath $timeoutRecordPath -DelayMilliseconds 0 -HoldSeconds 120
        $capture = Invoke-GitForTest -Arguments $args -WorkingDirectory $script:RepoPath -TimeoutSeconds 2
        $childRecord = Read-TestChildRecord $timeoutRecordPath
        Assert-True ($null -ne $childRecord -and $null -ne $childRecord.child_pid) 'Timeout fixture did not record its controlled descendant.'
        Assert-True $capture.timed_out 'Expected Git process timeout was not recorded.'
        Assert-True $capture.kill_attempted 'Process-tree kill was not attempted.'
        Assert-True $capture.kill_succeeded 'Process-tree termination was not recorded as successful.'
        Assert-True $capture.process_exit_confirmed 'Git process exit was not confirmed after timeout.'
        Assert-True ($capture.status -ne 'EXITED_ZERO') 'A timed out Git process was classified as successful.'
        $remaining = Get-OwnedChildProcess -ProcessId ([int]$childRecord.child_pid)
        if ($null -ne $remaining) {
            $cleanup = Stop-OwnedChild -ProcessId ([int]$childRecord.child_pid) -Reason 'emergency cleanup after Git tree termination assertion'
            Assert-True $cleanup.exit_confirmed 'Emergency child cleanup did not complete.'
            throw 'Invoke-GitCapture failed to terminate the controlled descendant as part of the Git process tree.'
        }
        return @{ passed = $true; expected = 'timeout recorded; kill attempted; process tree exited; no controlled descendant remains'; actual = @{ capture = $capture; child = $childRecord } }
    }

    Invoke-TestCase -Name '08_termination_failure_injection_limit' -Body {
        $record = [ordered]@{
            attempted = $false
            result = 'NOT_DETERMINISTICALLY_INJECTABLE_WITH_FROZEN_FUNCTION'
            reason = 'Invoke-GitCapture constructs System.Diagnostics.Process directly and exposes no kill/cleanup fault seam. A permission-denied or orphan fixture would exceed this harness safety contract.'
            observation = 'Timeout case 07 still asserts fail-closed status and records kill_error/cleanup_error, if naturally observed.'
        }
        return @{ unproven = $true; expected = 'runtime evidence or Owner-approved testability disposition'; actual = $record }
    }

    Invoke-TestCase -Name '09_output_deadline_and_task_terminal_state' -Body {
        $previous = $script:CaseRecords | Where-Object { $_.name -eq '06_git_exit_with_inherited_pipe_holder' } | Select-Object -First 1
        Assert-True ($null -ne $previous) 'Inherited pipe deadline case is missing.'
        $capture = $previous.git_invocations | Select-Object -Last 1
        Assert-True ($capture.output_timed_out -or $null -ne $capture.capture_error -or $capture.stdout_task_status -in @('Faulted', 'Canceled') -or $capture.stderr_task_status -in @('Faulted', 'Canceled')) 'Output deadline, task fault, or cancellation was not observed.'
        Assert-True ($capture.status -notin @('EXITED_ZERO', 'EXITED_NONZERO')) 'Incomplete output was treated as a completed Git result.'
        return @{ passed = $true; expected = 'output deadline/failure recorded; capture status is not an exit success'; actual = $capture }
    }

    Invoke-TestCase -Name '10_consecutive_invocation_records_are_independent' -Body {
        $first = Invoke-GitForTest -Arguments @('cat-file', '-e', '0000000000000000000000000000000000000000') -WorkingDirectory $script:RepoPath
        $second = Invoke-GitForTest -Arguments @('rev-parse', 'HEAD') -WorkingDirectory $script:RepoPath
        Assert-True ($first.exit_code -ne 0) 'First invocation did not fail as expected.'
        Assert-GitSuccess -Capture $second -Label 'second independent invocation'
        Assert-True ($first.process_id -ne $second.process_id) 'Consecutive Git calls reused one recorded PID.'
        Assert-True ($script:GitInvocations.Count -ge 2) 'Invocation history is incomplete.'
        $priorPair = @($script:GitInvocations[$script:GitInvocations.Count - 2], $script:GitInvocations[$script:GitInvocations.Count - 1])
        Assert-Equal $first.exit_code $priorPair[0].exit_code 'First recorded exit code'
        Assert-Equal $second.exit_code $priorPair[1].exit_code 'Second recorded exit code'
        return @{ passed = $true; expected = 'failure then success remain distinct in ordered invocation history'; actual = $priorPair }
    }

    Invoke-TestCase -Name '11_evidence_json_temporal_and_status_consistency' -Body {
        foreach ($case in $script:CaseRecords) {
            Assert-True ([DateTimeOffset]::Parse([string]$case.ended_utc) -ge [DateTimeOffset]::Parse([string]$case.started_utc)) "Case time order is invalid: $($case.name)"
            Assert-True ([double]$case.elapsed_ms -ge 0) "Case elapsed time is invalid: $($case.name)"
            foreach ($capture in $case.git_invocations) {
                if ($capture.status -eq 'EXITED_ZERO') {
                    Assert-True ($capture.exit_code -eq 0 -and $capture.stdout_complete -and $capture.stderr_complete -and $capture.cleanup_completed -and -not $capture.timed_out -and -not $capture.output_timed_out) 'EXITED_ZERO record contradicts its completion fields.'
                }
                if ($capture.timed_out -or $capture.output_timed_out -or $null -ne $capture.capture_error -or $null -ne $capture.cleanup_error -or $null -ne $capture.kill_error) {
                    Assert-True ($capture.status -notin @('EXITED_ZERO', 'EXITED_NONZERO')) 'Error/timeout record was classified as an ordinary completed exit.'
                }
            }
        }
        return @{ passed = $true; expected = 'timestamps and capture status fields are internally consistent'; actual = @{ cases = $script:CaseRecords.Count; invocations = $script:GitInvocations.Count } }
    }

    Invoke-TestCase -Name '12_exception_fails_closed_without_a_live_test_child' -Body {
        $emptyScalarRejected = $false
        try {
            [void](Get-RequiredGitScalar -Arguments @('rev-list', '--max-count=0', 'HEAD') -WorkingDirectory $script:RepoPath -Label 'empty scalar fixture')
        } catch { $emptyScalarRejected = $true }
        Assert-True $emptyScalarRejected 'Required scalar wrapper accepted empty stdout with exit zero.'
        $missing = Join-Path $script:TaskRoot 'missing-working-directory'
        $launchRejected = $false
        try {
            [void](Get-RequiredGitScalar -Arguments @('rev-parse', 'HEAD') -WorkingDirectory $missing -Label 'missing working directory')
        } catch { $launchRejected = $true }
        Assert-True $launchRejected 'Required scalar wrapper accepted a launch failure.'
        $capture = $script:GitInvocations[$script:GitInvocations.Count - 1]
        Assert-Equal 'LAUNCH_FAILED' $capture.status 'Invalid working directory status'
        Assert-True ($null -ne $capture.launch_error) 'Launch exception was not retained.'
        Assert-True ($capture.status -ne 'EXITED_ZERO') 'Launch failure was misclassified as success.'
        Assert-NoOwnedProcessRemains
        return @{ passed = $true; expected = 'launch exception retained; no success status; no task-owned child'; actual = $capture }
    }

    Invoke-TestCase -Name '13_final_evidence_json_temporal_and_status_consistency' -Body {
        foreach ($case in $script:CaseRecords) {
            $start = [DateTimeOffset]::Parse([string]$case.started_utc)
            $end = [DateTimeOffset]::Parse([string]$case.ended_utc)
            Assert-True ($end -ge $start) "Case time order is invalid: $($case.name)"
            Assert-True ([double]$case.elapsed_ms -ge 0) "Case elapsed time is invalid: $($case.name)"
            foreach ($capture in $case.git_invocations) {
                $captureStart = [DateTimeOffset]::Parse([string]$capture.harness_started_utc)
                $captureEnd = [DateTimeOffset]::Parse([string]$capture.harness_ended_utc)
                Assert-True ($captureEnd -ge $captureStart) 'Git invocation time order is invalid.'
                Assert-True ([double]$capture.harness_elapsed_ms -ge 0) 'Git invocation elapsed time is negative.'
                Assert-True ([int]$capture.configured_explicit_wait_budget_ms -ge [int]$capture.process_timeout_ms) 'Explicit wait budget is inconsistent.'
                if ($capture.status -eq 'EXITED_ZERO') {
                    Assert-True ($capture.exit_code -eq 0 -and $capture.stdout_complete -and $capture.stderr_complete -and $capture.cleanup_completed -and -not $capture.timed_out -and -not $capture.output_timed_out) 'EXITED_ZERO record contradicts its completion fields.'
                }
                if ($capture.timed_out -or $capture.output_timed_out -or $null -ne $capture.capture_error -or $null -ne $capture.cleanup_error -or $null -ne $capture.kill_error) {
                    Assert-True ($capture.status -notin @('EXITED_ZERO', 'EXITED_NONZERO')) 'Error/timeout record was classified as an ordinary completed exit.'
                }
            }
        }
        return @{ passed = $true; expected = 'all case and Git invocation timestamps/statuses are internally consistent'; actual = @{ cases_checked = $script:CaseRecords.Count; invocations_checked = $script:GitInvocations.Count } }
    }

    Assert-NoOwnedProcessRemains
    $script:Result.status = Get-RuntimeMatrixStatus
    if ($script:Result.status -eq 'PARTIAL_BLOCKED_FAULT_INJECTION_UNPROVEN') {
        $script:Result.status = 'PARTIAL_BLOCKED_FAULT_INJECTION_UNPROVEN'
        $script:Failure = 'At least one required cleanup-failure behavior had no deterministic, safe injection seam.'
    } elseif ($script:Result.status -eq 'RUNTIME_MATRIX_PASS') {
        $script:Result.status = 'RUNTIME_MATRIX_PASS'
    }
} catch {
    $script:Failure = if ($null -eq $script:Failure) { $_.Exception.ToString() } else { "$($script:Failure)`n$($_.Exception.ToString())" }
    $script:Result.status = 'FAIL'
} finally {
    try {
        Stop-RecordedChildren -ParentRecordPaths @(
            (Join-Path $script:TaskRoot 'pipe-holder.json'),
            (Join-Path $script:TaskRoot 'timeout-child.json')
        ) -Reason 'final safety cleanup'
        Assert-NoOwnedProcessRemains
    } catch {
        if ($null -eq $script:Failure) { $script:Failure = "Final cleanup failed: $($_.Exception.ToString())" }
        else { $script:Failure += "`nFinal cleanup failed: $($_.Exception.ToString())" }
        $script:Result.status = 'FAIL_CLEANUP_UNCONFIRMED'
    }
    $script:Result.cleanup = @($script:CleanupRecords.ToArray())
    $script:Result.cases = @($script:CaseRecords.ToArray())
    $script:Result.git_invocations = @($script:GitInvocations.ToArray())
    $script:Result.failure = $script:Failure
    $script:Result.ended_utc = [DateTime]::UtcNow.ToString('o')
    $script:Result.elapsed_ms = [Math]::Round(([DateTime]::UtcNow - $script:StartUtc).TotalMilliseconds, 3)
    try {
        Write-JsonFile -Path $runRecordPath -Value $script:Result
        $roundTrip = Get-Content -LiteralPath $runRecordPath -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
        if (@($roundTrip.cases).Count -ne $script:CaseRecords.Count) { throw 'Final evidence JSON round-trip lost a case record.' }
        if (@($roundTrip.git_invocations).Count -ne $script:GitInvocations.Count) { throw 'Final evidence JSON round-trip lost a Git invocation.' }
        foreach ($case in @($roundTrip.cases)) {
            if ($null -eq $case.expected -or $null -eq $case.actual) { throw "Final evidence case has no expected/actual value: $($case.name)" }
            if ($case.status -notin @('PASS', 'UNPROVEN', 'FAIL')) { throw "Final evidence case has an unknown status: $($case.name) / $($case.status)" }
            if ([DateTimeOffset]::Parse([string]$case.ended_utc) -lt [DateTimeOffset]::Parse([string]$case.started_utc)) { throw "Final evidence case time order is invalid: $($case.name)" }
        }
    } catch {
        $script:Result.status = 'FAIL_EVIDENCE_ROUNDTRIP'
        $script:Result.failure = "$($script:Result.failure)`nEvidence round-trip failed: $($_.Exception.ToString())"
        try { Write-JsonFile -Path $runRecordPath -Value $script:Result } catch { [Console]::Error.WriteLine("Could not rewrite failed runtime evidence: $($_.Exception)") }
        [Console]::Error.WriteLine("Could not validate final runtime evidence: $($_.Exception)")
    }
    try { Write-JsonFile -Path $summaryPath -Value ([ordered]@{ status = $script:Result.status; failure = $script:Result.failure; run_id = $env:GITHUB_RUN_ID; run_attempt = $env:GITHUB_RUN_ATTEMPT; ended_utc = $script:Result.ended_utc; cleanup = $script:Result.cleanup }) } catch {
        $script:Result.status = 'FAIL_EVIDENCE_WRITE'
        $script:Result.failure = "$($script:Result.failure)`nCould not persist summary evidence: $($_.Exception)"
        try { Write-JsonFile -Path $runRecordPath -Value $script:Result } catch { [Console]::Error.WriteLine($_.Exception.ToString()) }
        [Console]::Error.WriteLine($script:Result.failure)
    }
}

if ($script:Result.status -ne 'RUNTIME_MATRIX_PASS') { exit 1 }
exit 0
