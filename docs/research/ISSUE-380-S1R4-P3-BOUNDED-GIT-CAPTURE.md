# #380 S1R4-P3 bounded Git capture candidate

## Status and scope

`S1R4-P3 BOUNDED_GIT_CAPTURE_READY_FOR_OWNER_REVIEW`

This is an offline research candidate for the bounded Git capture defect in the P3 research Runner. It changes the Runner's Git capture section and its research contract Validator. It does not change the historical activation commit or Workflow, Run 38276002263 / attempt 1, the earlier frozen Runner, S0 Probe, P1 path guard, production code, Schema, or PR #386.

No PowerShell runtime was available in this Cloud environment. The Python model and source checks and Tree-sitter parse are not evidence that PowerShell 7 executed the implementation. No Hosted job, NSIS acquisition/compile/Probe, installer, Service, IPC, database, full CI, or Codex Review was run. `HOSTED_NSIS_NO_GO` and `U2_PRODUCTION_NO_GO` remain in force.

The accepted base was the final research HEAD `bfabd486bce84b7e5298276453dd01957049c91d`, Tree `3c00d21a3f4206256e2c542b5ebe16ce4039496d`. The branch was created from this exact commit. The code-freeze commit below is four narrowly scoped code commits after that base (`45e9762`, `ea8a6af`, `6e0c108`, `af305ad`); the subsequent documentation package is a child of the code freeze. Any future activation must start from the code-freeze commit, not the documentation package tip.

## Frozen candidate identity

| Item | Value |
|---|---|
| Code freeze HEAD | `af305adb8fba017dd9284a829b5101a209a9b4e2` |
| Code freeze Tree | `9d7716add5fde99c6e797065f336e0d0cf86a510` |
| Code freeze parent | `6e0c108aa5beeadceea531e7632d3667f18f3890` |
| Runner path | `scripts/issue380S1R4P3SyntheticNsisRunner.ps1` |
| Runner Git Blob | `c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa` |
| Runner raw SHA-256 | `92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece` |
| Previous null-safety candidate Runner Blob / SHA-256 | `2a80ac621f3cd0e9dd3386bb3488a362df67fdcf` / `5aaeff4dc7a78362b1caea21927c3a07180d62548180d09c37cab2559616f176` |

The frozen Runner's full Git capture diff from the accepted base is limited to `scripts/issue380S1R4P3SyntheticNsisRunner.ps1`; the targeted contract tests are in `scripts/validateIssue380S1R4P3RunnerNullSafety.py`. The source comparison test replaces the complete changed Git-capture-and-wrapper region with its base version and requires the rest of the Runner to match byte-for-byte. This covers the non-Git process helper, pinned NSIS download, compiler and Probe code, HKCU handling, process cleanup, and all unrelated Runner code.

The machine-readable identity and inert activation draft are separate from the prior null-safety report and remain outside `.github/workflows`:

- `docs/research/evidence/issue-380-s1r4-p3-bounded-git-capture/runner-identity.json`
- `docs/research/ISSUE-380-S1R4-P3-BOUNDED-GIT-CAPTURE-WORKFLOW.md`
- `docs/research/evidence/issue-380-s1r4-p3-bounded-git-capture/SHA256SUMS`

The previous null-safety report, workflow draft, identity, checksum file, original P3 activation and historical Run evidence were preserved unchanged.

## Bounded capture behavior

The change is confined to `Invoke-GitCapture`, `Get-RequiredGitScalar`, `Assert-GitNoOutputSuccess`, and `Assert-GitWorktreeClean` in the Runner.

`Invoke-GitCapture` starts `ReadToEndAsync()` for stdout and stderr before waiting on the Git process. It then applies these finite waits:

- Git process: 20 seconds.
- Process-tree termination after timeout: 5 seconds per `WaitForExit(timeout)` attempt.
- Concurrent completion of both output readers: 5 seconds.
- Output-reader completion after closing the redirected streams: 1 second per bounded post-close wait.

There is no parameterless `WaitForExit()` in this function. A process timeout attempts `Kill($true)` against the owned Git process tree and records the kill result, process exit confirmation, and exit code when it is available. The normal drain waits for both reader tasks concurrently, so neither redirected pipe is left unread while Git is running. A timeout or exception during normal capture enters finite cleanup waits, closes only that process's redirected output readers if needed, and records whether the reader tasks actually stopped.

`GetAwaiter().GetResult()` is only reached for a task whose `IsCompleted` is true. A forced stream close is never accepted as complete output: output completion flags remain false. The capture record retains process ID, per-call arguments and executable, exit code, individual task states, timeout values and individual wait status, stdout/stderr text where available, character counts and truncation flags, kill attempts, process-exit confirmation, stream-close errors, launch/process/capture errors, and cleanup status.

The result status fails closed for process timeout, incomplete output, task fault, process wait failure, or cleanup failure. All three Git admission helpers reject a capture unless its status represents a completed process, both streams completed successfully, no timeout/error occurred, cleanup completed, and the existing exit-code/stderr/output contract is satisfied. In particular:

- clean `git status`: exit 0, empty stdout, empty stderr remains accepted;
- nonempty status remains dirty and rejected;
- required HEAD/Tree/Blob scalar output still must be one nonempty line, exit 0 and no stderr;
- no-output ancestry commands still require exit 0 and no stdout/stderr;
- each process's exit code is captured from its own `Process` object, not `$LASTEXITCODE`.

The configured blocking waits are finite. The bound does not constitute a real-time guarantee for the synchronous operating-system operations `Process.Kill`, redirected-stream `Close`, or `Process.Dispose`; PowerShell/Windows runtime behavior for those calls remains unverified here. If a reader remains pending after the bounded post-close wait, the call records that cleanup did not complete and fails closed instead of waiting indefinitely.

## Targeted contract tests

`scripts/validateIssue380S1R4P3RunnerNullSafety.py` now has 24 tests: the original 12 null-safety/model/static contracts remain, followed by 9 bounded-capture model/source contracts and 3 identity/evidence contracts. The Python model validates expected classifications; source checks connect the contract to the actual Runner code. They do not execute the PowerShell function.

| Test IDs | Scenario | Expected and observed result |
|---|---|---|
| 1–12 | Prior clean/dirty status, Git failure, missing required identity, stderr/exit separation, independent calls, pre-NSIS failure ordering, identity checks, frozen sources and NSIS contracts | PASS — original 12 contracts retained |
| 13 | Normal process exit with concurrent stdout and stderr capture | PASS — model and source ordering check |
| 14 | Empty stdout and stderr with exit code 0 | PASS — model |
| 15 | stdout delayed, stderr delayed, and both delayed within the output bound | PASS — model |
| 16 | Git process timeout followed by successful tree termination | PASS — remains `PROCESS_TIMEOUT`, admission rejected |
| 17 | Process exited but stdout or stderr outlasted the output deadline | PASS — `OUTPUT_CAPTURE_TIMEOUT` if closed readers stop; `PROCESS_CLEANUP_FAILED` if a reader remains pending; both rejected |
| 18 | Tree termination failure or resource cleanup failure | PASS — fail-closed model and source evidence fields |
| 19 | Consecutive Git calls with independent nonzero and zero exits | PASS — model/static; no shared `$LASTEXITCODE` |
| 20 | Bounded waits, completion-guarded result reads, fail-closed admission wrappers | PASS — static |
| 21 | Only Git capture and its three wrappers changed in Runner | PASS — source reconstruction comparison |
| 22 | Frozen commit, Tree, Runner Blob and raw SHA | PASS — Git object and working-file comparisons |
| 23 | New report, inert Workflow and identity values agree | PASS — static |
| 24 | New evidence hashes agree and previous null-safety package remains byte-identical | PASS — SHA and Git object comparison |

The exact offline output and parser record are stored under `docs/research/evidence/issue-380-s1r4-p3-bounded-git-capture/`. The test command was `PYTHONDONTWRITEBYTECODE=1 python3 scripts/validateIssue380S1R4P3RunnerNullSafety.py`; the static syntax parse used Tree-sitter PowerShell and reported no `ERROR` or missing syntax nodes. `git diff --check` also passed.

The environment check found no `pwsh` executable. Thus the following test cases are **NOT RUN in PowerShell 7**: real Git with empty stdout; real stdout/stderr completion and delayed handles; process timeout and tree termination; output tasks remaining pending after stream close; and real sequential process exit/error capture. The Python cases are not substitutes for those runtime tests.

## Future activation contract — design only

No new activation is authorized by this report. The separately stored full Workflow draft proposes a new activation identity:

- branch: `research/issue-380-s1r4-p3-runner-null-safety-activation` (the frozen Runner's existing `ExpectedBranch` contract; no local or remote branch currently exists);
- one new Workflow path: `.github/workflows/issue-380-s1r4-p3-bounded-git-capture.yml`;
- parent: exact code-freeze HEAD `af305adb8fba017dd9284a829b5101a209a9b4e2`;
- only allowed activation diff: one direct child adding only that Workflow file;
- one Windows 2022 job, `contents: read`, no `workflow_dispatch`, and `GITHUB_RUN_ATTEMPT == 1`.

The future Preflight must verify the new freeze HEAD/Tree and Runner Blob/raw SHA before download. Immediately before starting the Runner it must recheck the same identity. The Runner then verifies the event SHA and activation parent plus its committed/worktree Blob and raw SHA. The Preflight's `git status` empty-output rule and the Runner's `Invoke-GitCapture` empty-output rule are separate contracts and both require their own PowerShell runtime evidence before any new Hosted decision. The old activation branch and its consumed Workflow path remain unchanged and cannot be reused for this candidate.

Any eventual activation requires a separate Owner decision. The inert YAML is not an executable Workflow and is not authorization to copy it into `.github/workflows`, start Hosted Windows, or run NSIS.

## Remaining risks and `NOT RUN` work

- PowerShell 7 runtime execution of `Invoke-GitCapture`: **NOT RUN**.
- Real Git empty-output, delayed pipe, timeout/kill, close, and cleanup cases: **NOT RUN**.
- Runtime parsing of the PowerShell script: **NOT RUN**; Tree-sitter syntax parsing is static only.
- GitHub Actions evaluation, Windows Hosted execution, NSIS download/compiler/Probe and synthetic exit 37/42/FileWrite assertions: **NOT RUN / NO-GO**.
- Formal product installer, Service, IPC, database, Schema, U2 production implementation: **NOT RUN / NO-GO**.

This candidate removes the unbounded task-result wait from the code path at source level, but Windows PowerShell behavior and full Runner behavior remain unverified. The historical Run 38276002263 remains a failure in Preflight before Runner/NSIS execution; neither this candidate nor its static checks repair or overwrite that evidence.
