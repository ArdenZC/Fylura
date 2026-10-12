# #381-D4-S7 — Production Integration Readiness Report

**Requested stage result:** `#381-D4-S7 READY_FOR_OWNER_INTEGRATION_READINESS_REVIEW`
**Production recommendation:** **NO-GO.** This report is an inventory and execution plan only; it does not authorize D4 implementation.
**Owner authorization:** [#381 D4-S7 review](https://github.com/ArdenZC/Zen-Canvas/issues/381#issuecomment-6125801466)

## 1. Audited revision and evidence boundary

The independent S7 branch starts from the frozen D4-S6 HEAD `895acaa2f0afc0f2c3f554e799107fdd55b05eb4`, Tree `8770037a9f9428066532e00cc8218d3c1f8485c7`. Production code was inspected at current local `origin/master` `3275ac7c72df8ae4139a30f16c7f649971324e97`, Tree `009576241cd4e8e2ba12cc644a368314f599faa6`. The master audit commit/tree and Schema 37 source hash match the exact master baseline already frozen in D4-S1. No production checkout was changed.

The prior D4-S1 text inventory was reused because its source commit is byte-identical to this master. It scanned 247 Rust source files, recorded 262 `is_stale` reference lines across 40 files, grouped 103 production/schema entries, and separately recorded 433 literal write references in 141 production/schema groups. The exact `function-inventory.csv`, `stale-reference-inventory.csv`, `mutation-function-inventory.csv`, `mutation-reference-inventory.csv`, and frontend consumer list are linked from [`docs/project/research/issue-381-d4-s1/source-audit/`](../issue-381-d4-s1/source-audit/). Their SHA-256 values and revision appear in [`evidence/INPUTS.json`](evidence/INPUTS.json).

Those generated lists are text-level heuristics, not a Rust call-graph or SQL parser. The [S7 consumer matrix](CONSUMER-MATRIX.csv) manually ties the important production functions and SQL to registered Tauri commands, API calls, triggers/views, service/background work and required migration tests. This is a static audit, not runtime evidence. D4-S7 ran no tests, CI, Windows/macOS, full validation, benchmark or Codex Review.

## 2. Current production authorities

### File Library and `files.is_stale` / `files.path`

Schema 37's `files` table starts with `id TEXT PRIMARY KEY` and `path TEXT NOT NULL UNIQUE` (`src-tauri/src/db/schema.rs:65-79`). The current Scanner sets both `InsertFileRequest.id` and `.path` to the normalized path (`src-tauri/src/scanner.rs:1928-1939`). There is no durable file-to-scan-root membership relation in the production schema; root ownership is reconstructed from `scan_roots` and path-prefix SQL. `files.is_stale` is the current effective File Library availability bit.

The main File Library V2 call chain is:

```text
FileLibrary UI/store
  -> src/api/libraryApi.ts:39-53
  -> Tauri invoke query_file_library_v2 / get_file_library_detail / reveal_file_library_entry
  -> src-tauri/src/main.rs:390-399 registrations
  -> src-tauri/src/db/commands.rs:93-145
  -> Database::query_file_library_v2 / get_file_library_detail / resolve_file_library_path
```

`query_file_library_v2` (`db/queries/library/mod.rs:727-887`) builds page/count SQL from `build_query_parts` (`:1406-1458`): optional `files_fts MATCH`, `bm25(files_fts,6.0,1.5)`, a `files` rowid join, `f.is_stale=0`, and the resolved root path scope. It returns one SQLite read-transaction revision and a query cursor. The current cursor binds a singleton `library_query_state.revision`, query/sort anchor and rank; it rechecks count and anchor tuple, not a complete result digest (`:559-572`, `:578-592`, `:1158-1165`, `:1906-1973`).

The by-ID detail path is materially different: `get_file_library_detail` selects `FROM files WHERE id=?` with no stale condition (`:949-1023`), then reads tags, active findings, content and duplicate group data. It can return a stale indexed row, with `isStale` as a field; the frontend uses that field to render an unavailable state. Path resolution does reject `is_stale=1` (`:1026-1040`), and reveal calls that resolver (`db/commands.rs:135-145`). Under the accepted D4 model, active detail and actions must resolve the effective manifest; history-only access needs a separate read-only route.

The legacy API remains registered in `main.rs`: `search_files` and `get_paged_files` (`:373`, `:389`), plus `insert_file`, `upsert_files_by_paths`, and `remove_files_by_paths` (`:370-372`). `db/queries/mod.rs:79-104` builds a path-scoped CTE with `f.is_stale=0`; `files.rs:885-960` applies FTS/BM25 and offset pagination. These are not automatically manifest-aware because V2 exists. They are old-client bypass routes until they are migrated, wrapped in a proven adapter, or rejected by the #380 generation gate.

### Managed Scan, Watcher and writers

`start_managed_scan` is registered from `scanner.rs` (`main.rs:497-505`) and flows into scan admission, `run_managed_session`, `run_scan_run`, `flush_scan_batch`, and DB persistence. `upsert_file_rows_tx` (`db/queries/scan.rs:2861-2900`) upserts by `id`, sets `is_stale=0` and clears content hash if path/size/mtime/type changed. `reconcile_missing` (`:1609-1675`) validates run/root/session revisions, coverage, cancellation and Watcher revision, then updates `files.is_stale=1` for unseen paths. `apply_watcher_exact_mutations` (`:1283-1360`) takes an immediate transaction, resolves paths under the root, inspects current filesystem metadata, upserts or sets the same global stale bit, invalidates stale data and bumps revisions. Watcher routing and reconciliation are driven from `watcher.rs:1334-1405` and its retry/revision helpers.

These are useful existing fences for the future generation writer, but a row-global stale transition cannot express “root A lost this membership while root B still proves the source.” Root and Watcher events therefore must become source/path-owned generation membership changes; the accepted CAS and event-gap behavior must remain, with a single manifest visibility fence.

### FTS, Global Index and BM25

`files_fts` is an external-content FTS5 index over `files.name/path`, rowid-backed and trigram-tokenized (`schema.rs:2780-2797`). `files_ai` and `files_ad` maintain insert/delete rows. `files_au` unconditionally deletes and reinserts on every update (`:2801-2817`). The current File Library query filters stale rows after the FTS table computes BM25. Thus no stale row is returned from that query, but hidden rows can still influence the BM25 corpus statistics and visible result scores. A separate manifest-pointer update will not fire the `files` triggers.

Global Search is a separate product authority, not a File Library view. `search_global_entries` validates and snapshots an offset request (`global_index/commands.rs:10-46`); `global_index/search.rs:54-101` combines exact-name, prefix, extension and FTS tiers. Its FTS query ranks `global_entries_fts` and filters enabled `global_volumes` plus `global_entries.is_stale=0` (`:441-468`). It has its own source/index health and pagination, independent of the managed file root manifest. Only the existing explicit managed-scope/AI bridge should acquire a manifest-aware source ID; path equality is not a valid bridge.

Content search is a third index: `content_artifact_fts` stores summary/keywords/language/raw text in an internal FTS5 table (`schema.rs:2296-2367`). Candidate collection filters `files.is_stale` (`content.rs:3159-3200`), and extraction revalidation compares stale/path/size/mtime/hash (`:3424-3515`). Query results filter current artifact status and scope, and join `files` (`:1212-1305`). `content_artifacts_file_changed` marks artifacts stale only after selected `files` columns change (`schema.rs:2349-2357`). A root-manifest change without a row update bypasses that trigger, so active content reads and FTS need an explicit effective-source predicate and content revision binding.

### Tags, notes and history

`user_tags` and `file_user_tags` retain tags through an FK to `files(id)` (`schema.rs:1900-1923`). `list_user_tags` and tag update/delete expected-use checks count raw association rows (`db/queries/library/tags.rs:4-105`); `mutate_file_user_tags` mutates target files selected through the library selection query (`:107-175`). D4 must preserve the existing association and show active counts from effective sources without transferring an association to a same-path replacement. The tag writer and publication must compare their own revisions/target snapshot.

No standalone per-file Notes table, File Library note API or file-note IPC command was found in the audited master production schema/routes. There is a `note` on `analysis_finding_decisions` (`db/queries/analysis/mod.rs:1398-1405`); it belongs to a finding decision and must stay with that finding. “File notes” are not listed as a current feature in this inventory.

Historical readers must not be filtered by current availability. For example, `get_operation_logs` reads `operation_logs` ordered by `created_at` without joining `files` (`db/queries/operations.rs:7-48`), and `classification_history` holds file name/path snapshots (`schema.rs:335-363`). Analysis findings/evidence and content/dedupe run records have their own durable ledgers. These remain historical records after a source is hidden. Any action obtained through history (especially Restore) must resolve an explicitly bound current source and recheck native identity; an old file ID/path cannot attach history to a path replacement.

### Dedupe, storage analysis, classification, AI and Organization

- Dedupe candidate collection excludes stale rows and uses path/source facts (`db/queries/dedupe/mod.rs:1028-1036`). Hash writes use a path/size/mtime/stale CAS (`:901`, `:941-1018`). `invalidate_stale_files_in_transaction` changes `files.content_hash`, fingerprints, groups and findings when files become stale (`:1749-1770`). Under root membership visibility, one root's removal cannot globally stale a still-visible source; active groups/counts must join the manifest and retained group/fingerprint state remains history.
- Managed analysis candidate/read paths use `files.is_stale` plus root path-prefix scoping (`db/queries/analysis/mod.rs:1277-1309`); exact duplicate subjects check stale and fingerprint identity (`:2122-2205`). Findings and decisions need source/manifest CAS and remain historical when inactive.
- Rule classification and AI classification query candidates through `db/classification/engine.rs:260-285,353-379,457-496` and `ai/classification.rs:538-565`. Rule proposal impacts and validation are stale-filtered (`db/queries/rule_proposals/mod.rs:900-1038,1230-1248`). Their output writers, learned feedback, source snapshots and canceled/running jobs all need source and manifest revisions.
- Organization plans materialize IDs, paths, stale/duplicate state and assessment projections (`organization/materialize.rs:5-24`, `organization/mod.rs:2409-2422`, `organization/projection.rs:158-177`, `organization/semantic.rs:108-183`). An old plan must not execute on a new same-path source. Preserve review/history; revalidate source/manifest and physical identity before operation execution.
- Managed AI jobs in `global_index/managed_worker_hardened.rs` are keyed by Global Index entry/AI queue revisions. The bridge `global_index/repository.rs:833-856` reads managed File Library rows. Keep these authorities distinct; queue, claim and completion must validate the explicit source mapping and current manifest, not a path match.

### IPC, Service, Native Bridge and frontend

`src-tauri/src/main.rs:368-505` is the command registration boundary. `db/commands.rs` wraps File Library and metadata operations; `scanner.rs`, `watcher.rs` and Global Index commands are backend/native entrypoints. The API facade uses `invokeCommand`; frontend is not the authority. Existing frontend raw stale consumers are enumerated at exact lines in the linked S1 `frontend-consumers.csv`: File Library list/detail/Quick Look, content sheet, permanent delete guards, organize suggestions, preview adapters and DTO fields. They must render effective availability from a revisioned backend response; action APIs revalidate. Browser mock stale state remains test/demo data and cannot stand in for native acceptance.

## 3. Unified authority read/write map

The accepted D4 target is a single published manifest over complete Root Heads, with root-scoped positive source/path membership and effective visibility. Convert the consumer matrix in this order:

1. **Identity and compatibility base:** keep every old `files.id`, rowid and `file_id` side-row association. Crosswalk each retained row to an opaque source ID; unknown legacy rows are `legacy_unverified`, read-only for active actions and absent from root membership. New path reuse creates a new source and opaque file ID. A verified same-entry rename preserves source identity and appends a path entry. Hardlink paths remain separate sources even with a shared physical identity. The physical storage mapping and no-transfer rule are accepted in D4-S2 research; see its [truth table](../issue-381-d4-s2/MIGRATION-TRUTH-TABLE.md).
2. **Membership writer:** Scanner full-coverage runs and Watcher exact events create root-owned source/path membership generations with run/session/root/Watcher/source revision CAS. A failed/canceled root or event gap retains the prior root head. Cross-root visibility is the union of independently valid effective memberships; losing A alone does not stale B.
3. **Single publication boundary:** one DB pointer selects a complete manifest for the intended root set. All selected root/source seals and path projection rows are checked before pointer CAS in the same transaction. Root/source CAS conflict or incomplete coverage rolls back. Publication changes active visibility, not user rows or history.
4. **Active readers:** File Library query/count/detail/selection/path, File FTS, content FTS, tag counts/filters, dedupe groups, analysis candidates/findings, classification/AI eligibility, Organization plan validation, operation previews and Global Index bridge switch to the same manifest-bound visibility relation. Global Search itself remains independent.
5. **Independent user and derived writers:** tags/metadata use their own revision plus a manifest-bound target set; AI/content/classification/dedupe/analysis/rules/organization completions compare captured source/manifest/domain/policy revisions. A publication does not clear metadata or blindly rewrite side tables. Mismatches preserve outputs as history/superseded and force refresh/retry.
6. **Historical reads and retention:** operation/classification/analysis history stays readable without current visibility filtering. Old path and source snapshots never rebind by equality. Active result lookup and any filesystem action requires an explicit current source link. No stale publication deletes files, tags, history, fingerprints, artifacts, groups, or AI evidence.
7. **Cursor, FTS and GC:** global cursors reject any changed effective global result authority. Root-scoped cursor survival is allowed only when its root/source seals and exact ranked result are unchanged. Readers/cursors and in-flight jobs must be represented in retention before GC can collect old generations or FTS rows.

The current master has **no** manifest, source ID, root-to-file membership, publish pointer or cross-table visibility predicate. It must not expose a half-migrated mode where some consumers read manifest and others read raw `files.is_stale`.

## 4. #380 dependency and migration/rollback blockers

The migration order and evidence required from #380 are in [MIGRATION-DEPENDENCY-DAG.md](MIGRATION-DEPENDENCY-DAG.md). The key concrete blocker is old-client exclusion. The Schema 37 command table still registers path-ID and raw stale writers. If a Schema 37 app or Service can open a post-migration database, a manifest cannot be authoritative: an old upsert can reoccupy `files.path`, a stale update can change FTS/content triggers without advancing the manifest, and a tag/action writer can use legacy IDs.

The latest local #380 status evidence captured by D4-S1 is: #380-S0 research accepted; one synthetic NSIS-only S1 observability run authorized; U2 production design NO-GO; PR #386 OPEN/Draft; Schema v38 unallocated. S7 did not query live GitHub state, so the exact current Owner record must be reconciled before a GO decision. Per this phase's gate, no #380 product-generation contract has passed. D4 therefore cannot migrate, allocate a schema number or start production implementation.

Before migration, #380 must prove a generation handshake and drain/reject rule for old app/Service/IPC writers, including already-running processes and same-path executable replacement. Then both Owners must accept one combined migration set, one version owner, one ordering, one client matrix and one rollback/recovery plan. Do not privately allocate Schema 38.

Before opening writes after migration, verify the whole DB transaction/marker, old row/rowid and association counts/digests, FKs, triggers, FTS integrity and legacy unverified state. Failure before commit leaves Schema 37 intact; failure after migration validation but before compatible client admission requires verified restore or forward repair under exclusive generation lock. Once new-generation writes occur, silently downgrading to Schema 37 risks losing new data and corrupting identity: only forward recovery or a stopped-client whole verified backup restore with its write-loss consequence acknowledged is acceptable.

## 5. FTS and Cursor readiness

The exact test plan, inputs and pass rules are in [FTS-CURSOR-VALIDATION-CONTRACT.md](FTS-CURSOR-VALIDATION-CONTRACT.md). Main unresolved points:

- The product target is SQLite 3.51.3; only exact linked product build/source ID is valid. The previous anomaly was broad BM25 timing from a D4-S0 research fixture and is not repaired by S7.
- External-content `files_fts` includes hidden rows; filtering returned rows does not give BM25 an effective visible corpus. Accepted research requires either proof that scores use the effective corpus or a separately Owner-approved corpus definition.
- A pointer flip does not fire `files_au` or `content_artifacts_file_changed`; manifest revision must be directly bound into query, content, count and cursor authority.
- D4-S6 research computes a digest over the complete ranked result tuple list each token request. That is O(number of matches) and unmeasured at production scales. Production Cursor acceptance requires correctness and approved 100k/500k/1M resource/latency budgets.
- Global cursors expire when effective global membership/rank changes. Root B may keep a cursor through root A changes only if B remains the SQL/token scope, B seals and shared-source identity remain current, and the exact visible B FTS result/ranks do not change. Otherwise reject.

S7 gives no product performance number or PASS claim. The repo inputs contain no Owner-approved File Library search/Cursor p95 budget; a future large run must receive this target before execution.

## 6. GO / NO-GO checklist

### Integration readiness review: GO for Owner review

- [x] Frozen S6 baseline and exact master source revision are recorded.
- [x] Real production consumer and writer paths, important SQL predicates/triggers and frontend/IPC chains are listed in the [matrix](CONSUMER-MATRIX.csv); exhaustive S1 literal inventories are referenced.
- [x] Absent per-file Notes capability is called out instead of invented.
- [x] Active-current reads are separated from historical reads and independent Global Index authority.
- [x] Migration ordering, client gating, rollback protections, FTS/Cursor plan and no-v38 coordination are explicit.
- [x] No production code/schema or prohibited CI/benchmark/platform activity was performed.

### Production D4: NO-GO

- [ ] #380 generation admission, Service/IPC drain and old-client rejection Owner-accepted and proved.
- [ ] Combined #380/#381 version-unassigned migration plan and rollback contract Owner-approved.
- [ ] Final physical manifest/source/cursor contracts approved for production, beyond research acceptance.
- [ ] Every consumer and writer is migrated from raw `files.is_stale`/path identity; no legacy IPC bypass remains.
- [ ] Exact SQLite 3.51.3 FTS/BM25 semantic and plan gate passes; broad BM25 anomaly resolved.
- [ ] Cursor digest costs and query budgets pass at 100k/500k/1M.
- [ ] Windows/macOS native identity and failure/recovery/GC/platform validation gates pass.
- [ ] Separate Owner authorization for D4 production implementation exists.

The first checklist is the S7 endpoint requested. The second remains entirely open. `READY_FOR_OWNER_INTEGRATION_READINESS_REVIEW` must not be interpreted as D4 production GO.
