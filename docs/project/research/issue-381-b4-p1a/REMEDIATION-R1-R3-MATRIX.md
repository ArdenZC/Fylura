# B4-P1A remediation matrix

`evidence/remediation-r1-r3-v1/test-matrix.json` is the machine-readable source of truth for the run. Its 16 prior gates and 11 R1–R3 expectations are satisfied; the 19 independent Oracle comparisons passed. A negative-path case is classified `EXPECTED_REJECTION` when its invalid operation is rejected with the recorded SQLite/file state.

## R1 — query pagination

| Check | Evidence / result |
| --- | --- |
| Original `query_rows(None, Some(1))` | Reproduced SQLite bind failure: 2 supplied parameters, 3 required; raw probe and receipt preserved. |
| Root/no-root × limit/no-limit | 4 combinations; real result counts are recorded in the machine matrix. |
| Real LIMIT/OFFSET | SQL paging exercised for first, continuation and boundary pages, plus an empty page. |
| Root before LIMIT | 7-row page returned after scope; global-LIMIT-then-scope adversary underfills at 4 rows. |
| BM25 contract | Global effective-visible statistics retained; weights `[6.0, 1.5]`. |
| Oracle | 12 existing full queries + 7 pages = 19/19; full IDs/order and score float bits match. |
| Cursor stale case | Test model rejects stale/forged fields. Product Cursor is not implemented by this harness; no product API claim. |

## R2 — Pin/GC and lease lifecycle

| Case | Result | Required state evidence |
| --- | --- | --- |
| Pin-first, then GC request | PASS | Lease row visible; GC returns `LIVE_PIN`; file remains until release and is then removed. |
| GC-first, then late pin | EXPECTED_REJECTION | Retirement/reclaim transaction owns the barrier; no late lease row; reclaimed file absent. |
| Register, retire before open | PASS | Committed registration and retirement rows; lease protects read-only handle and slot. |
| Real read-only handle during GC | PASS | GC blocked while real transaction/lease exists; release permits file removal. |
| Open fails after registration | EXPECTED_REJECTION | Injected open error; release changes active lease count; file removed by GC. |
| Lease expires | PASS | Persisted lease inactive after expiry; GC removes the file. |
| Process exits, DB reopened | PASS | Child exit observed; test-only recovery clears lease; subsequent GC removes the file. |
| Slot ID reuse / old lease replay | EXPECTED_REJECTION | New generation file exists; old generation is removed; stale replay rejected by generation fencing. |

All race ordering uses explicit `std::sync::mpsc` channels; no sleep is used to schedule the opposite race order. The old 12 Pin-first results remain historical. Eight distinct cases replace them. Pool max remains 8 and SQLite BUSY timeout remains 5,000 ms. `BEGIN IMMEDIATE` is test-only and is not a product approval.

## R3 — real manifest validation

| Invalid or replayed manifest | Result | Authority / old slot / user facts |
| --- | --- | --- |
| Wrong Slot ID | EXPECTED_REJECTION | Unchanged |
| Wrong SQLite file hash | EXPECTED_REJECTION | Unchanged |
| Wrong file length | EXPECTED_REJECTION | Unchanged |
| Wrong Root Revision | EXPECTED_REJECTION | Unchanged |
| Wrong Source Revision | EXPECTED_REJECTION | Unchanged |
| Wrong corpus digest | EXPECTED_REJECTION | Unchanged |
| Old manifest format missing version bindings | EXPECTED_REJECTION | Unchanged |
| Old slot manifest replayed against new slot | EXPECTED_REJECTION | Unchanged |

Each case uses an actual manifest file in a temporary directory and the harness's complete verifier, which hashes and opens the actual SQLite file read-only and checks integrity, count, source identity, slot identity and revision bindings. This does not claim that production has a verifier.

## Historical and current sample accounting

| Sample type | Before remediation | After remediation | Delta |
| --- | ---: | ---: | ---: |
| Oracle result snapshots | 12 | 19 | +7 page snapshots |
| Pin/GC case records | 12 repeated Pin-first attempts | 8 named synchronized cases | −4 records, with broader distinct-order coverage |
| SQLite lock-wait samples | 800 | 800 | 0 |
| Performance samples | 300 | 300 | 0 |

The original evidence and original `P1A_BLOCKED_ENVIRONMENT` outcome remain unchanged. Historical `evidence/final/MANIFEST.sha256` remains byte-identical. The original source snapshot and re-based 86-entry verification manifest are under `evidence/previous-pr408-source/`.

## Blocking platform gate

`P1A-T10 = ENVIRONMENT_UNAVAILABLE`. The run took place on Linux/overlayfs. It does not establish hosted Windows/macOS handle sharing, rename/unlink behavior, filesystem durability, or power-loss recovery. Overall state remains:

```text
P1A_HARNESS_REMEDIATION_COMPLETE
P1A_BLOCKED_ENVIRONMENT
READY_FOR_OWNER_REREVIEW
```
