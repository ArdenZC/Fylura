# #380-S1 Synthetic NSIS Trace Persistence

**Disposition: `#380-S1 OBSERVABILITY_BLOCKED`**

The authorized single Hosted Windows run stopped before NSIS was installed or the probe ran. The static preflight passed. Chocolatey received HTTP 504 from its package feed, so the generated installers, Abort outcomes, DetailPrint `/LOG` results, and explicit file trace were not tested. No second Hosted run was started.

## Frozen and tested identities

| Identity | Commit | Tree |
| --- | --- | --- |
| Frozen S0 base | `1f4951f3c93b2eb549d159fed2dd288992c625c8` | `5047093e3f81ce001b8940103311f275ed534fcf` |
| S1 code tested by the Hosted job | `fddbabb02333fb8e79907a79559a62e6a610dc2f` | `0d673e47a4c15d467801f9558d19c7dadce7118b` |

The S1 test commit has S0 as its direct parent. Its code diff is limited to the one-shot workflow, the Windows runner, and an offline scope validator. The original S0 Probe blob was checked against S0 both locally and in the Hosted preflight and was unchanged.

## Offline preflight

Before pushing the test branch, these checks passed:

- `scripts/validateIssue380S1Harness.py`: exact branch/base, unchanged Probe blob, sole push trigger, exact branch and path filter, one Windows job, read-only workflow permissions, and no product install, Service, database, full build, or benchmark paths.
- PowerShell syntax parsing for `scripts/issue380S1SyntheticNsisRunner.ps1` and every workflow `pwsh` block.
- Workflow YAML parsing and `git diff --check`.

The runner's static contract uses only `HKCU\Software\ZenCanvasS0Harness` in the 64-bit Registry view, refuses to overwrite that key if it already exists, and writes an ownership marker only after observing it absent. Runtime cleanup removes the key only when that marker exists. This contract was statically checked; the Windows runner itself did not execute.

## Single Hosted run

- Run: [38240644332](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38240644332)
- Job: [114782105120 — Synthetic NSIS runtime trace persistence](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38240644332/job/114782105120)
- Event/ref: `push` on `research/issue-380-s1-synthetic-nsis-runtime-trace`
- Tested SHA/tree: `fddbabb02333fb8e79907a79559a62e6a610dc2f` / `0d673e47a4c15d467801f9558d19c7dadce7118b`
- Environment: GitHub Hosted Windows Server 2022, OS `10.0.20348`, image `windows-2022` version `20261004.326.1`.
- Result: **FAIL** at `Install pinned NSIS compiler package`. Checkout and S0/source preflight passed; the probe step was skipped; evidence finalization, artifact upload, and cleanup steps passed.

The first failure was the Chocolatey v2 feed query for `nsis` version `3.11.0`: HTTP **504 Gateway Timeout** from `https://community.chocolatey.org/api/v2/`. Chocolatey reported `nsis not installed` and `0/1 packages installed`. The failure is a package-feed availability result, not an NSIS predicate or trace result.

The runner did not obtain `makensis.exe`. There is no compiler version/hash, no hash for the four direct compiler outputs, and no hash for generated uninstallers. No stage, installer Abort, or uninstaller Abort process ran. No output comparison between DetailPrint + `/LOG` and `FileOpen`/`FileWrite`/`FileClose` exists. Those runtime assertions remain **UNVERIFIED**.

The synthetic Registry key was not read or changed: the runner step was skipped. The workflow's cleanup step succeeded; the task temporary directories were removed after evidence upload.

## Preserved evidence

The byte-preserved GitHub Actions log (gzip-compressed for repository storage), run JSON, artifact metadata, extracted Chocolatey failure lines, and their hashes are committed under `docs/research/evidence/issue-380-s1/run-38240644332/`. Decompress the log with `gzip -dc github-actions-run.log.gz`.

| Evidence | SHA-256 |
| --- | --- |
| Compressed full Actions log | `19997b6a98ba9f6054bfc197ecea0d351d1b90fe0cca36938e85fab2ecec4790` |
| Full Actions log SHA-256 after decompression | `5896e8b98d848ed5c376982f7d7b105a6e9d34ae05de4f77d8e465ab2606ea3f` |
| Extracted NSIS package-install step | `9aa9f5d72223bf196087372c977757d3650b10d11211d5575baa3f5aec62b441` |
| Run/job metadata JSON | `fcd3f58efb0017bb377614823709f83ea02b757b6616652e0efe3a31db4f097a` |
| Artifact metadata JSON | `5f873496a6e6b5d7014a3b0eee6e69b60201441f3e5bb2d4859f4d318bb30b8d` |

GitHub uploaded artifact `issue-380-s1-synthetic-nsis-38240644332-1` (artifact ID `11742942653`, 1,246 bytes). The upload action recorded ZIP digest `sha256:bcff4b4623921c30a55d45bf63ecc2c765220fcb468665fb64a3e77a85e02db0`. It contains the preflight result, workflow-run metadata, and their SHA manifest; it contains no NSIS output. [Open the run artifact](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38240644332/artifacts/11742942653).

The GitHub API reports the artifact as unexpired. Downloading the ZIP from this execution environment returned HTTP 403 from its backing artifact store, so the uploaded artifact is linked above and its API metadata is retained locally. The raw Actions log and failure evidence are independently preserved in this branch.

## Scope and conclusion

No Zen Canvas installer, product Registry key, Service, application process, IPC request, or database was used. No UAC, actual product ownership predicate, EstimatedSize discrepancy, cross-generation request, or database isolation behavior was tested.

The S1 objective is **not proven**. The permitted Hosted attempt was consumed by the package-feed timeout. Owner approval is required before any later Hosted execution or change to the tool acquisition path.

`#380-S1 OBSERVABILITY_BLOCKED`
