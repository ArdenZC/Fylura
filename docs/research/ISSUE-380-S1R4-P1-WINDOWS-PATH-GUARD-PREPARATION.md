# #380 S1R4-P1 Windows Path Guard Preparation

## Status

`#380-S1R4-P1 READY_FOR_OWNER_WINDOWS_PATH_TEST_REVIEW`

Preparation status: `STATIC_PREPARED`.

Windows runtime status: **NOT RUN**. The PowerShell path matrix below is prepared to execute on Windows, but none of its Windows cases has been run in this phase. This report does not claim a Windows path result or a production fix.

## Frozen base

| Item | HEAD | Tree |
|---|---|---|
| S1R4-P0 | `e5e9ab5bc84136c15056d72db8904f26b24e2a64` | `96cdd08012021c5409c8dcdedcb623644a55e541` |
| S1R3 | `08bad6b19831d40dda9d296de880aaae6c83c312` | `5360d8c81e5949333c8d6ef16acd0c658d315598` |
| S0 Probe blob | `5805f8d04cd37420a63f3e156c3b6fd5b4012f33` | unchanged |

The dedicated branch is `research/issue-380-s1r4-p1-windows-path-guard-preparation`. The S1R4-P0 and S1R3 branches, S0 Probe, existing Workflow, master, and PR #386 remain unchanged.

## Path guard contract

[`scripts/issue380S1R4PathGuard.ps1`](../../scripts/issue380S1R4PathGuard.ps1) defines the shared resolver used by both the S1R4 Runner and the Windows path test.

- Inputs must be fully qualified paths on a local Windows drive. UNC paths, mapped network drives, extended/device namespace paths (`\\?\`, `\\.\`, `\??\`, and `\Device\`), alternate data streams, NULs, and any `..` path segment are rejected.
- TaskRoot must be equal to or a separator-bounded descendant of the current `RUNNER_TEMP`.
- `/` is normalized to `\`, `GetFullPath` canonicalizes the path, the volume root keeps its separator, and comparisons use `OrdinalIgnoreCase`.
- Containment uses exact-root equality or a root prefix ending at a directory separator. A shared string prefix without a separator does not qualify.
- A working directory may equal TaskRoot or be a real descendant; the selected object must exist as a directory.
- An executable must be a strict descendant, exist as an ordinary file, and have an `.exe` extension. TaskRoot itself and a directory cannot be accepted as an executable.
- Every existing component from the volume root through TaskRoot and the candidate is inspected. Any reparse point, including a symlink, junction, or reparse point in an ancestor, is rejected without traversing beyond the first detected reparse point.
- Candidate containment is checked before inspecting candidate filesystem attributes. Rejected external candidates are not opened or modified.

The reparse policy is deliberately conservative. A legitimate TaskRoot beneath a junction or symlink will also be rejected; the dedicated test includes that case. Windows execution is required to validate filesystem behavior.

## Runner integration and frozen constraints

[`scripts/issue380S1R4SyntheticNsisRunner.ps1`](../../scripts/issue380S1R4SyntheticNsisRunner.ps1) is a separate research copy of the S1R3 Runner. `Invoke-CapturedProcess` uses `Resolve-Issue380S1R4Path` for both executable and working-directory validation; the process cleanup snapshot uses the same helper to identify task-owned images. The helper also validates extracted compiler members and generated executables before use or evidence copying.

The following S1R3/S0 contracts remain byte-for-byte or semantically fixed in the research copy:

- S0 Probe blob `5805f8d04cd37420a63f3e156c3b6fd5b4012f33`.
- Official NSIS ZIP SHA-256 `c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1`.
- Forwarding launcher `nsis-3.11/makensis.exe`: 2,560 bytes, SHA-256 `f497e92deb9f179f7b8f7553fcb3bd04f511bb2949e5e4a2aee80a10f7b20431`.
- Compiler `nsis-3.11/Bin/makensis.exe`: 468,992 bytes, SHA-256 `42850802704ecb11163f7e0329d35ee54bd288953200d4966e226d572848cfc5`.
- `ProcessStartInfo.ArgumentList` remains the process argument interface.
- The synthetic uninstaller still appends `_?=<absolute install directory>` last and asserts that it is last.
- Installer and uninstaller expected exit codes remain 37 and 42.
- `TRACE_MODE=file` remains before the optional detail comparison.
- Task directory ownership markers, process-exit checks, synthetic HKCU preexistence checks, and cleanup gates remain present.

No Workflow was added, changed, or enabled. The existing S1R3 Workflow remains scoped to its exact S1R3 branch and does not match the P1 branch name. No NSIS binary, Probe, installer, unrelated executable, or Hosted job was run.

## Windows path matrix

[`scripts/testIssue380S1R4PathGuard.ps1`](../../scripts/testIssue380S1R4PathGuard.ps1) requires PowerShell 7 on Windows and imports the same path module as the Runner. It creates only tokenized fixtures under `RUNNER_TEMP`, verifies ownership markers, and removes only those fixtures. Its `.exe` fixture files contain text and are never executed. The script contains no process-launch API.

| Case | Expected result | Execution in P1 |
|---|---|---|
| TaskRoot as working directory | ALLOW | NOT RUN |
| Direct and nested child directories | ALLOW | NOT RUN |
| Shared-prefix sibling | REJECT | NOT RUN |
| Parent directory | REJECT | NOT RUN |
| Another `RUNNER_TEMP` directory | REJECT | NOT RUN |
| TaskRoot outside `RUNNER_TEMP` | REJECT | NOT RUN |
| Traversal that normalizes back inside, and traversal to parent | REJECT | NOT RUN |
| Uppercase path and forward-slash variants | ALLOW under Windows comparison rules | NOT RUN |
| Uppercase, forward-slash, and trailing-separator TaskRoot variants | ALLOW under Windows comparison rules | NOT RUN |
| Existing `.exe` file under TaskRoot | ALLOW | NOT RUN |
| Uppercase and forward-slash executable path variants | ALLOW under Windows comparison rules | NOT RUN |
| Nonexistent executable | REJECT | NOT RUN |
| Directory passed as executable | REJECT | NOT RUN |
| TaskRoot passed as executable | REJECT | NOT RUN |
| Executable outside TaskRoot | REJECT | NOT RUN |
| UNC and Windows device namespace paths as candidates or TaskRoot; relative and drive-relative paths; alternate data streams | REJECT | NOT RUN |
| Junction, directory symlink, file symlink, symlink TaskRoot, and reparse-point ancestor of TaskRoot | REJECT | NOT RUN |
| TaskRoot working directory for the `-VERSION` path check | ALLOW | NOT RUN; no command launched |
| TaskRoot working directory for the `-HDRINFO` path check | ALLOW | NOT RUN; no command launched |

The last two cases invoke only the path predicate. They do not launch NSIS.

## Static validation

[`scripts/validateIssue380S1R4Preparation.py`](../../scripts/validateIssue380S1R4Preparation.py) checks frozen identities, allowed file scope, Probe blob, tool hashes and lengths, Runner use of the shared predicate, strict exit codes, the final `_?=` argument, file-first ordering, no Workflow changes, all test case identifiers, and the no-process contract. By default it reports `STATIC_PREPARED` and `windows_runtime=NOT_RUN`.

It can separately verify a future Windows test JSON passed with `--windows-runtime-results`; only a Windows result with every required case and owned-fixture cleanup verified can produce `WINDOWS_RUNTIME_PASS`. No such result file exists in this phase.

The three PowerShell files were parsed with the repository's previously available Tree-sitter PowerShell grammar: zero syntax error nodes. This is static syntax evidence only, not PowerShell execution.

The final static validator invocation reports `status=STATIC_PREPARED`, `windows_runtime=NOT_RUN`, the exact P0/S1R3 identities, the unchanged Probe blob, `workflow_changes=NONE`, `hosted_jobs=NOT_STARTED`, and `process_launches=NONE; NSIS=NOT_RUN`.

## Files and SHA-256

The exact changed-file list and SHA-256 values are recorded in [`evidence/issue-380-s1r4-p1/SHA256SUMS`](evidence/issue-380-s1r4-p1/SHA256SUMS). No production file or S1R3 evidence file changed.

## Unexecuted tests and risks

- The PowerShell resolver and Windows test script have not been executed by Windows PowerShell.
- Windows drive canonicalization, case-insensitive lookup, slash normalization, junction/symlink flags, and cleanup of those links remain unverified runtime behavior.
- The process launch path in the copied Runner has not been run. No NSIS download, compiler invocation, Probe build, installer, uninstaller, Registry operation, or process cleanup was performed.
- The new test intentionally fails closed if Windows cannot create the requested symlink/junction fixtures. The runtime environment must support those fixture operations, or Owner must approve a reduced reparse-point test contract before any Windows run.

Owner review is required before any future Windows execution. This preparation does not authorize a Hosted job or U2 production work.
