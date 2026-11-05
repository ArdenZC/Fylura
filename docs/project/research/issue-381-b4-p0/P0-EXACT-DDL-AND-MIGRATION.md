# #381-B4-P0 — Exact expand-only Schema candidate and version arbitration
**Owner development contract • 2026-10-13 • P0_SCOPE_READY_FOR_CODEX_CANDIDATE**

## Frozen source and schema arbitration

- Current master `3275ac7c72df8ae4139a30f16c7f649971324e97` / tree `009576241cd4e8e2ba12cc644a368314f599faa6`: `CURRENT_SCHEMA_VERSION = 37`. Read fresh master immediately before modifying production.
- Existing BLOCKED Draft #383 `584621c32054e16ac0a61afc669a04380a976b7f` has a **provisional, UNACCEPTED** `37 -> 38` step only for a conditional `files_au` trigger. It declares itself non-frozen. #383 may NOT be merged/cherry-picked as v38 and must rebase and reconcile later. This is a real collision, not two mutually valid v38 schemas.
- Latest open #398 P1 `1acb65fa8623d9f52292cf8c6d1a321e2cbadc32` changes Global Index IPC and not Schema. #380 P2 may require a later schema step but has no production Schema allocation in master. Both tracks must compare latest master before a migration commit.
- **Owner arbitrates candidate version 38 for one and only one P0 expand-only migration from master Schema37**, conditionally on no other Schema-changing commit entering master first. This reservation is **not a statement that master has v38**. Any master HEAD or Schema version change BEFORE Codex work means STOP and request refreshed Owner assignment; do not choose 39 automatically.
- A standalone developer/test database already migrated by the blocked #383 provisional v38 may exist. It is **incompatible** with B4-P0 v38 because it lacks the new identity foundation. B4-P0 **must reject it explicitly without changing it**, not silently fill tables under the same user_version. Handle that disposable development fixture independently; do not erase data or downgrade.
- Product never publicly shipped; no old released app/installer matrix. Existing *local dev data and real user-owned data on current Schema37* still must be preserved.

## Phase P0 exact scope: define dormant schema, DO NOT ACTIVATE
All proposed tables use `library_` to avoid confusing independent `managed_entries` / `global_entries` (#380). Column and uniqueness contracts below are proposed precise SQL for Codex implementation, subject to actual same-HEAD Rust/SQLite compile and migration tests. The new tables MUST be initially empty except for the single authority row. No scheduled backfill, filesystem read, FTS rebuild, copying files, policy activation, reader/writer switch or UID attachment.

### Proposed SQL executed under the existing single migrate BEGIN IMMEDIATE

~~~sql
CREATE TABLE library_source_incarnations (
  source_uid TEXT PRIMARY KEY NOT NULL,
  verification_state TEXT NOT NULL DEFAULT 'unverified'
    CHECK (verification_state IN ('unverified','verified','conflicted','retired')),
  physical_platform TEXT,
  physical_volume_id TEXT,
  physical_object_id TEXT,
  verification_digest TEXT,
  source_revision INTEGER NOT NULL DEFAULT 1 CHECK (source_revision >= 1),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  CHECK (
    verification_state <> 'verified' OR (
      physical_platform IS NOT NULL AND physical_platform <> '' AND
      physical_volume_id IS NOT NULL AND physical_volume_id <> '' AND
      physical_object_id IS NOT NULL AND physical_object_id <> '' AND
      verification_digest IS NOT NULL AND verification_digest <> ''
    )
  )
);

CREATE TABLE library_source_observations (
  source_uid TEXT NOT NULL,
  observation_revision INTEGER NOT NULL CHECK (observation_revision >= 1),
  legacy_file_id_snapshot TEXT,
  observed_path TEXT NOT NULL,
  name TEXT NOT NULL,
  extension TEXT NOT NULL DEFAULT '',
  size INTEGER NOT NULL CHECK (size >= 0),
  mtime INTEGER NOT NULL,
  ctime INTEGER NOT NULL,
  is_dir INTEGER NOT NULL CHECK (is_dir IN (0,1)),
  physical_claim_digest TEXT,
  observation_digest TEXT NOT NULL CHECK (observation_digest <> ''),
  observed_at INTEGER NOT NULL,
  PRIMARY KEY (source_uid, observation_revision),
  FOREIGN KEY (source_uid) REFERENCES library_source_incarnations(source_uid)
    ON DELETE RESTRICT
);
CREATE INDEX idx_library_observations_legacy_id
  ON library_source_observations(legacy_file_id_snapshot);
CREATE INDEX idx_library_observations_path
  ON library_source_observations(observed_path, source_uid);

CREATE TABLE library_legacy_attributions (
  claim_id TEXT PRIMARY KEY NOT NULL,
  legacy_file_id TEXT NOT NULL,
  legacy_path_snapshot TEXT NOT NULL,
  source_uid TEXT,
  claim_state TEXT NOT NULL DEFAULT 'unresolved'
    CHECK (claim_state IN ('unresolved','verified','conflicted','user_confirmed')),
  evidence_digest TEXT,
  decision_revision INTEGER NOT NULL DEFAULT 1 CHECK (decision_revision >= 1),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  CHECK (claim_state IN ('unresolved','conflicted')
         OR (source_uid IS NOT NULL AND evidence_digest IS NOT NULL
             AND evidence_digest <> '')),
  FOREIGN KEY (source_uid) REFERENCES library_source_incarnations(source_uid)
    ON DELETE RESTRICT
);
CREATE INDEX idx_library_attributions_legacy
  ON library_legacy_attributions(legacy_file_id, decision_revision);
CREATE INDEX idx_library_attributions_source
  ON library_legacy_attributions(source_uid, claim_state);

CREATE TABLE library_generation_authority (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
  read_mode TEXT NOT NULL DEFAULT 'legacy'
    CHECK (read_mode IN ('legacy','generation')),
  active_catalog_epoch INTEGER NOT NULL DEFAULT 0
    CHECK (active_catalog_epoch >= 0),
  active_fts_slot_id TEXT,
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  updated_at INTEGER NOT NULL,
  CHECK ((read_mode='legacy' AND active_catalog_epoch=0 AND active_fts_slot_id IS NULL)
       OR (read_mode='generation' AND active_catalog_epoch>0 AND
           active_fts_slot_id IS NOT NULL AND active_fts_slot_id <> ''))
);
INSERT INTO library_generation_authority (
  singleton_id, read_mode, active_catalog_epoch, active_fts_slot_id,
  revision, updated_at
) VALUES (1, 'legacy', 0, NULL, 1, unixepoch());
~~~

**Important:** this deliberately does NOT add `library_root_epoch_claims`, F1 slot manifests or reader leases yet. Their ownership, FTS file durability and root-watch CAS belong to separately authorized P1/P2. P0's `read_mode='generation'` column is **schema capability only**: P0 must have **no production path that writes that value**. No duplicate `files.path` uniqueness requirement on immutable historical observations: the same path can legitimately recur across source incarnations/epochs. Do not make `physical_object_id` globally UNIQUE (hardlink alias, inode reuse).

**Immutable observation:** after P0 (no observation writers), the table is a container with a documented append-only writer contract. Future writer must INSERT by (UID,revision), never UPDATE text in place; tests for P0 must ensure no P0 caller writes it. Do not add a DELETE/UPDATE trigger now that would pre-decide later expiry/GC semantics.

**Legacy FK preservation:** `library_legacy_attributions.legacy_file_id` deliberately has **no FK to files(id)**. A legitimate existing restore may update/delete the legacy row and cascade other legacy FKs; the attribution ledger must retain the historical ID snapshot even then. Conversely, `source_uid` has RESTRICT and can never be automatically destroyed by legacy delete. Every user tag/notes/AI/operation/copy history remains owned by its existing table, not copied/moved in P0.

## Exact migration integration

- `src-tauri/src/db/schema.rs`: add the new versioned migration after the existing `if version < 37`, using a small private `migrate_library_identity_foundation` helper; bump `CURRENT_SCHEMA_VERSION` **37 to conditionally reserved 38** only if updated master remains 37.
- The **existing** `BEGIN IMMEDIATE ... COMMIT/ROLLBACK` from `schema.rs:59-746` must contain all new DDL and `set_schema_version(38)`; no 100k file UPDATE, filesystem walk or FTS rebuild.
- At the beginning of the existing `if version == CURRENT_SCHEMA_VERSION` fast path, **verify** all four table names/expected schema shape + singleton row signature before any `ensure_*` mutation. An unknown user_version=38 created by #383 provisional must error with a stable, non-destructive validation code (e.g. `library_schema38_identity_foundation_conflict`). Never repair such a database in place merely by `CREATE TABLE IF NOT EXISTS`.
- On the `version < 38` path, fail closed on pre-existing, incompatible `library_*` table names and preserve current SQLite transaction rollback. No `DROP TABLE`, `ALTER TABLE files`, `UPDATE files`, `DELETE FROM files`, FTS trigger manipulation or backfill.
- For fresh databases starting before schema 37, let existing migrations run and append P0 one time. A database at user_version > expected version already must fail as today.
- The fast-path signature check must not cause an O(number of files) table scan or rewrite. Compare `sqlite_schema.sql` structure (or robust `PRAGMA table_info` + signature) plus singleton sentinel; exact SQL whitespace-insensitive canonical checks are acceptable when backed by tests.
- Do NOT reuse #383 `replace_files_au_trigger` or its provisional conditional FTS migration in this P0. That work needs future rebase/explicit different next-version authority.

## Production test contract (no tests claimed passed yet)

- `P0-T01` fresh DB v0 ->38 exactly four tables + singleton legacy authority and no activated reader; repeat `Database::open/init` idempotent.
- `P0-T02` representative Schema37 DB ->38 preserves exact legacy files, tags/FKs, fingerprint/group, content run/artifact and operation/organization user snapshots, counts/hashes and schema37 FTS row results; no bulk rewrite.
- `P0-T03` Schema36/16 representative fixture upgrade reaches 38 with original migration history working; no forced old binary matrix.
- `P0-T04` simulate injected SQLite DDL failure within the migration transaction: user_version, four names, tags/old FTS and legacy rows roll back exactly; no orphan singleton.
- `P0-T05` v38 database with #383's trigger but **without** P0 foundation must be rejected read-only/non-destructively; don't insert tables, trigger replacement or force version downgrade.
- `P0-T06` corrupted/colliding managed table definition in a Schema37 fixture fails without changing user_version or old data.
- `P0-T07` negative tests: no user tag inherited across same-path identity reuse; no delete-on-cascade from P0, no automatic Verified claim or UID relationship.
- `P0-T08` FK/check constraints: malformed UID, invalid enum, repeated observation revision, forged verified identity without evidence, attribution without proof rejected; use only disposable fixtures.
- `P0-T09` Schema38 new authority row remains exactly `('legacy',0,NULL,1)` under normal File Library list/search, tag mutation, scan batch and restore (existing behavior must remain unchanged).
- `P0-T10` real product compiled Rust/SQLite 3.51.3, FTS5/trigram, existing 5000ms timeout; migration SQL lock occupancy measured without scanning files; normal Windows/macOS CI gates honestly reported run/skipped.

## Owner go/no-go and stop

The exact SQL is an Owner-directed **candidate contract**, not a proof of compiled production migration. Permit Codex Cloud to implement P0 on an isolated Draft PR only when latest master remains Schema37 and the version arbiter has no new competing live ownership. If either changes, STOP before code and return to Owner. No feature switch, no backfill or new F1 production code even if all P0 tests green. Complete P0 report must say `#381_PRODUCT_FIX_NO_GO`, with full exact HEAD/Tree, diff, measured migration/FK invariants, failing legacy v38 detection, native/CI limits and Phase P1/P2 dependencies.

Current #383 and #400 remain unmerged Drafts; P0 is new independent candidate. No merge/Issue close/master modification/Codex Review/Release or expanded old-release compatibility research.
