# Issue #381-B1 — Source-owner and Effective Visibility Entry Disposition (Owner review draft)

**Date:** 2026-10-13 · **Phase:** ACTIVE DEVELOPMENT · **Status:** B1_STATIC_DISPOSITION_WITH_SOURCE_EXCEPTIONS / OWNER_REVIEW_NEEDED / PRODUCT_CODE_NOT_AUTHORIZED

**Current master checked:** `3275ac7c72df8ae4139a30f16c7f649971324e97`, tree `009576241cd4e8e2ba12cc644a368314f599faa6`. **Actual #383 candidate:** `584621c32054e16ac0a61afc669a04380a976b7f` (OPEN/Draft/Design Blocked). **Frozen evidence:** D4-S8 `4005073337f198a245773e30a76178749bfd2920` → D4-S8R `a48f8849b1ba1baadcf84e217d505e2298cf35a1`; original S8/S8R must stay byte-for-byte unchanged. **No public release has occurred.** The frozen report's repeated #380 old-binary-generation prerequisite is a *historic pre-release misunderstanding* and is NOT a prerequisite for current #381 development. Current runtime writer/source safety remains required.

## Purpose, scope and method

B1 is a **triage of existing frozen static entry IDs**, not a second repo-wide inventory or a new AST. It enumerates every frozen `production/schema` **103 stale + 121 write + 54 direct files-read = 278** group IDs, all 209 registered Tauri commands and 10 real front-end consumers, preserving frozen scope, original symbols and source coordinates. The S8 ledger ID arrays independently matched the S8R ID-set proof exactly (no ID absent/extra). The two attached CSVs are keyed one-row-per-ID and are intended for follow-up source-local Owner tests, not direct Codex instructions to edit all rows.

**Critical source correction:** 16 entries frozen as "production/schema" are **definitely test-only** by exact current-master Rust `cfg` declarations. See the exception table below. This means the frozen totals **overstate the definitely verified runtime surface**: after removing the **known** test-only entries, provisional upper counts are at most **97 stale / 111 writes / 54 direct reads** until the remaining codepaths get direct provenance checks. These are **not** certified exhaustive production counts; other static misses or false positives may exist. No historical S8/S8R file is changed.

## Confirmed test-only contamination

| IDs | Pinned source proof | Consequence |
| --- | --- | --- |
| STG-114, STG-115, STG-116, STG-117, STG-118, STG-119 | `src-tauri/src/global_index/search.rs:241-300` and `:481-482`: SQL/diagnostic constructors each decorated `#[cfg(test)]` | These six are **test-only**, not runtime Global Search visibility consumers; retain as regression coverage |
| WFG-195, WFG-196, WFG-197, WFG-198, WFG-199, WFG-200, WFG-201, WFG-202, WFG-203, WFG-204 | `src-tauri/src/file_ops.rs:1008-1009`: enclosing `#[cfg(all(test, windows))] mod tests`; all listed owner lines fall within that module | These ten are **Windows test-only**, not production `files` writers; keep actual operation execution safety separately as a real gate |

This is an evidence correction, not an accusation that frozen S8/S8R's ID-set integrity hashes failed: a self-consistent inventory can still be semantically misclassified. Do **not** edit accepted frozen S8/S8R ledgers; apply the overlay at B1 only.

## Grouped B1 provisional responsibilities (278 frozen group IDs)

| B1 lane | Entries | Required responsibility |
| --- | ---: | --- |
| DERIVED_INPUT_READ | 57 | AI/content/dedupe/analysis/organize/rules consumer must revalidate source visibility plus its independent result/run identity. |
| ACTIVE_VISIBILITY_READ | 46 | Active File Library/operation read must consume a coherent EffectiveVisibility token, including count/selection/FTS if relevant. |
| DERIVED_REBUILD_OR_REVIEW | 45 | Derived fingerprint/content/analysis/AI/plan/queue changes cannot be blindly applied after visibility epoch shift; invalidate/refresh asynchronously and gate unsafe action. |
| SOURCE_RECONCILE_WRITER | 33 | Scanner/Watcher/upsert/reconcile source writer; publication and stale cleanup require owner/generation/revision CAS and bounded lock. |
| CROSS_AUTHORITY_BRIDGE | 23 | Global Index-managed scope/AI bridge must explicitly reconcile independent source and Managed effective visibility. |
| GLOBAL_INDEX_INDEPENDENT | 22 | Independent global_entries/Global Search source authority; no global index policy change from Managed Scan epoch. |
| TEST_ONLY_SOURCE_EXCEPTION | 16 | Manual source-confirmed cfg(test) or cfg(all(test,windows)): regression fixture; not a production code migration target. |
| SCHEMA_FTS_TRIGGER_GATE | 11 | Schema, files_fts external-content, SQL triggers, DTO/rowid and FK authority needs explicit contract before pointer-only publication. |
| USER_METADATA_WRITE_CAS | 8 | Tags/feedback/user metadata retained independent of visibility; targeted writes require live source ID and CAS, never broad stale GC. |
| USER_FILE_WRITE_CAS | 8 | File-path/restore/mutation command must bind source identity, current visibility, user confirmation, physical identity and revision. |
| SOURCE_PROOF_REQUIRED | 6 | Static entry proof insufficient; source-local data owner/call chain must be checked before implementation. |
| ACTION_EXECUTE_GATE | 3 | Durable operation/plan execution is not a background projection; action-time source identity/visibility, journal and filesystem checks required. |

These are **provisional primary lanes**, not one-hot program effects. A read guard may feed AI, and a scan writer may influence FTS; both duties must be tested through the owning semantic contract. `PINNED_SOURCE` means source coordinates or ownership were rechecked, **not** that the implementation has passed native/behavioral tests. `SOURCE_RECHECK` means verify the exact handler/source and writer ordering before any code plan. Details: [B1-GROUP-DISPOSITION.csv](B1-GROUP-DISPOSITION.csv).

## IPC and frontend scope (219 entries: 209 commands + 10 frontend)

| B1 lane | Entries |
| --- | ---: |
| IPC_CALLPATH_PENDING | 169 |
| D16_INDEPENDENT_SERVICE | 8 |
| D16_EXPLICIT_WORKSPACE | 22 |
| D16_STORAGE_ACTION | 10 |
| FRONTEND_DTO_GUARD | 9 |
| FRONTEND_LEGACY_COMPAT | 1 |

The exactly **40 D-16-only commands** are separated into **8 independent service/system**, **22 explicit user-selected File Workspace**, and **10 Storage Analyzer/journal** capability commands. These are *not* automatically 40 Managed Scan commands; Storage Analyzer actions still require finding/journal, physical file identity and source-overlap validation. The remaining **169 registered commands** are placed in `IPC_CALLPATH_PENDING` until their **actual** handler-to-repository call paths are traced; no inference of production changes from co-membership in D-16. Frontend includes **10** production consumers; FE-011 remains a fixture and is not counted. Details: [B1-IPC-FE-DISPOSITION.csv](B1-IPC-FE-DISPOSITION.csv).

## Source-grounded implementation choke points

1. **Active Library/selection/count/FTS**: `src-tauri/src/db/queries/mod.rs:79-103` centralizes `scoped_files_sql*` with `f.is_stale=0`; `db/queries/library/mod.rs` queries listings/details/counts, BM25, facets, paginated selection, tags and saved views. Any epoch overlay must route *every eligible active read* through a consistent snapshot or query token, or fail closed; a renderer-only guard is insufficient.
2. **Direct file operations and metadata**: `db/queries/files.rs:27-65` checks `is_stale=0` for identity/naming; `:68-155` upserts by global `files.id`/path with user metadata precedence. Tags have FK `file_user_tags.file_id REFERENCES files(id) ON DELETE CASCADE` in `db/schema.rs:1917-1927`. Never delete/migrate old files to implement stale or rebind tags via path equality. `file_ops.rs` contains *real production execute functions before its cfg-test module*, but the ten **frozen WFG-195..204 listed functions are tests**.
3. **Scan transaction bottleneck**: PR #383 `db/queries/scan.rs:1693-1934` advisory preflight/staging; `:1941-2277` final `BEGIN IMMEDIATE` includes candidate-set and physical boundary revalidation, 100k `UPDATE files.is_stale`, dedupe/fingerprint/content/analysis invalidation, revision updates and commit. Historical Hosted Windows **5,284ms** lock residency > unchanged **5,000ms** writer timeout. D3R Linux median change only ~1.4%; no Windows fix demonstrated.
4. **FTS external-content**: `db/schema.rs:2769-2845` defines `files_fts` as external-content `content='files'`; its `files_au` fires on name/path/rowid changes, **not** on an independent visibility epoch pointer. `BM25` ranking and `LIMIT` can be wrong if visibility is applied only *after* retrieving an over-broad result set; Owner must settle visible-corpus ranking semantics before code. Raw FTS terms need not disappear merely because source Effective Visibility changes.
5. **Root ownership**: `scan_roots` carries root `current_generation`, `active_run_id`, `revision`, `last_successful_generation` (Schema37); `files` has global ID/path. Root overlap, file rename/path reuse, Watcher intervening update and second source must be tracked separately. No per-root bulk stale conclusion from a name or stale timestamp alone.
6. **Derived trust and action authority**: D-11/D-12/D-13/D-14 require current-visible input checks and run/epoch/revision fencing for dedupe, analysis, content and AI; historical rows remain inspectable but not automatically actionable. Operation/Restore and explicit File Workspace/Storage Analyzer actions re-check OS file identity independently.
7. **Global Index boundaries**: D-06 `global_entries`/volume health/Global Search are independently governed, not silently replaced by Managed Scan visibility; cross-bridge managed entries and AI queue need explicit two-authority reconciliation. No ISG policy or default Smart Lean change.

## Hard questions before B2 test-only prototype

**H1 — Overlapping roots:** Are both managed-root membership claims independently durable, so one root's completed scan cannot hide a file still visible under another root? Show source & path/physical-identity crosswalk, not merely string prefix.

**H2 — Concurrent user writes:** Can an older staged missing-candidate set change visibility after a later Watcher/upsert/tag/note update, even if file path appears identical (ABA)? Define exact per-file/writer/source revision or safe monotonic watermark; if impossible, block the epoch publish.

**H3 — Query consistency:** Can list+count+pagination+selection+FTS/BM25 execute against the same committed visibility epoch, with stale cursors rejected or restarted and no post-LIMIT silent underfill?

**H4 — Derived users and FS actions:** At epoch change, are stale Dedupe/Content/AI/Analysis candidates instantly ineligible for irreversible actions until their own revisions are reconciled, without erasing prior user annotations?

**H5 — Crash/recovery:** Does a crash before and after pointer CAS recover to exactly one published visibility state? Can bounded cleanup resume idempotently without resurrecting untrusted metadata or doing a hidden file deletion?

**H6 — Timed writer contention:** Does Hosted Windows with real 100k fixture and concurrent real metadata writer complete with margin under unchanged 5s budget and satisfy action/FTS/source identity tests? Verify native macOS as well. No SQLite lock-timeout inflation.

**H7 — Honest scope:** S8/S8R are static regex/source-location inventories, not an AST, macro/dynamic SQL expansion or real call graph. B1's 16 confirmed exceptions MUST be reconciled. New unmapped consuming authority or ambiguous ownership means B2 is held for owner investigation.

## Owner decision and next gate

- **Accept:** B1 static ID overlay, sixteen source-proven test-only exceptions, top-level domain lanes, precise source hotspots, 209-command governance separation, and explicit unknown callpath list.
- **Do not claim:** complete production coverage, working epoch algorithm, O2 correctness/performance, passed 100k Windows writer liveness, public upgrade compatibility requirement, Schema v38 allocation, production fix of #381, or full CI/native PASS.
- **B2 candidate:** narrowly scoped **test-only Oracle** for effective visibility with overlapping selected roots, file-writer ABA, epoch CAS, FTS paging and on-action source revalidation, **only after separate Owner B2 authorization**; no production schema, Service/IPC, Watcher or index defaults. Before B2, owner must choose whether FE cursor/bm25 semantics are fixed and confirm source revisions needed for the Oracle.
- **Research decision:** report B1 as `SOURCE-GROUNDED_STATIC_OVERLAY_READY_FOR_OWNER_REVIEW`, **coverage provisional**. Frozen accepted D4-S8/S8R remain immutable, but the known source-label inaccuracies must be preserved as corrections.

**No master mutation, PR merge, Issue close, release, Code Review automation, native Owner-machine development or product-code implementation is authorized by this research artifact.**
