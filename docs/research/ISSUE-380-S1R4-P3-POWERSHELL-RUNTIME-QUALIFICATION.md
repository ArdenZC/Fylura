# #380 S1R4-P3 PowerShell Runtime Qualification Plan

## State and scope

This is an isolated, review-only plan for qualifying the frozen bounded Git capture functions with PowerShell 7 on `windows-2022`. It does not activate a workflow and does not claim runtime evidence.

Current gate:

```text
BOUNDED_GIT_CAPTURE_STATIC_ACCEPTED
POWERSHELL_RUNTIME_QUALIFICATION_REQUIRED
ISOLATED_RUNTIME_TEST_PLAN_ONLY_GO
HOSTED_NSIS_NO_GO
U2_PRODUCTION_NO_GO
```

The plan covers only `Invoke-GitCapture`, `Get-RequiredGitScalar`, `Assert-GitNoOutputSuccess`, and `Assert-GitWorktreeClean` from the frozen research Runner. It creates a temporary Git repository and controlled child processes under the hosted task's private temporary root. It does not run the full Runner, download or run NSIS, access HKCU, or open product databases.

The proposed workflow exists only inside this document. No file is added under `.github/workflows`, and this preparation does not start a GitHub job.

## Frozen identity

| Item | Frozen value |
| --- | --- |
| Code freeze HEAD | `af305adb8fba017dd9284a829b5101a209a9b4e2` |
| Code freeze Tree | `9d7716add5fde99c6e797065f336e0d0cf86a510` |
| Research final HEAD | `719c6c236b09980873eb8c214b09aaa16b505532` |
| Research final Tree | `2e5e3795c56131a9a6d76917592c70e2be2650a4` |
| Runner path | `scripts/issue380S1R4P3SyntheticNsisRunner.ps1` |
| Runner Git Blob | `c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa` |
| Runner raw SHA-256 | `92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece` |
| Historical Run | `38276002263`, Attempt `1`, `FAILURE`; immutable and not rerun |

The starting research HEAD and Tree were rechecked before editing; the origin branch `research/issue-380-s1r4-p3-bounded-git-capture` still pointed at the frozen research HEAD. The Runner's local Git Blob and raw SHA matched the values above. The current container has no `pwsh` executable, so no PowerShell parsing or runtime execution was performed here.

## Proving the test uses the frozen implementation

The future workflow must verify all of these before starting the test harness:

1. The checked-out commit is the activation commit on the dedicated one-shot branch; its direct parent is the Owner-reviewed plan commit, and its only change is the one future workflow file.
2. The frozen code HEAD resolves to the frozen Tree and is an ancestor of the activation commit.
3. `git rev-parse <freeze>:scripts/issue380S1R4P3SyntheticNsisRunner.ps1` is the frozen Runner Blob.
4. `git hash-object -- scripts/issue380S1R4P3SyntheticNsisRunner.ps1` is the same Blob, and the file's raw SHA-256 equals the frozen raw SHA.
5. P1 path guard/test sources and the S0 Probe still have their recorded historical Blobs, raw hashes, and working-file Blobs. Git checkout conversion attributes are inspected before any script is executed.

The PowerShell harness reads those verified Runner bytes as strict UTF-8 and parses them with `System.Management.Automation.Language.Parser`. It selects exactly one `FunctionDefinitionAst` for each of the four named functions and writes each `Extent.Text` directly to a separate UTF-8 file without a BOM or replacement. It records the function extent byte length and SHA-256 and checks that the bytes reread from disk hash identically. It then dot-sources those four extracted extents in dependency order. No function text is rewritten, wrapped, or copied from a model. The functions' only script-scope dependency, `$script:GitInvocations`, is initialized by the harness.

This proves byte identity of each extracted source extent relative to the verified Runner file. It does not prove that a PowerShell runtime interprets the functions as intended; that is the separate future runtime evidence.

## Runtime matrix

Each completed test writes a JSON record with its expected outcome, actual capture object, UTC start/end, monotonic elapsed milliseconds, and the Git invocation records created during that test. The harness stops on an unexpected result, records the failure, runs its owned-process cleanup, writes a final summary, and exits nonzero.

| ID | Runtime case | Fixture and required assertion |
| --- | --- | --- |
| 1 | Real Git, exit `0`, empty stdout/stderr | In a clean temporary repository, `Assert-GitWorktreeClean` and `Assert-GitNoOutputSuccess` must accept `git diff --quiet --exit-code`; both streams must be complete and empty. |
| 2 | Real Git, non-empty stdout | `Get-RequiredGitScalar` must accept one non-empty `git rev-parse HEAD` line. A synthetic untracked file must make `Assert-GitWorktreeClean` reject; raw porcelain output must be non-empty. |
| 3 | Real Git, nonzero exit and stderr | A `git cat-file -e` lookup for a known absent object must retain its nonzero process exit code and complete stderr independently. |
| 4 | Normal exit and stream close | `git --version` must exit normally with both read tasks `RanToCompletion`, both stream-complete flags true, confirmed process exit, and completed cleanup. |
| 5 | Delayed stdout and stderr | A temporary Git alias launches the tracked child fixture. It emits both synthetic markers after 1.2 seconds. Both streams must finish under 5 seconds, with the measured full call time recorded. |
| 6 | Git exits while a child holds inherited output handles | A Git alias starts a task-owned PowerShell child that inherits stdout/stderr and then exits. The capture must record the output deadline or a stream/task failure and may never be classified as an ordinary completed exit. The harness verifies and reaps the child by PID, image path, unique nonce, and command line. |
| 7 | Git process timeout and process-tree termination | A controlled alias stays alive with a task-owned descendant. `Invoke-GitCapture` is called with a 2-second process timeout. It must record timeout, a kill attempt, successful kill, confirmed root exit, and no surviving controlled descendant. Any mismatch fails the run. |
| 8 | Kill or resource-cleanup failure | **Not deterministically injectable through the frozen function.** It constructs `System.Diagnostics.Process` directly and exposes no test seam for forcing `Kill`, stream-close, or `Dispose` failure. The harness records this as `UNPROVEN`, not `PASS`. Case 7 still requires fail-closed timeout status and records any naturally observed kill/cleanup errors. Exhaustive runtime acceptance requires an Owner decision: accept this branch as a documented unproven limit or separately authorize a testability seam. |
| 9 | Output read task failure, cancellation, or deadline | The inherited-handle case must record the configured output deadline or a task fault/cancel state. Any incomplete or faulted/canceled read is a failure result, never success. The frozen code supplies no cancellation token, so a canceled-task result is observed if it occurs but is not claimed as injected. |
| 10 | Two consecutive calls | A real missing-object call followed by `rev-parse HEAD` must preserve distinct PIDs, exit codes, stderr, statuses, and ordered invocation records. |
| 11 | Full evidence consistency | UTC timestamps must be ordered; monotonic elapsed values nonnegative; a `EXITED_ZERO` record must have exit `0`, complete successful stream tasks, complete cleanup, and no timeout/error flags. Any timeout/error field forbids ordinary exit-success status. |
| 12 | Empty scalar and launch exception | `Get-RequiredGitScalar` must reject a real zero-exit empty `git rev-list --max-count=0 HEAD` result. A nonexistent working directory must be retained as `LAUNCH_FAILED` and rejected by the wrapper. No controlled child may remain. |
| 13 | Final evidence JSON | Re-read the complete case records after all cases and recheck every timestamp, actual/expected result, stream status, cleanup status, and error/status relationship before writing the final summary. |

Cases 1–4 and 10–12 use the actual Git executable discovered by the frozen function. Cases 5–7 use actual Git as the parent executable plus Git for Windows' shell alias mechanism to launch the tracked, test-only PowerShell child. The alias child does not replace or mock `Invoke-GitCapture`. The workflow records the Git executable, shell path, PowerShell path/version, Windows image, and all child PIDs, image paths, arguments, exit observations, and cleanup results.

Case 8 is the known qualification gap. It cannot be reported as passed by static checks, a Python model, or the `Kill` success case. The prepared harness records it as `UNPROVEN` and exits nonzero after collecting the other cases. Owner review must decide whether to authorize a narrowly scoped test seam or accept this precise limit before any future Hosted execution can be called a complete qualification.

## Timing contract and limits

The frozen function configures `WaitForExit(TimeoutSeconds * 1000)`, a 5,000 ms process-tree termination wait, a 5,000 ms output-task wait, and a 1,000 ms post-close output wait. At the default 20-second Git timeout, their arithmetic sum is 31,000 ms; for the 2-second timeout case it is 13,000 ms. These are configured waiting limits, not a proven whole-function worst-case completion time.

The function also performs synchronous `Process.Kill(true)`, stream `Close()`, `Dispose()`, process start, and PowerShell/.NET scheduling. Their strict upper bounds have not been established. The harness records monotonic elapsed time around each complete function call and separately records the configured per-stage values. It does not infer an upper bound from the sum, from `Task.WaitAll` returning, or from the `Task` being completed. It checks `IsCompletedSuccessfully`, task status, stream flags, errors, and process cleanup fields.

The proposed hosted test controller has a seven-minute worker deadline and the job has a ten-minute Actions deadline. On controller timeout it attempts process-tree termination, waits a bounded period, records all results, and fails closed. These outer deadlines protect the test job; they do not prove the frozen function's synchronous operations have strict time bounds. A GitHub job timeout may prevent later artifact steps from running, so Actions logs remain the fallback for a hard job timeout.

## Isolated environment and evidence

The future workflow creates one fresh run root under `RUNNER_TEMP`, with separate `task/` and `evidence/` directories and a run/attempt/SHA ownership record. Reuse, path escape, symlink, junction, or reparse-point conditions fail closed. The task directory contains only the temporary Git repository, byte-exact function fragments, and the tracked child fixture. The Git repository has one synthetic file and run-local identity configuration.

No HKCU key, user document, product repository, product database, Service, IPC, NSIS download, or installer is accessed. The synthetic Git process has system Git configuration disabled, an empty run-owned global Git configuration file and an empty repository template; only the temporary repository's user identity and test alias are configured. The test child is recognized by its unique run nonce plus exact PowerShell/Git image and script path. Cleanup never kills a process based only on a PID; it rechecks the image and command line first. If identity is ambiguous, cleanup refuses the action, reports failure, and preserves the evidence root.

The evidence contract is:

```text
workflow-run.json               Run/ref/attempt, Windows, PowerShell, Git identity
preflight.json                   frozen HEAD/Tree/Blob/raw SHA, worktree, and Git checkout-attribute results
harness-start.json               harness PID/path, nonce, triggering identity
controller.json                  when started: child PID/path/args, deadline, exit and cleanup state
controller.stdout.raw.txt        when started: raw harness stdout
controller.stderr.raw.txt        when started: raw harness stderr
case-*.json                      expected/actual, times, invocations, process and stream details
runtime-qualification.json       complete aggregate result and explicit unproven cases
cleanup.json                     owned process and task-root cleanup results
SHA256SUMS                       SHA-256 for each payload file (excluding the manifest itself)
```

Evidence upload is qualified independently of harness success. A dedicated `always()` step verifies the exact run root is inside `RUNNER_TEMP`, its ownership marker matches repository/ref/run/attempt/SHA, and no reparse points or path escapes exist. The upload candidate is limited to 250 files and 20 MiB. If ownership cannot be proven, upload is refused and the Actions log is retained. The future upload action uses `if-no-files-found: error`; missing evidence is a failure.

On a normal pass, the workflow removes only its own task directory after it verifies all controlled processes are gone; evidence remains until artifact upload. On any test or cleanup failure, it preserves the task directory and evidence for review. The outer controller also scans only processes whose command line contains the run's random nonce and the exact child script path, then requires the image to match the run-pinned Git or PowerShell path before acting. It does not clean unknown directories or processes.

## Future Workflow activation contract

The complete inert draft is embedded in [the workflow draft](ISSUE-380-S1R4-P3-POWERSHELL-RUNTIME-WORKFLOW.md). It proposes one `windows-2022` job, `contents: read`, a ten-minute job limit, one dedicated future branch, workflow-path filtering, exact repository/ref checks, and `run_attempt == 1`. It has no `workflow_dispatch`, no matrix, no retry, and no NSIS/Rust/Tauri/npm/product steps.

The draft intentionally contains a fail-closed activation parent sentinel because the reviewed plan commit does not exist yet. A later activation would require a separate Owner GO, replacement with the exact reviewed plan HEAD, and one direct child commit whose sole addition is `.github/workflows/issue-380-s1r4-p3-git-runtime.yml`. That activation commit and its new SHA values must receive a separate static review before any push. This plan does not authorize that activation or a Hosted run.

The existing workflows use different branch names and path filters. This research branch and all files added here do not match their triggers. The new workflow is not in `.github/workflows`, so it is not executable in this commit.

## Preparation validation and current result

The accompanying Validator checks the freeze identities, unchanged Runner/P1/S0 sources and historical workflow, expected research parent, allowed added paths, absence of an active workflow, the 13-case contract, and the explicit kill-failure injection gap. The SHA manifest binds this report, the inert draft, harness, child fixture, Validator, and freeze identity record.

Actual PowerShell 7 parsing, execution of the extracted functions, Windows process/pipe behavior, and the proposed GitHub workflow are **NOT RUN** in this stage. No Hosted job, NSIS process, full Runner, product process, CI matrix, or database was run.

Plan status:

```text
S1R4-P3 POWERSHELL_RUNTIME_QUALIFICATION_PLAN_READY_FOR_OWNER_REVIEW
```

This status means the isolated plan is ready for Owner review. It does not accept PowerShell behavior, close the fault-injection gap, authorize a Hosted run, or change `HOSTED_NSIS_NO_GO` / `U2_PRODUCTION_NO_GO`.

## Remaining plan risks

- The actual Windows behavior of inherited pipe handles and closing an in-flight `ReadToEndAsync()` task is unknown until an Owner separately authorizes a Hosted run.
- A deterministic `Kill`/`Close`/`Dispose` failure cannot be safely forced with the frozen function's current direct `Process` construction. The draft deliberately reports case 8 as `UNPROVEN` and therefore cannot return a full runtime pass until Owner resolves that testability boundary.
- Git for Windows' `sh.exe` alias behavior is required for delayed and inherited-handle fixtures. The workflow fails before those cases if the expected Git shell is absent; it has no alternate runtime or ambient fallback.
- Synchronous termination and stream/resource methods may exceed their configured neighboring waits. Observed durations and outer job deadlines are evidence, not strict worst-case guarantees.
