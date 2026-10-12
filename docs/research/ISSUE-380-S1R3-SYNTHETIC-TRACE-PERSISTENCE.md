# #380 S1R3 Synthetic NSIS Uninstaller Trace Persistence

## Scope and authorization

This is an isolated synthetic NSIS runtime probe authorized by the S1R2 Owner decision. It does not run the Zen Canvas installer, Service, IPC, database, UAC flow, or product Registry checks. U2 production work remains NO-GO.

The only runtime target is the frozen S0 Probe, compiled with the NSIS 3.11 x86 Unicode compiler from the pinned official ZIP. The S0 Probe source, synthetic Registry key/value, `SetErrorLevel 37/42`, and Abort logic remain byte-identical to S0.

## Frozen source identities

| Source | HEAD | Tree |
|---|---|---|
| S1R2 base | `6fe01abae7640a93f4a694616d758fd54c714960` | `af88309c65999d73564cf3876bb5bd03a15bf597` |
| S1R base | `45a2dfe0e51cee5708dfaa5335affbad947c30c2` | `9bcc5a2a03a8335d2cb2e81c3d497efb275b8724` |
| S0 Probe source | `1f4951f3c93b2eb549d159fed2dd288992c625c8` | Probe blob `5805f8d04cd37420a63f3e156c3b6fd5b4012f33` |

The pinned NSIS ZIP SHA-256 is `c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1`. The launcher/compiler member paths, lengths, and hashes are checked before execution by the runner and independent offline validator.

## S1R3 contract

- The runner uses `ProcessStartInfo.ArgumentList` and records process ID, image path and SHA-256, argument array, working directory, timeout state, exit code, and captured stdout/stderr.
- The generated uninstaller receives `/S` and, as the final argument, `_?=<absolute task-private directory>`. Its working directory, `%TEMP%`, and `%TMP%` remain under the unique task temp root. This is intended to suppress NSIS's self-copy/restart path; only the Windows runtime result can establish the expected exit code.
- `TRACE_MODE=file` runs before optional DetailPrint and `/LOG` comparison. The file-mode Abort calls omit `/LOG`; the evidence succeeds only if both direct FileWrite records persist after the corresponding processes exit and contain context, predicate, type, expected, actual, and `result=FAIL`.
- Installer Abort must exit 37 and uninstaller Abort must exit 42. Assertions are not relaxed.
- The task checks for processes whose image paths are inside its private temp root before deleting the synthetic HKCU key and again before removing task files. It does not terminate unrelated processes. If process state cannot be checked or a task process remains, it preserves the owned temp root and does not delete the Registry key.
- The one-job workflow has only `contents: read`, is push/path-filtered to the S1R3 branch and harness paths, and rejects attempts other than the first.

## Change set

Relative to S1R2, the intended changes are limited to the S1R3 workflow, the copied/updated S1R3 Runner, the independent offline validator, this report, and run-specific evidence. No production code, schema, IPC, S0 Probe, previous research branch, master, or PR #386 is changed.

## Validation and runtime result

### Offline preflight before Hosted execution

- The S1R3 branch is based on the exact S1R2 HEAD/Tree above; the old S1R2 worktree remains clean.
- The current frozen Probe blob matches the S0 commit and S1R2 base: `5805f8d04cd37420a63f3e156c3b6fd5b4012f33`.
- Independent validator: PASS. It checks the branch/base identities, permitted file scope, Workflow trigger and job restrictions, pinned ZIP/member identities, compiler selection, `_?=` final-argument assertion, file-first order, strict 37/42 checks, and process-safe cleanup contract.
- PowerShell source and every `pwsh` Workflow step parse with the available Tree-sitter PowerShell grammar: PASS, 0 parser errors. This is a syntax parse; a Windows `pwsh` interpreter is not available in the local environment.
- The one Hosted job's first preflight step also uses Windows PowerShell's built-in `System.Management.Automation.Language.Parser` on the Runner before any NSIS download or execution.
- Workflow YAML parses with PyYAML and has one `windows-2022` job, `contents: read` only, exact branch/path push filters, a 15 minute job limit, a 13 minute probe limit, and a first-attempt guard.
- Offline ZIP validation: PASS; 441 members, 7,134,287 uncompressed bytes, archive SHA-256 and both fixed executable member identities match the approved constants.
- No product, Service, IPC, database, application build, full CI, or benchmark path is present in the S1R3 job.

The final tested HEAD/Tree, exact diff, run/job/artifact identities, raw evidence SHA-256 values, assertions, cleanup result, and remaining risks are added after the one authorized Hosted run. No automatic retry or second Hosted run is permitted.

## Final result

### One authorized Hosted run

- Run [38250509824](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38250509824), attempt `1`, completed with `failure`. Job `114815262954` is [available here](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38250509824/job/114815262954). This was the only S1R3 Hosted run.
- Tested HEAD: `81bf1e7de4352e5971095167b8a4b18395c09338`; Tree: `b165d4c0fe0bcf454d790c962e4f7cef99a38b19`.
- The Hosted source identity and Windows PowerShell `Parser::ParseFile` preflight passed. The synthetic HKCU key was initially absent. Both pinned compiler archive members passed pre-extract and extracted-file SHA/length verification; the downloaded ZIP SHA matched the approved value.
- The Runner then stopped before invoking `makensis -VERSION`. Its task-private path guard rejected the working directory when it was exactly the task root (`D:\a\_temp\issue380-s1r3-task`) because the guard accepted descendants only. `results.json` records `processes: []`; no NSIS compiler process, Probe compilation, installer, or uninstaller ran.
- The three executed Runner assertions passed: the synthetic key began absent, both pinned archive members were valid, and both extracted members stayed under the task root. The compiler version/architecture checks, four compile outputs, two generated uninstallers, installer exit `37`, uninstaller exit `42`, and FileWrite persistence checks were not reached. No direct trace or `/LOG` comparison exists for this run.
- Cleanup evidence reports the synthetic HKCU key absent, no task-owned process remaining, and the owned task temp directory removed. Workflow cleanup and artifact upload succeeded; the overall job failed because the Probe step failed.

### Raw evidence and integrity

The GitHub Artifact is `11746953497`, `issue-380-s1r3-uninstaller-trace-38250509824-1`, 2,526,925 bytes, SHA-256 `2dc1e6c8a5595f8fefd968bffdf66ddcb7f62b0c19126e92c4b01e64c28ff89e`. Its internal 10-entry `SHA256SUMS` verified without mismatch. The full workflow log SHA-256 is `9526d2b0365846a2ffb6dfe29ba005a779fcf31018e1bc73a7047963b52b26d4`.

Committed evidence is under `docs/research/evidence/issue-380-s1r3/run-38250509824/`: the original artifact ZIP, LF-normalized JSON/text sidecars, complete [workflow log](evidence/issue-380-s1r3/run-38250509824/workflow-run.txt), and [run metadata](evidence/issue-380-s1r3/run-38250509824/run-metadata.json). The [evidence README](evidence/issue-380-s1r3/run-38250509824/README.md) explains raw-versus-normalized copies. Review [`results.json`](evidence/issue-380-s1r3/run-38250509824/results.json), [`cleanup.json`](evidence/issue-380-s1r3/run-38250509824/cleanup.json), [`failure.txt`](evidence/issue-380-s1r3/run-38250509824/failure.txt), and the [raw Artifact ZIP](evidence/issue-380-s1r3/run-38250509824/artifact.zip).

### Disposition and remaining risk

The run identifies a Harness path-guard defect, not an NSIS runtime result. It does not resolve whether `_?=` causes the uninstaller's actual process to return `42`, and it does not prove direct FileWrite persistence after either Abort. The proposed runner correction is to allow the exact task root as a working directory while continuing to require every launched executable and child temporary path to stay within that root. That correction remains untested.

No production files, S0 Probe, S1R2, master, or PR #386 changed. No second Hosted run was started. U2 remains NO-GO.

`#380-S1R3 OBSERVABILITY_BLOCKED`
