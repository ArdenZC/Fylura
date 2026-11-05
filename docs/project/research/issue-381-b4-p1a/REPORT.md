# B4-P1A test report

## Decision

**`P1A_BLOCKED_ENVIRONMENT` — not ready.** The isolated harness met all 16 declared matrix expectations: 11 positive gates ran and passed, T10 returned the required `ENVIRONMENT_UNAVAILABLE`, and all four negative designs were rejected. The 12 independent query snapshots matched every ordered row and exact floating-point score bit. T12 recorded 100 samples at each of 0, 10k, and 100k generated documents.

T10 is still a blocking gate. The executor is Linux on `overlayfs`; this run cannot establish Windows/macOS real-handle sharing, unlink, replacement, or cleanup behavior. No native run was inferred from Linux results, and the draft remains blocked pending those runs.

## Scope and provenance

The source harness is bound to commit `fd5fec9595baca1c62ae781ac5cf5e2d67d117f8` and tree `05168efc9c05f614a5a33defd98632179b4337ae`, based directly on Schema 38 `master` commit `ab3915e3cdcc7c729317498b3b97501862832a91` and tree `ca22989646575bba9a7224ce10becbb89c01e0ee`.

All test databases use synthetic data and disposable temporary directories. The separate Cargo lock contains 69 registry packages; every package version and checksum matches `src-tauri/Cargo.lock`. The bundled runtime reported SQLite 3.51.3, FTS5 enabled, trigram query support, and no SQLite override environment variables.

The current product schema and its `library_generation_authority=('legacy', 0, NULL, 1)` sentinel were left untouched. This test-only prototype does not claim application-level or native-platform integration.

## Gate outcome

| Group | Result |
| --- | --- |
| T01–T09, T11–T12 | PASS |
| T10 native Windows/macOS handles | ENVIRONMENT_UNAVAILABLE; blocks readiness |
| N01–N04 negative designs | EXPECTED_REJECTION |
| Independent query Oracle | 12/12 exact ordered-row and float-bit matches |
| T12 performance | 100 samples per size; 300 raw sample rows retained |
| Overall | `P1A_BLOCKED_ENVIRONMENT` |

See [MATRIX.md](MATRIX.md) and the unabridged [test-matrix.json](evidence/final/test-matrix.json) for case-level evidence.

## Performance observations

Times are milliseconds; file size is MiB. Values are fixture-only Linux `overlayfs` measurements using nearest-rank percentiles over 100 independent temporary databases per size. “Delta” means copying the sealed slot and then updating three synthetic rows, including FTS trigger work. “Read” is an unfiltered high-hit `common` query. These observations do not predict production hosts or larger corpora.

| Generated docs | Effective docs | Build p50 / p95 / p99 | Delta p50 / p95 / p99 | High-hit read p50 / p95 / p99 | GC p50 / p95 / p99 | Slot file p50 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0.472 / 0.645 / 0.823 | 0.00004 / 0.00005 / 0.00005 | 0.127 / 0.175 / 0.217 | 0.051 / 0.079 / 0.097 | 0.04 MiB |
| 10,000 | 9,000 | 304.609 / 326.624 / 335.831 | 100.099 / 106.978 / 108.668 | 11.839 / 12.911 / 13.915 | 0.211 / 0.295 / 0.329 | 5.14 MiB |
| 100,000 | 90,000 | 3,432.375 / 3,704.196 / 3,923.964 | 979.709 / 1,076.521 / 1,094.938 | 133.554 / 148.190 / 158.127 | 1.739 / 2.198 / 2.510 | 44.81 MiB |

At 100k, copy p50/p95/p99 was 10.704 / 12.216 / 15.091 ms; authority-lock transaction was 0.012 / 0.015 / 0.046 ms. Sealed WAL size was zero in all 300 samples. Process high-water RSS for the complete run was 181,636 KiB. The low sync times are specific to this overlay filesystem and are not durability evidence.

## Durability and concurrency limits

- T05 exercised 12 pin/GC interleavings, held-reader protection, lease release after open failure, expiry recovery, and rejection of a late pin after GC. Its `BEGIN IMMEDIATE` prototype is test-only; it is not a product safety approval.
- T06 kept the product-equivalent pool size at 8 and busy timeout at 5,000 ms. The contending writer returned SQLite error 5 after 5,007.3 ms without a partial write. All 800 competing-writer wait durations are retained in the native JSONL record; their p50/p95/p99 was 1.111 / 8.344 / 18.485 ms. They do not justify taking a write lock per read request.
- T08 killed Linux child processes at eight build/seal/pointer/GC barriers and checked SQLite recovery, candidate integrity, manifest hashes, the authority pointer, and preserved user facts. Process kill is not sudden power loss.
- T09 used an injected capacity error and a truncated backup. No physical disk-full, storage-controller, filesystem crash, or power-loss test was run.
- T11 routed missing and corrupt current slots to `READ_ONLY_RECONCILIATION`, with no mutable-FTS fallback and no destructive repair.

## Evidence index

The exact command outputs, test stream hashes, Cargo feature tree, dependency-lock parity, query Oracle checks, and benchmark rows are under [evidence/final/](evidence/final/). The two pre-commit `--samples 0` development runs are retained separately under [evidence/development/](evidence/development/); their expected T12 failure is not part of the formal result.
