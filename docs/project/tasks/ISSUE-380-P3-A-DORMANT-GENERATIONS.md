# #380 P3-A — Schema 42 dormant generation foundation

Authority: [Owner allocation 6228987894](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6228987894).
BASE: `f4661aaf77c7f89319b39b116c1065a3246cbc43`, tree
`f462464514c3d810e0d82537b99bcf04be9275fb`, Schema 41. Its postmerge
[CI 38998442077](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38998442077)
succeeded. Open PR/branch inspection found no competing Schema 42 migration.
This task implements only P3-A. **The production full-rebuild defect remains
unfixed. P3-B publication/consumer integration is not activated.**

## Representation and migration

DDL: [schema_global_generations.sql](../../../src-tauri/src/db/schema_global_generations.sql).
The existing migration transaction installs the new objects and sets version 42
atomically. Existing Schema 41 Library signatures are checked before upgrading;
Schema 42 opens validate the complete new owned namespace (tables, constraints,
indexes, FTS shadows and triggers). Conflicting/partial DDL rolls back, signature
drift fails closed and future schema versions remain rejected.

- `global_index_generations`: source/stable identity/provider, existing admitted
  owner/revision, captured rebuild-intent revision, initial/rebuild/reconcile
  capture kind, materialization revision/count, staging/sealed/aborted lifecycle,
  verified manifest digest, candidate checkpoint and completeness.
- `global_index_generation_heads`: source-conditioned future published/candidate
  foreign keys. Existing sources receive only `LEGACY_UNVERIFIED` with **no**
  published or candidate pointer. Newly discovered sources initialize this row
  lazily only if the candidate API is explicitly called. Missing heads also imply
  legacy-unverified; current production discovery does not call this API.
- `global_index_generation_entries`: `(generation_id, logical_entry_id)` uniqueness
  plus independent non-reused integer physical row identity and complete canonical
  `GlobalEntryInput` metadata. Name/path/extension and normalized search fields
  have explicit columns. No FK requires a candidate logical ID to exist in legacy
  rows, so newly discovered entries can be staged without publishing them.
- `global_index_generation_fts`: separate external-content trigram FTS bound to
  `version_rowid`. Two generations can hold the same logical ID with different
  metadata and different physical FTS rowids. A future query can join version row
  → generation → source head and select `published_generation_id`. No such
  production query or switch exists in P3-A.

No legacy rows, foreign keys, user tags/corrections, source health, owner,
checkpoint, intent, legacy FTS or AI eligibility are copied or rewritten. The
only migration initialization reads the small source table to create empty head
rows; it does not enumerate/backfill corpus entries. A legacy READY flag, prior
full-index timestamp or lack of owner does not prove completeness.

## Candidate-only repository contract

`global_index/generation_repository.rs` has no production callsite. Its private
module exposes only crate-scoped candidate operations; it has no publish method,
Tauri command, enable switch, worker, polling loop or AI notification.

1. Begin requires the **existing** running admitted source owner. It captures
   identity/provider/intent under the same IMMEDIATE reservation, assigns a UUID
   and attaches one source candidate. Unique `(source, operation_revision)`
   prevents recreating a candidate under an aborted/sealed revision (ABA).
2. Append accepts at most 256 entries / 2 MiB encoded metadata per transaction.
   Every batch revalidates source owner, revision, enabled state, candidate pointer,
   stable identity and provider. Foreign source/provider, duplicate IDs, sealed,
   aborted, UNKNOWN or superseded writers are rejected. A batch failure rolls
   back version rows, FTS and count/revision together; earlier batches persist.
3. Seal scans sorted materialized rows in a **read** transaction and recomputes a
   domain-separated, length-framed BLAKE3 manifest over logical IDs plus complete
   metadata. It checks both caller capture count/digest and stored count, then
   uses a short IMMEDIATE owner/materialization-revision CAS to seal. Concurrent
   appends cannot make a stale proof succeed. This verifies materialization of
   the caller's declared capture, not native filesystem coverage; P3-B must
   supply an authoritative complete native capture and reject partial/degraded
   collections. Empty manifests are representable without implying source health.
4. Seal stores candidate-only checkpoint/completeness. It does not advance live
   checkpoint/timestamps, mark READY, clear intent, release source ownership,
   enable Managed rows or enqueue AI. Sealed is not published.
5. Abort requires the current candidate owner and records a bounded reason before
   detaching its pointer; it neither deletes staged data nor restores live state.
   Aborted/sealed writes and repeated terminal writes fail closed without mutation.
6. Recovery requires a **newer actually admitted** owner and only touches that
   source's older candidate. An old staging candidate becomes aborted; a sealed
   candidate is detached without altering its manifest. Repeated recovery is a
   no-op. Existing Service UNKNOWN → Direct replay refusal and process liveness
   rules remain in the source claim; recovery cannot bypass or grant admission.

Serialization happens outside the writer. Materialization verification is O(N)
read work; final seal and recovery perform bounded metadata writes. No entire
100k+ corpus writer transaction is introduced. Existing pool size, 5000 ms lock
wait timeout, scheduler and wake policy are unchanged; no five-second measured
hold-time or native performance qualification is inferred from a batch-size cap.

## Evidence and focused regression scope

Real product `Database`/repository tests use SQLite, not a fake state machine:

- Schema 41 populated fixture → production migration → 42 preserves complete
  legacy Global/Managed/AI/source rows, unknown Service ownership, checkpoint,
  pending intent, user tags/corrections and physical legacy FTS. READY legacy
  data is still unverified. Fresh install and reopen exercise the same code.
- A collision at the third migration DDL statement proves mid-DDL rollback of
  the earlier tables and version with user data intact. Removing that collision
  then permits the actual migration. Signature drift and version 43 are rejected.
- One combined candidate test compares all legacy facts before/after partial
  staging, real batch-2 SQLite uniqueness failure, cancellation and replay. It
  checks no AI wake, old Search visibility, separate-source progress, distinct
  versions of one logical ID and real versioned FTS mapping.
- Restart recovery retains an UNKNOWN Service candidate, refuses Direct admission
  and stale writes, admits a successor Service owner, recovers idempotently,
  verifies manifest mismatch and sealing, retains live intent/checkpoint, rejects
  sealed writes and preserves sealed metadata after reopen/successor recovery.

Existing source ownership, managed AI, migration, SQLite contention and snapshot
suites remain the regression authorities. Historical fixture teardown/assertions
are updated for the additive version; the Owner explicitly allowed the one
`library_identity.rs` **test-only** fixture adjustment. Its production SourceUID
logic and F1 activation authority are unchanged. Exact local checks and the
single normal routed CI run/attempt are recorded in the Draft PR, not inferred
from baseline success. Because schema.rs changes, the existing full routing,
including `all_domains_100k`, must run without waiver.

## Deferred P3-B and acceptance boundary

Before activation, Owner must authorize cross-domain readers and bounded atomic
publication: independently preserve last committed health/checkpoint, gate all
full replacements (including incremental macOS full reconcile), CAS source head
with validated native completeness, publish checkpoint/count/intent coherently,
and revalidate version-aware Managed/AI claim/completion and user policy. User
references/corrections stay keyed to stable logical IDs; do not copy them onto
candidate metadata or let an unpublished generation authorize AI execution.
P3-A deliberately stores no duplicated user-owned policy.

Future GC must first prove a generation unreachable from published/candidate
heads and live work, then delete a bounded number of version rows per transaction
with corresponding FTS cleanup. P3-A blocks deletes and activates no GC. A future
DDL/trigger change and explicit retention/recovery contract are needed before
removing unreachable generations; unbounded cascade cleanup is not a publish step.
An interrupted Schema 41 corpus cannot be certified by migration. P3-B must
establish a complete replacement or expose incompleteness explicitly.

Windows Service/Direct and macOS production scans still write the legacy tables
exactly as before. No full P3, #328 historical lock-holder, native power-loss,
Windows/macOS machine, release or publication acceptance is claimed. No merge,
Issue closure, Release or Codex Review is authorized.
