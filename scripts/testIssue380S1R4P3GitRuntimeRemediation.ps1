[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$OutputRoot)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
[void][System.IO.Directory]::CreateDirectory($OutputRoot)
$script:checks = [System.Collections.Generic.List[string]]::new()
function Check([bool]$Value, [string]$Label) { if (-not $Value) { throw $Label }; $script:checks.Add($Label) }
function Parse([string]$Path) {
    $tokens=$null; $errors=$null
    $ast=[System.Management.Automation.Language.Parser]::ParseFile($Path,[ref]$tokens,[ref]$errors)
    Check ($errors.Count -eq 0) "parse $([System.IO.Path]::GetFileName($Path))"
    return $ast
}
function Import-Functions($Ast) {
    foreach ($node in $Ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] },$false)) {
        . ([scriptblock]::Create($node.Extent.Text))
        # Import into script scope via the function provider, without executing entrypoints.
        Set-Item -Path "function:script:$($node.Name)" -Value (Get-Item "function:$($node.Name)").ScriptBlock
    }
}
$harness = Parse (Join-Path $PSScriptRoot 'issue380S1R4P3GitRuntimeQualification.ps1')
$runner = Parse (Join-Path $PSScriptRoot 'issue380S1R4P3SyntheticNsisRunner.ps1')
$controller = Parse (Join-Path $PSScriptRoot 'issue380S1R4P3GitRuntimeController.ps1')
$child = Parse (Join-Path $PSScriptRoot 'issue380S1R4P3GitRuntimeChild.ps1')
$artifact = Parse (Join-Path $PSScriptRoot 'issue380S1R4P3GitRuntimeArtifact.ps1')
Import-Functions $harness
# Only the four frozen functions. Never execute the full Runner or harness entrypoint.
foreach ($name in @('Invoke-GitCapture','Get-RequiredGitScalar','Assert-GitNoOutputSuccess','Assert-GitWorktreeClean')) {
    $fragment=Get-FunctionFragment $name ([System.IO.File]::ReadAllText((Join-Path $PSScriptRoot 'issue380S1R4P3SyntheticNsisRunner.ps1')))
    . ([scriptblock]::Create($fragment.text))
}
$script:GitInvocations=[System.Collections.Generic.List[object]]::new()
$script:CaseRecords=[System.Collections.Generic.List[object]]::new()
$script:UnprovenCases=[System.Collections.Generic.List[string]]::new()
$script:CleanupRecords=[System.Collections.Generic.List[object]]::new()
$script:Failure=$null
$script:EvidenceRoot=Join-Path $OutputRoot 'case-evidence'
$script:TaskRoot=Join-Path $OutputRoot 'fixture'
$script:RepoPath=$script:TaskRoot
[void][System.IO.Directory]::CreateDirectory($script:EvidenceRoot)
[void][System.IO.Directory]::CreateDirectory($script:RepoPath)
$env:GIT_CONFIG_NOSYSTEM='1'; $env:GIT_CONFIG_GLOBAL=Join-Path $OutputRoot 'empty.config'
[System.IO.File]::WriteAllText($env:GIT_CONFIG_GLOBAL,'')
$env:GIT_TERMINAL_PROMPT='0'
& git init -q $script:RepoPath
if ($LASTEXITCODE -ne 0) { throw 'git init failed' }
& git -C $script:RepoPath -c user.name=fixture -c user.email=fixture@example.invalid commit -q --allow-empty -m fixture
if ($LASTEXITCODE -ne 0) { throw 'git fixture commit failed' }
$commands=@($harness.FindAll({param($n) $n -is [System.Management.Automation.Language.CommandAst] -and $n.GetCommandName() -eq 'Invoke-TestCase'},$true))
foreach ($prefix in @('01_','03_','03b_','08_')) {
    $command=@($commands | Where-Object { $_.CommandElements[2].Value.StartsWith($prefix) })
    Check ($command.Count -eq 1) "unique original Case $prefix"
    . ([scriptblock]::Create($command[0].Extent.Text))
    $record=$script:CaseRecords[$script:CaseRecords.Count-1]
    $expected=if($prefix -eq '08_'){'UNPROVEN'}else{'PASS'}
    Check ($record.status -eq $expected) "real original Case $prefix $expected"
}
Check ((Get-RuntimeMatrixStatus) -eq 'PARTIAL_BLOCKED_FAULT_INJECTION_UNPROVEN') 'UNPROVEN final matrix is not PASS'
$valid=@(
    @{passed=$true;expected='e';actual=@()},
    [pscustomobject]@{passed=$true;expected='e';actual=@()},
    @{passed=$true;expected='e';actual=@(1)},
    [pscustomobject]@{passed=$true;unproven=$false;expected='e';actual=1;status='PASS'},
    @{passed=$false;expected='e';actual=$false},
    @{unproven=$true;expected='e';actual='a'},
    @{passed=$false;unproven=$true;expected='e';actual='a'}
)
foreach($value in $valid) { $r=ConvertTo-TestCaseResult $value; Check ($r.status -in @('PASS','FAIL','UNPROVEN')) 'valid result accepted' }
$invalid=@($null, @(), @(1,2), 'text', 1, @{passed=$true}, @{passed=$true;expected='e'}, @{passed=$true;expected=$null;actual=1}, @{passed='true';expected=1;actual=1}, @{passed=1;expected=1;actual=1}, @{unproven=$null;expected=1;actual=1}, @{passed=$true;unproven=$true;expected=1;actual=1}, @{expected=1;actual=1}, @{unproven=$false;expected=1;actual=1}, @{passed=$true;status='UNPROVEN';expected=1;actual=1}, [pscustomobject]@{passed=$true;actual=1})
foreach($value in $invalid) { $rejected=$false;try { [void](ConvertTo-TestCaseResult $value) }catch{$rejected=$true};Check $rejected 'invalid result rejected' }
Invoke-TestCase 'exception-with-git' { [void](Invoke-GitForTest @('--version') $repo); throw 'ORIGINAL_SENTINEL_ERROR' }
$failed=$script:CaseRecords[$script:CaseRecords.Count-1]
Check ($failed.status -eq 'FAIL' -and $failed.exception.Contains('ORIGINAL_SENTINEL_ERROR') -and $null -ne $failed.error_record -and $failed.git_invocations.Count -eq 1) 'exception keeps original error and Git record'
Invoke-TestCase 'malformed-result' { @{passed=$true} }
Check ($script:CaseRecords[$script:CaseRecords.Count-1].status -eq 'FAIL') 'missing required fields recorded without secondary StrictMode exception'
Invoke-TestCase 'next-case' { @{passed=$true;expected='e';actual='a'} }
Check ($script:CaseRecords[$script:CaseRecords.Count-1].status -eq 'PASS') 'scheduler continues after FAIL'
Check ((Get-RuntimeMatrixStatus) -eq 'FAIL') 'FAIL takes precedence over UNPROVEN'
foreach ($array in @(@(), @(1))) {
    Invoke-TestCase "array-$($array.Count)" { @{passed=$true;expected=$array;actual=$array} }
    $arrayRecord=$script:CaseRecords[$script:CaseRecords.Count-1]
    Check ($arrayRecord.actual -is [array] -and $arrayRecord.actual.Count -eq $array.Count) 'Case record preserves empty/single-element array'
    $arrayJson=ConvertTo-Json -InputObject $arrayRecord -Depth 10 | ConvertFrom-Json -AsHashtable
    Check ($arrayJson.actual -is [array] -and $arrayJson.actual.Count -eq $array.Count) 'Case JSON preserves empty/single-element array'
}
# Empty TaskRoot must preserve the boundary error, not throw a .Name exception.
$previousTask=$script:TaskRoot; $previousEvidence=$script:EvidenceRoot
$env:RUNNER_TEMP=Join-Path $OutputRoot 'boundary-temp';$env:GITHUB_RUN_ID='boundary'
$script:TaskRoot=Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-git-runtime-boundary/task'
$script:EvidenceRoot=Join-Path $env:RUNNER_TEMP 'issue380-s1r4-p3-git-runtime-boundary/evidence'
[void][System.IO.Directory]::CreateDirectory($script:TaskRoot)
[void][System.IO.Directory]::CreateDirectory($script:EvidenceRoot)
$boundaryError='';try{Test-TaskRootBoundary}catch{$boundaryError=$_.Exception.Message}
Check ($boundaryError.StartsWith('Refusing to run in an unexpected task directory state:')) 'empty TaskRoot retains intended boundary error'
$script:TaskRoot=$previousTask;$script:EvidenceRoot=$previousEvidence
# Child records with absent optional fields, malformed identity, and null JSON.
$script:Nonce='offline-child-nonce'
$childRecordPath=Join-Path $OutputRoot 'child-record.json'
Write-JsonFile $childRecordPath @{mode='HoldPipeChild';nonce=$script:Nonce;pid=123}
$loaded=Read-TestChildRecord $childRecordPath
Check ($loaded.pid -eq 123) 'child record without optional child_pid accepted'
function Stop-OwnedChild { param($ProcessId,$Reason) return @{exit_confirmed=$true} }
Stop-RecordedChildren @($childRecordPath) 'offline double'
Check $true 'cleanup reads absent optional child_pid under StrictMode'
foreach($value in @($null,@{mode='HoldPipe';nonce=$script:Nonce},@{mode='HoldPipeChild';nonce='foreign';pid=123},@{mode='HoldPipeChild';nonce=$script:Nonce;pid='123'})) {
    Write-JsonFile $childRecordPath $value
    $rejected=$false;try{[void](Read-TestChildRecord $childRecordPath)}catch{$rejected=$true}
    Check $rejected 'malformed child record fails closed'
}
# Execute the actual finalizer with offline cleanup doubles; not Windows cleanup evidence.
function Stop-RecordedChildren { param($ParentRecordPaths,$Reason) $script:CleanupRecords.Add(@{exit_confirmed=$false;error='cleanup sentinel'});throw 'cleanup sentinel' }
function Assert-NoOwnedProcessRemains {}
$script:Result=[ordered]@{status='FAIL';cleanup=@();cases=@();git_invocations=@();failure=$null;ended_utc=$null;elapsed_ms=$null}
$script:StartUtc=[DateTime]::UtcNow
$runRecordPath=Join-Path $script:EvidenceRoot 'runtime-qualification.json'
$summaryPath=Join-Path $script:EvidenceRoot 'summary.json'
$mainTry=@($harness.EndBlock.Statements | Where-Object {$_ -is [System.Management.Automation.Language.TryStatementAst]})[-1]
. ([scriptblock]::Create($mainTry.Finally.Extent.Text.Trim().Substring(1).TrimEnd().TrimEnd('}')))
$roundTrip=Get-Content $runRecordPath -Raw | ConvertFrom-Json -AsHashtable
Check ($roundTrip.cases.Count -eq $script:CaseRecords.Count -and $roundTrip.cleanup.Count -eq 1 -and $roundTrip.failure.Contains('ORIGINAL_SENTINEL_ERROR')) 'actual finalizer preserves failed cases original error cleanup and arrays'
Check ($roundTrip.status -eq 'FAIL_CLEANUP_UNCONFIRMED') 'actual finalizer never reports full PASS'
# Actual finalizer summary write failure and round-trip failure keep original error.
$finalizer=[scriptblock]::Create($mainTry.Finally.Extent.Text.Trim().Substring(1).TrimEnd().TrimEnd('}'))
$savedSummary=$summaryPath
$summaryPath=Join-Path $OutputRoot 'absent-parent/summary.json'
. $finalizer
Check ($script:Result.status -eq 'FAIL_EVIDENCE_WRITE' -and $script:Result.failure.Contains('ORIGINAL_SENTINEL_ERROR')) 'summary write failure fails closed and retains original error'
$summaryPath=$savedSummary
$script:CaseRecords.Add(@{name='bad-roundtrip';expected=$null;actual=1;status='FAIL';started_utc=[DateTime]::UtcNow.ToString('o');ended_utc=[DateTime]::UtcNow.ToString('o')})
. $finalizer
Check ($script:Result.status -eq 'FAIL_EVIDENCE_ROUNDTRIP' -and $script:Result.failure.Contains('ORIGINAL_SENTINEL_ERROR')) 'round-trip failure retains original error'
$script:CaseRecords.RemoveAt($script:CaseRecords.Count-1)
# Controller output helper: zero and one completed task, raw streams preserved.
Import-Functions $controller
$record=[ordered]@{output_wait_ms=100;post_close_wait_ms=100;output_wait_returned=$null;stdout_task_status='';stderr_task_status='';stdout_complete=$false;stderr_complete=$false;stream_close_attempted=$false;stream_close_error=$null;errors=[System.Collections.Generic.List[string]]::new()}
$process=$null; $stdoutTask=$null; $stderrTask=$null; $stdoutText=''; $stderrText=''
$stdoutPath=Join-Path $OutputRoot 'stdout.txt';$stderrPath=Join-Path $OutputRoot 'stderr.txt'
Complete-ControllerOutput $false
Check ((Get-Item $stdoutPath).Length -eq 0) 'controller zero-task output persists'
$stdoutTask=[System.Threading.Tasks.Task]::FromResult([string]'stdout sentinel')
Complete-ControllerOutput $false
Check ((Get-Content $stdoutPath -Raw) -eq 'stdout sentinel' -and $record.stdout_complete -and -not $record.stderr_complete) 'controller single-task output persists'
# Execute the controller's actual evidence-write catch after a forced write error.
$controllerFinally=@($controller.EndBlock.Statements | Where-Object {$_ -is [System.Management.Automation.Language.TryStatementAst]})[-1].Finally
$writeTry=@($controllerFinally.Statements | Where-Object {$_ -is [System.Management.Automation.Language.TryStatementAst]})[-1]
$record.status='WORKER_EXIT_ZERO';$record.cleanup_completed=$true
try { Write-ControllerJson (Join-Path $OutputRoot 'missing-parent/controller.json') $record } catch {
    . ([scriptblock]::Create($writeTry.CatchClauses[0].Body.Extent.Text.Trim().Substring(1).TrimEnd().TrimEnd('}')))
}
Check ($record.status -eq 'CONTROLLER_EVIDENCE_WRITE_FAILED' -and -not $record.cleanup_completed) 'controller evidence write failure cannot exit zero'
# Actual artifact entrypoint in a local fixture with simulated run metadata.
Import-Functions $artifact
$env:RUNNER_TEMP=Join-Path $OutputRoot 'runner-temp'; $env:GITHUB_RUN_ID='offline-fixture';$env:GITHUB_RUN_ATTEMPT='1';$env:GITHUB_REPOSITORY='ArdenZC/Zen-Canvas';$env:GITHUB_REF='refs/heads/offline-fixture';$env:GITHUB_SHA='fixture'
$run=Join-Path $env:RUNNER_TEMP "issue380-s1r4-p3-git-runtime-$env:GITHUB_RUN_ID"
$archive=Join-Path $env:RUNNER_TEMP "issue380-s1r4-p3-git-runtime-archive-$env:GITHUB_RUN_ID"
foreach($dir in @('evidence','task/temporary-git-repository/.git/objects','task')) { [void][System.IO.Directory]::CreateDirectory((Join-Path $run $dir)) }
$owner=@{repository=$env:GITHUB_REPOSITORY;ref=$env:GITHUB_REF;run_id=$env:GITHUB_RUN_ID;run_attempt='1';triggering_sha='fixture'}
Write-JsonFile (Join-Path $run 'evidence/runtime-owner.json') $owner
Write-JsonFile (Join-Path $run 'task/runtime-task-owner.json') $owner
foreach($name in @('controller.json','cleanup.json','error.json','case-01.json')) { Write-JsonFile (Join-Path $run "evidence/$name") @{status='FAIL';error='retained'} }
Write-JsonFile (Join-Path $run 'task/pipe-holder.json.child') @{pid=123}
[System.IO.File]::WriteAllText((Join-Path $run 'task/temporary-git-repository/.git/objects/secret'),'excluded')
& (Join-Path $PSScriptRoot 'issue380S1R4P3GitRuntimeArtifact.ps1') -RunRoot $run -ArchiveRoot $archive | Out-Null
$proof=Assert-RuntimeArchive $archive
Check ($proof.payload_files -eq 7 -and $proof.uploaded_files -eq 8) 'actual archive exact file set and every SHA-256 verified'
Check (-not (Test-Path (Join-Path $archive 'task/temporary-git-repository'))) 'Git database excluded'
$manifest=Join-Path $archive 'SHA256SUMS';$original=[System.IO.File]::ReadAllText($manifest)
foreach($mode in @('duplicate','omission','missing','extra','hash')) {
    [System.IO.File]::WriteAllText($manifest,$original)
    switch($mode) {
        duplicate { [System.IO.File]::AppendAllText($manifest,($original.Split("`n")[0]+"`n")) }
        omission { [System.IO.File]::WriteAllText($manifest,($original.Split("`n") | Select-Object -Skip 1) -join "`n") }
        missing { [System.IO.File]::AppendAllText($manifest,('0'*64+'  evidence/missing.json'+"`n")) }
        extra { [System.IO.File]::WriteAllText((Join-Path $archive 'extra.txt'),'extra') }
        hash { [System.IO.File]::WriteAllText($manifest,$original.Replace($original.Substring(0,64),('0'*64))) }
    }
    $rejected=$false; try {[void](Assert-RuntimeArchive $archive)}catch{$rejected=$true}
    Check $rejected "archive rejects $mode"
    if($mode -eq 'extra'){Remove-Item (Join-Path $archive 'extra.txt')}
}
[System.IO.File]::WriteAllText($manifest,$original)
# Same entrypoint rejects ownership, source links, count and size limits.
foreach($mode in @('owner','reparse','count','size')) {
    $negativeArchive=$archive
    Remove-Item -LiteralPath $negativeArchive -Recurse -Force
    switch($mode) {
        owner { Write-JsonFile (Join-Path $run 'evidence/runtime-owner.json') @{run_id='foreign'} }
        reparse { [void](New-Item -ItemType SymbolicLink -Path (Join-Path $run 'evidence/link') -Target $script:RepoPath) }
        count { foreach($n in 1..251){[System.IO.File]::WriteAllText((Join-Path $run "evidence/limit-$n.txt"),'x')} }
        size { [System.IO.File]::WriteAllBytes((Join-Path $run 'evidence/oversize.txt'),[byte[]]::new(21MB)) }
    }
    $rejected=$false;try{ & (Join-Path $PSScriptRoot 'issue380S1R4P3GitRuntimeArtifact.ps1') -RunRoot $run -ArchiveRoot $negativeArchive | Out-Null }catch{$rejected=$true}
    Check $rejected "archive rejects $mode boundary"
    switch($mode) {
        owner { Write-JsonFile (Join-Path $run 'evidence/runtime-owner.json') $owner }
        reparse { Remove-Item -LiteralPath (Join-Path $run 'evidence/link') }
        count { foreach($n in 1..251){Remove-Item -LiteralPath (Join-Path $run "evidence/limit-$n.txt")} }
        size { Remove-Item -LiteralPath (Join-Path $run 'evidence/oversize.txt') }
    }
    & (Join-Path $PSScriptRoot 'issue380S1R4P3GitRuntimeArtifact.ps1') -RunRoot $run -ArchiveRoot $archive | Out-Null
}
Write-JsonFile (Join-Path $OutputRoot 'archive-proof.json') (Assert-RuntimeArchive $archive)
# Run the inactive draft cleanup block with a missing owner; its error evidence must persist.
$draft=[System.IO.File]::ReadAllText((Join-Path $repo 'docs/research/ISSUE-380-S1R4-P3-POWERSHELL-RUNTIME-WORKFLOW.md'))
$block=[regex]::Match($draft,'(?s)- name: Preserve failure task files.*?run: \|\n(.*?)(?=\n      - name:)').Groups[1].Value
$block=($block -split "`n" | ForEach-Object { $_ -replace '^          ','' }) -join "`n"
Remove-Item -LiteralPath (Join-Path $run 'task/runtime-task-owner.json')
$cleanupFailed=$false;try{& ([scriptblock]::Create($block))}catch{$cleanupFailed=$true}
$cleanup=Get-Content (Join-Path $run 'evidence/cleanup.json') -Raw | ConvertFrom-Json -AsHashtable
Check ($cleanupFailed -and $cleanup.status -eq 'CLEANUP_CHECK_FAILED' -and $null -ne $cleanup.reason) 'workflow cleanup failure persists evidence before throwing'
Write-JsonFile (Join-Path $OutputRoot 'regression-results.json') @{status='PASS';powershell=$PSVersionTable.PSVersion.ToString();platform=[Environment]::OSVersion.ToString();checks=@($script:checks.ToArray());not_run=@('Windows Git aliases Cases 05-07/09','Windows CIM cleanup','Hosted workflow','Full P3/NSIS/installer/service/IPC/database')}
"PASS: $($script:checks.Count) real PowerShell checks"
