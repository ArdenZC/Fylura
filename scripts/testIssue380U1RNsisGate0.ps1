param(
    [Parameter(Mandatory = $true)][string]$Makensis,
    [Parameter(Mandatory = $true)][string]$PluginDirectory,
    [string]$EvidenceRoot = (Join-Path (Split-Path -Parent $PSScriptRoot) "docs/research/evidence/issue-380-u1r-fix/gate0")
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot
$harnessPath = Join-Path $PSScriptRoot "issue380U1HostedWindows.ps1"
$u1rHead = "dcd1b0ea5e7a2d055bba2a041f022587bcd37316"
$oldSha = "3275ac7c72df8ae4139a30f16c7f649971324e97"
$oldTree = "009576241cd4e8e2ba12cc644a368314f599faa6"
$d1aSha = "d260131f9d6ec354f2361e6c7460c79e62e38391"
$d1aTree = "4765285668095fb3d7c0dcefd8a8ba5f2bd4fb69"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("issue-380-u1r-gate0-" + [Guid]::NewGuid().ToString("N"))
$evidence = [ordered]@{
    status = "RUNNING"
    baseline = [ordered]@{ u1rHead = $u1rHead; oldSha = $oldSha; oldTree = $oldTree; d1aSha = $d1aSha; d1aTree = $d1aTree }
    compiler = [ordered]@{ path = $Makensis; pluginDirectory = $PluginDirectory; version = $null }
    tests = [ordered]@{}
    candidates = [ordered]@{}
    failures = @()
}

function Get-GitText([string[]]$Arguments) {
    $output = & git -c "safe.directory=$repoRoot" -C $repoRoot @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw "git $($Arguments -join ' ') failed: $($output | Out-String)" }
    return (($output | Out-String).Trim())
}

function Get-FunctionNode($Ast, [string]$Name) {
    $nodes = $Ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)
    foreach ($node in $nodes) { if ($node.Name -ieq $Name) { return $node } }
    throw "PowerShell function '$Name' was not found"
}

function Get-ParsedFileAst([string]$Path) {
    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count -gt 0) { throw "PowerShell parse failed for $Path`: $($parseErrors | Out-String)" }
    return $ast
}

function Get-ParsedTextAst([string]$Text, [string]$Label) {
    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput($Text, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count -gt 0) { throw "PowerShell parse failed for $Label`: $($parseErrors | Out-String)" }
    return $ast
}

function Export-FrozenCandidate([string]$Label, [string]$Sha, [string]$ExpectedTree, [string[]]$Paths, [string]$Destination) {
    $actualTree = Get-GitText @("rev-parse", "$Sha^{tree}")
    if ($actualTree -cne $ExpectedTree) { throw "$Label candidate Tree changed: got $actualTree, expected $ExpectedTree" }
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    $archivePath = Join-Path $tempRoot "$Label-source.tar"
    & git -c "safe.directory=$repoRoot" -C $repoRoot archive --format=tar "--output=$archivePath" $Sha @Paths
    if ($LASTEXITCODE -ne 0) { throw "git archive failed for $Label candidate $Sha" }
    & tar -xf $archivePath -C $Destination
    if ($LASTEXITCODE -ne 0) { throw "tar extraction failed for $Label candidate $Sha" }
    Remove-Item -LiteralPath $archivePath -Force
    return $actualTree
}

function Get-TextSha256([string]$Text) {
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try { return [Convert]::ToHexString($algorithm.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text))).ToLowerInvariant() }
    finally { $algorithm.Dispose() }
}

function Normalize-U1RHooks([string]$Text) {
    $normalized = [regex]::Replace($Text, '(?ms)^Function U1RWriteInstallerTrace\r?\n.*?^FunctionEnd\r?\n(?:\r?\n)?', '')
    $normalized = [regex]::Replace($normalized, '(?ms)^Function un\.U1RWriteUninstallerTrace\r?\n.*?^FunctionEnd\r?\n(?:\r?\n)?', '')
    $normalized = [regex]::Replace($normalized, '(?m)^[ \t]*(?:Push "\[U1R(?:-OLD-UN)?\][^"\r\n]*"|Call (?:un\.)?U1RWrite(?:Installer|Uninstaller)Trace|DetailPrint "\[U1R(?:-OLD-UN)?\][^"\r\n]*")[ \t]*\r?\n', '')
    return $normalized
}

function Assert-U1RDetailPrintSemantics([string]$Text, [string]$Label) {
    $u1rLines = @($Text -split "`r?`n" | Where-Object { $_ -match '\[U1R(?:-OLD-UN)?\]' })
    if ($u1rLines.Count -eq 0) { throw "$Label contains no test-only U1R diagnostic instructions" }
    foreach ($line in $u1rLines) {
        if ($line -notmatch '^\s*DetailPrint "\[U1R(?:-OLD-UN)?\]') { throw "$Label changed a diagnostic into a stack/error/file instruction: $line" }
    }
    if ($Text -match 'u1r_(?:file|d1a)_trace_\d+_' -or $Text -match 'Call (?:un\.)?U1RWrite(?:Installer|Uninstaller)Trace' -or $Text -match '(?m)^Function (?:un\.)?U1RWrite(?:Installer|Uninstaller)Trace') {
        throw "$Label contains macro-expanded numeric labels or context-unsafe helper calls"
    }
}

function Invoke-NSISCompiler([string]$ScriptPath, [string]$WorkingDirectory, [string]$LogPath) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Makensis
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.ArgumentList.Add("-V3")
    $startInfo.ArgumentList.Add($ScriptPath)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "could not start NSIS compiler at $Makensis" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $exitCode = $process.ExitCode
    $process.Dispose()
    $combined = $stdout + $stderr
    [IO.File]::WriteAllText($LogPath, $combined, [Text.UTF8Encoding]::new($false))
    return [pscustomobject]@{ exitCode = $exitCode; output = $combined; log = $LogPath; sha256 = (Get-FileHash -LiteralPath $LogPath -Algorithm SHA256).Hash.ToLowerInvariant() }
}

function New-NSISCompileFixture([string]$Label, [string]$CandidateRoot, [string]$CompileRoot, [string]$GeneratedTemplateSnippet = "") {
    New-Item -ItemType Directory -Path $CompileRoot -Force | Out-Null
    $windowsRoot = Join-Path $CandidateRoot "src-tauri/windows"
    $sourceParts = [Collections.Generic.List[string]]::new()
    foreach ($file in Get-ChildItem -LiteralPath $windowsRoot -Filter "*.nsh" -File -Recurse) {
        $sourceParts.Add([IO.File]::ReadAllText($file.FullName))
    }
    if ($GeneratedTemplateSnippet) { $sourceParts.Add([IO.File]::ReadAllText($GeneratedTemplateSnippet)) }
    $allSource = $sourceParts -join "`n"

    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add("Unicode true")
    $lines.Add('!include "FileFunc.nsh"')
    $lines.Add('!insertmacro GetSize')
    if ($PluginDirectory) { $lines.Add(('!addplugindir "' + $PluginDirectory.Replace('\', '/') + '"')) }
    $knownDefines = [ordered]@{
        MANUFACTURER = "startlan"
        PRODUCTNAME = "Zen Canvas"
        VERSION = "0.1.40"
        HOMEPAGE = "https://example.invalid"
        MAINBINARYNAME = "zen-canvas.exe"
        UNINSTKEY = "Software\Microsoft\Windows\CurrentVersion\Uninstall\Zen Canvas"
        MANUPRODUCTKEY = "Software\startlan\Zen Canvas"
        ESTIMATEDSIZE = "1"
    }
    foreach ($entry in $knownDefines.GetEnumerator()) { $lines.Add(('!ifndef ' + $entry.Key)); $lines.Add(('!define ' + $entry.Key + ' "' + $entry.Value + '"')); $lines.Add('!endif') }

    $alreadyDefined = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($match in [regex]::Matches($allSource, '(?m)^\s*!define\s+([A-Za-z_][A-Za-z0-9_]*)')) { [void]$alreadyDefined.Add($match.Groups[1].Value) }
    foreach ($entry in $knownDefines.GetEnumerator()) { [void]$alreadyDefined.Add($entry.Key) }
    $builtinDefines = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($value in @("If", "AndIf", "OrIf", "Else", "ElseIf", "EndIf", "IfThen", "Errors", "NSIS_PTR_SIZE", "__COUNTER__", "__FILEDIR__", "GetSize")) { [void]$builtinDefines.Add($value) }
    $tokens = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($match in [regex]::Matches($allSource, '\$\{([A-Za-z_][A-Za-z0-9_]*)\}')) { [void]$tokens.Add($match.Groups[1].Value) }
    foreach ($token in ($tokens | Sort-Object)) {
        if ($alreadyDefined.Contains($token) -or $builtinDefines.Contains($token)) { continue }
        $lines.Add(('!ifndef ' + $token))
        $lines.Add(('!define ' + $token + ' "1"'))
        $lines.Add('!endif')
    }

    $declaredVars = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($match in [regex]::Matches($allSource, '(?m)^\s*Var\s+([A-Za-z_][A-Za-z0-9_]*)')) { [void]$declaredVars.Add($match.Groups[1].Value) }
    $builtinVars = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($value in @("INSTDIR", "OUTDIR", "EXEDIR", "TEMP", "TMP", "SYSDIR", "WINDIR", "PROGRAMFILES", "PROGRAMFILES32", "PROGRAMFILES64", "SMPROGRAMS", "DESKTOP", "STARTMENU", "APPDATA", "LOCALAPPDATA", "PLUGINSDIR", "HWNDPARENT", "CMDLINE", "LANGUAGE", "PROFILE", "DOCUMENTS", "SENDTO", "RECENT", "FAVORITES", "MUSIC", "PICTURES", "VIDEOS", "NETHOOD", "QUICKLAUNCH", "ADMINTOOLS", "INTERNET_CACHE")) { [void]$builtinVars.Add($value) }
    $referencedVars = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($match in [regex]::Matches($allSource, '\$([A-Za-z_][A-Za-z0-9_]*|[0-9])')) { [void]$referencedVars.Add($match.Groups[1].Value) }
    foreach ($variable in ($referencedVars | Sort-Object)) {
        if ($declaredVars.Contains($variable) -or $builtinVars.Contains($variable) -or $variable -match '^(?:R)?[0-9]$') { continue }
        $lines.Add(('Var ' + $variable))
    }

    $outFile = (Join-Path $CompileRoot "$Label.exe").Replace('\', '/')
    $lines.Add(('OutFile "' + $outFile + '"'))
    $wrapperPath = (Join-Path $windowsRoot "installer-lifecycle-wrapper.nsh").Replace('\', '/')
    $lines.Add(('!include "' + $wrapperPath + '"'))
    if ($GeneratedTemplateSnippet) {
        $lines.Add([IO.File]::ReadAllText($GeneratedTemplateSnippet))
    }
    $lines.Add('Section "Issue 380 U1R offline macro expansion fixture" U1R_GATE0')
    $lines.Add('  Call DetectZenCanvasPreexistingProduct')
    $lines.Add('  Call ValidateZenCanvasPreexistingProduct')
    $lines.Add('  Call ValidateZenCanvasPreviewCore')
    $lines.Add('  Call ValidateZenCanvasIndexServiceOwnership')
    $lines.Add('  WriteUninstaller "$TEMP\issue-380-u1r-gate0-uninstaller.exe"')
    $lines.Add('SectionEnd')
    $scriptPath = Join-Path $CompileRoot "$Label.nsi"
    [IO.File]::WriteAllText($scriptPath, ($lines -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
    return $scriptPath
}

function Assert-HarnessOrderContracts {
    $source = [IO.File]::ReadAllText($harnessPath)
    $ast = Get-ParsedFileAst $harnessPath
    $buildCandidate = (Get-FunctionNode $ast "Build-Candidate").Extent.Text
    if ($buildCandidate -notmatch 'if \(\$Mode -in @\("U1R", "U1R_FOCUS"\)\)\s*\{\s*Add-U1RTestInstrumentation \$Label \$Path\s*Add-U1RNsisDiagnostics \$Label \$Path') { throw "U1R_FOCUS no longer applies both test-only instrumentation steps to each candidate label" }
    if ($source -notmatch 'ConvertFrom-Json -AsHashtable') { throw "archived registry snapshot JSON is not parsed as an ordered hashtable with empty keys" }
    $emptyKey = '{"":"unnamed-default","named":"value"}' | ConvertFrom-Json -AsHashtable
    if ($emptyKey[""] -cne "unnamed-default") { throw "PowerShell JSON parser did not preserve the empty registry value name" }

    $samePath = (Get-FunctionNode $ast "Invoke-U1RSamePathAndDirectTests").Extent.Text
    $samePathStart = $samePath.IndexOf('$oldApp = Start-Process -FilePath $exe', [StringComparison]::Ordinal)
    $samePathRelease = $samePath.IndexOf('Set-Content -LiteralPath (Join-Path $gateRoot "continue")', [StringComparison]::Ordinal)
    $samePathPreSnapshot = $samePath.IndexOf('$samePath.preSnapshot = Invoke-U1RProfile "snapshot" $samePathProfile', [StringComparison]::Ordinal)
    $samePathDbHash = $samePath.IndexOf('$samePath.databaseSha256BeforeRequest = Get-Hash $samePath.databasePath', [StringComparison]::Ordinal)
    if ($samePathPreSnapshot -lt 0 -or $samePathDbHash -lt 0 -or $samePathStart -lt 0 -or $samePathRelease -lt $samePathStart -or $samePathPreSnapshot -gt $samePathStart -or $samePathDbHash -gt $samePathStart) { throw "same-path SQLite pre-snapshot is not completed before old client startup" }
    $samePathBeforeRelease = $samePath.Substring($samePathStart, $samePathRelease - $samePathStart)
    if ($samePathBeforeRelease -match 'Invoke-U1RProfile|Get-Hash \$samePath\.databasePath') { throw "same-path probe reads the locked SQLite database before releasing the old-client request gate" }
    $samePathStop = $samePath.IndexOf('Stop-U1Process $oldApp', $samePathRelease, [StringComparison]::Ordinal)
    $samePathPostSnapshot = $samePath.IndexOf('$samePath.postSnapshot = Invoke-U1RProfile "snapshot" $samePathProfile', $samePathRelease, [StringComparison]::Ordinal)
    if ($samePathStop -lt 0 -or $samePathPostSnapshot -lt 0 -or $samePathStop -gt $samePathPostSnapshot) { throw "same-path post-snapshot is not taken after the old client exits" }
    if ($samePath -notmatch 'if \(Test-Path -LiteralPath \$installDir\) \{ throw "U1R same-path gate requires a clean canonical install path') { throw "same-path scenario does not fail closed on a non-clean canonical install path" }

    $installer = (Get-FunctionNode $ast "Invoke-U1RInstallerDiagnostics").Extent.Text
    if ($installer -notmatch 'if \(Test-Path -LiteralPath \$installDir\) \{ throw "U1R NSIS diagnostic found a preexisting canonical install directory') { throw "NSIS scenario does not require a clean canonical install path" }
    $installerSnapshot = $installer.IndexOf('$isolatedProfileSnapshot = Invoke-U1RProfile "snapshot" $isolatedAppData', [StringComparison]::Ordinal)
    $installerStart = $installer.IndexOf('$app = Start-Process -FilePath $exe', [StringComparison]::Ordinal)
    $installerTransition = $installer.IndexOf('$transition = Wait-ForInstaller', [StringComparison]::Ordinal)
    $installerStop = $installer.IndexOf('Stop-U1Process $app', $installerTransition, [StringComparison]::Ordinal)
    $installerPostSnapshot = $installer.IndexOf('$databaseAfter = Invoke-U1RProfile "snapshot" $isolatedAppData', [StringComparison]::Ordinal)
    if ($installerSnapshot -lt 0 -or $installerStart -lt 0 -or $installerSnapshot -gt $installerStart -or $installerTransition -lt $installerStart -or $installerStop -lt $installerTransition -or $installerPostSnapshot -lt 0 -or $installerStop -gt $installerPostSnapshot) { throw "installer diagnostic reads the profile before launch or after the old process has released its database" }
    $installerBeforeTransition = $installer.Substring($installerStart, $installerTransition - $installerStart)
    if ($installerBeforeTransition -match 'Invoke-U1RProfile') { throw "installer diagnostic queries SQLite while the old desktop process is running" }

    $evidence.tests["focus_mode_old_and_d1a_injection"] = "PASS: Build-Candidate calls both injection functions for each label; candidate-specific anchors are applied below"
    $evidence.tests["json_empty_registry_value_name"] = "PASS"
    $evidence.tests["clean_canonical_path_per_scenario"] = "PASS"
    $evidence.tests["no_pre_request_locked_database_read"] = "PASS"
}

function Write-Evidence {
    New-Item -ItemType Directory -Path $EvidenceRoot -Force | Out-Null
    $path = Join-Path $EvidenceRoot "u1r-gate0-evidence.json"
    $evidence["path"] = $path
    [IO.File]::WriteAllText($path, ($evidence | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
}

try {
    if (-not (Test-Path -LiteralPath $Makensis)) {
        $resolved = Get-Command $Makensis -ErrorAction SilentlyContinue
        if (-not $resolved) { throw "NSIS compiler was not found: $Makensis" }
        $Makensis = $resolved.Source
        $evidence.compiler.path = $Makensis
    }
    if (-not (Test-Path -LiteralPath $PluginDirectory)) { throw "Tauri NSIS plugin directory was not found: $PluginDirectory" }
    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $EvidenceRoot -Force | Out-Null

    $versionOutput = & $Makensis -VERSION 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "NSIS compiler version query failed: $versionOutput" }
    $evidence.compiler.version = $versionOutput.Trim()
    if ($evidence.compiler.version -ne "v3.11") { throw "Gate 0 requires the same NSIS 3.11 release as the failed Hosted run; got $($evidence.compiler.version)" }
    if (-not $env:NSISDIR) { throw "NSISDIR must point to the isolated NSIS 3.11 includes, stubs, and plugins" }

    $harnessAst = Get-ParsedFileAst $harnessPath
    $functionNames = @("Replace-U1RExactText", "Append-U1RDetailAfterAll", "Convert-U1RDetailPrintsToFileTrace", "Add-U1RTestInstrumentation", "Add-U1RNsisDiagnostics", "Build-Candidate", "Invoke-U1RSamePathAndDirectTests", "Invoke-U1RInstallerDiagnostics")
    $functionNodes = @{}
    foreach ($name in $functionNames) {
        $node = Get-FunctionNode $harnessAst $name
        $functionNodes[$name] = $node
        if ($name -in @("Replace-U1RExactText", "Append-U1RDetailAfterAll", "Convert-U1RDetailPrintsToFileTrace", "Add-U1RTestInstrumentation", "Add-U1RNsisDiagnostics")) { Invoke-Expression $node.Extent.Text }
    }

    Assert-HarnessOrderContracts

    $oldPaths = @("src-tauri/windows", "src-tauri/src/main.rs", "src-tauri/src/global_index/windows/mod.rs")
    $d1aPaths = @("src-tauri/windows", "src-tauri/src/global_index/windows/service.rs", "src-tauri/src/global_index/windows/service_host.rs", "scripts/buildWindowsPackage.mjs")
    $oldRoot = Join-Path $tempRoot "old"
    $d1aRoot = Join-Path $tempRoot "d1a"
    $oldActualTree = Export-FrozenCandidate "old" $oldSha $oldTree $oldPaths $oldRoot
    $d1aActualTree = Export-FrozenCandidate "d1a" $d1aSha $d1aTree $d1aPaths $d1aRoot

    $oldMain = Join-Path $oldRoot "src-tauri/src/main.rs"
    $oldProvider = Join-Path $oldRoot "src-tauri/src/global_index/windows/mod.rs"
    Add-U1RTestInstrumentation "old" $oldRoot
    $oldMainText = [IO.File]::ReadAllText($oldMain)
    $oldProviderText = [IO.File]::ReadAllText($oldProvider)
    if ($oldMainText -notmatch 'u1r_startup_gate\(\);' -or $oldMainText -notmatch 'u1r_forced_rebuild_begin' -or $oldProviderText -notmatch 'u1r_service_transport_error' -or $oldProviderText -notmatch 'u1r_direct_fallback_started') { throw "old frozen candidate did not receive all startup gate, target request, and Direct trace anchors" }
    $gateCallIndex = $oldMainText.IndexOf("u1r_startup_gate();", [StringComparison]::Ordinal)
    $coordinatorCreateIndex = $oldMainText.IndexOf("let global_index_coordinator = GlobalIndexCoordinator::new(db.clone());", [StringComparison]::Ordinal)
    $coordinatorStartIndex = $oldMainText.IndexOf("global_index_coordinator.start()", $coordinatorCreateIndex, [StringComparison]::Ordinal)
    $forcedRequestIndex = $oldMainText.IndexOf("u1r_forced_rebuild_begin", [StringComparison]::Ordinal)
    if ($gateCallIndex -lt 0 -or $coordinatorCreateIndex -lt 0 -or $gateCallIndex -gt $coordinatorCreateIndex -or $coordinatorStartIndex -lt 0 -or $forcedRequestIndex -lt $coordinatorStartIndex) { throw "old frozen client can start same-generation coordinator recovery before the startup gate is released" }
    $evidence.tests["old_client_recovery_gate_precedes_target_request"] = "PASS: exact old candidate was instrumented with gate before Coordinator construction and forced Rebuild after Coordinator start"

    $d1aService = Join-Path $d1aRoot "src-tauri/src/global_index/windows/service.rs"
    $d1aHost = Join-Path $d1aRoot "src-tauri/src/global_index/windows/service_host.rs"
    Add-U1RTestInstrumentation "d1a" $d1aRoot
    $d1aServiceText = [IO.File]::ReadAllText($d1aService)
    $d1aHostText = [IO.File]::ReadAllText($d1aHost)
    if ($d1aServiceText -notmatch 'u1r_pipe_identity_accepted_client_pid' -or $d1aServiceText -notmatch 'u1r_service_request_accepted' -or $d1aHostText -notmatch 'u1r_service_operation_claimed' -or $d1aHostText -notmatch 'u1r_service_source_validated') { throw "D1-A frozen candidate did not receive all Service identity/admission/source trace anchors" }
    $evidence.tests["focus_mode_old_and_d1a_injection"] = "PASS: Build-Candidate and exact frozen-source anchor application succeeded for both old and D1-A candidates"

    $currentConverterText = $functionNodes["Convert-U1RDetailPrintsToFileTrace"].Extent.Text
    $legacyScript = (Get-GitText @("show", "$u1rHead`:scripts/issue380U1HostedWindows.ps1"))
    $legacyAst = Get-ParsedTextAst $legacyScript "frozen U1R old NSIS generator"
    $legacyConverterText = (Get-FunctionNode $legacyAst "Convert-U1RDetailPrintsToFileTrace").Extent.Text
    $legacyRoot = Join-Path $tempRoot "old-legacy-generator"
    [void](Export-FrozenCandidate "old-legacy-generator" $oldSha $oldTree $oldPaths $legacyRoot)
    Invoke-Expression $legacyConverterText
    Add-U1RNsisDiagnostics "old" $legacyRoot
    $legacyWrapperDir = Join-Path $tempRoot "legacy-compile"
    $legacyNsi = New-NSISCompileFixture "legacy" $legacyRoot $legacyWrapperDir
    $legacyLogPath = Join-Path $EvidenceRoot "legacy-generator-expected-failure.log"
    $legacyResult = Invoke-NSISCompiler $legacyNsi $legacyWrapperDir $legacyLogPath
    if ($legacyResult.exitCode -eq 0 -or $legacyResult.output -notmatch 'label "u1r_file_trace_\d+_had_error:" already declared in function') { throw "old generator negative regression did not reproduce the deterministic macro label collision: $($legacyResult.output)" }
    $evidence.tests["legacy_generator_duplicate_macro_label"] = [ordered]@{ status = "PASS_EXPECTED_COMPILER_FAILURE"; exitCode = $legacyResult.exitCode; log = $legacyResult.log; logSha256 = $legacyResult.sha256; firstError = ([regex]::Match($legacyResult.output, 'Error: label "[^"]+" already declared in function').Value) }

    Invoke-Expression $currentConverterText
    $candidateCases = @(
        [ordered]@{ label = "old"; root = $oldRoot; sha = $oldSha; tree = $oldActualTree; templateSnippet = $null },
        [ordered]@{ label = "d1a"; root = $d1aRoot; sha = $d1aSha; tree = $d1aActualTree; templateSnippet = $null }
    )
    foreach ($candidate in $candidateCases) {
        $hooksPath = Join-Path $candidate.root "src-tauri/windows/installer-hooks.nsh"
        $baseHooks = [IO.File]::ReadAllText($hooksPath)
        $baselineCopy = Join-Path $tempRoot "$($candidate.label)-installer-hooks-baseline.nsh"
        [IO.File]::WriteAllText($baselineCopy, $baseHooks, [Text.UTF8Encoding]::new($false))
        Add-U1RNsisDiagnostics $candidate.label $candidate.root
        $instrumentedHooks = [IO.File]::ReadAllText($hooksPath)
        Assert-U1RDetailPrintSemantics $instrumentedHooks $candidate.label
        $normalizedHooks = Normalize-U1RHooks $instrumentedHooks
        $baselineEvidence = Join-Path $EvidenceRoot "$($candidate.label)-baseline-installer-hooks.nsh"
        $normalizedEvidence = Join-Path $EvidenceRoot "$($candidate.label)-normalized-installer-hooks.nsh"
        Copy-Item -LiteralPath $baselineCopy -Destination $baselineEvidence -Force
        [IO.File]::WriteAllText($normalizedEvidence, $normalizedHooks, [Text.UTF8Encoding]::new($false))
        if ($normalizedHooks -cne $baseHooks) {
            $difference = Compare-Object -ReferenceObject ($baseHooks -split "`n") -DifferenceObject ($normalizedHooks -split "`n") | Format-Table -AutoSize | Out-String
            [IO.File]::WriteAllText((Join-Path $EvidenceRoot "$($candidate.label)-normalization-diff.txt"), $difference, [Text.UTF8Encoding]::new($false))
            throw "$($candidate.label) NSIS security source changed after U1R instrumentation was removed; integrity/Registry/Preview/Service predicates are not byte-preserved"
        }
        $candidate["baselineHooksSha256"] = Get-TextSha256 $baseHooks
        $candidate["normalizedHooksSha256"] = Get-TextSha256 $normalizedHooks
        $candidate["instrumentedHooksSha256"] = Get-TextSha256 $instrumentedHooks
        $candidate["normalizedHooksExactMatch"] = $true
        $evidence.candidates[$candidate.label] = [ordered]@{ sha = $candidate.sha; tree = $candidate.tree; baselineHooksSha256 = $candidate.baselineHooksSha256; normalizedHooksSha256 = $candidate.normalizedHooksSha256; instrumentedHooksSha256 = $candidate.instrumentedHooksSha256; normalizedHooksExactMatch = $true; traceInstruction = "DetailPrint-only; no functions, labels, Push/Pop, or file operations" }
        $destination = Join-Path $EvidenceRoot "$($candidate.label)-instrumented-installer-hooks.nsh"
        Copy-Item -LiteralPath $hooksPath -Destination $destination -Force
        $evidence.candidates[$candidate.label]["instrumentedHooksEvidence"] = $destination
        $evidence.candidates[$candidate.label]["instrumentedHooksEvidenceSha256"] = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
    }

    $d1aBuildScript = Join-Path $d1aRoot "scripts/buildWindowsPackage.mjs"
    $d1aBuildText = [IO.File]::ReadAllText($d1aBuildScript)
    $d1aBuildEvidence = Join-Path $EvidenceRoot "d1a-instrumented-buildWindowsPackage.mjs"
    Copy-Item -LiteralPath $d1aBuildScript -Destination $d1aBuildEvidence -Force
    if ($d1aBuildText -match 'u1r_d1a_trace_\d+_' -or
        -not $d1aBuildText.Contains('DetailPrint "[U1R] old_uninstaller_trace=$TEMP\\issue-380-u1r-old-uninstaller.log"') -or
        -not $d1aBuildText.Contains('StrCpy $R1 "$R1 /LOG=$TEMP\\issue-380-u1r-old-uninstaller.log _?=$4"')) {
        throw "D1-A generated template diagnostics contract mismatch: traceLabels=$($d1aBuildText -match 'u1r_d1a_trace_\d+_'); uninstallerLogDetail=$($d1aBuildText.Contains('DetailPrint "[U1R] old_uninstaller_trace=$TEMP\\issue-380-u1r-old-uninstaller.log"')); uninstallerLogArgument=$($d1aBuildText.Contains('StrCpy $R1 "$R1 /LOG=$TEMP\\issue-380-u1r-old-uninstaller.log _?=$4"'))"
    }
    $node = Get-Command "node" -ErrorAction SilentlyContinue
    if (-not $node) { throw "Node.js is required only for syntax-checking and exercising the injected D1-A NSIS trace converter" }
    $nodeCheckLog = Join-Path $EvidenceRoot "d1a-build-script-node-check.log"
    $nodeCheck = & $node.Source --check $d1aBuildScript 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "injected D1-A package script has invalid JavaScript syntax: $nodeCheck" }
    [IO.File]::WriteAllText($nodeCheckLog, $nodeCheck, [Text.UTF8Encoding]::new($false))

    $converterSmoke = @'
const fs = require("fs");
const buildScript = fs.readFileSync(process.argv[2], "utf8");
const startMarker = "      const inject = (before, after, label) => {";
const endMarker = "      fs.writeFileSync(generatedTemplatePath, diagnostic, \"utf8\");";
const start = buildScript.indexOf(startMarker);
const end = start < 0 ? -1 : buildScript.indexOf(endMarker, start);
if (start < 0 || end < 0) throw new Error("cannot locate the actual injected D1-A generated-template transformations");
const transformationSource = buildScript.slice(start, end).trim();
if (!transformationSource.endsWith("}")) throw new Error("injected D1-A diagnostic transformation block is incomplete");
const transform = new Function("diagnostic", transformationSource + "\nreturn diagnostic;");
const fixture = [
  "Function .onInit",
  "FunctionEnd",
  "Function PageReinstall",
  "  nsis_tauri_utils::SemverCompare \"${VERSION}\" $R0",
  "  Pop $R0",
  "  reinst_uninstall:",
  "    HideWindow",
  "    BringToFront",
  "",
  "    ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed, set fake exit code",
  "      StrCpy $R1 \"$R1 _?=$4\" ; append uninstall directory",
  "FunctionEnd",
  "Function PageLeaveReinstall",
  "  Call ZCPrepareInstallLifecycleFinal",
  "FunctionEnd",
].join("\n") + "\n";
const output = transform(fixture);
if ((output.match(/DetailPrint \"\[U1R\]/g) || []).length < 8) throw new Error("actual D1-A transformer did not add the expected DetailPrint diagnostics");
if (!output.includes("StrCpy $R1 \"$R1 /LOG=$TEMP\\issue-380-u1r-old-uninstaller.log _?=$4\"")) throw new Error("actual D1-A transformer did not add the isolated nested-uninstaller /LOG argument");
if (/u1r_d1a_trace_\d+_|Call U1RWriteInstallerTrace/.test(output)) throw new Error("actual D1-A converter emitted macro-expanded labels or helper calls");
fs.writeFileSync(process.argv[3], output, "utf8");
'@
    $nodeSmokePath = Join-Path $tempRoot "d1a-template-converter-smoke.cjs"
    $d1aTemplateSnippet = Join-Path $tempRoot "d1a-generated-template-trace-smoke.nsh"
    [IO.File]::WriteAllText($nodeSmokePath, $converterSmoke, [Text.UTF8Encoding]::new($false))
    $nodeSmokeOutput = & $node.Source $nodeSmokePath $d1aBuildScript $d1aTemplateSnippet 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "actual D1-A generated-template conversion smoke failed: $nodeSmokeOutput" }
    if (-not (Test-Path -LiteralPath $d1aTemplateSnippet)) { throw "D1-A generated-template converter did not emit its NSIS smoke source" }
    $d1aSnippetEvidence = Join-Path $EvidenceRoot "d1a-generated-template-trace-smoke.nsh"
    Copy-Item -LiteralPath $d1aTemplateSnippet -Destination $d1aSnippetEvidence -Force

    foreach ($candidate in $candidateCases) {
        $snippet = if ($candidate.label -eq "d1a") { $d1aTemplateSnippet } else { "" }
        $compileRoot = Join-Path $tempRoot "$($candidate.label)-compile"
        $scriptPath = New-NSISCompileFixture $candidate.label $candidate.root $compileRoot $snippet
        $nsiEvidence = Join-Path $EvidenceRoot "$($candidate.label)-generated-macro-expansion-fixture.nsi"
        Copy-Item -LiteralPath $scriptPath -Destination $nsiEvidence -Force
        $logPath = Join-Path $EvidenceRoot "$($candidate.label)-new-generator-nsis-3.11.log"
        $compileResult = Invoke-NSISCompiler $scriptPath $compileRoot $logPath
        if ($compileResult.exitCode -ne 0) { throw "$($candidate.label) new trace generator did not compile with NSIS 3.11: $($compileResult.output)" }
        $evidence.candidates[$candidate.label]["compile"] = [ordered]@{ status = "PASS"; exitCode = $compileResult.exitCode; log = $compileResult.log; logSha256 = $compileResult.sha256; fixture = $nsiEvidence; fixtureSha256 = (Get-FileHash -LiteralPath $nsiEvidence -Algorithm SHA256).Hash.ToLowerInvariant() }
    }
    $evidence.tests["nsis_3_11_old_generator_expected_failure_new_generator_pass"] = "PASS"
    $evidence.tests["actual_d1a_template_detailprint_contract_and_compile"] = [ordered]@{ status = "PASS"; node = (& $node.Source --version | Out-String).Trim(); source = $d1aSnippetEvidence; sourceSha256 = (Get-FileHash -LiteralPath $d1aSnippetEvidence -Algorithm SHA256).Hash.ToLowerInvariant(); nodeCheckLog = $nodeCheckLog; nodeCheckLogSha256 = (Get-FileHash -LiteralPath $nodeCheckLog -Algorithm SHA256).Hash.ToLowerInvariant() }
    $evidence.tests["security_predicate_byte_preservation"] = "PASS: old and D1-A instrumented installer-hooks normalize byte-for-byte to their frozen candidate source"
    $evidence.status = "PASS_GATE_0"
    Write-Evidence
    Write-Output "U1R_GATE0_PASS"
    Write-Output (Get-Content -LiteralPath (Join-Path $EvidenceRoot "u1r-gate0-evidence.json") -Raw)
} catch {
    $evidence.status = "BLOCKED"
    $evidence.failures = @($_.Exception.ToString())
    try { Write-Evidence } catch { }
    throw
} finally {
    if (Test-Path -LiteralPath $tempRoot) { Remove-Item -LiteralPath $tempRoot -Recurse -Force }
}
