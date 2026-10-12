# Migration Dependency and Rollback Matrix

## Current #380 facts used by S8

Per the latest Owner decision supplied for this phase:

- `#380-S1R4-P2 WINDOWS_PATH_MATRIX_ACCEPTED`; the Windows Hosted path matrix is **40/40 PASS**.
- `P3-P0_PREPARATION_ONLY_GO`.
- `NSIS_RUNTIME_NO_GO / U2_PRODUCTION_NO_GO`.
- Old client, Service, IPC and database-generation isolation is **not complete**.

The 40/40 path result accepts that bounded path matrix only. It does not prove client-generation admission, old-process drain, DB handle isolation or an installer/runtime production path. The older #380-S0/S1 status retained in D4-S1 `INPUTS.json` is historical evidence and is not the current project status used here.

## Required dependency order

| Order | Gate | Required proof | If absent |
|---:|---|---|---|
| 1 | #380 database-generation admission | Every app, renderer, Tauri IPC connection, resident Service, worker and open DB handle is bound to the DB generation; old generation is rejected before any read that can lead to a write. Include already-running/second process and same-path executable replacement. | D4 production migration cannot start; `files.is_stale` and path-ID writers remain able to bypass a pointer. |
| 2 | #380 installer/service runtime and recovery | NSIS/service handoff, drain/reject, DB backup/WAL handling, restore/downgrade generation association and restart behavior are Owner-accepted. | No binary/database safe rollout or rollback proof. Current gate remains `NSIS_RUNTIME_NO_GO / U2_PRODUCTION_NO_GO`. |
| 3 | Joint #380/#381 migration decision | Both tracks agree on one Schema version owner, combined migration set, ordering, supported-client matrix, backup, post-commit recovery and rollback limit. Schema v38 remains unallocated until this decision. | No schema DDL, trigger migration or client matrix can be frozen. |
| 4 | D4 physical/effective authority | Owner accepts source incarnation, Root Membership + Effective Visibility, manifest, cursor, history, global bridge and native identity contracts. | A partially converted table/query set could expose or hide inconsistent sources. |
| 5 | Implementation authorization | Owner separately authorizes exact candidate and test work after prerequisites. | Research acceptance is not production authorization. |
| 6 | Stop/drain and migration rehearsal | Stop or reject old generation; backup/reopen exact Schema 37 DB including WAL; rehearse conversion on disposable copy with row/rowid/FK/tag/history/FTS counts/digests. | Do not open writes under new authority. |
| 7 | Atomic migration and validation | One coordinated transaction and marker; preserve all legacy rows/associations; check source crosswalk, foreign keys, integrity, triggers, FTS/rebuild and migration idempotency. Unclear legacy rows remain read-only. | Keep DB closed or restore verified pre-migration backup before new-generation writes. |
| 8 | Convert all writers/readers/actions/triggers | All active routes use source IDs and Effective Visibility; all async writers use source/manifest/run/cancel/policy CAS; no legacy fallback. | Pointer is not authoritative until every route is converted and old clients cannot bypass. |
| 9 | Runtime/SQLite/native/recovery/scale gates | Product-linked SQLite 3.51.3, Windows/macOS native identity, FTS/Cursor/Content, crash, disk/WAL/GC and Owner-set scale budgets pass on authorized exact inputs. | Keep D4 production NO-GO. |

## Schema 37 physical risks that the migration owner must preserve

| Current fact | Consequence for migration |
|---|---|
| `files.id TEXT PRIMARY KEY` is assigned from the enumerated path in `scanner.rs:1928-1939`. | It is a path identity, not source incarnation identity. A same-path replacement must not inherit the old identity or side data. |
| `files.path TEXT NOT NULL UNIQUE` in `db/schema.rs:65-79`. | Old and new path incarnations cannot coexist in the current table. Any future physical mapping must retain both history and active path projection without transferring old associations. S8 specifies no DDL. |
| `files_fts` uses `content='files'`, `content_rowid='rowid'` and `trigram`; `files_ai/ad/au` mirror `files.rowid` (`schema.rs:2780-2817`). | Keep a verified rowid/source crosswalk and FTS re-index/rebuild/integrity checks; do not assume new opaque source IDs are FTS rowids. |
| FK/side data use `files.id`, including `file_user_tags`, operation records, content artifacts, fingerprints/groups, findings and other records. | Preserve and explicitly map every old ID/FK. A path equality join to a new source is forbidden. Never delete or reassign historical associations during hide/publication. |
| Old commands/services write legacy path-shaped IDs and stale state. | `files` migration cannot be admitted while a Schema 37 client or open DB handle can write. #380 is a hard prerequisite, not a cleanup task after D4. |

## Migration and rollback requirements

1. Before migration, capture a verified DB+WAL backup and prove it opens. Record exact app/Service/database generations and stop or reject old writers.
2. Rehearse on a disposable representative Schema 37 copy. Inventory all FK relationships and source payload/rowid/association/FTS mapping before any DDL. Ambiguous legacy ownership remains `legacy_unverified`; path, prefix or stale bit is not a destructive claim.
3. Any migration interruption before commit must leave Schema 37 intact. A duplicate retry must be idempotent. No app opens a partially converted database.
4. Before new-generation writes, failure may recover by verified whole-backup restore. After the new client has written, silently downgrading to Schema 37 is unsafe; use forward recovery or stop every writer and restore the whole pre-migration DB with explicit acknowledgement that later writes are lost.
5. No GC/pruning may remove old files rows, tags, history, content, fingerprint/duplicate evidence or FTS data to make migration/publication easier. Unknown reader/cursor/job ownership means retain.

No step above is executed in S8. No Schema number is reserved or allocated.
