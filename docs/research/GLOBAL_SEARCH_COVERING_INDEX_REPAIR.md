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
any new failure. Results and remaining bottlenecks will be appended after runs.

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
inputs are unchanged from the successful 100k source. One next natural CI on
the compatibility fix is required; the failed run is retained, not retried.

## Moving master during validation

Master advanced through unrelated UI PR #472 to
`b7a505fdddd7f248c4260edf783b5bae42a93ec5`, tree
`990655287de662fdfbb40b6ab7c7e32de751edb1`, still Schema 42. The candidate
merges this base to resolve only the STATUS.md conflict, preserving both task
records. No Global Search, Schema43 DDL, benchmark or query inputs differ from
the sampled `f89412b` source. The 1M run remains bound to that exact source/tree;
it is not repeated. Natural integration CI validates the updated branch.
