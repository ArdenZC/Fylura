# #380 S1R4-P3 Runner null-safety research candidate

## Status and scope

`S1R4-P3 RUNNER_NULL_SAFETY_CANDIDATE_READY_FOR_OWNER_REVIEW`

This is an offline research candidate. It repairs the research Runner's Git output handling and prepares a new inert activation contract. It does not modify the consumed activation commit, its Workflow, historical Run or Artifact, the old frozen Runner, S0 Probe, P1 path guard, production code, Schema, or PR #386.

No PowerShell runtime, Windows machine, Hosted job, NSIS download/compiler/Probe, installer, Service, IPC, database, full CI, or Codex Review was run. The source and Python model checks below are not PowerShell runtime proof. `HOSTED_NO_GO` and `U2_PRODUCTION_NO_GO` remain in force.

Preflight and Runner status are separate:

- Historical Run `38276002263`, attempt 1, failed in the activated Workflow Preflight at its empty-output `.Trim()` before the Runner step. This candidate does not claim that the Preflight correction has passed a PowerShell runtime test.
- Owner independently corroborated a second null-output hazard in the frozen Runner at its line 665. That Runner was skipped in the historical Run. This candidate prepares a distinct Runner version; it does not rewrite the historical source or prove a Windows execution.

## Identity and ancestry

The accepted research base was `740d99379c584589c92bd855f101e06858e8ee38`, Tree `5d3b70dd4f2f877623f74b2c7cade3bceadf9552`.

The new Runner freeze point is `21383e774c692da59a9f6877cebed2acd9faee38`, Tree `e937d58068c6ebf020f6239df13b1185c6d287e1`, parent `38947478b9a4bea8df1893872e75e193d0dd4f29`, and a descendant of that accepted base. This code-freeze commit includes the Runner candidate and the offline Validator. The final research package adds documentation and identity evidence after that freeze point; any future activation branch must fork from the code-freeze commit above, not from the later documentation package head.

| Runner identity | Value |
|---|---|
| Path | `scripts/issue380S1R4P3SyntheticNsisRunner.ps1` |
| New Runner Git Blob | `2a80ac621f3cd0e9dd3386bb3488a362df67fdcf` |
| New Runner raw SHA-256 | `5aaeff4dc7a78362b1caea21927c3a07180d62548180d09c37cab2559616f176` |
| Candidate freeze commit / Tree | `21383e774c692da59a9f6877cebed2acd9faee38` / `e937d58068c6ebf020f6239df13b1185c6d287e1` |
| Planned activation branch | `research/issue-380-s1r4-p3-runner-null-safety-activation` |
| Planned added Workflow | `.github/workflows/issue-380-s1r4-p3-runner-null-safety.yml` |

The machine-readable identity record is `docs/research/evidence/issue-380-s1r4-p3-runner-null-safety/runner-identity.json`. The corresponding complete Workflow is documentation only at `docs/research/ISSUE-380-S1R4-P3-RUNNER-NULL-SAFETY-WORKFLOW.md`.

## Runner correction

The frozen Runner contained direct native invocations throughout the source-identity block, not only the reported status line. Examples were `rev-parse HEAD`, `rev-parse HEAD^{tree}`, `git status`, P1/P2 and earlier-stage Tree/ancestry checks, P1 path Blob checks, and the frozen/current S0 Probe Blob check. Some calls grouped multiple invocations before checking `$LASTEXITCODE`; required outputs were also directly trimmed. The clean `git status --porcelain --untracked-files all` case can supply no stdout, so direct `.Trim()` on that value can fail before the empty-status condition is evaluated.

The candidate replaces every direct Git invocation in the Runner with one-process-at-a-time `ProcessStartInfo` capture:

- `Invoke-GitCapture` captures stdout, stderr, process exit code, timeout, process ID and launch/cleanup errors independently for each Git process. It uses a bounded 20-second wait and a bounded wait after process-tree termination. Captured text in the JSON record is capped at 8192 characters and includes full character counts and truncation flags.
- `Assert-GitWorktreeClean` accepts only exit code 0 with exactly zero stdout characters and empty stderr. Any status output is dirty and rejected. A nonzero exit, timeout, launch failure or stderr is failure-closed.
- `Get-RequiredGitScalar` requires exit code 0, empty stderr and exactly one non-empty output line. Empty, multi-line, timed-out or failed HEAD/Tree/Blob output is rejected before comparison.
- `Assert-GitNoOutputSuccess` handles ancestry predicates by requiring exit code 0 and no output. Nonzero ancestry results are rejected.
- No Runner Git call uses `$LASTEXITCODE`; the exit code belongs to that individual `Process` object and is recorded before another call can occur.
- The result JSON includes the ordered `git_invocations` evidence. The existing outer failure path records the error, finalizes failure evidence where possible, and exits 1. All Git identity checks occur before `Download-And-VerifyPinnedZip`.

The Runner also checks that the current checked-out HEAD equals the event SHA, the preflight's candidate freeze identity equals the actual `HEAD^`, and the committed Runner Blob, worktree Runner Blob, raw file SHA-256 and preflight-recorded Runner identities agree. The future Workflow Preflight checks the exact freeze commit and direct-child activation diff before the Runner step is eligible. This creates two separately inspectable identity boundaries without embedding a self-referential hash constant inside the Runner.

The remaining `.Trim()` in the Runner applies to captured NSIS version text, not native Git output.

## Frozen semantics retained

The candidate keeps the P1, P2 and S0 source identities and test behavior. The focused Validator compares the frozen Runner and current protected files and checks the fixed test values. It verifies that the following values and contracts remain unchanged from the frozen Runner:

- NSIS 3.11 ZIP SHA-256 `c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1`;
- forwarding launcher SHA-256 `f497e92deb9f179f7b8f7553fcb3bd04f511bb2949e5e4a2aee80a10f7b20431` and compiler SHA-256 `42850802704ecb11163f7e0329d35ee54bd288953200d4966e226d572848cfc5`;
- installer exit code `37`, uninstaller exit code `42`, file mode before optional detail mode, and direct FileWrite evidence assertion;
- `_?=<controlled absolute install directory>` as the final uninstaller argument;
- existing path-guard calls, synthetic HKCU ownership, task-process checks, and owned task-root cleanup rules.

The old P3 Runner remains the exact frozen Blob `1d11c3add3c0d1f45e7d7ac082d09c616cca51a1` with raw SHA-256 `c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd` at its historical source commit. The old activated Workflow and S0 Probe remain unchanged. The source hash list and comparisons are recorded in the Validator and `SHA256SUMS`.

## Offline contract results

Run the committed Validator with:

```sh
python3 scripts/validateIssue380S1R4P3RunnerNullSafety.py
```

The tests use a typed Python Git-capture model and static source checks. They do not invoke PowerShell or claim Windows behavior.

| # | Input / invariant | Expected outcome | Result |
|---:|---|---|---|
| 1 | `git status`: exit 0, empty stdout and stderr | Clean status accepted | PASS — model |
| 2 | `git status`: exit 0, porcelain row in stdout | Dirty worktree rejected | PASS — model |
| 3 | `git status`: exit 128, empty stdout, fatal stderr | Fail closed | PASS — model |
| 4 | Required HEAD/Tree/Blob: exit 0, empty stdout | Reject missing identity | PASS — model |
| 5 | Required scalar: exit 0 with stderr, and separate case exit 2 with stdout/stderr | Reject stderr and nonzero exit independently | PASS — model |
| 6 | Sequential Git calls return exit 1 then exit 0 | First result remains failure; second scalar is independently accepted; no `$LASTEXITCODE` in Runner | PASS — model + static |
| 7 | Runner Git preflight throws before NSIS acquisition | Failure is recorded and Runner exits 1; download is not reached | PASS — static order/control-flow check |
| 8 | Preflight identity fields versus committed/worktree Runner Blob and file SHA | All identities must match before later operations | PASS — static |
| 9 | Frozen old Runner, historical Workflow, P1 files and S0 Probe | Frozen Blob/SHA and source bytes remain unchanged | PASS — local Git/object checks |
| 10 | Pinned NSIS hashes, exit codes 37/42, FileWrite-first, final `_?=`, process and cleanup contracts | Values and assertions unchanged | PASS — static comparison |
| 11 | New freeze commit, Tree, Runner Blob/SHA in identity record and Workflow draft | All candidate identities agree | PASS — static |
| 12 | Prior Preflight empty-output capture and Runner clean-status predicate | Both accept only successful empty status; other results fail | PASS — static/model |

The PowerShell source and all embedded PowerShell blocks in the inert Workflow were parsed with Tree-sitter for syntax shape; the YAML draft was parsed with PyYAML. These checks are not a PowerShell runtime, GitHub Actions evaluation, or `actionlint` result.

## Future Preflight + Runner activation contract

No activation is authorized by this report. If the Owner later grants a new, separate GO, the proposed one-shot process is:

1. Treat `21383e774c692da59a9f6877cebed2acd9faee38` / Tree `e937d58068c6ebf020f6239df13b1185c6d287e1` as the frozen Runner candidate base. Keep the candidate Runner Blob and raw SHA above fixed.
2. Create the planned activation branch from that exact commit. Its activation must be one direct child commit whose entire diff adds only `.github/workflows/issue-380-s1r4-p3-runner-null-safety.yml`.
3. Add a new Workflow with a push trigger restricted to that exact branch and that one path. It has no `workflow_dispatch`, uses one Windows 2022 job, `contents: read`, and requires `GITHUB_RUN_ATTEMPT == 1`.
4. Before any NSIS download, Preflight verifies the checkout HEAD, direct parent, exact one-file diff, clean status, Windows LF checkout policy, P1/P2/S0 identities, and the candidate Runner's source Blob, worktree Blob, and SHA-256. Runner execution remains conditional on successful Preflight.
5. Immediately before process start, the Workflow rechecks the same Runner and protected-file identities. The Runner then independently checks its event SHA, actual activation parent against Preflight, clean status, current committed/worktree Runner Blob, raw SHA-256, frozen P1/P2 ancestry and trees, protected P1 files, and S0 Probe Blob. Any failure returns failure before NSIS acquisition.
6. The historical Workflow remains constrained to its original branch and old Workflow path; the new activation branch and new Workflow path cannot match that trigger. Do not alter the old activation branch or Run `38276002263`; retain its original Artifact and JSON as immutable failure evidence.
7. Any later activation or Windows run requires a separate Owner decision. Do not interpret these static checks as that authorization.

The complete proposed YAML and embedded PowerShell are in the inert Workflow document. The draft is stored outside `.github/workflows` and cannot trigger Actions.

## Not run / remaining risks

- PowerShell 7 execution of `Invoke-GitCapture`, including the actual empty native-output case: **NOT RUN**.
- PowerShell execution of the new Preflight + Runner identity handshake: **NOT RUN**.
- GitHub workflow validation/actionlint or any Hosted run: **NOT RUN / NO-GO**.
- NSIS ZIP download, compilation, frozen Probe stage/Abort, exit codes 37/42, and FileWrite persistence: **NOT RUN**.
- Service, IPC, UAC, database, Schema, production installer, U2 production work: **NOT RUN / NO-GO**.

Potential risks remain in Windows `ProcessStartInfo` behavior, Git for Windows executable resolution, stream decoding, process-tree termination and PowerShell object serialization. Tree-sitter cannot settle them. A future Owner-authorized Windows run must be a new single-use run against the new identities and must preserve the consumed historical failure evidence.
