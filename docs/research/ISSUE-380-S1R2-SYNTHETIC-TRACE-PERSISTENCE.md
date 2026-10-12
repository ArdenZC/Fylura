# #380-S1R2 Synthetic NSIS Trace Persistence

**Disposition: `#380-S1R2 OBSERVABILITY_BLOCKED`**

The single authorized Hosted Windows run validated the pinned NSIS launcher/compiler identities and compiled the frozen S0 probe. It then failed at the first synthetic uninstaller Abort exit-code assertion: the installer Abort returned expected `37`, while the generated uninstaller exited `0` instead of expected `42`. The file-write trace mode was not reached, so persistence after Abort remains unproven. No second Hosted run was started.

U2 production implementation remains **NO-GO**. This test-only harness run establishes no Zen Canvas product behavior and changes no production code.

## Frozen and tested identities

| Identity | Commit | Tree |
| --- | --- | --- |
| Frozen S1R base | `45a2dfe0e51cee5708dfaa5335affbad947c30c2` | `9bcc5a2a03a8335d2cb2e81c3d497efb275b8724` |
| Source tested by Hosted | `f5651e3d26d1ccb2c2ac3922e9863295b6970b27` | `50a9477b6d1b3d1c398ddf96d426de5cd94b4f91` |
| Frozen S0 Probe blob | `5805f8d04cd37420a63f3e156c3b6fd5b4012f33` | unchanged |

The tested source diff from S1R contains only:

- `.github/workflows/issue-380-s1r2-synthetic-nsis-launcher-probe.yml`
- `scripts/issue380S1R2SyntheticNsisRunner.ps1`
- `scripts/validateIssue380S1R2Harness.py`

The S0 Probe, U1R-FIX, master, PR #386, production installer, Service, IPC, Schema, and Coordinator were not changed.

## Offline preflight

Before the Hosted run, the exact official NSIS ZIP was downloaded locally and verified:

- ZIP size: `2,361,546` bytes; SHA-256: `c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1`.
- ZIP entries: `441`; uncompressed size: `7,134,287` bytes.
- All archive paths were checked for traversal, duplicate/case-colliding entries, symlinks, and containment under `nsis-3.11/`.
- Only the two approved `makensis.exe` members were present, with Owner-supplied lengths and hashes.
- Offline ZIP/source scope validator passed.
- YAML validation passed: one push-only job, exact branch/path filters, `contents: read`, Windows 2022, 15-minute job timeout and 13-minute probe timeout.
- Tree-sitter PowerShell parsing passed for the Runner and all six Workflow PowerShell blocks. `git diff --check` passed.
- No Rust, Cargo, UI, product installer, Service, database, IPC, benchmark, or full CI ran.

## One Hosted run

- Run: [38245775766](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38245775766)
- Job: [114799132299 — S1R2 synthetic NSIS launcher and trace persistence](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38245775766/job/114799132299)
- Event/ref/attempt: `push` / `research/issue-380-s1r2-nsis-launcher-verification` / `1`.
- Hosted source: `f5651e3d26d1ccb2c2ac3922e9863295b6970b27` / `50a9477b6d1b3d1c398ddf96d426de5cd94b4f91`.
- Result: failure at `detail_uninstaller_abort_expected_exit`. Preflight, cleanup, evidence finalization, upload, and temporary-file cleanup succeeded.
- Environment: Windows x64, image `win22` / `20261004.326.1`.
- Runtime: PowerShell `7.6.6`, Windows `10.0.20348.0`.

### Compiler and execution evidence

| File | Bytes | SHA-256 | Outcome |
| --- | ---: | --- | --- |
| `nsis-3.11/makensis.exe` forwarding launcher | 2,560 | `f497e92deb9f179f7b8f7553fcb3bd04f511bb2949e5e4a2aee80a10f7b20431` | Executed |
| `nsis-3.11/Bin/makensis.exe` underlying compiler | 468,992 | `42850802704ecb11163f7e0329d35ee54bd288953200d4966e226d572848cfc5` | Verified; not invoked directly |

Launcher `-VERSION` returned `v3.11`, exit 0. `-HDRINFO` returned exit 0 with `NSIS_CPU=x86`, `NSIS_UNICODE`, `NSIS_PTR_SIZE=4`, and `NSIS_VERSION=v3.11`. No ambient compiler or fallback was used.

| Assertion | Result | Observed evidence |
| --- | --- | --- |
| Synthetic HKCU key absent before run | PASS | `HKCU\Software\ZenCanvasS0Harness` was absent. |
| Two pinned archive members verified before extraction | PASS | Both exact paths, sizes, and hashes matched. |
| Two extracted members verified before execution | PASS | Paths remained within task temp; sizes and hashes matched. |
| NSIS 3.11 launcher identity | PASS | PID 6528, `-VERSION`: `v3.11`, exit 0. |
| x86 Unicode compiler identity | PASS | PID 2656, `-HDRINFO` passed. |
| Detail stage compile and run | PASS | Compile PID 4196 exit 0; stage PID 7132 exit 0; synthetic value was `REG_DWORD=35149`. |
| Detail installer Abort compile | PASS | Compile PID 7460 exit 0; EXE was 39,020 bytes. |
| Detail installer Abort exit | PASS | PID 2328 returned 37; expected 37. |
| Generated uninstaller Abort exit | **FAIL** | PID 1320 returned 0; expected 42. First failing predicate. |
| File mode and post-exit trace assertions | NOT RUN | Execution stopped at the preceding failure. |
| Four compiler outputs/two generated uninstallers | NOT RUN | Only two detail-mode EXEs and one detail-mode uninstaller were produced. |

The detail stage EXE was 38,822 bytes; generated uninstaller was 38,022 bytes. Their hashes, process records, raw stdout/stderr, and compiler outputs are recorded in `results.json`. The observed exit mismatch does not establish why NSIS returned 0.

### Trace and `/LOG` result

The runner stopped before its `/LOG` comparison and before compiling/running `TRACE_MODE=file`. Direct `FileOpen`/`FileWrite`/`FileClose` persistence is **NOT PROVEN**; the file-trace assertions were **NOT RUN**. The `/LOG` comparison array is empty, so no conclusion about `/LOG` reliability is claimed.

## Cleanup and evidence

Cleanup verified the synthetic Registry key was absent/removed and the owned task temp directory was removed. The uploaded artifact contains 28 files; all 27 entries listed in its internal `SHA256SUMS` were independently verified.

- Artifact: [issue-380-s1r2-nsis-launcher-38245775766-1](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38245775766/artifacts/11746355270)
- Artifact ID: `11746355270`; size 2,606,566 bytes.
- GitHub-reported and local artifact ZIP SHA-256: `a133db5c944ecf43966af16d8c37734a188f768d879652a958c8752849b662d6`.
- Actions log archive SHA-256: `0b9400e411d6977cfea85c656001e308667f7b4ebddb2e5a9cc9fd7219553cfc`.

Evidence is committed under [the run evidence directory](evidence/issue-380-s1r2/run-38245775766/): raw artifact ZIP and Actions log ZIP, extracted result/acquisition/preflight/cleanup metadata, artifact metadata, and SHA-256 manifests. The readable extracted text copies use LF endings; the original unmodified artifact bytes remain in the ZIP, with its original internal manifest. The local artifact matched GitHub's digest. No user data, product Registry, Service, IPC, database, or product installer was used.

## Remaining unknowns

- Why the generated uninstaller exits 0 after the frozen Probe's `un.onInit` mismatch path: **UNKNOWN**.
- `/LOG` behavior and direct file trace persistence: **UNVERIFIED**.
- The file-trace mode and all four compile outputs were not reached.
- No product root cause is inferred. No second Hosted run is authorized; no rerun occurred.

`#380-S1R2 OBSERVABILITY_BLOCKED`
