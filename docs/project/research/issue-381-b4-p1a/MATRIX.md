# B4-P1A gate matrix

All results below come from the committed isolated harness, not from the application database. `EXPECTED_REJECTION` is the intended pass condition for negative designs. The complete machine-readable record, with details for every gate, is [evidence/final/test-matrix.json](evidence/final/test-matrix.json).

| Gate | Verification | Observed | Evidence |
| --- | --- | --- | --- |
| T01 | Bundled SQLite, FTS5, trigram capability, override variables | PASS — SQLite 3.51.3; FTS5/trigram available; no overrides | `matrix-native` source-info record; Cargo feature tree |
| T02 | Synthetic fixture sizes, UID deduplication and effective root claims | PASS — 0, 10k, 100k | Native JSONL snapshot records |
| T03 | Trigram/legacy short-query distinction, escaped wildcard behavior, and ranked snapshots | PASS — short `fi`: trigram 0 vs literal legacy 1,800; wildcard `_ %`: 0; Oracle 12/12 exact | `matrix-oracle-checks.jsonl`; compressed native JSONL |
| T04 | E0 held/fresh reader and E1 candidate across pointer publication | PASS — held E0 row and exact rank bits stable; current E1 sees changed row; old cursor rejected | Native JSONL case record |
| T05 | Pin/GC barrier, open failure cleanup, late pin rejection, lease expiry | PASS — 12/12 race probes observed the live lease | Native JSONL case record |
| T06 | Pool and contention match product settings | PASS — pool 8, WAL, synchronous NORMAL, busy timeout 5,000 ms; cancel writer received SQLite 5 after 5,007.3 ms, zero partial rows | Native JSONL case record; all 800 raw lock wait samples retained |
| T07 | Sealed DB hash/manifest, WAL checkpoint, integrity and read-only behavior | PASS — file digest matched, integrity `ok`, writes rejected, sealed WAL 0 | Native JSONL case record |
| T08 | Child kill at build, close, sync, seal, pointer pre/post commit, GC pre/post unlink | PASS — 8/8 recovery expectations, candidate checks, and user-fact retention | Native JSONL barrier results |
| T09 | Injected capacity failure and interrupted backup | PASS — unchanged authority/user facts; old slot retained; partial manifest absent | Native JSONL case record |
| T10 | Native Windows/macOS real-handle behavior | ENVIRONMENT_UNAVAILABLE — Linux executor; gate remains blocking | Native JSONL case record; unchanged CI is not native evidence for this harness |
| T11 | Missing/corrupt pointer target handling | PASS — read-only reconciliation, no mutable-FTS fallback, no destructive repair | Native JSONL case record |
| T12 | Build/copy/delta/read/GC/lock/disk/WAL/RSS observations | PASS — 100 repeats for each of 0, 10k, 100k; 300 raw sample records | `matrix-native.stdout.jsonl.gz`; percentile table in [REPORT.md](REPORT.md) |
| N01 | Reject F0 BM25 across mutable physical corpus and post-limit filtering | EXPECTED_REJECTION — rank mismatch and underfilled post-limit page observed | Native JSONL case record |
| N02 | Reject unguarded F2/current mutable physical FTS | EXPECTED_REJECTION — E1 revision guard rejects; immutable F1 E0 row remains | Native JSONL case record |
| N03 | Reject path-only user-fact inheritance on same-path/new-UID reuse | EXPECTED_REJECTION — no fact inheritance or new-UID action authority | Native JSONL case record |
| N04 | Reject cursor tampering/replay, stale CAS, manifest replay, slot reuse, double GC | EXPECTED_REJECTION — all invalid transitions rejected | Native JSONL case record |

## Oracle details

The independent Python Oracle does not import or invoke SQLite. It regenerates the fixture strings, computes trigram-token document lengths and the FTS5 BM25 formula, applies the contract weights `6.0/1.5` and tie ordering, then compares every UID/name/path/mtime and exact score bits. The preserved summary is:

```text
query_snapshots_checked=12 passed=12 failed=0 exact_float_bits=true
```

## Overall state

All declared expectations were met, including the explicit unavailable status for T10. Because T10 is a required native gate, the result is `P1A_BLOCKED_ENVIRONMENT`, not ready.
