# T10-A hosted native research

## Source and execution boundary

- Branch: `test/issue-381-b4-p1a-native-t10`
- Frozen start: HEAD `d5ba2b07b7f6c1be6dce5f58b67718bb5b7ef84f`, Tree `ba664783eb69c388c5846cd1e05ad76a315da8d4`
- Hosted workflow: `.github/workflows/issue-381-b4-p1a-native-t10.yml`
- Trigger: pushes to this branch that change the isolated harness or this workflow; matrix: `windows-latest` and `macos-latest`
- Permissions: `contents: read`; checkout and artifact actions use the repository's pinned immutable SHAs
- Build: `cargo build --release --locked --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml`
- Entry point: `issue381-b4-p1a-f1 --t10-native`; the legacy all-suite runner is not invoked
- All fixtures and Cargo build output are rooted under each hosted runner's temporary directory

## Evidence rules

The executable emits eight per-platform rows (`T10-01` through `T10-08`) with one of `PASS`, `EXPECTED_REJECTION`, `FAIL`, or `ENVIRONMENT_UNAVAILABLE`. It records the exact pushed HEAD and Git Tree, OS build, filesystem type, architecture, Rust target and versions, SQLite `sqlite_source_id()`, Cargo lock parity, raw filesystem outcomes and OS error codes, child exit/restart observations, manifest and slot digests, and cleanup results.

Windows directory-sync errors are preserved with their `ErrorKind` and raw OS code. The T10-only sealer may continue using disposable fixtures only after the SQLite file and manifest files have passed real `sync_all`; a failed directory sync remains `ENVIRONMENT_UNAVAILABLE` and never counts as persistence success. Linux execution, child-process termination, untested disk-full behavior, and unsupported directory sync are not power-loss or native persistence proof.

Raw per-platform stdout, stderr, exit codes, environment records, per-case JSONL and SHA-256 manifests are uploaded as GitHub Actions artifacts even when build or test steps fail. No user files are opened, scanned, indexed or logged.

## Hosted execution record

Research result: `T10_A_HOSTED_NATIVE_RESEARCH_READY_FOR_OWNER_REVIEW`. The final exact workflow/source head ran on both hosted platforms and has retained raw artifacts. Its harness summary is `T10_NATIVE_GATE_BLOCKED`: this research does not establish power-loss durability, Windows directory-entry durability, or disk-full behavior.

### Final execution

- Run: [38523556768](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38523556768), attempt 1, push event, GitHub conclusion `success`.
- Tested HEAD: `dc5fc6cba15f4a5c2a648970bb8641f2ccdea6ba`; Tree: `612d8003aff86974a8f4c24f57df32c77713cd27`; base: `d5ba2b07b7f6c1be6dce5f58b67718bb5b7ef84f`.
- Both platform builds, T10 execution, hashing, and artifact uploads completed successfully. Both JSONL summaries report `T10_NATIVE_GATE_BLOCKED`, with no failed cases and `power_loss_durability_proven=false`.

| Hosted platform | Environment | Artifact archive |
| --- | --- | --- |
| Windows | Windows NT 10.0.26100.0, NTFS, `win25-vs2026/20260925.250.1`, x86_64, `x86_64-pc-windows-msvc`, rustc 1.97.1, cargo 1.97.1 | ID `11875151791`, SHA-256 `cc908ee8926af9e2b35db98ff8c178486c32d138bd8ff26e7bb035feeb742188` |
| macOS | macOS 26.6.2, APFS, `macos26/20260907.0351.1`, arm64, `aarch64-apple-darwin`, rustc 1.97.1, cargo 1.97.1 | ID `11875306430`, SHA-256 `6e4ae256e49855a933e497bc001de09709c71976ce5456abbadb28e69855e1e3` |

Final per-case result:

| Case | Windows | macOS |
| --- | --- | --- |
| T10-01 source, toolchain, SQLite, and lock parity | PASS | PASS |
| T10-02 SQLite seal, read-only reopen, WAL checkpoint | ENVIRONMENT_UNAVAILABLE: both directory-sync calls returned `PermissionDenied`, OS error 5; file sync and seal checks passed; WAL sidecar was 0 bytes and a 32 KiB `-shm` file remained after close | PASS: file and directory sync passed; WAL sidecar was 0 bytes after close. A 32 KiB `-shm` file remained and is recorded as an observation, not WAL content |
| T10-03 live Pin versus GC, real read-only handle, rename/replace/delete | PASS: rename and delete denied with OS error 32; replace denied with error 5 while the copied SQLite file had a live reader | PASS: rename, replace, and delete succeeded on the disposable copy while its read-only handle remained queryable |
| T10-04 GC-first versus late Pin | EXPECTED_REJECTION: the lock barrier let GC commit first; late Pin observed `GC_ELIGIBLE` and was rejected | EXPECTED_REJECTION: same ordering and rejection |
| T10-05 Lease recovery after child process termination | PASS: exact PID/generation/nonce reclaim after kill and wait; this is process-exit evidence only | PASS: exact PID/generation/nonce reclaim after kill and wait; this is process-exit evidence only |
| T10-06 manifest tamper, source/root mismatch, pointer restart boundaries | PASS | PASS |
| T10-07 permission, directory sync, disk-full constraints | ENVIRONMENT_UNAVAILABLE: permission denial observed with OS error 5; directory sync returned error 5; no bounded disposable quota was available for disk-full testing | ENVIRONMENT_UNAVAILABLE: permission denial observed with OS error 13; directory sync passed; no bounded disposable quota was available for disk-full testing |
| T10-08 connection count and busy timeout | PASS | PASS |

The final run used bundled SQLite 3.51.3 (`rusqlite` 0.39.0, `libsqlite3-sys` 0.37.0); all 69 harness registry package versions and checksums matched the 588-package product lock. The source SHA matched the push event on both runners. All fixtures, Cargo targets, and captured evidence used runner temporary storage.

### Earlier push attempts and failure record

| Run | Tested HEAD / Tree | Outcome and exact failure | Raw artifact archives |
| --- | --- | --- | --- |
| [38522265914](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38522265914), attempt 1 | `18c82cd7dd8764e7bb5cb4de0d0eaf65e6ab8e55` / `7389b3d2dcfdd88c2d1430baa0e220c4c1f0a389` | Both builds exited 101 before T10 execution. Rust errors: E0603 imported private `db::file_sha256`; E0308 passed a capturing closure to `Connection::busy_handler`, which requires a function pointer. Windows was NTFS / Windows NT 10.0.26100.0 / x86_64 MSVC. macOS 26.6.2 / arm64 used the same Rust/Cargo versions; filesystem detection incorrectly returned `unreported` on this attempt. | Windows ID `11874450950`, SHA-256 `df7502ca2bcb886e5f3df3ff1630a911f6e6dc01302cbd32887d2cb7a10b6fa0`; macOS ID `11874715126`, SHA-256 `2e2791b07fda1ef666a39edfd78b7dfd9585f458311222e237978d684393a1bc` |
| [38522699893](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38522699893), attempt 1 | `40e3418abf08f8d5e66b74c7714622be432beb21` / `15abc95809c651a78701760d01c5a9286aa53f8e` | Both builds succeeded. Windows T10-02 through T10-07 failed with I/O `PermissionDenied`, OS error 5: the harness called `sync_all` on read-only file handles. macOS T10-02 treated a nonzero `-shm` sidecar as seal failure; T10-04 compared a serialized `(slot,generation)` array against a string. Both code assertions were corrected. T10-07 reported disk-full as unavailable because there was no safe bounded test volume. | Windows ID `11873699507`, SHA-256 `e1432240495b070705d8ca7b4c2fdde811773376269061f0c1c1a4a356e3e09d`; macOS ID `11874540965`, SHA-256 `2778909519ec611d3a1a6adf7c7ec7a28058fd038ec7b2759dd674d6016900bd` |
| [38523022039](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38523022039), attempt 1 | `45df2d0b17d2e2c9b6eb3f975bbf186a0163ee5a` / `bd735059743fc192cd747a8df55d33b53cc6731a` | Both builds succeeded and the native probes ran. T10-02 still failed on both OSes solely because the harness required the 32 KiB `-shm` sidecar to be absent after close, despite a completed truncate checkpoint and 0-byte WAL. The check was corrected to retain the sidecar observation and gate on WAL/checkpoint/seal evidence. Windows/macOS other probes completed with the same platform-specific file-operation results as the final run. | Windows ID `11875125771`, SHA-256 `e4c9b9c5f3a4caba7e347ef3830385dbad5982e56909433c14ac7bc707621d93`; macOS ID `11874991202`, SHA-256 `0d9b48a7937922e0b797ccdad1f129400ef82f20d8b660b135dbd48ea73f22d0` |
| [38523207023](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38523207023), attempt 1 | `cfe9b7b49f90da71d7aca38c14b43da84e13b3fd` / `c4f5e248443a2ec72a3773a83103629124f3b7f8` | Both platform jobs succeeded with no failed cases. The harness summary was `T10_NATIVE_GATE_BLOCKED`: Windows directory sync returned OS error 5; disk-full remained unavailable on both platforms. This run used the same T10 source as the final run; the later workflow-only change restricted push paths to the harness/workflow. | Windows ID `11875091273`, SHA-256 `647a7d6fb22acd1c14b40eb690aaff1f7d9e3309e60d6532ce14249b24a472ff`; macOS ID `11875061200`, SHA-256 `41ebe4e2bfe28fa22b5e719a53e7c3e1829ae2389090f7be4454a737eb519636` |

The evidence-index commit that adds this execution record changes only this document; the final run above is bound to the exact tested source/workflow HEAD and Tree shown above.

### Acceptance boundary

The hosted native research ran on Windows/NTFS and macOS/APFS; Linux was not used as native evidence. Process termination and pointer reopen barriers do not model power loss. Windows directory sync returned access denied and is classified `ENVIRONMENT_UNAVAILABLE`; disk-full injection was not attempted without a disposable quota. The final Owner review can assess the captured OS results, but no case or summary promotes those gaps into native persistence success.
