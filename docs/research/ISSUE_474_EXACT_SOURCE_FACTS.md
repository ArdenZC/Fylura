# Issue 474 — Source Facts / Schema44 final integration

## Owner rereview: merged #479 integration

Existing Draft [#478](https://github.com/ArdenZC/Zen-Canvas/pull/478) integrates
latest checked `master@f302f5da8ab86f7d121b5cad9a71e5e552b720b6`, tree
`1bb9e6ee83381c4cbe3a7be8d64542c2e814a510` (normal master CI
[40432147790](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40432147790) SUCCESS).
The complete focused Windows/macOS integration and cost run
[40452161859](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40452161859)
**SUCCESS** tested source `768464b3ac59b4227521ae82dd3827170f3803c7`, tree
`57b3c1faa4c4163bba149f478ec4002ad2480d1f`. The subsequent evidence/docs-only
closeout preserves this exact production/test source. Final HEAD, Tree, diff and
normal CI run/result are recorded in the PR description and final handoff;
[PR checks](https://github.com/ArdenZC/Zen-Canvas/pull/478/checks) track that head.
No Full Validation, Codex Review, Release, Issue closure or merge is performed.

Schema44 preserves #479's native foreground worker/modal, responsive cancel,
SQLite progress interruption, Retry/Exit, background deferral, transaction and
commit-failure rollback, low-memory reserve, FILE temporary storage, 8MiB main
cache, zero mmap/auxiliary sort workers and future-version rejection. The
Windows/macOS widget implementations, main startup entry point, database
connection and patched dependency versions are identical to merged #479.
Source Facts triggers, exact COUNT(*)/COUNT(id)/MAX, ordered Source Revision
JSON/BLAKE3, shared Snapshot read transaction and prepared UPSERT batches remain
intact. FTS BM25 8:2:1, ordering/pagination/deduplication/4096 window are unchanged.
File Library/F1 #417 retains its existing authority; no cutover is introduced.

The installed-database capacity gate now covers 25..43, including both 42→44
and 43→44, under `BEGIN IMMEDIATE` before migration DDL. A streaming GROUP BY
on the existing volume index (42) or active Source Facts covering index (43)
estimates distinct active source key bytes plus registered keys, including
orphan/empty sources. Registered/active overlap is deliberately overcounted.
Facts table/WAL and possible sort space use conservative payload allowances and
64MiB reserve per storage role; 42 additionally reserves both index rebuilds.
Same-volume reserves combine; free pages are not credited. For the four-source
fixtures, 43→44 reserves 67,213,744B database and 67,110,960B temp, independent
of 100k/200k row count. These are planning margins, not allocation guarantees
or RSS caps. The original 128MiB available-memory working reserve remains.
Preflight/backfill remain cancellable. Both required covering-index signatures
are checked on43/44; absent/forged indexes fail closed. 44 reopen avoids both
capacity scans and the one-time backfill.

## Focused verification results

On **both Windows x86_64 and macOS14 ARM64**:

- Upgrade/recovery filter: **21 PASS**, one intentionally ignored cost test
  executed separately below. Includes real SQLite FULL during facts INSERT,
  insufficient-capacity rejection, cancellation before and after backfill,
  injected post-backfill SQL failure, read-only/I/O failure, and actual child
  termination after `sqlite3_db_cacheflush` spilled uncommitted facts/index pages.
  42 and43 retain installed versions/indexes and user-data fingerprints on
  failure; fresh open retries to44 and integrity checks pass. Missing required
  indexes at43/44 fail closed. Existing schema43/future45 checks: **2 PASS**.
- Populated42→44,43→44 and44→44: **1 combined test PASS**, with disabled/stale,
  empty/multiple sources and NULL ID. All12 canonical complete Snapshots retain
  fields/results/BM25/source health/revision/status. A test hook rejects any
  repeated backfill on44 reopen. Exact facts writer/property tests: **3 PASS**,
  including2,500 randomized mutation statements, NULL/max/stale/delete/rehome/
  reactivation and rollback/signature contracts. The existing normal CI covers
  broader reader/writer and migration regressions.
- Native widget QA: **PASS for both42 and43**. Two native attempts per route
  exercise cancel/recovery/Retry,18–19 timer ticks, visibility before each worker,
  final44, and a real `--background` Tauri child with exit0 and original42/43.
  Background logs contain neither database-ready nor a native upgrade dialog.
  Worker delay is QA-only; it is excluded from cost measurement.

Initial integrated run [40451711869](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40451711869)
failed because the FULL fixture violated foreign keys and the tiny crash fixture
fit inside the restored8MiB cache. Both were corrected without relaxing FKs,
WAL-spill assertions, policy or recovery semantics: use distinct registered
sources and explicitly flush uncommitted cache pages. It generated no200k
fixture. Earlier run40451330419 was cancelled before population to correct the
historical Status oracle to the existing canonical test path; production still
fails closed when required facts are missing. Superseded normal runs were
cancelled naturally by branch updates; none is promoted to final evidence.

## Bounded actual migration cost

One200k fixture per native platform; no1M generation or search latency matrix.
Full production schema/FTS/indexes/triggers,67-byte ge_/gv_ IDs, four source
groups including one disabled, stale every13 rows and a legacy NULL ID remain.
Controlled compacted copies supply100k/200k at real installed42 and43.
Population (Windows76.988s/macOS87.004s), copy/reshape/VACUUM/checkpoint and43
index preparation are outside `Database::open` timing. The SQL interval covers
canonical facts backfill/empty-source initialization, excluding DDL validation;
Open includes preflight, migration, pool initialization and commit.

| Platform | Rows | Route | Open ms | Backfill SQL ms | Logical B-tree delta KiB | Observed WAL KiB | Sampled RSS MiB / samples | 44 reopen ms |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| windows | 100,000 | 42→44 | 1065.222 | 19.078 | 20612 | 31330.54 | 36.11 / 41 | 15.022 |
| windows | 100,000 | 43→44 | 56.244 | 25.151 | 12 | 28.20 | 23.21 / 3 | 14.660 |
| windows | 200,000 | 42→44 | 2200.954 | 43.610 | 41380 | 62813.94 | 31.24 / 83 | 14.964 |
| windows | 200,000 | 43→44 | 98.896 | 48.576 | 12 | 28.20 | 23.10 / 5 | 14.591 |
| macos | 100,000 | 42→44 | 1098.192 | 24.440 | 20608 | 31326.52 | 100.94 / 14 | 23.487 |
| macos | 100,000 | 43→44 | 60.291 | 29.023 | 8 | 24.17 | 150.31 / 1 | 16.305 |
| macos | 200,000 | 42→44 | 2172.563 | 40.370 | 41376 | 62809.91 | 190.33 / 20 | 19.691 |
| macos | 200,000 | 43→44 | 143.253 | 83.961 | 8 | 24.17 | 243.92 / 2 | 21.144 |

All eight migration observations and their44 reopen checks preserve all12
complete Snapshots and canonical per-volume facts. Some query cases are empty
because of stale/disabled fixture distribution; this is equality evidence, not
a claim that all12 cases have positive matches in these upgrade copies. Every backfill plan is
`SCAN global_entries USING COVERING INDEX idx_global_entries_active_source_facts`;
there is no temp GROUP BY sort.43→44 adds a4KiB facts table plus Schema directory
pages:12KiB logical B-trees on Windows,8KiB on macOS. Physical main-file growth
is still0 before checkpoint, with committed pages in WAL (28,872B Windows /
24,752B macOS), so0 is **not** a zero persistent-storage claim.42→44 includes
covering-index rebuilds and their larger WAL, measured separately by route.

The20ms sampling sleep adds OS/sysinfo overhead, particularly on macOS.43→44
has only1–5 samples: sampled RSS includes fixture/cache history and is neither
an OS peak nor migration-attributed memory. Use the recorded after-file WAL
when sampling misses its brief lifetime; observed WAL is max(sampled,after).
Free-volume decline includes unrelated Hosted activity and is not exclusive temp
allocation. At200k it was101.29MiB Windows /171.42MiB macOS for42→44, and0 /
5.66MiB for43→44; zeros can miss small writes and are not a zero-cost guarantee.
Peak observed SHM is128KiB (42) /32KiB (43). These warm, compacted Hosted
observations do not bound cold/slow disks, low-memory hardware, arbitrary source
cardinality, separate temp volumes or a1M backfill. No need for a1M backfill run
was demonstrated by the bounded integration: the plan streams the existing
covering index, persistent facts scale with source cardinality, and failure/
recovery contracts are exercised directly. **Actual1M Schema44 duration and true
OS peak resources remain unmeasured.** No extrapolated performance PASS is made.

Durable raw observations and native events:
[summary CSV](evidence/source-facts/schema44-integration-summary.csv),
[Windows JSONL](evidence/source-facts/schema44-integration-windows.jsonl.gz),
[macOS JSONL](evidence/source-facts/schema44-integration-macos.jsonl.gz),
[Windows UI](evidence/source-facts/schema44-integration-windows-ui.json),
[macOS UI](evidence/source-facts/schema44-integration-macos-ui.json).

## Remaining acceptance and performance limits

Historical1M pair below remains bound to70b1dff/tree8dab75d, not the integrated
HEAD:12/12 equality; no-result Snapshot p95 216.397→1.246ms and name-prefix
299.060→68.729ms. FTS report/invoice pooled p95 remain220.633/233.568ms above
unchanged100ms; extension-prefix pooled p99+26.10% and whole-volume retirement
+16.9% remain material. Sustained Watcher storms/full1M retirement are unqualified.
Human keyboard/focus/screen-reader/contrast, packaged first upgrade/tray and
Owner Windows acceptance remain **UNVERIFIED**. Actual hardware IOERR, OS temp
exhaustion, very low memory and separate-temp-drive recovery were not injected.
Existing #446 SIGBUS release qualification remains unresolved. Automated
integration evidence supports Owner rereview; green CI does not grant release
qualification or authorize a merge. Draft/OPEN and Owner review/merge authority
remain unchanged.

---

# Issue 474 — exact Source Facts production slice

Owner contract: [#474](https://github.com/ArdenZC/Zen-Canvas/issues/474).
Task-start remote master HEAD `9bc4aa9e3fb2c6bb557192383be168517a182651`, Tree
`174caefce872848988075846a5a52707daee9728`, Schema43 (#473 merged).
Baseline CI [40361605473](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40361605473) completed SUCCESS. #473 raw evidence,
12-class generator, exact mapper, source oracles and PROFILE trace are reused.

## Producer audit before choosing materialization

- `global_index/coordinator.rs` owns provider write admission. `write_batch`
  calls `upsert_global_entries_batch_owned`, and retire/rebuild calls the owned
  stale helpers. These delegate to the same Repository IMMEDIATE transactions.
- `repository.rs` UPSERT writes all metadata, including last_seen_at and stale
  reactivation. Windows MFT/USN, macOS provider and recursive fallback ultimately
  use this transaction owner. Global Index Watcher wakes the coordinator; it
  does not directly author metadata facts. The explicit activation/probe API
  also uses the existing Repository upsert. Real upsert uses ON CONFLICT UPDATE,
  not INSERT OR REPLACE.
- Deletes/cascades and direct SQL in existing regressions are covered by SQLite
  entry triggers; rehome, ID nullability, stale transitions and last-seen changes
  are included. Nothing depends on only one Rust callsite doing the bookkeeping.
- Managed Scan/Watcher, SourceUID/RootEpoch and F1 #417 retain their own authority.
  No F1 cutover or shared index is introduced. Its schema-version admission
  plumbing alone must admit the additive Schema44 without relaxing signatures.

## Production design and exactness

Schema44 adds one WITHOUT ROWID row per encountered volume, containing separate
active COUNT(*) and COUNT(id), plus exact MAX(last_seen_at). These are new
write-owned exact facts, not the mutable legacy entry_count cache. Insert,
delete and relevant update triggers update facts in the entries write
transaction. Inactive→active, active→inactive, rehome and nullable-ID changes
have distinct arithmetic; max deletion/lowering seeks the existing Schema43
(volume_id,last_seen_at,id) partial covering index. Nonmax timestamp changes
avoid writing an unchanged fact row. Registered empty sources get zero rows;
orphan-volume entry facts are retained but source/status joins still exclude
unregistered volumes. Missing required fact rows fail closed, not as invented
zero counts. Schema/trigger signatures are checked on reopen.

Migration performs one explicit canonical backfill under the existing migration
write transaction, with version update in that same transaction. Current startup
never scans entries to repair counts: legacy display entry_count is repaired
from the exact fact rows. No per-search DDL or startup-wide data rebuild.
Source Revision uses the unchanged ordered fact struct serialization/BLAKE3.
Snapshot derives enabled active row count from the same already-read facts in
its existing read transaction; standalone Status remains one atomic SQL
statement and sums exact active_rows. TEMP entries shadows retain the original
canonical path rather than applying main facts to another relation.

Migration can take the writer lock and requires disk/WAL headroom. DDL/backfill
failure rolls back facts/version. Old Schema43 binaries reject 44; installation
rollback needs a compatible pre-upgrade backup, never a manual version decrement.
Unsupported direct replacement SQL must not be added as a producer without
qualification; actual writers use UPSERT (SQLite REPLACE delete-trigger behavior
is different when recursive_triggers is off).

## Bounded qualification in progress

Portable Cloud tests execute the exact production facts SQL: NULL/max mutations,
2,500 deterministic randomized mutations checked after every statement, signature
drift and transactional rollback pass. A preliminary real-schema/FTS/count-trigger
and actual Repository UPSERT SQL replay used 15,360 rows in 30×512 batches:
insert totals 1223.199→1262.382ms; metadata update 408.459→485.298ms; bulk stale
126.018→211.271ms. This exposes real added write CPU, especially retire, and is
not supported-platform or full Repository/Watcher qualification. The Hosted
pair measures committed real Repository insertion, metadata refresh, identical
replay, rename, descending-max retirement, whole-volume retirement, WAL bytes
and trigger-inclusive changes. Do not describe this as free or sustained write
PASS based only on read improvements.

Use one 100k fixture and a controlled copy. Canonical facts vs prepared facts
alternate 5 warmups/30 samples for all 12 classes, limit80/full semantics,
SQL route, pooled search, source, status and complete Snapshot. SQL tiers,
pagination/full BM25 objects and separately traced actual SQL/VM/plans are
retained. At most one 1M run after the full 100k preflight passes. Fixtures live
in workspace temp on Hosted, reusing the earlier staging lesson.

FTS is the second coherent slice: a test-only native rank stream with unchanged
bm25(8,2,1) and an exact bounded heap exhausts the cutoff rank tie group before
fetching full rows. It preserves stable secondary ordering, filters before
selection, and pins a read snapshot across both phases. It is different from
NOT MATERIALIZED, naive rank-only LIMIT, or a recency scan. It remains test-only
until same-fixture correctness and performance qualify a production change.
No production FTS change or second research PR is implied by this probe.

Keep Draft/OPEN. No merge, Issue closure, Release, threshold changes, Full
Validation retry or Codex Review. Owner reviews each complete production slice.


## Hosted 100k preflight and production write correction

The first pair (24f2b4c / tree 1622e38c322c11304bf74b61100eb3c68bbce1cc,
run 40366460347) passed exactness but exposed a material writer CPU regression.
Original Repository transaction.execute reparsed the full trigger program for
each row. Production now prepares the unchanged UPSERT once per batch. The
canonical paired variant retains the previous per-row preparation and disables
only new facts maintenance; it is the old producer/read behavior.

Corrected preflight [40369517784](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40369517784)
used 70b1dff308fde45763d118fd4603cb19d87e092f,
tree 8dab75d8676712835aa99f79b0463bbaab03eb48. All 12 complete results,
BM25, offsets, limit80/4096, source health/revision and index status matched;
all focused real Repository/null/disabled/concurrent/snapshot tests passed.

| 100k Snapshot / real writer | Canonical p95 ms | Prepared p95 ms |
| --- | ---: | ---: |
| no result Snapshot | 29.857 | 1.631 |
| name prefix Snapshot | 29.823 | 6.505 |
| extension prefix Snapshot | 43.519 | 11.671 |
| report FTS Snapshot | 43.782 | 22.157 |
| invoice FTS Snapshot | 54.630 | 31.698 |
| insert 512 | 298.494 | 184.637 |
| metadata update 512 | 134.979 | 60.730 |
| identical replay 512 | 133.802 | 55.657 |
| rename 512 | 287.017 | 192.006 |
| descending-max retirement 16 | 17.492 | 20.313 |

Added fact triggers still cost work: descending-max retirement p99
18.453→21.435ms; one remaining-volume retirement 742.210→894.989ms (+20.6%).
Insert WAL 12,677,272→12,685,512 bytes; metadata WAL
5,038,792→5,051,152. Trigger-inclusive changes increase as expected. These are
real API commits with scope/AI checks and existing FTS/count triggers, not an
end-to-end Watcher event-storm or long-duration platform qualification. The
writer loop improvement outweighs added facts maintenance for these measured
upsert stages; retirement cost remains explicit. No cost-free materialization
claim is made.

The one controlled 1M Source Facts pair [40371160127](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40371160127)
completed SUCCESS on the same 70b1dff / 8dab75d source/tree. All 12 complete
result/page/BM25/health/revision/status contracts matched. No repeated 1M fixture
or failed FTS rerun. Later production changes only rewrite a contiguous schema
match pattern to a Rust range (same admitted versions); remaining corrections
are tests, dependency security, documentation and latest-master status integration.

| 1M path | Canonical p95 / p99 ms | Prepared p95 / p99 ms |
| --- | ---: | ---: |
| no-result Snapshot | 216.397 / 218.985 | 1.246 / 1.306 |
| name-prefix Snapshot | 299.060 / 303.289 | 68.729 / 78.157 |
| name-prefix Source Health | 170.844 / 173.001 | 0.234 / 0.239 |
| name-prefix Index Status | 76.676 / 90.086 | 0.344 / 0.358 |
| extension-prefix Snapshot | 327.342 / 347.110 | 92.805 / 99.502 |
| extension-prefix pooled | 91.163 / 94.806 | 91.560 / 119.551 |
| report FTS pooled | 219.387 / 220.993 | 220.633 / 224.379 |
| invoice FTS pooled | 233.000 / 243.448 | 233.568 / 234.479 |
| report FTS Snapshot | 426.874 / 430.205 | 220.977 / 221.505 |
| invoice FTS Snapshot | 439.191 / 440.673 | 234.237 / 235.194 |

Pooled p95 remains 10/12 under the unchanged 100ms target; complete Snapshot
now also passes 10/12. FTS report/invoice remain over target. This is a paired
comparison, not a claim that historical machine timings are directly comparable.
Extension-prefix pooled p99 +26.10% remains a material tail risk despite improved
full Snapshot p99. No search SQL or wide-row route was changed, and no extra 1M
run is used to chase this tail. Owner must assess this retained risk; do not
label the entire phase or every tail accepted.

Real 1M-fixture 512-row writer p95 insert 264.109→165.143ms, metadata
144.339→73.049ms, identical replay 146.926→63.137ms, rename
329.379→223.927ms. Descending-max retirement p95/p99
24.758/30.739→24.052/25.646ms; median 3.493→5.812ms still exposes maintenance
CPU. Remaining-volume retirement 776.204→907.701ms (+16.9%). Insert WAL
9,274,152→9,282,392 bytes and metadata WAL 4,993,472→5,005,832; replay WAL is
unchanged. These are actual Repository commits, not a claim of long-running
Watcher/event-storm qualification. One-time Schema43→44 backfill time at 1M
was not separately measured and remains an upgrade planning limitation.

## FTS algorithm decision

The different exact tie-frontier algorithm passed 100k result equality but was
slower: report pooled p95 38.565→56.415ms, invoice 42.034→65.179ms
(40366460347, native bm25(8,2,1), complete secondary-key tie group). The native
FTS scorer still visits the match set and Rust exact tie handling adds work.
Do not promote it or spend the allowed 1M run on this failed preflight. Production
FTS SQL, rank weights and deterministic ordering are unchanged. The test-only
probe and raw initial evidence reproduce this decisive result. A further FTS
production PR requires a different qualified algorithm; the historical 1M
report/invoice >100ms remains unresolved. This does not relax the target.

Raw records, per-stage quantiles, SQL tiers/plans/VM counters and real writer
samples are in [evidence/source-facts](evidence/source-facts). #477 separately
addresses Schema43 upgrade UX. Owner should review upgrade integration/merge
order before shipping Schema44; this PR does not claim installer or visible
startup qualification for the new facts backfill.


## CI corrections and security

The first normal CI (40373263933) passed release compiles and applicable
performance domains but stopped Rust quality on three Clippy findings: a
contiguous version OR-pattern, a test modulo expression and a test summary
range loop. They are corrected without changing admitted schemas or benchmark
semantics. A newly published high-severity happy-dom advisory
GHSA-834v-8h66-3h79 also affects the unchanged baseline. The dev dependency is
updated to patched 21.0.4; no audit level, ignore list or CI gate is relaxed.
The applicable checks and exact final head/tree are recorded on Draft #478.
A prior added trace test held one pooled handle while reserving all eight; the
fixture now inspects through a separate connection. This was a test pool
exhaustion, after the populated-43 backfill/result assertions had passed. No
production pool size/timeout or required gate is changed.


Latest integrated master is 1098af35727cdd3bea042deba41bb59238be586e, tree
e0346ea07d8a917f634516c5848daa564d10d0cd (Schema43), normal CI40425918531
SUCCESS. U2 frontend/status changes are retained. No upstream Rust search or
migration change invalidates the one paired 1M observation; no fixture rerun.
The prior review head ac749fa / d024e5e passed normal CI40424584559. Updated
integration checks are attached to the latest Draft #478 head.
