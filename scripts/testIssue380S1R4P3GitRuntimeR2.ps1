[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$OutputRoot)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Checks = [System.Collections.Generic.List[string]]::new()
function Check([bool]$Value, [string]$Label) {
    if (-not $Value) { throw $Label }
    $script:Checks.Add($Label)
}
function Parse-Source([string]$Path) {
    $tokens = $null; $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errors)
    if ($errors.Count -ne 0) { throw ($errors | Out-String) }
    return $ast
}
function Import-Functions($Ast, [string[]]$Names) {
    foreach ($node in $Ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
        if ($null -ne $Names -and $node.Name -notin $Names) { continue }
        . ([scriptblock]::Create($node.Extent.Text))
        Set-Item -Path "function:script:$($node.Name)" -Value (Get-Item "function:$($node.Name)").ScriptBlock
    }
}
$repo = Split-Path -Parent $PSScriptRoot
$harness = Parse-Source (Join-Path $PSScriptRoot 'issue380S1R4P3GitRuntimeQualification.ps1')
$runner = Parse-Source (Join-Path $PSScriptRoot 'issue380S1R4P3SyntheticNsisRunner.ps1')
Import-Functions $harness $null
Import-Functions $runner @('Invoke-GitCapture', 'Get-RequiredGitScalar', 'Assert-GitNoOutputSuccess', 'Assert-GitWorktreeClean')
[void][System.IO.Directory]::CreateDirectory($OutputRoot)
$script:EvidenceRoot = Join-Path $OutputRoot 'cases'
$script:TaskRoot = Join-Path $OutputRoot 'task'
$script:RepoPath = Join-Path $script:TaskRoot 'repo'
foreach ($path in @($script:EvidenceRoot, $script:RepoPath)) { [void][System.IO.Directory]::CreateDirectory($path) }
$script:GitInvocations = [System.Collections.Generic.List[object]]::new()
$script:CaseRecords = [System.Collections.Generic.List[object]]::new()
$script:UnprovenCases = [System.Collections.Generic.List[string]]::new()
$script:CleanupRecords = [System.Collections.Generic.List[object]]::new()
$script:Failure = $null
$env:GIT_CONFIG_NOSYSTEM = '1'
$env:GIT_CONFIG_GLOBAL = Join-Path $OutputRoot 'empty-global.config'
$env:GIT_TEMPLATE_DIR = Join-Path $OutputRoot 'empty-template'
[System.IO.File]::WriteAllText($env:GIT_CONFIG_GLOBAL, '')
[void][System.IO.Directory]::CreateDirectory($env:GIT_TEMPLATE_DIR)
$env:GIT_TERMINAL_PROMPT = '0'
foreach ($args in @(@('init', '-q'), @('config', 'user.name', 'fixture'), @('config', 'user.email', 'fixture@example.invalid'))) {
    $capture = Invoke-GitForTest $args $script:RepoPath
    Assert-GitSuccess $capture 'fixture setup'
}
$tracked = Join-Path $script:RepoPath 'tracked.txt'
[System.IO.File]::WriteAllText($tracked, "original`n")
foreach ($args in @(@('add', '--', 'tracked.txt'), @('commit', '-q', '-m', 'fixture'))) { Assert-GitSuccess (Invoke-GitForTest $args $script:RepoPath) 'fixture seed' }
# Real Git command semantics and clean-gate cases; no ignore files or gate relaxation.
Assert-GitWorktreeClean $script:RepoPath
Check $true 'clean repository passes the real wrapper'
foreach ($mode in @('untracked', 'modified', 'nested')) {
    $path = switch ($mode) {
        untracked { Join-Path $script:RepoPath 'untracked.txt' }
        modified { $tracked }
        nested { Join-Path $script:RepoPath 'nested/deeper/untracked.txt' }
    }
    [void][System.IO.Directory]::CreateDirectory((Split-Path -Parent $path))
    [System.IO.File]::WriteAllText($path, "dirty $mode`n")
    $errorText = ''; try { Assert-GitWorktreeClean $script:RepoPath } catch { $errorText = $_.Exception.Message }
    $invocation = $script:GitInvocations[$script:GitInvocations.Count - 1]
    Check ($errorText.StartsWith('Checkout is not clean before Runner starts:') -and $invocation.exit_code -eq 0 -and $invocation.stdout_char_count -gt 0) "$mode is rejected by the real clean gate"
    if ($mode -eq 'modified') { [System.IO.File]::WriteAllText($tracked, "original`n") }
    else { Remove-Item -LiteralPath $path -Force }
}
Assert-GitWorktreeClean $script:RepoPath
Check $true 'restored clean fixture passes without exclusions'
$notRepo = Join-Path $script:TaskRoot 'not-a-repository'
[void][System.IO.Directory]::CreateDirectory($notRepo)
$errorText = ''; try { Assert-GitWorktreeClean $notRepo } catch { $errorText = $_.Exception.Message }
$failedGit = $script:GitInvocations[$script:GitInvocations.Count - 1]
Check ($errorText.StartsWith('Git worktree status failed:') -and $failedGit.status -eq 'EXITED_NONZERO' -and $failedGit.exit_code -ne 0) 'Git execution failure fails the clean gate'
# Only Windows CIM is replaced for Case 12. All Git calls/assertions are the actual Case body.
function Assert-NoOwnedProcessRemains { }
$commands = @($harness.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] -and $n.GetCommandName() -eq 'Invoke-TestCase' }, $true))
foreach ($prefix in @('01_', '02_', '03_', '03b_', '04_', '08_', '10_', '11_', '12_', '13_')) {
    $command = @($commands | Where-Object { $_.CommandElements[2].Value.StartsWith($prefix) })
    Check ($command.Count -eq 1) "unique Case $prefix"
    . ([scriptblock]::Create($command[0].Extent.Text))
    $case = $script:CaseRecords[$script:CaseRecords.Count - 1]
    $expected = if ($prefix -eq '08_') { 'UNPROVEN' } else { 'PASS' }
    Check ($case.status -eq $expected) "actual Case $prefix $expected (CIM offline double only)"
}
Check ((Get-RuntimeMatrixStatus) -eq 'PARTIAL_BLOCKED_FAULT_INJECTION_UNPROVEN') 'Case 08 remains UNPROVEN and cannot yield complete PASS'
$launch = $script:GitInvocations[$script:GitInvocations.Count - 1]
Check ($launch.status -eq 'LAUNCH_FAILED' -and $null -ne $launch.launch_error -and $null -eq $launch.process_id -and
    -not $launch.stream_close_attempted -and $null -eq $launch.stream_close_error -and $launch.cleanup_completed) 'real invalid-directory launch failure retains original error and avoids uninitialized streams'
# Raw and decorated invocations are legal; optional timing must be all-or-none, real and ordered.
$raw = Invoke-GitCapture @('--version') $script:RepoPath
$rawJson = ConvertTo-Json -InputObject $raw -Depth 8
Assert-GitInvocationConsistency $raw
Check ((ConvertTo-Json -InputObject $raw -Depth 8) -ceq $rawJson) 'raw PSCustomObject validation neither adds timestamps nor changes capture'
$decorated = Invoke-GitForTest @('--version') $script:RepoPath
Assert-GitInvocationConsistency $decorated
Check $true 'actual decorated record has a valid ordered timing group'
$rawRecord = $rawJson | ConvertFrom-Json -AsHashtable
Assert-GitInvocationConsistency $rawRecord
Check $true 'raw hashtable from JSON has no required Harness timestamps'
foreach ($mode in @('unknown-status', 'invalid-budget-type', 'invalid-Boolean', 'missing-base', 'partial-timing', 'null-time', 'invalid-time', 'reverse-time', 'negative-elapsed', 'nan-elapsed', 'wrong-budget', 'exit-zero-contradiction', 'exit-nonzero-contradiction', 'incomplete-streams', 'incomplete-cleanup', 'error-with-success', 'timeout-with-success')) {
    $fields = ConvertTo-Json -InputObject $decorated -Depth 8 | ConvertFrom-Json -AsHashtable
    switch ($mode) {
        unknown-status { $fields['status'] = 'UNKNOWN' }
        invalid-budget-type { $fields['process_timeout_ms'] = '20000' }
        invalid-Boolean { $fields['stdout_complete'] = 'true' }
        missing-base { $fields.Remove('exit_code') }
        partial-timing { $fields.Remove('harness_ended_utc') }
        null-time { $fields['harness_started_utc'] = $null }
        invalid-time { $fields['harness_started_utc'] = 'not-a-date' }
        reverse-time { $fields['harness_started_utc'] = '2099-01-01T00:00:00Z' }
        negative-elapsed { $fields['harness_elapsed_ms'] = -1 }
        nan-elapsed { $fields['harness_elapsed_ms'] = [double]::NaN }
        wrong-budget { $fields['configured_explicit_wait_budget_ms'] = 0 }
        exit-zero-contradiction { $fields['exit_code'] = 1 }
        exit-nonzero-contradiction { $fields['status'] = 'EXITED_NONZERO'; $fields['exit_code'] = 0 }
        incomplete-streams { $fields['stdout_complete'] = $false }
        incomplete-cleanup { $fields['cleanup_completed'] = $false }
        error-with-success { $fields['capture_error'] = 'sentinel' }
        timeout-with-success { $fields['timed_out'] = $true }
    }
    $rejected = $false; try { Assert-GitInvocationConsistency $fields } catch { $rejected = $true }
    Check $rejected "invocation contract rejects $mode"
}
# Execute the candidate's exact initialization, finally and result classification AST.
# Controlled objects inject otherwise nondeterministic faults; these are PowerShell branch tests,
# not OS cleanup-failure injection or Windows acceptance evidence.
$captureAst = @($runner.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Invoke-GitCapture' }, $false))[0]
$statements = @($captureAst.Body.EndBlock.Statements)
$tryIndex = 0
while ($statements[$tryIndex] -isnot [System.Management.Automation.Language.TryStatementAst]) { $tryIndex++ }
$script:CaptureInitial = [scriptblock]::Create(($statements[0..($tryIndex-1)] | ForEach-Object { $_.Extent.Text }) -join "`n")
$script:CaptureFinally = [scriptblock]::Create(($statements[$tryIndex].Finally.Statements | ForEach-Object { $_.Extent.Text }) -join "`n")
$script:CaptureResult = [scriptblock]::Create(($statements[($tryIndex+1)..($statements.Count-1)] | ForEach-Object { $_.Extent.Text }) -join "`n")
function Invoke-FinalizerFault([string]$Mode) {
    $Arguments = @('--version'); $WorkingDirectory = $script:RepoPath; $TimeoutSeconds = 1
    . $script:CaptureInitial
    $process = [pscustomobject]@{ HasExited = $true; ExitCode = 0; Disposed = $false; FailDispose = ($Mode -in @('dispose-fault', 'launch-plus-cleanup-fault')) }
    $process | Add-Member -MemberType ScriptMethod -Name Dispose -Value { $this.Disposed = $true; if ($this.FailDispose) { throw 'DISPOSE_SENTINEL' } }
    $processStarted = $true
    if ($Mode -in @('unstarted', 'launch-plus-cleanup-fault')) { $processStarted = $false; $capture.launch_error = 'ORIGINAL_LAUNCH_SENTINEL' }
    else {
        $stdoutReader = [pscustomobject]@{ Closed = $false; FailClose = ($Mode -eq 'close-fault') }
        $stdoutReader | Add-Member -MemberType ScriptMethod -Name Close -Value { $this.Closed = $true; if ($this.FailClose) { throw 'CLOSE_SENTINEL' } }
        if ($Mode -ne 'partial-reader') {
            $stderrReader = [pscustomobject]@{ Closed = $false }
            $stderrReader | Add-Member -MemberType ScriptMethod -Name Close -Value { $this.Closed = $true }
            $stdoutTask = [System.Threading.Tasks.Task]::FromResult([string]'stdout')
            $stderrTask = [System.Threading.Tasks.Task]::FromResult([string]'stderr')
            if ($Mode -in @('read-fault', 'close-fault')) { $stdoutTask = [System.Threading.Tasks.Task]::FromException([InvalidOperationException]::new('READ_SENTINEL')) }
        } else { $capture.process_error = 'PARTIAL_SETUP_SENTINEL' }
    }
    . $script:CaptureFinally
    $result = . $script:CaptureResult
    return [pscustomobject]@{ capture = $result; process = $process; stdout_reader = $stdoutReader; stderr_reader = $stderrReader }
}
$faults = [System.Collections.Generic.List[object]]::new()
foreach ($mode in @('normal', 'unstarted', 'partial-reader', 'read-fault', 'close-fault', 'dispose-fault', 'launch-plus-cleanup-fault')) {
    $scenario = Invoke-FinalizerFault $mode
    $capture = $scenario.capture
    Check $scenario.process.Disposed "$mode process object disposed"
    switch ($mode) {
        normal { Check ($capture.status -eq 'EXITED_ZERO' -and $capture.stdout -eq 'stdout' -and $capture.stderr -eq 'stderr') 'normal initialized streams remain complete' }
        unstarted { Check ($capture.status -eq 'LAUNCH_FAILED' -and $capture.launch_error -eq 'ORIGINAL_LAUNCH_SENTINEL' -and -not $capture.stream_close_attempted) 'unstarted process does not close absent streams' }
        partial-reader { Check ($scenario.stdout_reader.Closed -and $null -eq $scenario.stderr_reader -and $null -eq $capture.stream_close_error -and $capture.status -eq 'PROCESS_WAIT_FAILED') 'partial initialization closes only existing reader and retains setup failure' }
        read-fault { Check ($capture.status -eq 'OUTPUT_CAPTURE_FAILED' -and $capture.capture_error.Contains('READ_SENTINEL') -and -not $capture.stdout_complete) 'read exception is retained and fails capture closed' }
        close-fault { Check ($capture.status -eq 'PROCESS_CLEANUP_FAILED' -and $capture.stream_close_error.Contains('CLOSE_SENTINEL') -and $capture.capture_error.Contains('READ_SENTINEL')) 'actual injected close failure overrides capture failure and retains both' }
        dispose-fault { Check ($capture.status -eq 'PROCESS_CLEANUP_FAILED' -and $capture.cleanup_error.Contains('DISPOSE_SENTINEL')) 'injected Dispose failure remains cleanup failure' }
        launch-plus-cleanup-fault { Check ($capture.status -eq 'PROCESS_CLEANUP_FAILED' -and $capture.launch_error -eq 'ORIGINAL_LAUNCH_SENTINEL' -and $capture.cleanup_error.Contains('DISPOSE_SENTINEL')) 'launch error never masks genuine injected cleanup failure' }
    }
    $faults.Add(@{ mode = $mode; capture = $capture; kind = 'EXACT_CANDIDATE_AST_WITH_CONTROLLED_OBJECTS' })
}
# No entrypoint or timeout/kill behavior is replaced in the candidate.
Write-JsonFile (Join-Path $OutputRoot 'fault-branch-results.json') @($faults.ToArray())
Write-JsonFile (Join-Path $OutputRoot 'invocation-records.json') @($script:GitInvocations.ToArray())
Write-JsonFile (Join-Path $OutputRoot 'regression-results.json') @{ status = 'PASS'; checks = @($script:Checks.ToArray()); powershell = $PSVersionTable.PSVersion.ToString(); platform = [Environment]::OSVersion.ToString(); cases = @($script:CaseRecords.ToArray()); fault_tests = 'PowerShell candidate AST with controlled objects, not OS fault injection'; not_run = @('Windows aliases/inherited handles/process-tree kill/CIM', 'Hosted R2', 'Full Runner/NSIS/service/IPC/database') }
"PASS: $($script:Checks.Count) additional real PowerShell checks (includes explicitly labeled controlled-object branch tests)"
