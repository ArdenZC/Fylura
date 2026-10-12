# D4 Production FTS, BM25 and Cursor Validation Contract

**Phase boundary:** plan only. D4-S7 did not run a benchmark, product SQLite experiment, Hosted Windows/macOS job, or full CI. The D4-S0 bundled-SQLite BM25 anomaly remains a separate blocker. Product qualification must use the linked product SQLite 3.51.3 build, not the 3.53.1 research host.

## Current master behavior to qualify

| Surface | Current SQL/index behavior at master `3275ac7…` | Consequence for D4 |
|---|---|---|
| File Library FTS | `src-tauri/src/db/schema.rs:2780-2817` defines external-content `files_fts(name,path,content='files',content_rowid='rowid',tokenize='trigram')`; `files_ai/ad/au` keep rowid terms. `files_au` deletes and reinserts on every `files` UPDATE. | An `is_stale` update does not remove the row's name/path terms. Candidate queries must apply effective visibility before exposing any ID, page, count, preview or cursor. A root pointer change will not invoke these row triggers. |
| File Library BM25 | `db/queries/library/mod.rs:1417-1450` materializes `files_fts MATCH ?`, computes `bm25(files_fts,6.0,1.5)`, joins `files` by rowid, then adds `f.is_stale=0` and a path-root scope. `files.rs:2422-2455` uses the same weights for the legacy search CTE. | Hidden rows are removed from returned rows, but BM25 statistics still come from the full FTS table. This is not the accepted effective-corpus contract unless Owner explicitly accepts whole-index ranking. Exact SQLite 3.51.3 query-plan and score behavior is unqualified. |
| Content FTS | `schema.rs:2296-2367` creates `content_artifacts` keyed by `file_id` and `content_artifact_fts` over artifact/summary/keywords/language/raw_text. `content.rs:1212-1305` filters current artifacts and MATCH but does not itself join a manifest. Candidate and extraction revalidation currently check `files.is_stale`; a file-update trigger marks artifacts stale. | Manifest-only visibility changes do not fire `content_artifacts_file_changed`. Active content pages, by-ID detail, cursor, and FTS result must check the same effective source manifest and artifact revision without deleting historical artifacts. |
| Global Index FTS | `global_entries_fts` and `global_entries.is_stale` are separate; `global_index/search.rs:441-468` uses `bm25(global_entries_fts,8.0,2.0,1.0)`, enabled-volume/stale filters, and a tiered offset-based result. | Do not replace Global Search with managed visibility. Any managed bridge remains an explicit cross-authority check. |
| Current File Library Cursor | `library/mod.rs:559-572` token has query/sort anchor and `library_query_state.revision`; the query transaction returns `snapshot_revision`. `validate_cursor_authority` rechecks current count and the anchor tuple/rank, not a digest of every ranked result. | This is a global library revision contract, not the D4 root-scoped cursor. Future cursor format needs manifest/root/source seals and must never re-run against a captured old FTS snapshot. |
| D4-S6 research Cursor | `issue-381-d4-s6/harness/run.py:555-599` enumerates visible MATCH rows, including `bm25` values, then hashes the complete result tuple list for global and root-scoped tokens. The prior synthetic correctness experiment did not measure this path at 100k/500k/1M. | Per-page full-result validation is O(M) in matching rows, plus result materialization, sorting and serialization. Its cost must be measured before adopting it in production. |

The accepted D4-S2 ranking contract defines BM25 over exactly the managed-visible sources in the pinned manifest, unless Owner approves a different corpus definition. Filtering results after `bm25()` is insufficient to satisfy that definition. A hidden row that remains in `files_fts` may alter document frequency, average document length or scores for visible results. The validation must demonstrate the chosen corpus contract, not merely assert that hidden IDs are absent from the returned page.

## Correctness inputs

Use the exact proposed combined schema/query commit and the exact product-linked SQLite 3.51.3 build. Before any run, capture:

- app and migration source SHA, schema DDL and trigger hash, DB `user_version`, database/page/WAL sizes, and all relevant FTS/index/statistics definitions;
- `SELECT sqlite_version(), sqlite_source_id();` and `PRAGMA compile_options;` from the actual library used by the app; fail the run if the library is not the approved product build;
- the full bound product SQL, bind values, query fingerprint, scope/root IDs, page size, cursor token version, manifest/root/source revisions and a deterministic seed;
- a test database created from the actual Schema 37 DDL and candidate migration, with related tables, external-content FTS triggers and tags/history/analysis/content/dedupe associations. A simplified `files` table is not an acceptable input.

The 100k/500k/1M fixture should include deterministic mixed paths and names, overlapping roots, same-path old/new incarnations, hidden/stale and `legacy_unverified` rows, sparse/common search tokens, long names/paths, current/stale content artifacts, tags, and retained history. Use independent expected row-ID/source-ID and rank oracle outputs. Any synthetic scale data must be labelled as such; it cannot establish Windows/macOS identity correctness.

## Exact correctness test sequence

1. **Baseline and visible-set oracle:** execute the current product File Library query and candidate Effective Visibility query on the same fixture and captured read snapshot. Compare exact visible source IDs, counts, path projections, filter sets, deterministic order and all page boundaries. No hidden, unverified, failed-root or superseded source may appear.
2. **External-content FTS integrity:** record `rowid/name/path` source rows and `files_fts MATCH` candidates; run the FTS5 external-content integrity check (the equivalent of `INSERT INTO files_fts(files_fts, rank) VALUES('integrity-check', 1)` on the linked build). Rebuild the external-content index, close the DB, reopen it, and compare the same candidates and final visible results. Repeat for rename, delete, rowid preservation and path reuse.
3. **Hidden-document ranking:** hold visible B source rows constant. Add, remove, hide and change a hidden A document that contains the query token; compare `bm25` score and order for B. The visible-corpus contract passes only if hidden A cannot affect B's score/order or if the approved design computes the documented effective corpus. If it changes the rank, record a failure; do not treat a filtered result page as a pass.
4. **History separation:** retain a stale/unverified old source with old terms and operation/classification/analysis history, then insert a new identity at the same path. Active File Library and active FTS must return only the new visible source when its manifest admits it; historical reads retain old snapshots. No old tag/artifact/history result may attach to the new source by path.
5. **Content FTS:** search `content_artifact_fts` with current and stale artifacts under the same root/source manifest. Pointer hide must exclude the old artifact from active search, detail, counts and cursors without deleting it. A changed artifact revision invalidates its content cursor. Reopen/rebuild verifies content FTS rows still correspond to retained artifact IDs.
6. **Pointer and cursor races:** issue page 1, change a root/source/tag/content revision between transactions, then request page 2. Global Library Cursor expires whenever the effective global root/source collection or its ranked results change. A root-B-only cursor may survive root A changes only if the actual query is constrained to B, B's root/source/path seals are unchanged, and the exact visible ranked B result set and every rank are unchanged. A shared-source identity change always rejects B's cursor. Each page reapplies current effective visibility; never replay an old projection/FTS snapshot.
7. **Restart and recovery:** close all connections after stage, after pre-publication readiness, and after commit; reopen under the same compatible database generation and re-run oracle/query/FTS integrity checks. Before commit only the old pointer may be visible; after commit only the complete new pointer may be visible. Old cursor tokens reject or use an independently proven reader lease, never silently cross snapshots.

## Query-plan and correctness evidence (plan only in D4-S7)

For each exact production statement, log `EXPLAIN QUERY PLAN` using the actual bound parameters and the product library. Capture at least:

- File Library first-page FTS query, exact/deferred count, relevance keyset continuation, and root-only query;
- the stale Schema 37 query and the proposed manifest join on the same rows;
- content FTS search and Global Index FTS as independent controls;
- the exact current product materialization mode, FTS MATCH, BM25 expression, visibility join, root/source seals, rank sort and limit.

Record virtual-table MATCH constraint usage, join order, materialization, full scans, temp B-trees, automatic indexes, and which predicates constrain candidate rows before return. The plan review passes only when the actual SQL returns the correct oracle set and the team can explain the 3.51.3 plan. Host SQLite 3.53.1 output cannot substitute.

No new broad BM25 benchmark is permitted before this plan-only gate resolves the existing product-runtime anomaly and Owner authorizes measurement.

## Cursor digest cost and scale contract

At 100k, 500k and 1M managed file rows, compare the current production anchor/revision check to the D4-S6 research full-result digest. This is a future authorized measurement, not an instruction to benchmark in S7.

For each scale, run deterministic rare, medium-selectivity and broad MATCH expressions; global and root-scoped queries; relevance page 1, page 2 and a deep page; page sizes 50 and 200; and a query whose matching set approaches the full collection. Measure separately:

- SQL MATCH and BM25 execution;
- ranked visible-row materialization;
- complete result serialization and digest;
- cursor validation before page query;
- total page latency and query-plan temp/sort work;
- process peak memory, SQLite temp storage, DB/WAL growth and any writer wait caused by readers.

Warm and cold-cache conditions must be stated. Use at least 30 repeated query cycles per condition after a separately reported warm-up; publish p50/p95/p99/max and all raw rows. Keep baseline and candidate on the same machine/build/data and keep page size, query expression, scope and SQLite cache settings identical.

**Pass rules:** exact visible IDs/counts and approved BM25 scores/order; no old or hidden source returned by any cursor page; root B survives only under the scope/seal/ranked-result conditions above; restart/rebuild equivalence; no unexplained SQLite 3.51.3 plan anomaly; and cursor validation latency/memory remains inside an Owner-approved product budget at every scale. The repository has no Owner-approved numeric File Library search/Cursor p95 budget in the audited inputs. Until one is set before the future run, performance cannot be declared PASS.

Any attempt to improve cost by hashing only the current page, skipping current visibility/seal validation, or accepting old FTS state changes the correctness contract and fails this gate. Any optimization must preserve the exact ranked result and cursor rejection behavior.

## Stop conditions

- Product source ID/compile options do not prove SQLite 3.51.3.
- Hidden rows influence visible ranking while the agreed contract requires an effective corpus.
- `files_fts` or `content_artifact_fts` returns a hidden/replaced source or disagrees after reopen/rebuild.
- A Global Index-only result is changed by managed publication without explicit bridge ownership.
- Global Cursor remains valid after an effective collection/rank change, or a root-scoped cursor resurrects a shared invalid source.
- Full-result digest cost has no approved p95/memory target at 100k/500k/1M.
- The current broad BM25 anomaly remains unexplained or exact query-plan evidence is missing.

Any stop condition keeps D4 production NO-GO and requires a separate Owner-reviewed FTS/Cursor design or validation gate.
