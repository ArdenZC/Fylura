# T10-B1 isolated SQLite fault model

**Review state:** `T10_B1_FAULT_MODEL_READY_FOR_OWNER_REVIEW`

**Native and real ENOSPC gates:** `BLOCKED_ENVIRONMENT`

This is the isolated test-only continuation for [Issue #413](https://github.com/ArdenZC/Zen-Canvas/issues/413), implementing the contract and the latest Owner VFS security review at [comment #6163450374](https://github.com/ArdenZC/Zen-Canvas/issues/413#issuecomment-6163450374). The source and evidence remain on `test/issue-381-b4-p1a-t10-b1-fault-model`, starting from HEAD `b9c44a0a05fc8dc34744563b43d88eeab7509b35` (Tree `2becfa34995d1872df74115adc4c7c941b353334`). The exact tested harness-source Tree is `618f77d196989163230642cd640a70a5d3bcca08`.

## Result

B1-F0 passed through an explicitly named SQLite VFS, then the runnable B01–B12 checks completed. B04, B10, and B11 remain `ENVIRONMENT_UNAVAILABLE`; the matrix therefore does **not** qualify Windows native rename/share behavior, real media exhaustion, or a general power-loss recovery guarantee. The final executable status means the isolated model is ready for Owner review, while the environment-dependent gate remains blocked.

The complete stdout JSONL (including every callback), stderr, and exit code are preserved for all five F0 attempts and all eight matrix attempts in [evidence/t10-b1](evidence/t10-b1/). Attempt 008 is the final matrix run; attempt 005 is the standalone F0 run. Earlier failures are retained as part of the record. SHA-256 entries for all raw files, final source, lock file, binary, and report are in [SHA256SUMS](evidence/t10-b1/SHA256SUMS). Run metadata is in [run-metadata.json](evidence/t10-b1/run-metadata.json).

## B1-F0 executable VFS gate

The harness registers a uniquely named wrapper around SQLite's default Unix VFS with `sqlite3_vfs_register(..., 0)` and opens each SQLite connection by that name. It does not replace the process default VFS. Per-file callbacks delegate to the parent VFS except for the selected one-shot injection. `xFetch`/`xUnfetch` are delegated; `mmap_size=0` is set on the short-read fault connection so its reads go through `xRead`. The pool probe opened eight named-VFS connections and checked WAL, synchronous NORMAL, and 5,000 ms busy timeout. Handles are closed before unregistering.

The library was bundled SQLite 3.51.3, source ID `2026-03-13 10:38:09 737ae4a34738ffa0c3ff7f9bb18df914dd1cad163f28fd6b6e114a344fe6d618`. Final standalone F0 logs: [stdout](evidence/t10-b1/f0/attempt-005/stdout.jsonl), [stderr](evidence/t10-b1/f0/attempt-005/stderr.txt), [exit code](evidence/t10-b1/f0/attempt-005/exit-code.txt) (`0`). The final matrix run repeats F0 first and records a PASS gate before executing B01.

| Fault | Actual callback and location | Callback / SQLite result | Recovery check |
| --- | --- | --- | --- |
| `SQLITE_FULL` | WAL `xWrite`, offset 0, 32 requested bytes, injected prefix 0 | VFS `13`; COMMIT extended code `13 SQLITE_FULL` | Fresh default-VFS reopen: integrity `ok`, FTS5 integrity PASS, original 16 rows, injected row absent |
| `SQLITE_IOERR_FSYNC` | WAL `xSync`, sync flags 2 | VFS and COMMIT extended code `1034 SQLITE_IOERR_FSYNC` | Fresh default-VFS reopen: integrity `ok`, FTS5 integrity PASS; baseline rows intact |
| `SQLITE_IOERR_SHORT_READ` | Main DB `xRead`, offset 4096, 4096 requested; parent read 2048, zero-filled remaining 2048 | VFS returned `522 SQLITE_IOERR_SHORT_READ`; callback verified every tail byte was zero. SQLite surfaced `11 SQLITE_CORRUPT` for the statement after parsing the zero-filled partial page. | Fresh default-VFS reopen: integrity `ok`, FTS5 integrity PASS, all 16 baseline rows intact |

This demonstrates SQLite's behavior when these return codes are delivered by the wrapper. It does not simulate a kernel or storage device reporting real ENOSPC, lost writes, torn sectors, or sudden power loss.

## B01–B12 matrix

| Case | Status | Result and evidence class |
| --- | --- | --- |
| B01 | PASS | Actual partial main-database page write: `xWrite` wrote 256 of 4096 requested bytes then returned `778 SQLITE_IOERR_WRITE`. COMMIT returned extended code 778; candidate recovered with integrity `ok`, and the pointer stayed on E0 with the same digest. `ACTUAL_OS` plus actual VFS callback; publication authority is a test model. |
| B02 | PASS | A committed WAL row preceded a checkpoint fault. WAL `xTruncate(0)` returned `1546 SQLITE_IOERR_TRUNCATE`; the checkpoint reported that error. Reopen retained the committed row (17 rows total), integrity `ok`, FTS5 PASS. WAL/SHM sidecar state and file hashes are in the raw record. Actual SQLite VFS callback plus isolated recovery check. |
| B03 | PASS | Wrote and synced a 55-byte prefix of a 110-byte staged manifest, before rename. Final manifest stayed absent and the E0 pointer was unchanged. `MODEL_ONLY`; this was a deterministic interrupted-stage fixture, not process crash or power loss. |
| B04 | ENVIRONMENT_UNAVAILABLE | Linux atomic rename and directory `sync_all` both succeeded and their result is recorded. Windows handle sharing/access-denied OS error 5 injection and close/retry semantics were not run on this Linux executor. |
| B05 | PASS | Named-VFS SQLite pointer transaction rolled back before commit to E0; committed E1 survived a simulated lost acknowledgement and idempotent replay. A stale epoch-guarded compensation changed zero rows. `MODEL_ONLY`; commit acknowledgement loss and replay are modeled. |
| B06 | PASS | Pointer remained on E1 generation 1 while E1 was deleted or truncated/corrupted. Both resolve as unavailable; E0's digest stayed unchanged; no automatic generation substitution, mutable physical FTS fallback, pointer rewrite, or repair was used. Actual Linux file deletion/truncation and SQLite/hash checks; recovery decision is `MODEL_ONLY`. |
| B07 | PASS | Re-ran the isolated P1A-T05 model: pin-first, GC-first late-pin rejection, live-handle pin, failed-open release, expiry, child exit/reopen, generation ABA, and idempotent GC. All eight reported predicates passed. `MODEL_ONLY`; it is not a production lease/concurrency guarantee. |
| B08 | PASS | Actual named-VFS injections: `SQLITE_FULL` 13 at WAL `xWrite`; partial WAL-frame write at offset 56 (128/4096 bytes) then `SQLITE_IOERR_WRITE` 778; WAL `xSync` returned `SQLITE_IOERR_FSYNC` 1034; main DB short-read returned 522 with a verified 2048-byte zero tail; and B02's WAL truncate returned 1546. Reopen and integrity/FTS checks passed as recorded. `ACTUAL_OS` VFS callbacks; no physical-media claim. |
| B09 | PASS | Re-ran deterministic child-process barriers for build, close, sync, seal, pointer pre/post commit, and GC pre/post unlink, plus missing/corrupt target reconciliation. All eight barrier recovery predicates passed. Linux process kill only; `MODEL_ONLY`, not power loss. |
| B10 | ENVIRONMENT_UNAVAILABLE | No bounded quota/mount was prepared or used. Real ENOSPC was not attempted; VFS-returned `SQLITE_FULL` is not evidence of media exhaustion. `NOT_TESTED`. |
| B11 | ENVIRONMENT_UNAVAILABLE | Executor was Linux. Native Windows sharing flags, permissions, antivirus contention, and macOS handle behavior were not tested. Existing P1A-T10 remains unavailable; no claim was added. `NOT_TESTED`. |
| B12 | PASS | 30 paired disposable WAL transaction samples, default VFS vs named wrapper, `synchronous=FULL`. Median/p95 were 0.142/0.530 ms (default) and 0.177/0.458 ms (wrapper); paired delta median/p95 0.018/0.319 ms. 60 xOpen, 150 xRead, 120 xWrite, and 120 xSync callbacks were observed. Database files totaled 491,520 bytes and additional temporary bytes after close measured approximately 0. These noisy micro-observations are not product performance or storage-overhead claims. |

The complete per-case recovery data, SQLite primary/extended codes, callback sequence, requested byte counts, actual prefix sizes, file hashes, and sidecar state are in [matrix attempt 008](evidence/t10-b1/matrix/attempt-008/stdout.jsonl). Its exit code is `0`. B04/B10/B11 remain unavailable, so the environment gate remains `BLOCKED_ENVIRONMENT`.

## Build and scope

Final run command:

```text
CARGO_HOME=/workspace/.cache/cargo
RUSTC=/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc
/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/cargo run --locked --offline --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml -- --b1-matrix
```

The final F0 command uses the same arguments with `--b1-f0`. Rust/Cargo, SQLite source ID, source hashes, binary hash, and the exact tested harness-source Tree are in the run metadata and SHA manifest. The final build exited 0; stderr contains one existing `unused_mut` warning in untouched `src/t10_native.rs`.

Only these source paths changed:

- `tests/issue381_b4_p1a_f1/src/b1_vfs.rs` — named VFS wrapper, fault injection, and isolated B1-F0/B01–B12 driver.
- `tests/issue381_b4_p1a_f1/src/main.rs` — test-only CLI entry points and reuse of existing lease/GC and process-barrier model results.
- `tests/issue381_b4_p1a_f1/src/db.rs` — the test helper now opens a directory handle when syncing a directory; this fixes the Linux `EISDIR` blocker observed in the first B09 run.

Research prose and raw evidence are under `docs/project/research/issue-381-b4-p1a/**`. No production code, schema, CI/workflow, dependency, or lock file changed. The harness Cargo lock is unchanged (SHA-256 in metadata); all other repository lock files are also unchanged. `git diff --check` passed. No PR, merge, issue close, or Codex Review was created or run.
