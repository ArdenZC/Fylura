# Global Search covering-index production candidate

Owner scope: two covering indexes only; one bounded A0/A1/A2/A3 comparison.
Latest remote master verified 2026-10-26: HEAD
`8ffc7d04a8c392e28299445b845d39e606f43f5c`, tree
`57a6d066b58d53746cf3fc0ad95709bc6d3737f3`, Schema 42.
[Baseline CI 40298939633](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40298939633)
succeeded; its routed Rust/release jobs were skipped. This is not a fresh full
platform qualification. Issue #364 and Draft research PRs #456/#457 remain
background evidence; their SQL candidates are not included.

## Production scope and migration

Schema 43 appends `volume_id` to the active Name order index and adds active
`(volume_id, last_seen_at, id)` Source Facts. The 42→43 migration explicitly
drops/recreates the Name index in the existing BEGIN IMMEDIATE migration
transaction, with user_version updated only in that transaction. Existing rows,
FTS, source/status/search SQL, COUNT(id), result mapping, BM25, filtering,
candidate window, sorting, paging, deduplication and source revision JSON/BLAKE3
remain unchanged. No extension-prefix, FTS or mtime index change.

Migration risk: index creation holds the migration write lock, scans/sorts active
entries and needs additional disk/WAL/temp space. Time and storage depend on the
installed database. DDL failure rolls back the old index and version; retry is
possible after fixing the resource failure. Schema 42 binaries reject version 43;
rolling back an installation requires restoring a compatible pre-upgrade backup,
not manually decrementing user_version. No automatic destructive downgrade.

## Bounded protocol

The existing Rust generator and its production FTS/count triggers populate one
A0 fixture per scale. After closing/checkpointing it, three byte copies are made.
Each copy changes only the two specified indexes, outside query timers:
A0 original; A1 Name only; A2 Source only; A3 both. All keep Schema 43 as a test
container; A0's two index definitions match Schema 42. No fourfold generation.

All twelve original queries, limit 80, five warmups and thirty samples. Actual
on-connection search route, actual pooled search, production Source Health
helper (including JSON/hash), actual Index Status and actual Repository Snapshot
are interleaved across variants with rotating start order each round. Each call
is timed independently; no percentile subtraction. Complete ordered objects,
BM25, source health/revision and status equal A0 on every call. Pagination at
limits/offsets 1/0, 80/40, 200/4016 and 80/4096 is compared separately. SQL-tier
measurements use current production SQL accessors and full result mapper, with
five/thirty samples per tier; they are isolated tier measurements at limit 80,
not a claim that every tier executes in every production request.

Separate real Snapshot PROFILE traces record executed SQL, VM/Sort/Fullscan.
Nested FTS statements are retained, not added to parent totals. Trace is off
for public-call latency. Source/Status EQP is collected after tracing, and every
tier has its own SQL, parameters, plan and counters. Name/Source covering use
is asserted. Extension-prefix keeps its full projection; its plan and wide-row
cost are explicitly retained. dbstat reports per-B-tree bytes. Five rollback-only
256-row insert samples retain production triggers; this is a bounded index
maintenance diagnostic, not sustained ingestion qualification. DDL time includes
drop/build and is excluded from all query latency samples.

Run 100k first; inspect correctness/performance before dispatching a single
1M Windows Hosted run. Do not repeat 1M generation, rerun full CI, relax 100ms,
merge, or run Codex Review. The natural PR CI remains authoritative for normal
required checks. Prior macOS SIGBUS #446 is historical, not assumed to explain
any new failure. Results, remaining bottlenecks and final disposition are recorded below.

## 100k preflight (Windows Hosted, debug)

[Run 40317166095](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40317166095),
source `d2bc7fdab1453fa783e455781b3c3ad5a1033317`, tree
`c85420a31d053fcaeb1fc320fa52ca3acaa24598`.
[Raw JSONL](evidence/covering-indexes/100k.jsonl.gz) includes all samples, exact
SQL/plans/counters, costs and source identity;
[all-stage percentile CSV](evidence/covering-indexes/100k-summary.csv).
12/12 correctness, full fields/BM25, pages, source revision/status equality and
both covering-plan assertions passed. Schema migration/rollback, existing
Source Health tests and the hostile multi-source/concurrent test passed.
Original pooled 100ms gates pass 12/12 for all four configurations at this scale.

Representative values in ms, p50 / p95 / p99:

| Path / query | A0 | A1 | A2 | A3 |
|---|---|---|---|---|
| Pooled name_prefix | 11.879 / 18.448 / 23.997 | 7.898 / 9.324 / 11.161 | 11.973 / 13.044 / 26.364 | 7.906 / 13.740 / 28.155 |
| Source Health (name_prefix samples) | 50.645 / 69.999 / 77.133 | 51.103 / 58.424 / 61.038 | 18.717 / 20.456 / 21.576 | 18.682 / 21.287 / 61.454 |
| Snapshot name_prefix | 70.180 / 90.490 / 107.990 | 66.325 / 72.794 / 80.389 | 38.449 / 45.130 / 66.591 | 34.244 / 59.125 / 104.032 |
| Pooled extension_prefix | 10.508 / 11.487 / 12.125 | 10.322 / 11.567 / 14.577 | 10.662 / 12.283 / 13.170 | 10.413 / 11.168 / 12.301 |
| Snapshot punctuation_prefix | 70.643 / 82.347 / 100.126 | 66.296 / 77.059 / 89.157 | 38.544 / 44.602 / 46.573 | 34.161 / 45.149 / 126.760 |

Do not describe these as tail-qualified: A3 punctuation Snapshot p99 increases
26.60%, +26.634ms; A3 name-prefix pooled p99 increases 17.33%, +4.158ms.
A3 exact-extension pooled p95/p99 rise 1.494/1.593→2.149/2.494ms (small
absolute cost, retained). No outliers are removed, no noise attribution is
proved, and no run will be repeated to obtain a green tail. The controlled 1M
run is a single further scale comparison, not a claim that this preflight meets
all acceptance requirements.

Source SQL remains COUNT(id)/MAX and O(active rows): executed work falls
1,500,077→1,200,075 VM Steps, while FullscanStep remains 99,999, Sort 0. Status
is unchanged at 300,261 VM Steps / 0 Fullscan / 1 Sort. The covering index removes
wide-table lookups; it does not remove per-request aggregation or status recount.

Natural PR CI 40317317201 exposed Schema43 compatibility omissions: the dormant
read-only File Library shadow reader admitted only 41/42; a concurrent-opener
test still expected 42; prepared fixture Rust/JS identities still said 42.
These are corrected without relaxing unknown/future-schema rejection, dormant
signature validation or read-only behavior. Versions 41/42/43 remain explicitly
admitted, 40/44 rejected; fixture cache identity now uses 43. This is required
Schema43 plumbing, not new shadow functionality. Global Search SQL/measurement
inputs are unchanged from the successful 100k source. The next natural CI on
the compatibility fix passed; the failed run is retained, not retried.

## Moving master during validation

Master advanced through unrelated UI PR #472 to
`b7a505fdddd7f248c4260edf783b5bae42a93ec5`, tree
`990655287de662fdfbb40b6ab7c7e32de751edb1`, still Schema 42. The candidate
merges this base to resolve only the STATUS.md conflict, preserving both task
records. No Global Search, Schema43 DDL, benchmark or query inputs differ from
the sampled `f89412b` source. The 1M run remains bound to that exact source/tree;
it is not repeated. Natural integration CI validates the updated branch.

## CI completed before 1M closeout

- Source `f89412b` / tree `af6bad542ccf8faaaf06b4fdd91e86ca1b32330e`:
  [natural CI 40319993890](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40319993890)
  SUCCESS, 20 jobs succeeded / 7 planned skips; natural benchmark smoke
  [40319993876](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40319993876) SUCCESS.
- Latest-master integration candidate `692a2c2a8bf04f644171086ff61958c037a8b865`,
  tree `fbb37258979910ce4c2e27ae2085974ad9767faf`:
  [natural CI 40322645985](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40322645985)
  SUCCESS. Both OS Rust/strict Clippy, release compiles, selected performance
  suites, native macOS performance and Windows service qualification passed.
- New master `b7a505f` [CI 40320096881](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40320096881)
  SUCCESS. The original master baseline, sampled sources and latest integration
  candidate are distinct identities; no evidence is relabelled to another SHA.

Initial workflow context/compilation errors occurred before fixture generation.
The first natural production-candidate CI exposed concrete compatibility bugs;
they were fixed in code, with the failure retained. No failed-job manual retry,
Full Validation dispatch, gate waiver, Codex Review or PR merge occurred.

The 100k Name candidate plan changes only the index scan to `USING COVERING
INDEX idx_global_entries_active_name_order`; MATERIALIZE, the order temp B-tree,
outer rowid lookup for full objects and managed EXISTS remain. Its representative
VM work changes 127,673→122,672, Sort remains 1. Extension-prefix remains
`USING INDEX idx_global_entries_active_extension_order`, managed EXISTS plus
`USE TEMP B-TREE FOR ORDER BY`, 141,737 VM Steps / 0 Fullscan / 1 Sort in all
variants. This retained wide-row cost is not addressed by the candidate.

At 100k, Name B-tree bytes are A0/A2 6,316,032 and A1/A3 7,127,040. Source Facts
adds 3,805,184 bytes in A2/A3; combined incremental B-tree bytes are 4,616,192.
Observed main DB sizes A0/A1/A2/A3 are 131,473,408 / 131,473,408 / 134,324,224 /
135,135,232 bytes; these include reusable/free pages, not VACUUM-packed size.
DDL times are 3172.717 / 2755.919 / 2143.028 / 1737.956ms; fixed DDL execution
order and filesystem warming mean these are observed setup costs, not proof
that two-index construction is faster. Five rollback-only 256-row insert
medians are 48.009 / 52.419 / 51.993 / 48.280ms, with production triggers.
Five-sample write tails are descriptive; sustained ingestion is not qualified.

## Single 1M Hosted paired result — complete

[Run 40319987170](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40319987170) / [artifact 12654481067](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40319987170/artifacts/12654481067) SUCCESS, attempt 1. Source `f89412b92157bafac9052a7b60a96a3c81c19d5f`, tree `af6bad542ccf8faaaf06b4fdd91e86ca1b32330e`, Windows x86_64 debug. Exactly one million-row generation and three controlled copies; no repeat run. Generation 2,046.385ms and SQLite population 2,245,282.607ms in 1,954 batches are setup costs, excluded from query timers. The job uses the runner default temp directory; historical workspace-temp runs are not a causal speed comparison.

[Raw JSONL](evidence/covering-indexes/1m.jsonl.gz) and [all five paths, all variants, all queries CSV](evidence/covering-indexes/1m-summary.csv) retain source identity, 1,440 measured public-call sample records, 240 public-stage summaries, 236 SQL-tier summaries and 48 separately traced real Snapshots. The retained raw samples independently reproduce every public-stage p50/p95/p99 to 0.001ms. Five warmups/thirty measured samples; full-object comparisons and terminal 12/12 correctness passed. Both covering-plan assertions passed. Diagnostic SUCCESS is not a blanket performance acceptance.

### Original pooled 100ms target

All four variants pass **10/12** in this run. The remaining failures are FTS report and invoice. Historical 1M 4/12 results remain historical: this run's A0 already passes 10/12, so that cross-run difference is not credited to these indexes. No threshold changes. All Snapshot p95 values still exceed 100ms; Snapshot has no separate historically established 100ms gate, and none is invented here.

Pooled p95, milliseconds:

| Query | A0 | A1 | A2 | A3 |
|---|---:|---:|---:|---:|
| exact_basename | 0.823 | 0.665 | 0.700 | 0.669 |
| name_prefix | 79.280 | 46.352 | 78.931 | 42.333 |
| common_prefix_high_fanout | 78.264 | 44.486 | 79.625 | 41.284 |
| fts_substring_report | 181.982 | 186.567 | 193.987 | 175.343 |
| fts_substring_invoice | 193.952 | 181.397 | 191.536 | 179.552 |
| extension_exact | 0.703 | 0.713 | 0.711 | 0.672 |
| extension_prefix | 80.378 | 85.459 | 73.760 | 78.213 |
| duplicate_basename | 0.451 | 0.479 | 0.493 | 0.512 |
| no_result | 0.796 | 0.817 | 0.869 | 0.755 |
| chinese_prefix | 78.490 | 38.415 | 77.960 | 41.611 |
| punctuation_prefix | 80.463 | 37.644 | 98.402 | 38.128 |
| unicode_accented_prefix | 84.120 | 38.659 | 79.775 | 38.536 |

### Representative complete-path distributions

Milliseconds, p50 / p95 / p99. All remaining distributions are in the CSV.

| Path / query | A0 | A1 | A2 | A3 |
|---|---|---|---|---|
| sql_route / name_prefix | 75.746 / 79.390 / 93.073 | 37.302 / 48.120 / 55.937 | 75.391 / 80.526 / 82.136 | 37.221 / 40.470 / 41.382 |
| pooled / name_prefix | 75.713 / 79.280 / 79.745 | 37.204 / 46.352 / 56.652 | 75.716 / 78.931 / 80.722 | 37.461 / 42.333 / 43.004 |
| source_health / name_prefix | 435.109 / 450.871 / 455.329 | 434.170 / 468.466 / 487.138 | 109.729 / 116.695 / 118.736 | 112.053 / 118.559 / 119.426 |
| index_status / name_prefix | 38.852 / 40.795 / 41.779 | 39.142 / 42.965 / 56.293 | 38.805 / 40.856 / 47.417 | 38.753 / 45.822 / 49.169 |
| snapshot / name_prefix | 553.355 / 587.343 / 602.023 | 513.925 / 545.941 / 550.801 | 211.577 / 221.677 / 244.975 | 172.594 / 187.207 / 189.446 |
| snapshot / no_result | 478.232 / 494.587 / 511.459 | 480.250 / 508.427 / 537.527 | 137.728 / 142.277 / 145.649 | 136.259 / 141.608 / 147.627 |
| snapshot / fts_substring_report | 665.097 / 694.988 / 732.265 | 661.645 / 684.103 / 697.511 | 308.979 / 318.765 / 331.038 | 307.897 / 316.117 / 327.219 |
| pooled / extension_prefix | 71.949 / 80.378 / 83.151 | 71.492 / 85.459 / 98.753 | 71.145 / 73.760 / 75.075 | 71.712 / 78.213 / 106.106 |
| snapshot / extension_prefix | 560.960 / 595.029 / 610.228 | 559.450 / 578.788 / 582.650 | 207.764 / 238.935 / 260.534 | 211.068 / 234.206 / 290.368 |

### Remaining tail failures and limits

- **1M extension-prefix pooled p99 83.151→106.106ms: +27.61%, +22.955ms**, although p95 improves 80.378→78.213ms. This is a retained material tail regression; no noise/root-cause attribution is proved.
- Separately sampled FTS report SQL-tier p99 is 193.974→216.153ms (+11.43%, +22.179ms); its actual pooled p99 is 190.728→191.694ms. Do not subtract or conflate these distinct measurements.
- Index Status is unchanged SQL/work, but has measured tails: name-prefix p95 40.795→45.822ms; common-prefix p99 40.421→52.162ms; FTS-report p99 48.116→61.000ms. These are retained, not discarded as noise.
- All twelve 1M full-Snapshot p95/p99 improve for A3 versus A0. This does not erase the separate 100k punctuation Snapshot p99 regression or qualify every path/tail.
- FTS pooled p95 remains 175.343ms (report), 179.552ms (invoice), both above the unchanged 100ms target. Source Health still scans active rows; its representative A3 p95 is 118.559ms. Index Status still recounts rows; complete Snapshots remain above 100ms.
- **Performance is not fully qualified.** The no-material-p95/p99-regression acceptance requirement is not met. No further experiment, threshold relaxation or production tuning is included.

### Actual plans and executed work

| Statement | A0 VM / Fullscan / Sort | A3 VM / Fullscan / Sort | Plan change |
|---|---:|---:|---|
| Name Prefix tier | 1,252,673 / 79 / 1 | 1,202,672 / 79 / 1 | Name scan becomes COVERING; same materialization, ordering and outer full-row lookup |
| Source Health | 15,000,077 / 999,999 / 0 | 12,000,075 / 999,999 / 0 | volume index scan becomes COVERING active_source_facts |
| Index Status | 3,000,261 / 0 / 1 | 3,000,261 / 0 / 1 | unchanged covering volume index for COUNT(*) |
| Extension Prefix tier | 1,384,777 / 0 / 1 | 1,384,777 / 0 / 1 | unchanged extension index, full projection, managed EXISTS and order temp B-tree |
| FTS report tier | 1,302,670 / 79 / 1 | 1,302,670 / 79 / 1 | unchanged FTS-first materialization/BM25/order |

Full SQL, bound parameters and EQP are in the raw file. FullscanStep is a work counter, not proof of a wide-table scan: Source now scans the covering index. Source COUNT(id), MAX, ordered facts/JSON/BLAKE3, all Source Health states and same-read-transaction Snapshot are preserved.

### 1M storage, construction and bounded writes

| Variant | Name B-tree bytes | Source B-tree bytes | Main DB bytes | DDL ms | 256-row insert p50 / p95 / p99 ms |
|---|---:|---:|---:|---:|---|
| A0 | 63,389,696 | 0 | 1,309,356,032 | 28619.532 | 35.557 / 36.044 / 36.060 |
| A1 | 71,540,736 | 0 | 1,309,356,032 | 31025.689 | 55.765 / 62.782 / 64.091 |
| A2 | 63,389,696 | 38,289,408 | 1,338,044,416 | 30950.485 | 34.782 / 35.427 / 35.442 |
| A3 | 71,540,736 | 38,289,408 | 1,346,195,456 | 30163.671 | 37.019 / 55.819 / 59.497 |

A3 adds 46,440,448 B-tree bytes over A0 (about 44.29 MiB). Main-file size includes reusable pages; raw metrics also retain WAL/SHM. DDL and writes use the same protocol/limitations as 100k. A1's five-sample insert median is 56.83% above A0; A3's median is 4.11% higher and p99 is 65.00% higher (36.060→59.497ms). Do not treat five descriptive write samples as sustained write qualification.

## Owner handoff disposition

**READY_FOR_OWNER_REVIEW / PERFORMANCE_NOT_FULLY_QUALIFIED / DRAFT_OPEN_UNMERGED.** Schema43 production candidate and bounded paired evidence are delivered. All original 12 query classes preserve correctness; the covering indexes work, but retained tail regressions prevent blanket acceptance. Owner decides merge and any next phase. No Codex Review, manual full-CI retry, extra 1M generation, gate change or release occurred.
