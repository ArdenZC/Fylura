# Consumer Coverage Reconciliation

## Result

All requested source records now have stable row IDs and an explicit handling disposition. S8 expands S7's 17 summary domains into the committed S1 raw and grouped source entries, then adds direct file-table reads and the complete Tauri command registry. The full row-level crosswalk is `ledgers/DOMAIN-TO-ENTRY-MAP.csv`; exact entry details are in the eight linked ledgers below.

| Ledger | Rows | What each row names |
|---|---:|---|
| [Stale function/schema groups](ledgers/STALE-GROUP-LEDGER.csv) | 149 | 103 source-reconciled production/schema groups and 46 test groups, with owner lines, raw line lists, domain, visibility disposition, preserved data, contract and responsibility. |
| [Stale literal references](ledgers/STALE-REFERENCE-LEDGER.csv) | 262 | Every source line containing `is_stale`, its group ID, exact source text, scan role and disposition. |
| [Write function groups](ledgers/WRITE-FUNCTION-LEDGER.csv) | 236 | The original 141 production-labeled and 95 test groups, reclassified by containing source module and target table. |
| [Write literal references](ledgers/WRITE-REFERENCE-LEDGER.csv) | 433 | Every regex-matched write line, exact table/source text, parent function ID, corrected scope and handling. |
| [Direct files read groups](ledgers/FILES-READ-GROUP-LEDGER.csv) | 99 | Functions/modules with a literal `FROM files` or `JOIN files`, including ID/path lookups and active/historical classification. |
| [Direct files read references](ledgers/FILES-READ-REFERENCE-LEDGER.csv) | 125 | Each individual `FROM/JOIN files` source line and parent function group; 72 production/schema lines and 53 test lines. |
| [Tauri command registry](ledgers/TAURI-COMMAND-REGISTRY.csv) | 209 | Every command registered by `src-tauri/src/main.rs` at master, resolved to its command definition and routed to an authority/test contract. |
| [Frontend consumers](ledgers/FRONTEND-CONSUMER-LEDGER.csv) | 11 | Every S1 frontend stale-state consumer, including mock and DTO entries. |

The code inventory says a production function group is a distinct `(path, function, function line)` tuple. Individual regex hits remain separate IDs. “Production/schema” is a source-scope classification, not a claim that a reference is a live visibility filter or that a function executes in every configuration.

## S1 false-positive and classification adjudication

### Write scanner test-scope false positives

S1's writer scanner treated `tests.rs` filenames and `trigger_tests/` children as production because those child files do not repeat their parent `#[cfg(test)]` attribute. S8 checked the parent module declarations in the same production snapshot:

| Child source | Parent `#[cfg(test)]` declaration | Formerly mislabeled entries | S8 disposition |
|---|---|---:|---|
| `src-tauri/src/ai/cleanup/tests.rs` | `src-tauri/src/ai/cleanup.rs:577-578` | 4 SQL references / 4 groups | test fixture |
| `src-tauri/src/db/automation/tests.rs` | `src-tauri/src/db/automation/mod.rs:21-22` | 7 / 5 | test fixture |
| `src-tauri/src/db/automation/trigger_tests.rs` | `src-tauri/src/db/automation/mod.rs:24-25` | 1 / 1 | test fixture |
| `src-tauri/src/db/automation/trigger_tests/publication.rs` | child of the cfg-gated `trigger_tests` module | 4 / 2 | test fixture |
| `src-tauri/src/db/automation/trigger_tests/recovery.rs` | child of the cfg-gated `trigger_tests` module | 6 / 3 | test fixture |
| `src-tauri/src/db/automation/trigger_tests/service.rs` | child of the cfg-gated `trigger_tests` module | 4 / 2 | test fixture |
| `src-tauri/src/file_workspace/integration/tests.rs` | `src-tauri/src/file_workspace/integration/mod.rs:22-23` | 1 / 1 | test fixture |
| `src-tauri/src/global_index/tests.rs` | `src-tauri/src/global_index/mod.rs:25-26` | 3 / 2 | test fixture |
| **Total** |  | **30 / 20** | Retained in ledgers; excluded from production migration counts. |

The “groups” above are from the S1 function-level grouping, not unique test cases. The generated ledger keeps the S1 group IDs and `s1_scope`, then sets `reconciled_scope=test/fixture` using the owning module. This changes the corrected counts to 121 production/schema groups and 115 test groups, with 210 production and 223 test literal SQL references. It is a scan-scope correction, not a production-code finding.

### `is_stale` is not a single authority

The 103 non-test function/schema groups include distinct meanings. The S1 inventory has 33 groups whose source is in Global Index and whose `is_stale` belongs to `global_entries`; these are independent of Managed File visibility. One group (`enqueue_managed_ai_for_library_files`) explicitly bridges Global Index and the Managed File Library and requires both authorities. Schema declarations, FTS/content triggers, migration compatibility, DTO fields and row projections are not all active SQL predicates. The row ledger retains their exact text and treats them as follows:

- Managed File active readers, scopes, selections, details, previews and operation guards must use the published Effective Visibility relation in the future D4 integration. A raw stale field is not sufficient.
- Projection/DTO references are outputs, not independent membership checks. They must carry derived effective availability and the relevant manifest/revision token once an Owner-approved implementation exists.
- `global_entries.is_stale`, its FTS, volume state and jobs remain a separate Global Index authority. Only explicit managed-bridge routes resolve the current Managed File source and root state.
- Schema and trigger references describe database-maintained effects. They require explicit pointer-publication invalidation because a manifest pointer is not a `files` row update.
- Test fixtures are evidence/tests, not production entrances; keep relevant cases as regression contracts.

### Direct query reconciliation

The supplemental read scan matches the literal SQL tokens `FROM files` and `JOIN files` in the pinned Rust source. It found 125 references in 99 function groups; after source-scope correction 72 references in 54 groups are production/schema, and 53 references in 45 groups are test fixtures. Each production row is assigned to the current file reader, an active derived worker, a Global Index bridge or an owning history/API exemption. This inventory narrows the file-table read surface but is not a substitute for reviewing dynamic SQL, query builder fragments or Rust calls that receive a `file_id` without spelling `FROM files`; those call chains are called out in S7 and the command ledger.

## Special bypass paths and current source facts

### Tauri / IPC and by-ID/path operations

The [command registry ledger](ledgers/TAURI-COMMAND-REGISTRY.csv) reconciles all **209** registered symbols in `main.rs:368-578`; each definition path/line was found, so there are no unresolved command definitions. The following five legacy DB commands are registered at `main.rs:370-374` and wrap path-shaped identity or legacy query contracts in `db/commands.rs:18-89`:

- `db::insert_file`
- `db::remove_files_by_paths`
- `db::upsert_files_by_paths`
- `db::search_files`
- `db::get_paged_files`

These are the clearest post-migration bypasses. An old client or process can insert/update the path-ID row, mutate stale state or search without a manifest cursor. They must be rejected or compatibility-adapted only under a jointly approved generation gate; UI hiding is not a guard.

Additional high-risk routes in the full registry include `get_file_library_detail`, `get_file_library_selection_summary`, `reveal_file_library_entry`, `get_operation_previews_by_file_ids`, permanent-delete previews, `file_ops::reveal_in_folder`, move/restore/recovery operations, and tag batch mutation. The active File Library detail repository at `db/queries/library/mod.rs:949-1024` reads by ID and loads associated data; path resolution at `:1026-1032` separately rejects stale rows. Once current visibility is manifest-based, the active detail endpoint must verify visibility before returning an active-file DTO. A historical detail/read must be a separate explicitly historical API. Every reveal/delete/restore triggered from a retained history record must resolve a current source ID and revalidate native file identity and boundary immediately before the OS operation.

`mutate_file_user_tags` and repository batch writes (`db/commands.rs:187+`, `db/queries/library/tags.rs:107-175`) modify user metadata, not membership. The tag catalog remains independent. A selection-bound batch must validate the selection's captured manifest and every source ID in the write transaction; a manifest change causes rejection/refresh, not reassignment. Tags are never deleted or transferred when a source becomes hidden or a path is reused.

### Scanner / Watcher / recovery and late writers

`scanner.rs` creates `InsertFileRequest.id` and `.path` from the same entry path (`:1928-1939`). `db/queries/scan.rs` owns root/run/session writes, Watcher application (`apply_watcher_exact_mutations`, `:1283-1360`), missing reconciliation (`reconcile_missing`, `:1609-1675`) and file row upserts (`upsert_file_rows_tx`, `:2861-2900`). These are future Root Membership / generation producers. Their status mutations must not publish partial authority and must retain their run/root/Watcher revision CAS under the approved model.

Content, Dedupe, Analysis, classification, rule/Organization and AI jobs can finish after the file or root changes. Their S1 rows are grouped by exact function and table in the write ledger. A successful historical worker result may be retained, but its active projection is only current when source incarnation, effective visibility, run/cancellation, domain policy and captured revisions still match. A late completion may not resolve a replacement via equal path. Dedupe invalidation currently affects file-level fingerprints/groups; root A losing a member must not invalidate a still-valid source through root B.

The Global Index bridge is explicit at `global_index/repository.rs:833-856` (`enqueue_managed_ai_for_library_files`) and must preserve two IDs/authorities. Global Search's own `global_entries`, FTS, volume state and `is_stale` are excluded from Managed File Effective Visibility. A managed bridge queue/claim/completion may act only after current source/manifest validation.

### Trigger and derived-state gap

Master Schema 37 does not contain a manifest pointer or pointer listener. The relevant row-trigger surfaces are:

- `files_ai`, `files_ad`, `files_au`: `db/schema.rs:2804-2815`; `files_au` runs after **any** files UPDATE and rewrites the external-content FTS row.
- Content catalog triggers `content_catalog_artifact_insert/update/delete`: `:2331-2348`; `content_artifacts_file_changed` at `:2350-2357` reacts to a files update. A root-pointer-only visibility change would not fire it.
- `active_duplicate_membership` view at `:1319+` derives from duplicate tables and current file state.
- Global Index FTS/count/AI-job triggers at `:1696-1708`, `:1806-1868` observe `global_entries`/`ai_jobs` and remain independent.

Thus no trigger on master listens for a future Manifest Pointer. Pointer publication must explicitly revise query/cursor/content/derived visibility authority, or approved reads must always resolve it. A pointer-only change cannot be treated as though `files_au` or `content_artifacts_file_changed` ran. Existing content, FTS and Global Index records remain retained; this inventory proposes no trigger or DDL changes.

## Schema 37 path and history risks

1. `files.id` is currently also path-shaped because Scanner assigns path to `id` (`scanner.rs:1928-1939`). Code and FKs treat that ID as durable; equal paths are not stable identity.
2. `files.path` is `NOT NULL UNIQUE` (`db/schema.rs:65-79`). The current physical table cannot represent retained old and new incarnations at the same path at once.
3. `files.rowid` is the external-content `files_fts` row key (`schema.rs:2780-2817`). Rebuilding/mapping rows must preserve or explicitly crosswalk rowid and `files.id`, all FKs, tags, operation/classification/analysis history, content, fingerprints, duplicate membership and AI/Organization snapshots.
4. Same-path replacement must be a new source identity. Existing user metadata and historical artifacts remain bound to the old source and never auto-attach to a new row by path.
5. Legacy rows with no trusted root/identity evidence remain `legacy_unverified` and read-only under the accepted research contract. Explicit claim and verified observation are separate from migration; path-prefix or stale-bit backfill is not proof of membership.
6. Any old application, Tauri command, renderer, worker or resident Service that can open the migrated DB with Schema 37 semantics can bypass the manifest. This blocks the first production migration write.

## Record disposition

No source entry remains unassigned: stale raw lines map to STG groups; write raw lines map to WFG groups; direct `files` queries map to RFG groups; every Tauri symbol resolves to a command definition; all 11 frontend records map to their contract; and the 17 domain rows link to those IDs. Test/fixture records are explicitly excluded from production migration, Global Index and history rows have named exemptions, and all other current managed-file paths target Effective Visibility or a revision-bound action boundary.

The remaining risks are readiness blockers, not unknown inventory entries: no Schema 38 allocation, incomplete #380 client/Service/database-generation isolation, product SQLite 3.51.3 FTS semantics, Owner-approved BM25 corpus and performance budgets, native identity qualification, and production migration/recovery authorization.
