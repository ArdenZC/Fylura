# Issue #413 T10-B1 R4 Fail-Closed Remediation

**Status: `T10_B1_R4_FAIL_CLOSED_REMEDIATION_READY_FOR_OWNER_REREVIEW`**

This record addresses Owner review comment [#6166755250](https://github.com/ArdenZC/Zen-Canvas/issues/413#issuecomment-6166755250). It is limited to the isolated T10-B1 test harness and research evidence. The R1–R3 remediation report, attempt-008 matrix evidence, and prior checksum files remain in place.

## Identity and scope

- Branch: `test/issue-381-b4-p1a-t10-b1-fault-model`
- Starting HEAD: `76f00475899fea1b037c5975130d28913b77f741`
- Starting Tree: `4611e23b0dacae3971e6fe73ed3a37cd31535c00`
- Parent: `e596a7bb03c24e5dd6dc1b1f6be3c8e7a205b8c0`
- Test-only implementation: `tests/issue381_b4_p1a_f1/src/b1_vfs.rs`
- New evidence: `evidence/t10-b1/remediation-r4/attempt-001/` and `attempt-002/`

No production code, schema, CI, workflow, or lock file was changed.

## R4 — reject an existing empty Manifest

`validate_fixture_manifest()` now distinguishes a successful read of an empty file from a missing or unreadable Manifest. For a successful zero-byte read it records the actual byte count and SHA-256, adds `MANIFEST_EMPTY`, and does not parse the file. This remains invalid even if the authority digest equals SHA-256 of the empty byte string.

The final B03 test creates an actual zero-byte candidate Manifest and invokes the executable `publication_admission()` guard with the matching empty-file SHA-256 (`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`). The actual result is `REJECTED`; `MANIFEST_EMPTY` is the sole reason and the digest check reports a match. The valid Manifest control remains `ADMITTED`, and the existing truncated staging Manifest remains `REJECTED`.

The final B06 test creates an actual zero-byte E1 Manifest and a queried Authority pointer whose Manifest digest also matches that empty file. `resolve_generation()` returns `UNAVAILABLE`, selects E1, and reports only `MANIFEST_EMPTY`. It neither substitutes E0 nor rewrites the pointer. The valid E1 control still returns `VALIDATED_GENERATION`; missing E1, truncated E1, disk digest mismatch, and invalid E0 negative cases remain.

For both new empty-file cases, pre/post snapshots compare the queried Authority, E0, candidate or selected slot, user facts, and Manifest bytes/digest. The snapshots are unchanged after rejection. The B06 snapshot also checks no automatic generation substitution or pointer rewrite.

## Executions

The final code was run with only the requested isolated commands, locked and offline:

```text
CARGO_HOME=/workspace/.cache/cargo RUSTC=/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc /workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/cargo run --locked --offline --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml -- --b1-f0
CARGO_HOME=/workspace/.cache/cargo RUSTC=/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc /workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/cargo run --locked --offline --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml -- --b1-matrix
```

Final attempt `attempt-002` exited `0` for both commands. F0 emitted 822 JSONL records and passed. The actual named VFS returned `SQLITE_FULL=13` from `xWrite` and SQLite returned 13; returned `SQLITE_IOERR_FSYNC=1034` from `xSync` and SQLite returned 1034; and returned `SQLITE_IOERR_SHORT_READ=522` from `xRead` after a 2048-byte prefix of a 4096-byte request. The callback verified zero filling of the remaining 2048 bytes; SQLite then returned `SQLITE_CORRUPT=11` for the query. Fresh default-VFS reopen checks reported SQLite integrity `ok`, FTS5 integrity `PASS`, and all 16 baseline rows for each fault case.

The B01–B12 matrix emitted 8,087 JSONL records and 32 case-result records: 26 `PASS`, 3 `EXPECTED_REJECTION`, 3 `ENVIRONMENT_UNAVAILABLE`, and no failed executable cases. B04, B10, and B11 remain environment unavailable; the native gate remains `BLOCKED_ENVIRONMENT`. Raw stdout, stderr, and exit-code files are retained with the result summary and run metadata in `attempt-002/`. `attempt-001/` preserves the first R4 run before tightening the fixtures to use the matching empty-file digest; the final owner-review result is `attempt-002/`.

These runs exercise synthetic SQLite VFS errors and a deterministic recovery model. They do not test real media exhaustion or physical power loss.

## Historical evidence and checksums

- R1–R3 attempt-001 `SHA256SUMS` was left byte-for-byte unchanged; its SHA-256 remains `42be8b09518a7caa9ae851c43e1c0d85ce3753ffbd536ae80bf42683032474ab`.
- The original `evidence/t10-b1/SHA256SUMS` was left byte-for-byte unchanged; its SHA-256 remains `8008886239432ad26af702857417fdde9854f9669600b6531b8b9578d526ecfd`.
- Both prior source baselines are saved under `attempt-001/`: the original T10 source from `e596a7b` matches `1486ec999abc102f3db165abd1ba2abb6c0f01bf82a036d9eb888c227ce0a451`, and the R1–R3 source matches its recorded `412650af6333c94b50644d186d7f927a64d4ff300d1118a1a01a6660527241a7`.
- The other entries of the R1–R3 attempt checksum list and the remaining extant entries of the original T10-B1 list were verified; see `attempt-001/historical-artifacts-check.txt`.
- The old R1–R3 report, raw logs, and matrix attempt-008 files remain unchanged.

The original broad T10-B1 checksum list also names the live test source and an ignored Cargo target executable. R4 necessarily advances the source and the requested Cargo runs regenerate that ignored executable: the original expected executable digest was `064aee7d961ae70d3a462399e76d1f1c2d1a8eeab76fc6ca12f7c2c0ee21ff02`, while the current generated file has SHA-256 `e7bf6d81dcb4a3b8134a30f90ec3b5dbff32efc77bca6b89107aed8962f52e4f`. Those two historical entries therefore do not describe the current worktree. Their original manifest bytes are preserved; the original and R1–R3 source files matching their old digests are archived separately. This does not alter or replace the old evidence records.

The independent final R4 checksum manifest is `evidence/t10-b1/remediation-r4/attempt-002/SHA256SUMS`.

## Remaining limits

This is a test-only fail-closed model for Owner re-review. It does not change or prove product recovery behavior. B04 native rename behavior, B10 real bounded `ENOSPC`, and B11 native platform sharing/permission contention remain unavailable in this Linux environment. No real disk-full or physical power-loss claim is made.
