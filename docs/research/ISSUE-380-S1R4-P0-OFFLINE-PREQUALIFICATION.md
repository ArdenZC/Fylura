# #380 S1R4-P0 Offline Prequalification — BLOCKED

## Disposition

`#380-S1R4-P0 OFFLINE_PREQUALIFICATION_BLOCKED`

No path-guard behavior test was executed. The current environment is Linux and has no PowerShell runtime, .NET runtime, or Windows compatibility runtime. The required Windows PowerShell path predicate cannot be truthfully marked PASS here. No Hosted job was started.

This report records the blocker on an independent continuation branch. It does not claim that the S1R3 path guard was fixed or that any positive or negative path case passed.

## Frozen source and branch

| Item | Identity |
|---|---|
| S1R3 final HEAD | `08bad6b19831d40dda9d296de880aaae6c83c312` |
| S1R3 final Tree | `5360d8c81e5949333c8d6ef16acd0c658d315598` |
| Starting branch | `research/issue-380-s1r3-nsis-uninstaller-trace-verification` |
| S1R4-P0 continuation branch | `research/issue-380-s1r4-p0-offline-harness` |
| S0 Probe blob at frozen S1R3 | `5805f8d04cd37420a63f3e156c3b6fd5b4012f33` |

The local starting worktree was clean. The origin S1R3 branch pointed to the frozen HEAD. No S1R4-P0 origin ref existed at the start of this work.

## Execution environment evidence

The following command was run in `/workspace/Zen-Canvas-380-S1R3`:

```text
command -v pwsh       -> no result
command -v powershell -> no result
command -v dotnet     -> no result
command -v wine       -> no result
uname -srm            -> Linux 6.18.44 x86_64
```

The current managed cloud environment reported `capabilities: []`; it exposes no attached Windows executor. No other runtime capable of executing the actual Windows PowerShell predicate is available in this task. A Python model, a syntax parser, or a Linux-only approximation would not establish Windows path and reparse-point behavior, so none is counted as a test.

## Static source facts only

The S1R3 Runner's `Invoke-CapturedProcess` is at `scripts/issue380S1R3SyntheticNsisRunner.ps1:132`.

- It canonicalizes the executable and working-directory strings with `System.IO.Path.GetFullPath`.
- It builds `TaskRoot + DirectorySeparatorChar` and checks both paths with `StartsWith(..., OrdinalIgnoreCase)` at lines 142–147. This excludes a working directory equal to `TaskRoot`; that is the known deterministic cause of the prior S1R3 stop before `makensis -VERSION`.
- The executable check does not itself require an existing leaf file. Other parts of the current Runner check pinned compiler members, but the process guard has no separate executable-kind contract.
- The check does not detect a symlink, junction, or other reparse point in the task-root path or its descendants.
- `ProcessStartInfo.ArgumentList` is used at line 158. In the synthetic uninstaller path, `_?=<absolute install directory>` is appended last and asserted at lines 671–678. This is source inspection only; no process was launched in this environment.
- The Runner lists strict expected exit codes 37 and 42 and performs them in `TRACE_MODE=file` before optional `detail` mode. The frozen Probe itself was not edited.
- Registry, process, and temp cleanup are guarded in the Runner/workflow by preexistence checks, ownership markers, and task-process checks. Their runtime behavior was not re-executed.

These observations do not validate path normalization, containment, case handling, separator handling, filesystem identity, reparse-point detection, or process behavior.

## Required cases and result

| Case | Result |
|---|---|
| Exact TaskRoot as working directory | NOT RUN — requires actual PowerShell predicate execution |
| Direct child and nested child working directories | NOT RUN |
| Shared-prefix sibling | NOT RUN |
| Parent directory | NOT RUN |
| Another `RUNNER_TEMP` directory | NOT RUN |
| `..` traversal | NOT RUN |
| Windows case and separator variants | NOT RUN — Windows path semantics unavailable |
| Symlink/junction/reparse-point rejection or verification contract | NOT RUN — Windows filesystem semantics unavailable |
| TaskRoot working directory for both compiler identity commands | NOT RUN — no compiler process may be launched in P0 tests |
| Existing in-root executable file accepted; external executable rejected | NOT RUN |
| Strict 37/42 assertions, FileWrite persistence, and owned-process cleanup | NOT RUN |

The table contains no simulated PASS results. No NSIS executable, S0 Probe, installer, uninstaller, or unrelated process was run.

## Safe contract to validate when a PowerShell runtime is available

The next offline validator should execute the same PowerShell functions imported by the Runner, without launching any executable:

1. Canonicalize the root and candidate paths with Windows path rules. Compare with `OrdinalIgnoreCase`; permit exact root equality only for a working directory. For descendants, compare against the root with a directory separator appended, so shared-prefix siblings cannot match.
2. For an executable, require strict descendant containment, an existing file leaf, and no directory at that path. For a working directory, require an existing directory and allow either exact root equality or a strict descendant.
3. Reject traversal inputs and any path whose resolved component chain contains a reparse point, including the task root itself. This is the conservative contract; the intended test must exercise it on Windows with an actual reparse point.
4. Keep task-root ownership markers, process identity checks, HKCU preexistence checks, and evidence/temp cleanup guards unchanged in effect.
5. Invoke only the path predicate in the test. In particular, the `-VERSION`/`-HDRINFO` expectations in the test matrix are working-directory predicate checks, not permission to launch NSIS during offline P0.

This contract is a proposed test target, not an implementation or a validated result.

## Evidence and hashes

At the frozen S1R3 baseline, the inspected source file hashes were:

| File | SHA-256 |
|---|---|
| `scripts/issue380S1R3SyntheticNsisRunner.ps1` | `748d177df97369d2abcb571185d824f27c9f24ff0d275990be755003c4867db9` |
| `scripts/validateIssue380S1R3Harness.py` | `1a05583693f36492352bef21d3d75a942c136f736d5a03f628c3f59cfd3e70ac` |

These are baseline identities, not test outputs. No new test output or runtime artifact was produced. The only intended change in this blocked continuation is this report.

## Stop condition

The exact PowerShell predicate must be executed on a Windows runtime before S1R4-P0 can leave BLOCKED. This report does not authorize a Hosted job, and it does not authorize U2 production work.
