# Issue 474 — exact Source Facts production slice

Owner contract: [#474](https://github.com/ArdenZC/Zen-Canvas/issues/474).
Fresh remote master HEAD `9bc4aa9e3fb2c6bb557192383be168517a182651`, Tree
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

The one controlled 1M Source Facts pair is run 40371160127 on the same production
candidate. Its results are pending. No repeated 1M fixture or failed FTS rerun.

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
