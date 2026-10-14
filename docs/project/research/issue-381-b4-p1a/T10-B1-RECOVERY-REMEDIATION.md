# Issue #413 T10-B1 Recovery Model Remediation

**Status: `T10_B1_RECOVERY_MODEL_REMEDIATION_READY_FOR_OWNER_REREVIEW`**

This is a new remediation record for Owner review comment [#6165110762](https://github.com/ArdenZC/Zen-Canvas/issues/413#issuecomment-6165110762). The original [T10-B1 fault matrix report](T10-B1-FAULT-MATRIX.md), matrix `attempt-008`, and the original `evidence/t10-b1/SHA256SUMS` remain historical evidence and were not edited.

## Identity and scope

- Branch: `test/issue-381-b4-p1a-t10-b1-fault-model`
- Starting HEAD: `e596a7bb03c24e5dd6dc1b1f6be3c8e7a205b8c0`
- Starting Tree: `698f5fbb96b8353577a2ff2d0a03d189eb4366cb`
- Parent: `b9c44a0a05fc8dc34744563b43d88eeab7509b35`
- Implementation change: `tests/issue381_b4_p1a_f1/src/b1_vfs.rs`
- New evidence: `evidence/t10-b1/remediation-r1-r3/attempt-001/`

No production code, schema, CI, workflow, or lock file was changed. The B1 Manifest and recovery/publish decisions added here are test-only fixtures and functions; they are not the product reader or product publication implementation.

## R1 — B05 replay, operation ID, epoch CAS, and facts

`apply_publish_operation` now applies pointer updates inside an SQLite `IMMEDIATE` transaction using an explicit expected-epoch compare-and-swap and a unique operation ID. The test calls that function a second time with the same operation after a successful SQLite commit.

- First call returned `APPLIED`; SQLite commit returned primary and extended code `0` (`SQLITE_OK`). The simulated client response was dropped only after commit returned success.
- The actual replay returned `ALREADY_APPLIED`. Pointer state and operation rows matched the post-commit snapshot, with exactly one row for the operation ID.
- Reusing that ID with a different target returned `REJECTED_OPERATION_ID_REUSE`; a different operation with stale expected epoch `0` returned `REJECTED_EPOCH_CAS`. Neither negative call changed the snapshot.
- The two `user_facts` rows were identical before and after replay and the negative controls.
- A separate case committed successfully and then restored a captured pre-commit SQLite file as the model's restart-visible snapshot. That model returned pointer E0 and zero operation rows even though the caller had observed `SQLITE_OK` before the simulated restart.

The latter is a deterministic snapshot-selection model, not a process reboot, real power loss, or evidence that SQLite loses a successfully reported commit. It is kept distinct from “commit succeeded but response was lost”; no physical durability or cross-file ACID guarantee is claimed.

## R2 — B06 executable generation recovery decisions

The new callable `resolve_generation` reads the authority pointer through the test VFS in read-only mode. It selects only the generation named by that row and checks its epoch, expected SQLite digest, pointer-bound Manifest digest, Manifest generation/slot/source/revision, SQLite source ID, file length, file SHA-256, and read-only `PRAGMA integrity_check`. It returns `VALIDATED_GENERATION` or `UNAVAILABLE` with observed inputs and rejection reasons. It performs no repair, pointer update, or generation substitution.

| Case | Resolver result | Observed reason / assertion |
| --- | --- | --- |
| Valid E1 | `VALIDATED_GENERATION`, E1 | Manifest, pointer, digest, source/revision, and SQLite integrity matched |
| Missing E1 | `UNAVAILABLE`, E1 | `SLOT_FILE_MISSING`; E0 was not substituted |
| Truncated E1 | `UNAVAILABLE`, E1 | Disk digest/length mismatch and SQLite integrity rejection |
| Valid SQLite with E1 digest mismatch | `UNAVAILABLE`, E1 | `DISK_DIGEST_MISMATCH`; SQLite integrity remained `ok` |
| Invalid E0 while authority selects E0 | `UNAVAILABLE`, E0 | Disk digest/length mismatch and SQLite integrity rejection; valid E1 was not substituted |

For each case, before/after snapshots show that the resolver did not mutate the authority pointer, generation files, or Manifests.

## R3 — B01/B03 publication admission

`publication_admission` reads the pointer row and requires an exact expected-parent match plus the next epoch/generation. It validates the candidate's actual Manifest bytes and SHA-256 against the staged publication expectation, then checks candidate file digest, length, source/revision, and read-only SQLite integrity. B01 and B03 use its returned decision; their publication fields are computed from that decision and the observed resolver/pointer state.

- **B01:** actual named-VFS main-file `xWrite` wrote 256 of 4096 requested bytes and returned `SQLITE_IOERR_WRITE=778`; COMMIT reported extended code 778. The queried pointer remained E0 at epoch 0, the resolver validated E0, and candidate admission returned `REJECTED` with `MANIFEST_MISSING`. `candidate_publish_allowed` and `candidate_was_published` were derived from these results.
- **B03:** the staging file contained 144 of 288 Manifest bytes. Admission on that actual staged file returned `REJECTED` with `MANIFEST_DIGEST_MISMATCH` and `MANIFEST_INVALID_JSON`; the queried pointer remained E0. A complete valid Manifest for the same candidate passed the positive control and returned `ADMITTED`. The candidate fields were derived from the guard and resolver results.

## Executions and evidence

Only the requested isolated B1 runs were invoked, with locked offline dependencies:

```text
CARGO_HOME=/workspace/.cache/cargo RUSTC=/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc cargo run --locked --offline --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml -- --b1-f0
CARGO_HOME=/workspace/.cache/cargo RUSTC=/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc cargo run --locked --offline --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml -- --b1-matrix
```

Both exit codes were `0`. The F0 log contains 822 JSONL records. The matrix log contains 7,354 JSONL records: B01/B02/B03/B05/B06/B07/B08/B09/B12 passed; B04/B10/B11 remain `ENVIRONMENT_UNAVAILABLE`; no executable matrix case failed. The matrix runner's terminal record keeps its earlier generic `T10_B1_FAULT_MODEL_READY_FOR_OWNER_REVIEW` label; this remediation's review status is the token at the top of this report.

F0 confirmed actual named-VFS callback interception and SQLite results:

- `xWrite` returned `SQLITE_FULL=13`; COMMIT returned extended code 13. Fresh default-VFS reopen passed SQLite and FTS5 integrity checks with the prior 16 rows intact.
- `xSync` returned `SQLITE_IOERR_FSYNC=1034`; COMMIT returned extended code 1034. Fresh default-VFS reopen passed SQLite and FTS5 integrity checks with the prior 16 rows intact.
- `xRead` returned `SQLITE_IOERR_SHORT_READ=522` after reading a 2048-byte prefix of a 4096-byte request. The callback verified zero-fill of the remaining 2048 bytes. SQLite then reported `SQLITE_CORRUPT=11` to the statement. Fresh default-VFS reopen passed SQLite and FTS5 integrity checks with the prior 16 rows intact.

The VFS returned synthetic SQLite errors. `SQLITE_FULL` here does not establish real media exhaustion; neither this run nor the B05 snapshot model tests physical power loss.

Raw stdout/stderr and exact exit-code files are retained in `evidence/t10-b1/remediation-r1-r3/attempt-001/`. Its independent `SHA256SUMS` covers the new source, report, raw logs, run metadata, and selected historic evidence. The original `evidence/t10-b1/SHA256SUMS` had SHA-256 `8008886239432ad26af702857417fdde9854f9669600b6531b8b9578d526ecfd` before remediation and remains unchanged; the new pre-run receipt also records the historic F0 attempt-005 and matrix attempt-008 hashes.

## Remaining limits

B04 Windows OS error 5/native rename behavior, B10 real bounded `ENOSPC`, and B11 native platform sharing/permission contention remain unavailable in this Linux environment. The VFS wrapper is Linux/Unix-specific. The B05 restart-lost-commit scenario is an explicit model that restores an old snapshot. These results qualify only the isolated test model for Owner re-review; they do not qualify real power-loss recovery, full T10/native readiness, P1A acceptance, a product fix, or P1B.
