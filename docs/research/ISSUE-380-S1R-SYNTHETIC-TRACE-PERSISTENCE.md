# #380-S1R Synthetic NSIS Trace Persistence

**Disposition: `#380-S1R OBSERVABILITY_BLOCKED`**

The authorized single Hosted Windows run downloaded and extracted the pinned NSIS 3.11 ZIP, then stopped before invoking NSIS. The archive contained two `makensis.exe` candidates; the runner failed closed because it required exactly one. No second Hosted run was started.

## Frozen and tested identities

| Identity | Commit | Tree |
| --- | --- | --- |
| Frozen S1 base | `bd56788aa317d808dadf930260e191e9d1b45adb` | `45abf0197c7fab7ff39dcf37e644f7689bf50511` |
| S1R harness tested by Hosted | `aa3f0b02ae7f86b455af94c311288627c895a838` | `8b3342d2f29cf53f09a8499415c5a3f5a50ec86b` |

S1R was created as a child of the frozen S1 commit. The original S0 Probe blob was checked locally and again by Hosted preflight. The tested diff from S1 adds only the one-shot workflow, its Windows runner, and the offline validator. No production code, Schema, IPC, installer, or PR #386 files changed.

## Offline preflight

Before the branch was pushed, the following checks passed on the exact tested commit:

- `scripts/validateIssue380S1RHarness.py`: exact branch and S1 ancestry, unchanged S0 Probe, one push-only trigger, exact S1R branch and narrow path filter, one Windows job, `contents: read`, and no product, Service, database, release, benchmark, or application-build paths.
- PowerShell grammar parsing for the runner and all six workflow PowerShell blocks.
- Workflow YAML parsing: one job and eight bounded steps, including a 13-minute probe-step timeout and a 15-minute job timeout.
- Exact NSIS ZIP URL and SHA-256 constant checks, plus `git diff --check`.

The runner downloads only the approved SourceForge URL into the task-specific `$RUNNER_TEMP` directory. It checks the archive SHA-256 before extraction, has no Chocolatey or ambient-compiler fallback, and removes task-owned temporary content after upload. The only Registry path is the synthetic `HKCU\Software\ZenCanvasS0Harness` key in the 64-bit view. A preexisting key is preserved; cleanup is permitted only after the runner proves the key was absent and records ownership.

## Single Hosted run

- Run: [38243481115](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38243481115)
- Job: [114791539023 — Synthetic NSIS ZIP trace persistence](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38243481115/job/114791539023)
- Event/ref: `push` on `research/issue-380-s1r-nsis-zip-trace-persistence`, attempt 1.
- Tested SHA/tree: `aa3f0b02ae7f86b455af94c311288627c895a838` / `8b3342d2f29cf53f09a8499415c5a3f5a50ec86b`.
- Environment: GitHub Hosted Windows Server 2022, OS `10.0.20348`, image `windows-2022` version `20261004.326.1`.
- Result: **FAIL** at compiler discovery. Hosted static identity/tool preflight passed. The runner passed its pinned archive verification and extraction gate, then reported `Expected exactly one makensis.exe in the verified archive; found 2.` It stopped before launching either candidate.

The runner code cannot enter extraction unless the downloaded archive SHA-256 equals `c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1`; it cannot enumerate compiler candidates until extraction completes. The Hosted failure therefore establishes that the approved archive was fetched, passed the hash gate, and extracted. The run's `results.json` records the actual candidate paths and acquisition metadata inside the uploaded artifact. This execution environment could not retrieve the artifact ZIP from GitHub's backing store (HTTP 403), so those member records were not independently unpacked here. No candidate path is inferred from the count.

The runner never invoked `makensis -VERSION` or `-HDRINFO`. Compiler version, compiler binary hash, architecture/Unicode identity, and compile output were therefore not established. The four compiler-produced EXEs and generated uninstallers were not created. Stage, installer Abort, uninstaller Abort, `/LOG`, and direct `FileOpen` trace persistence were **NOT RUN / UNVERIFIED**. No direct trace result is claimed.

The synthetic Registry key was absent at the initial check; otherwise the runner would have stopped before download. The runner's cleanup and the workflow cleanup, artifact upload, and task-temp cleanup steps all succeeded. No application, product Registry, Service, IPC, database, or user data was used.

## Preserved evidence

GitHub uploaded artifact `issue-380-s1r-nsis-zip-38243481115-1` (artifact ID `11743917718`, 2,317,564 bytes). Its GitHub-reported ZIP digest is `sha256:bdc3b8ff4e91782a8665c7c3193e6461761fd7e40823bfc2a7f057cd89818cef`. This digest identifies the uploaded evidence bundle, not the NSIS source ZIP. The artifact contains nine files and includes its `SHA256SUMS` manifest. [Open the run artifact](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38243481115/artifacts/11743917718).

The complete GitHub Actions log archive is committed under [`docs/research/evidence/issue-380-s1r/run-38243481115/`](evidence/issue-380-s1r/run-38243481115/). The folder's `SHA256SUMS` records the committed evidence hashes. The GitHub-reported artifact metadata and bundle digest are recorded there as well. Both `gh run download` and the GitHub artifact ZIP endpoint returned HTTP 403 from this execution environment; the hosted artifact itself remains available at the GitHub link above.

## Remaining gate

The acquisition path is now shown to reach the approved ZIP, but this one-shot run did not test runtime trace persistence. Compiler selection within the official archive must be reviewed from the uploaded `results.json` before designing any future probe. This run is consumed. Any follow-up Hosted execution requires a new Owner authorization.

`#380-S1R OBSERVABILITY_BLOCKED`
