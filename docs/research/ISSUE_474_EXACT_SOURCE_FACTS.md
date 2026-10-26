# Issue 474 — exact Source Facts production slice

Owner contract: [#474](https://github.com/ArdenZC/Zen-Canvas/issues/474).
Fresh remote master HEAD `9bc4aa9e3fb2c6bb557192383be168517a182651`, Tree
`174caefce872848988075846a5a52707daee9728`, Schema43 (#473 merged).
Baseline CI 40361605473 was in progress at task start. #473 raw evidence,
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
