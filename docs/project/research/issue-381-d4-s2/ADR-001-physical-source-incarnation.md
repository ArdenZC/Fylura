# ADR-001 Amendment — Physical Source Incarnation and Legacy Compatibility

**Track:** Zen Canvas #381 Managed Scan, D4-S2

**Status:** Candidate for Owner physical ADR review

**Production status:** NO-GO. This amendment is architecture and test-only research. It does not change production DDL, IPC, installer, or a schema version.

**Supersedes:** D4-S1 ADR-001 clauses for physical identity, legacy claiming, effective path projection, multi-root conflict handling, retention, and source ID mapping. The remaining D4-S1 logical visibility and source-audit clauses stand unless this amendment says otherwise.

## 1. Frozen inputs and evidence boundary

| Input | Exact value |
|---|---|
| D4-S1 HEAD / Tree | `bb46ba8bd0d4a2a01af976fb9b05d6c4224b4697` / `501bc4e54d9a391625888e4aab3485119e971f4b` |
| Master / Tree | `3275ac7c72df8ae4139a30f16c7f649971324e97` / `009576241cd4e8e2ba12cc644a368314f599faa6` |
| Schema | 37; frozen DDL evidence is D4-S0 `schema37-sqlite-schema.sql` |
| Existing direct FKs to `files(id)` | `file_user_tags`, `file_fingerprints`, `duplicate_group_members`, `content_artifacts`, `content_run_items` |
| Existing Scanner identity | `scanned_entry_to_insert_request` sets `id = entry.path`; `upsert_file_rows_tx` upserts on `id` |

The Harness builds a disposable SQLite database from the complete captured Schema 37 `sqlite_master` DDL, including FTS virtual tables, indexes, and triggers, then inserts synthetic historical rows. It is not a copy of a user's database and does not qualify platform filesystem identity. No production source, Schema 37 DDL, PR #383, PR #386, master, Hosted CI, or broad benchmark was changed or run.

Reproduce with:

```sh
python3 docs/project/research/issue-381-d4-s2/harness/run.py
```

Inputs, raw SQL, output, source IDs, SQLite source ID, and SHA-256 values are listed in `INPUTS.json`, `harness/physical-candidate.sql`, and `evidence/`.

## 2. Decision requested

Accept the following **physical candidate** for further design only:

1. Rebuild Schema 37 `files` once so `path` is no longer unique and add `source_id TEXT NOT NULL UNIQUE`. Keep one retained payload row per source incarnation; preserve every old `files.id`, every rowid, and every existing child `file_id` unchanged. In migrated rows, `files.id` remains a legacy storage/FK key even when its value is the old path. New rows use opaque generated IDs; the Scanner must stop assigning paths as IDs.
2. Add a durable `file_sources` crosswalk: opaque app `source_id` ↔ immutable `files.id`. A trigger creates the crosswalk from a new `files` row's required source ID; source/file bindings and `files.source_id` are immutable. `source_id` is the stable identity for visibility, API selections, and future revisions. A source row and its `files.id` are never reused for a different observed object.
3. Record immutable `file_path_entries` and physical-identity epochs separately. Root-generation membership binds a `source_id` to the exact path-entry evidence observed by that root. A path is never an identity key.
4. Keep Root Membership + Effective Visibility as the authority. Build an immutable, manifest-scoped path projection (`manifest_file_paths`) from the selected complete root generations. Its unique constraints detect path/source conflicts before pointer CAS. It is derived data, not an independent authority. A mutable singleton `path → source` table is rejected.
5. Keep legacy side tables on their existing, retained `files.id` foreign-key anchor for this migration. They remain attached to the old source. All new code resolves an API `source_id` through `file_sources`; it never reuses the old `files.id` merely because a path matches.

This preserves existing foreign-key values and old payload rows while making same-path, different-source rows physically representable. It still requires a large, coordinated reader/writer/API conversion. The Harness demonstrates SQLite data-preservation feasibility only; it does not authorize implementation.

## 3. Identity definitions and invariants

| Identity | Meaning | Stable across | Does not prove |
|---|---|---|---|
| `source_id` | One app-level source incarnation and the anchor for source-specific user state and derived results. | A verified rename/move of the same directory entry; membership in multiple roots. | Same path, equal content hash, matching basename, or an OS file ID observed after an unobserved absence. |
| `path_entry_id` | One directory-entry incarnation at a normalized path, with verbatim path text and the identity evidence captured at observation. | Membership in multiple roots that observed the same entry. | A hard link at another path, a path later reused, or a source identity by itself. A rename creates a new immutable path-entry observation linked to the same source only after identity verification. |
| `physical_identity_id` | A verified physical-object epoch, based on platform-specific handle/volume evidence. | Multiple hard-link path entries observed to refer to the same live object. | A permanent inode/file-ID value. OS identifiers can be reused after deletion and must not have a global uniqueness constraint. |
| `files.id` | Existing storage row key and current foreign-key target. | The lifetime of its source row and its retained associations. | A path after migration. Legacy rows may still have path-valued IDs, but new rows must not. |

One logical `source_id` represents one library path entry. A hard link at another path receives a different `source_id` and `path_entry_id`, even when both path entries cite the same verified physical-identity epoch. This avoids silently sharing path-level tags or user corrections across aliases. Dedupe may group the aliases; it does not merge their identity or metadata.

The physical-identity tuple `(platform_kind, volume_identity, platform_file_identity)` is indexed but not unique. An identity epoch is assigned only when continuity is supported by current native evidence. After an absence, inode/file-ID reuse, mount change, unsupported platform identity, or uncertain watcher history, allocate a new physical epoch or leave identity unknown. Content hashes are evidence for dedupe/content, never source identity.

## 4. Candidate comparison

| Candidate | Path reuse and existing FKs | Main cost/risk | D4-S2 disposition |
|---|---|---|---|
| **A. Keep Schema 37 `files` as the unique current-path projection; add source history elsewhere** | The old row must be archived before a replacement can occupy its unique path. Existing tag, fingerprint, duplicate, and content FKs point at the projection row; they must be rekeyed/copied to source history before update/delete. A partial failure can attach old data to the new projection or cascade it. | Most existing reads look familiar, but every source-bound relation needs migration and dual-write rules. Historical `files` fields (classification and user corrections) also need a second owner. | Rejected. The current row remains overloaded as both identity and location, preserving the original ambiguity.
| **B. Rebuild `files` as retained incarnation payload; remove `UNIQUE(path)`** | Keep legacy IDs and rowids, allow multiple incarnation rows with the same path, and retain current FKs to each row. A new path reuse inserts a new opaque row ID and source ID; old associations remain on the old row. | Requires a `files` table rebuild, a `file_sources` map, and conversion of Scanner plus all source-ID consumers. `files.path` becomes a last-observed snapshot rather than a uniqueness constraint. | Chosen as the durable payload and FK anchor.
| **C. Add an independent current-path projection** | A mutable global path table would be another live authority and could get ahead of or behind the manifest. A per-manifest immutable projection can represent one effective path per source and reject two sources claiming one path. | A projection copy/materialization may add write/WAL cost; all path queries and FTS must bind to the same manifest. Cost is unmeasured here. | Use only as **derived immutable `manifest_file_paths`**, built from root heads and atomically selected by the manifest pointer. Reject a mutable singleton projection as authority.

The choice combines B's retained source rows with C's immutable, manifest-scoped path projection. Root generations and the active manifest remain authoritative. The projection may be replaced by a derived SQL view if query plans and conflict checks can prove equivalent behavior; that is an implementation choice for a later authorized phase.

## 5. Physical DDL shape

The complete test-only SQL is `harness/physical-candidate.sql`. Production names remain candidates. The core relation and key rules are:

| Relation | Primary/unique keys | Foreign keys and purpose |
|---|---|---|
| `files` (rebuilt) | `id` PK; `source_id` NOT NULL UNIQUE; **no unique constraint on `path`** | Retained source payload. Existing child FKs continue to reference `files(id)`. `rowid` is copied to preserve the external-content FTS mapping. |
| `file_sources` | `source_id` PK; `file_id` UNIQUE | `file_id → files(id)`. One stable app identity maps to one retained payload/FK row. An insert trigger creates this map for every new file row; a binding guard rejects mismatches. New IDs are opaque and never path-derived. |
| `file_physical_identity_epochs` | `physical_identity_id` PK | Stores verified platform/volume/file-ID observations plus an app-managed epoch and evidence digest. No uniqueness on the OS identity tuple. |
| `file_path_entries` | `path_entry_id` PK; `(path_entry_id, source_id)` UNIQUE | Immutable path and identity observations. A new path entry is appended for a verified rename; the prior path entry remains. |
| `source_identity_claims` | `claim_id` PK | Audits native matching or an explicit user claim of an unverified legacy source to a selected native observation. |
| `source_global_index_links` | Link ID PK; source/global pair indexed | Evidence-bound, revision- and DB-generation-tagged bridge to `global_entries`. Path equality cannot create a link. |
| `source_generations` | `generation_id` PK; `run_id` UNIQUE | Links a root generation to existing `scan_roots` and `scan_runs`; keeps root revision, watcher revision, count, and digest. |
| `source_generation_members` | `(generation_id, source_id, path_entry_id)` PK | Positive observation membership; references source/path-entry pair. Omission implies absence only for a coverage-complete accepted generation. |
| `visibility_manifests`, `manifest_root_heads` | Manifest ID; `(manifest_id, root_id)` | Immutable complete root-head map. Existing roots remain independently owned. |
| `manifest_file_paths` | `(manifest_id, source_id)` PK; `(manifest_id, path_key)` UNIQUE | Derived projection from the manifest's effective generations; enforces one path per source and one source per normalized path in that manifest. |
| `active_visibility_pointer` | Singleton key `1` plus revision | CAS target. Only this pointer makes a ready manifest visible. |
| `source_reader_leases` | `lease_id` PK | Retains the manifest for a cursor, cross-transaction reader, plan, or job until release/expiry policy permits GC. |

Existing Schema 37 FKs to `files(id)` are deliberately not rewritten in this candidate. During this migration the stable `file_id` value remains their retention anchor, while new application contracts use `source_id`. The five direct FK tables are preserved and verified by the Harness. `classification_history`, `classification_feedback`, `operation_logs`, and analysis findings store IDs/paths without a direct FK; they remain history and must gain an explicit source link in a later coordinated data/API design if active source lookups are required. Global Index AI state remains keyed to `global_entries`, not a managed file row; a path match never creates a bridge.

The DDL includes a guarded source-row delete and an audit authorization shape for retention-aware GC. Root removal and stale visibility never authorize deletion. A future GC may remove a source only after no published manifest, retained generation, user-history policy, FTS reader, cursor, job, or lease needs it; eligible deletions and their policy must be audited. The candidate tables alone do not define the product's retention period.

## 6. Schema 37 migration and restart contract

1. Obtain the #380 exclusive database-generation gate, drain/reject older app, service, and IPC clients, checkpoint/backup the database, and validate backup plus WAL state. This gate is a prerequisite, not part of this D4-S2 implementation.
2. Capture the old `files` SQL, indexes, triggers, column list, and rowids. With foreign-key enforcement disabled on the migration connection before beginning the transaction, create a replacement `files` table from the frozen definition with `source_id TEXT NOT NULL UNIQUE` added and only `path UNIQUE` removed; copy all old columns and rowids and generate one opaque legacy source ID per row; drop/rename transactionally; recreate captured indexes and triggers; then create the candidate relation tables and legacy crosswalk.
3. Preserve every Schema 37 `files.id` verbatim as `file_sources.file_id`. Assign an opaque new `source_id` to each legacy row. Do not infer root ownership from string prefixes, `is_stale`, fingerprints, or a matching current path. Preserve all old `files` fields and all child rows. Keep `files.is_stale` as migration evidence only.
4. Mark every migrated source `legacy_unverified`, with no root membership and no active manifest path. An old non-stale row is offered in a labelled read-only compatibility/history slice; it cannot be tagged, renamed, opened through a managed action, classified, or treated as active until claimed. Old stale rows remain retained history.
5. Commit the table rebuild, crosswalk, journal marker, and `PRAGMA foreign_key_check` result as one SQLite migration transaction. On an exception or process exit before commit, SQLite must recover the original Schema 37 files table and associations. After commit, startup validates the journal, all `files.id → source_id` mappings, FKs, row counts/digests, FTS rowids, and migration marker before opening managed writes. Retry is a no-op after the committed marker; no schema version is allocated by this ADR.
6. An old client is not allowed to open and mutate the post-migration database, even if the executable path or version label appears unchanged. If recovery cannot prove which generation owns the database, all writes fail closed and a verified backup or the coordinated forward-recovery path is required. Do not reverse the table rebuild by dropping source/history data.

The test used SQLite DDL transactions and verified rollback and process-exit recovery. The exact product migration still needs a supported SQLite build, Windows/macOS restart tests, real backup/WAL behavior, and the coordinated #380 client gate.

## 7. Path and legacy truth table

| Situation | Source/path-entry result | User data and visibility |
|---|---|---|
| Schema 37 legacy row, `is_stale=0`, no identity/ownership proof | Create `source_id`; preserve old `files.id`; record only a `legacy_unverified` path snapshot. No generation membership. | Read-only compatibility/history slice. `files.is_stale=0` does not make it active. No destructive action or automatic tag mutation. |
| Schema 37 legacy row has `is_stale=1` | Same retained `source_id` and old row; no current path claim. | Historical only. A new same-path object is not that source. |
| Scanner sees an unclaimed file at the legacy path | Create a new source and verified path-entry from current native evidence; new `files.id` is opaque. | New file gets no old tags, notes, classification, history, fingerprint, content, duplicate, or AI state. The old source remains unverified history. |
| User explicitly chooses “claim this legacy record as this observed file” | Keep the selected legacy `source_id`, append a verified path entry, and append a `source_identity_claims` audit record with actor and authorization token. Reject if verified evidence conflicts with a previously established identity. | Existing user state stays with that explicitly selected source. This is a user-authorized association, not scanner inference. |
| User chooses “import as a new file” or identity conflicts | Create a new `source_id`. A separate explicit transfer UI may copy selected tags/notes with provenance; it must not relabel the source identity. | No implicit transfer. Old associations stay with old `files.id`. |
| Same physical directory entry is renamed/moved and native handle evidence is continuous | Keep `source_id` and payload/FK row; append a new immutable `path_entry_id`; old path entry remains history. Update path display/search projection in the same accepted manifest publication. | Tags, notes, content and history remain attached to the same source. Cross-volume copy/delete or an unobserved gap does not meet continuity; create a new source unless platform evidence proves otherwise. |
| Hard-link alias added at another path | New `source_id` and `path_entry_id`; link both path entries to the same verified physical-identity epoch. | Path-level user data is separate. Dedupe may report a hard-link alias; no tag/AI/content inheritance. |
| Same path is observed by two roots with compatible identity | Both complete generation memberships reference the same source and exact path-entry. Projection emits one source/path. | Removing one root leaves the source visible through the other published root. |
| Same path is claimed by two roots with conflicting identity | Conflicting generation is rejected/quarantined before publication. | Prior manifest and all associations remain unchanged; no merge or metadata transfer. |
| Same `source_id` appears at two different paths in effective root heads after a cross-root move | Candidate manifest fails the unique `(manifest_id, source_id)` projection check. Publish a session that reconciles all affected roots together, or retain the prior complete manifest. | No nondeterministic path winner and no duplicate active library row. |
| A root is removed or one root's complete generation omits a member | New complete manifest drops only that root head/membership. Other root memberships continue to count. | If no effective root retains the source, active queries hide it; source row and every association remain. Root deletion is not file deletion. |
| Any included root fails, cancels, loses coverage, or has a Watcher gap | The candidate session pointer does not move. Previous heads for every included root remain. | A caller may request a new explicit subset-root session; partial success is never reported as one atomic session. |

## 8. Unified association, search, and retention semantics

| Domain | D4-S2 rule |
|---|---|
| File Library and path actions | Read source rows through the pinned manifest projection; return `source_id`, manifest revision, and effective availability. Raw `files.id`, `path`, and `is_stale` cannot authorize a row, selection, or action. A path action reopens the native object and checks root/volume/reparse boundary immediately before mutation. |
| Tags, notes, classification, feedback | Remain anchored to the same retained `files.id`, with `source_id` crosswalk. Effective tag counts and mutations join a pinned manifest. New same-path sources do not inherit. User edits are never overwritten by root publication. |
| History and operation logs | Preserve existing snapshots and old IDs. Their current-source lookup is history-only until a source ID/provenance binding is established; path equality does not attach a record to a replacement. |
| Fingerprints and duplicate groups | Keep fingerprint rows and group membership against the old retained `files.id`. Physical keys support dedupe evidence only. Active groups/counts join effective source membership; one root removal cannot invalidate a source still present through another root. |
| Content artifacts and content FTS | Keep artifact rows, raw text, policy, provenance, and run history. Candidate/by-ID/search responses require the same effective manifest and matching source/content revisions. Root removal hides active content; it does not delete or make it current for a replacement. |
| Analysis/AI | Preserve findings, evidence, provider outputs, and corrections as source/path snapshots. A job must carry source ID, source revision, manifest revision, input/domain revision, and policy revision. Global Index `ai_analysis_state` remains independent; bridge rows are explicit, source-ID and physical-evidence bound, and coordinated with #380. |
| File FTS | Raw `files_fts MATCH` may still contain retained/historical terms. It is candidate generation only; all result IDs, snippets, counts, and cursors must join the same pinned manifest before exposure. FTS trigger rowids survive the table rebuild because rowids are copied. |
| BM25 corpus | Required product ranking semantics are BM25 over exactly the managed-visible sources in the pinned manifest (or a separately Owner-approved alternative). Filtering hidden rows after BM25 does not make its IDF corpus equal to the visible corpus. Schema 37 FTS currently indexes all file rows; therefore search semantics remain a production blocker until exact SQLite 3.51.3 query-plan/correctness work proves a compliant implementation or Owner explicitly accepts a different corpus definition. No broad benchmark is authorized here. |
| Cursors and cross-transaction readers | Cursor token includes manifest ID/revision, query fingerprint, sort state, and a lease. On pointer change, either continue a pinned history snapshot under lease or return a stable expired-token response; never combine pages from different manifests. |
| WAL and GC | A reader lease retains every referenced manifest/generation/source row and needed FTS state. GC mark/sweep is chunked, repeatable, and rechecks pointer, roots, jobs, runs, history policy, and leases. Unknown reader/job ownership means retain. Space pressure may stop staging; it cannot delete source history to complete publication. |

The existing Schema 37 cascade FKs are safe only while source rows are retained. The migration candidate's delete guard blocks ordinary `DELETE FROM files`; a later GC must have an explicit audited authorization after retention and reference checks. Stale visibility and root membership changes never invoke cascade deletion.

## 9. Revision fences and concurrent writers

| Revision | Writer and fence |
|---|---|
| `manifest_id/revision` | Active reader pins once per SQLite snapshot. Publication CAS compares the expected parent pointer; cursor and selection carry it. |
| `root_revision/path_revision` | Root enable/retire/path/volume/policy change. Stage is rejected or rebuilt on mismatch. |
| `watcher_revision/applied_revision` | Durable event arrival and application. Publish only if the staged generation represents all events through its captured sequence and has no gap. Any pre-commit change rejects the entire candidate. A post-commit event marks the root degraded and enters the same ownership/reconciliation path. |
| `run_revision/session_revision/lease/cancel` | Existing Scanner CAS and operation ownership. Run must be complete, uncancelled, coverage-eligible, and still owned by the publishing session. One failed included root prevents the pointer swap. |
| `source_revision/path-entry/evidence digest` | Identity/path mutation or accepted explicit claim. Source membership and a job's read set compare exact source/path-entry identity; count-only checks are insufficient. |
| `metadata_revision` and `tag_revision` | Metadata/tag writers use their own expected revision and transactional target-set check. Root publication does not rewrite those rows. A selection from a changed manifest refreshes/rejects; a committed user edit is retained. |
| Content/fingerprint/dedupe/analysis/AI domain revisions | Worker captures source, input hash, domain/policy revision, manifest ID, and cancellation state. Completion rechecks visibility and revisions in its write transaction; mismatch stores superseded history or rejects active publication, then retries from a fresh snapshot. |
| #380 database-generation token | Every service/IPC/old binary admission must prove the compatible DB generation before reads that can lead to writes. Missing or stale token rejects open/write; process name/path is not identity. |

Filesystem observations cannot be atomically committed with SQLite. Handle checks narrow the gap but do not eliminate an event that occurs after the last check and before COMMIT, especially with delayed or lost notifications. The database contract is “last complete, fenced observation accepted by the pointer,” not omniscience about the filesystem. Destructive OS actions revalidate at action time. Unknown boundary, root ownership, or identity fails closed.

## 10. Failure and recovery requirements

| Failure | Required result |
|---|---|
| Table rebuild or candidate DDL fails before COMMIT | SQLite rollback restores Schema 37 `files.path UNIQUE`, all original rowids, child rows, triggers/indexes, and FTS terms. No source crosswalk is visible. |
| Process exits during migration transaction | Startup recovery sees either the intact pre-migration DB or the fully committed migration marker. It never treats partial DDL as migrated. The recovery Harness exited after DDL and before COMMIT, reopened the DB, then reran the migration. |
| Migration reruns after successful commit | Marker and schema/data digest verify; migration is a no-op. It does not generate a second `source_id` map or duplicate legacy associations. |
| FTS rowid mismatch or FK check error after rebuild | Do not admit managed writes. Rebuild/recover only under the exclusive #380 gate from a verified backup or forward migration plan. |
| Watcher gap, path reappearance, same-path conflict, nested-root boundary change | Preserve active manifest and old history; require reconciliation. Do not move tags/records or infer a missing member. |
| Partial multi-root failure | Preserve every previous head in the candidate session. A separately requested subset session may later publish its explicitly included roots atomically. |
| Tag/metadata/AI job races a publish | SQLite order plus domain CAS prevents cross-source writes; user data stays on its original `source_id`; stale manifest-bound selection rejects. |
| Cursor/reader pins old manifest; WAL or disk grows | Retain the snapshot and WAL pages. Backpressure staging/publication; do not GC a pinned generation or history row. |
| GC crashes mid-sweep | Explicit authorization and short chunks make the operation idempotent. Published/pinned rows remain; unknown ownership retains. Recovery resumes only after refcount/lease revalidation. |
| Old Schema 37 client or service opens post-migration database | #380 generation admission blocks before SQL mutations. No fallback to path-as-ID or `is_stale` authority. |
| Global Index and managed source disagree | Keep authorities separate, do not join by path. Quarantine bridge work until explicit identity-bound reconciliation. |

## 11. #380 installation, IPC, and schema-version dependency

No schema number is assigned. PR #386 and its installer/database-generation design remain independent and frozen; this ADR does not choose version 38 or an ordering between #381/#380 changes.

Before production work, both Owners must review one combined migration contract and assign a single schema-version owner. The combined design must specify backup/WAL recovery, old-client drain/rejection, process/IPC generation handshake, migration ownership, crash resume, and rollback by a verified whole-database restore. `#380` must prove old binaries cannot access the upgraded DB through an already-running Service, a second app process, or an unchanged executable path. Only after that proof may `files.id` stop being path-as-ID or a new manifest become authoritative.

The shared order remains: approve #380 DB-generation gate → approve #381 physical and logical authority → coordinate one combined migration and supported-client matrix → authorize implementation → test crash/restart and old/new client rejection → Owner accepts a single rollout/rollback plan. Until then, master remains Schema 37 and all production tracks remain NO-GO.

## 12. Harness result

The small, repeatable Harness passed against the captured Schema 37 schema and current workspace SQLite:

| Check | Result |
|---|---|
| Schema 37 source DDL SHA-256 | `b8405a675f05e55f40fa19af14b76f2390e22466b6fb185bc25ef743e5255cd5` |
| Candidate DDL SHA-256 | `df3fc906526823c47c5b1236ead105d2529163766c873e8acd693760ad65002f` |
| Harness runner SHA-256 | `03cb943d2b4c872ab7e99164c5f7c0462fe6c44c09ed1c65985a94d00ab12213` |
| Runtime | SQLite `3.53.1`, source ID `2026-05-05 10:34:17 c88b22011a54b4f6fbd149e9f8e4de77658ce58143a1af0e3785e4e6475127e9` |
| Migration | First run committed; second run `already_applied`; synthetic `PRAGMA user_version` stayed 37 |
| Historical relationships | Tags, classification history/feedback, fingerprints, duplicate groups/members, content artifacts/FTS, analysis findings/evidence, operation logs, and Global Index AI state preserved |
| Value integrity | SHA-256 digests for 13 original tables (`files` including rowid, plus 12 association/history tables) were identical before and after the rebuild |
| Old/new isolation | A new source occupied the same path with a different physical identity; no old tags, fingerprints, content, duplicate membership, or source identity were attached to it |
| Other identity cases | Explicit legacy claim audited; one-root removal retained visibility through the second root; full removal hid without deleting; verified rename kept source ID with a new path entry; hard-link paths used separate source IDs over one physical identity |
| Failure cases | Injected constraint failure rolled back the full DDL; process exit before COMMIT recovered old Schema 37; startup rerun committed once; conflicting path projection left pointer unchanged |
| Checks | `foreign_key_check=ok`, `integrity_check=ok`, external-content FTS integrity check `ok` |

See exact assertions and raw output in `evidence/schema37-physical-harness-results.json` and `evidence/schema37-physical-harness-run.log`. This is a migration model proof with synthetic data, not a performance result or native OS identity proof. It did not run on SQLite 3.51.3, Windows, or macOS.

## 13. Production GO / NO-GO

**D4-S2 recommendation:** the physical identity/history model is feasible enough for Owner review; production remains **NO-GO**.

Owner acceptance and separate implementation authorization require:

- agreement on the exact meaning and public lifetime of `source_id`, path-entry rename continuity, hard-link alias separation, and cross-root move conflict behavior;
- migration of the full Schema 37 table/FK list, including non-FK history and derived records, with all source consumers switched from path-as-ID to source-ID mapping;
- #380-exclusive DB/client generation gating and one jointly reviewed, version-unassigned migration plan;
- exact SQLite 3.51.3 FTS/BM25 plan and visible-corpus correctness; the current full-row FTS corpus is unresolved;
- platform-native evidence for volume identity, rename, hard links, ID reuse, symlink/Junction/reparse/mount boundaries, ACL change, and delayed Watcher delivery on Windows/macOS/Linux;
- tests for cancellation, Watcher changes, tag/metadata/AI races, multi-root failures, old-client rejection, reader leases, WAL pressure, disk-full, GC, and forward recovery;
- separate performance qualification for the candidate manifest path projection and writer waits. D4-S2 did not benchmark it.

Unresolved risks include an OS change between final verification and SQLite COMMIT, product FTS/BM25 corpus semantics, migration behavior with a live real database and external readers, Schema-version coordination with #380, and whether existing history tables gain durable source links or remain unlinked history. None is hidden by the Harness result.

**Requested stop state:** `#381-D4-S2 READY_FOR_OWNER_PHYSICAL_ADR_REVIEW`.
