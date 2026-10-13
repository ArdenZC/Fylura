# #381-B3 — Production Integration Blueprint: Effective Visibility, FTS and Writer Ownership

**Owner review snapshot:** 2026-10-13 · **State:** B3_INTEGRATION_DESIGN_READY_FOR_OWNER_REVIEW / IMPLEMENTATION_NO_GO

**Live master examined:** `3275ac7c72df8ae4139a30f16c7f649971324e97` / tree `009576241cd4e8e2ba12cc644a368314f599faa6`. #383 production candidate HEAD `584621c32054e16ac0a61afc669a04380a976b7f` remains OPEN/Draft/**DESIGN BLOCKED**. #394 B1 source-label corrections and #395 B2 isolated Oracle remain OPEN/Draft. #395 HEAD `1ccf1ccbc0b8d0d840e746a12973c1d8df87d50b`, tree `91a218571c153fceb293b4cb4e23c99d9dfff50a`, exact-head CI #38376866627 completed SUCCESS (9 actual success / 18 conditional skip), 16/16 positive and 6/6 negative tests. This **only** accepts isolated semantic evidence. No public release has occurred; no release timeline or obsolete client compatibility requirement. No unmerged migration version is allocated.

## 1. Mandatory decisions

**D1 — Distinguish physical existence, membership, root health and action eligibility.** The current production `files.is_stale` is a global bit with path/ID identity, whereas `scan_roots` has separately versioned roots, sessions, runs and watcher recovery. A single root must never make a file under an overlapping second root globally absent. File selection must require *published root membership for the requested scope* and a non-superseded source incarnation; it must not derive root provenance only from path-prefix. A transient `reconciliation_required` signal must be visibly differentiated from `healthy`: an older published view may remain **readable with a stale/limited-coverage badge** when known safe, but unsafe actions require live identity/access and current owner proof. A known permission/identity revocation cannot be rendered as an actionable E0 file.

**D2 — Mutable `files` rows are an additional generation hazard.** `scan.rs` batch upserts change `files` metadata and `files.is_stale` *during* a scan, before any final stale publication. Merely staging root->file memberships outside `files`, then pointer-flipping a root epoch, does NOT give an immutable E0 file-attribute/FTS snapshot: prepublish in-place updates can replace E0 file name/path/mtime or invalidate its observation. Candidate must either (a) version row projections/relevant attributes, with root-epoch references and safe retention, or (b) rigorously define accepted live-observation semantics, prove no premature claim/underfill/action privilege, and show correct cursor/rank invalidation. Do not assert E0 old-or-new for everything from B2 membership alone.

**D3 — Publish short and fenced; prepare all expensive material first.** A candidate root+scan-run+session+Watcher/source-writer revision/physical identity candidate should be sealed before commit, with freshness checked by bounded CAS. Publication must be O(number of affected roots/authority pointers), **not O(number of files)**; no directory FS walk, 100k update, FTS rebuild or mass duplicate/AI invalidation under `BEGIN IMMEDIATE`. Old version/row/derived records retained through in-flight readers and abort. A crash before/after COMMIT yields one published authority; bounded idempotent GC only after safe reader lifetime/identity checks. No blind status restoration.

**D4 — FTS ranking is its own blocking authority.** Existing `files_fts` is an external-content FTS5 virtual table with trigram tokenizer indexing **physical `files`**, including retained stale rows, and `files_ai/files_ad/files_au` triggers; **current files_au triggers on every UPDATE**, even when name/path did not change (master `schema.rs:2801-2816`). Production `library/mod.rs:1417-1445` projects `bm25(files_fts,6.0,1.5)` over this global physical corpus then applies `f.is_stale` and scope outside the FTS CTE. Filtering *after* FTS ranking or after LIMIT does not reproduce B2's **BM25 over the globally effective-visible document corpus**, nor guaranteed full scoped pagination. Old FTS rowid/index origin and `files` ID identity have to be stable across an effective projection swap. Do not claim a `WHERE EXISTS` adjustment fixes corpus statistics. Present-only SQL filters may be usable for set membership but **not** by themselves for agreed relevance scores.

**D5 — Query/cursor/count revision is already useful, not enough.** `library/mod.rs:727-849` runs rows/count/scope in one SQLite transaction; `library_query_state.revision` is checked for cursor/count tokens (`:557-591,:739-790,:890-929,:1906-1971`). Reuse this fail-closed revision approach while binding the effective published root set, active file incarnation, FTS corpus/rank authority, user-tag/saved-view revision and query fingerprint. On published visibility or rank change, invalidate relevant cursors/cached counts **in the same commit**; conservative global revision bump is correctness-safe but may invalidate unrelated scoped cursors, so record that UX/performance tradeoff. UI must not be a substitute for DB query authority.

**D6 — Metadata preservation and mutation fence.** `file_user_tags.file_id REFERENCES files(id) ON DELETE CASCADE` (`schema.rs:1911-1921`); deleting a stale row would erase user tags. `library/tags.rs:107-183` already has a transaction/revision and selection contract; preserve it, adding current incarnation/visibility checks for actions as needed. `files.rs:27-65` direct identity/naming lookups use `is_stale=0` and OS size/mtime. Path equality or existing path-shaped ID is never sufficient to transfer private metadata to a new physical file. Mutations, restore, preview and organization execution must check the latest actual physical/permission identity independently of metadata visibility. Historical user facts are retained, but never automatically attached to a replacement inode/file ID.

**D7 — Derived authority** is not the same as physical deletion. `dedupe/mod.rs:1749+` currently invalidates stale rows and hashes within the caller transaction; content artifact triggers also depend on `files.is_stale`. With epoch publication, invalidate **effective action eligibility/revision** atomically; regenerate expensive Content/Dedupe/AI/Analysis and old compatibility flags in small idempotent batches while preserving history. Only a matching source-incarnation + scan/root epoch + worker revision can publish a new actionable derived result. No content/AI jobs may change visibility or grant filesystem mutation rights.

**D8 — Scope boundaries.** This is Managed File Library (#381) identity and visibility; Global Search `global_entries`, Index Scope ADR-0014, AI consent, explicit Browse Folder, Storage Analyzer and Windows Service #380 source admission remain separate authorities. Define bridge checks where a feature consumes two authorities; no global filter activation or default behavior switch.

## 2. Source-verified current integration points

| Production owner | Current exact master evidence | Necessary B3 responsibility / gate |
| --- | --- | --- |
| Core scoped SQL | `src-tauri/src/db/queries/mod.rs:79-103` uses `f.is_stale=0` | Introduce reusable effective-visibility predicate for Managed callers, root membership/identity/epoch; avoid universal application to Browse/Global Index |
| Library UI query+count | `db/queries/library/mod.rs:727-849,890-946,1167-1329,1406-1519` | Same transaction list/count/health, root selection, cursor rank/cached count authority and FTS plan |
| Library selection+tag mutation | `db/queries/library/mod.rs:1349-1403,2198-2355`; `db/queries/library/tags.rs:107-183` | Source scoped selection under writer transaction; tag/user state retained and CAS |
| File identity/action/legacy search | `db/queries/files.rs:27-65,175-268,885-950,1398-1453,2422-2465` | Unify active identity and direct ID reader guards; prevent unmanaged mass stale bypass and path-reuse action |
| Scan root/run/Watcher | `db/schema.rs:939-1021`; `db/queries/scan.rs` run ownership and scope | Persist root claim, sealed candidates, monotonic revision/Watcher coverage; overlap claims and non-healthy states |
| Candidate stale publication | #383 `db/queries/scan.rs:1693-2330` | Replace 100k writer-lock domain with O(1) CAS root/authority publication only **after** safety proof; PR #383 itself remains NO-GO |
| FTS schema/triggers | `db/schema.rs:2763-2818` | Physical vs effective document corpus, rowid mapping, tokenizer, trigger migration and rank parity |
| Dedupe/content/AI | `db/queries/dedupe/mod.rs:901,1018,1036,1749-1761`; `db/schema.rs:2350+` | Revision-scoped derived validity, safe async invalidation; no large sweep under publication |
| Operation & path actions | `db/queries/files.rs:27-65`; `file_ops.rs` production execution (before cfg-test) | Current source/physical identity and permission check **at execute time**, not only when list was built |
| Frontend | File Library DTO/cursor consumers in B1 and `library/mod.rs:861-887` | Display degraded/reconciliation state without claiming healthy; controlled snapshot expiration |

**B1 source-label correction remains binding:** six test-only `STG-114..119` and ten Windows test-only `WFG-195..204` were incorrectly included in original frozen production labels. The historical S8/S8R ledgers are immutable; use the #394 B1 overlay. 169 other Tauri registrations still need **targeted dynamic callpath proof for affected handlers**, not an automatic 169-command rewrite.

## 3. FTS production alternative comparison — exact BM25 is non-negotiable unless Owner changes product semantics

| Approach | Correct membership/FTS corpus | Performance / concurrency caveat | B3 disposition |
| --- | --- | --- | --- |
| F0 — FTS all retained files + SQL member join/filter | Set membership can be correct; **BM25 corpus wrong**; filter after LIMIT can underfill | Existing code likely easiest; **not** acceptance-equivalent | **REJECT as an exact-BM25 production solution** |
| F1 — Prepared effective-only FTS projection per published index generation/slot | Potential exact visible corpus; root-scope filtering after calculating **global** visible corpus scores | Must build stage away from writer critical section, atomically bind pointer + external-content payload, preserve old pinned readers, avoid full-rescan/FTS rebuild per small root change; changing SQLite virtual table/layout is nontrivial | **RESEARCH CANDIDATE, no implementation GO** |
| F2 — All-file FTS only as candidate retrieval, exact effective-corpus BM25 computed with maintained indexed effective global term/document stats | Potential exact corpus if tokenizer and stats parity proved | Trigram token frequencies/document length, average length, ranking/candidate pruning, update cost and cursor p95 need source-linked proof; candidate FTS can still omit valid results if it prematurely LIMITs | **RESEARCH CANDIDATE, no implementation GO** |
| F3 — Recompute full visible FTS and rank per query (B2 method) | Exact for small fixtures | O(visible corpus) allocations/rebuild/query; cannot assume 100k+ interactive budget | **RESEARCH ORACLE ONLY, NOT PRODUCTION** |

**Owner decision:** compare F1 and F2 by a **single narrow product-linked test-only feasibility probe** under actual SQLite build and trigram, with current query shape and adversarial hidden docs. F0 must fail parity; F3 serves only as independent small fixture oracle. If neither meets exact BM25 + pooled latency + writer budget, escalate a conscious product ranking-semantic decision; do not silently redefine rank. No new indexed structure or schema version is authorized in this B3 design.

## 4. Reconciliation and root authority contract

Provisional entity model (fields NOT yet approved DDL):
- `RootAuthority(root_id, active_epoch, root_rev, watcher_rev/applied_rev, active_run/session_rev, health)` keyed by selected configured source, independent of pathname.
- `SourceIncarnation(source_uid, volume/FS observation key, path, source_observation_rev, user_metadata_owner)`. A same-path physical replacement receives a distinct UID; when reliable identity is not available, fail closed for action.
- `RootEpochMembers(root_id, epoch, source_uid, observation_rev, proved_coverage_digest)`, append in small transactions. Cross-root union is a deduplicated **logical source incarnation**, not a single-root tombstone.
- `ProjectionAuthority(global_effective_rev, fts_corpus_rev, derived_rev, library_query_rev)`. Visibility/bm25/cursor/derived consumers must bind to a coherent version; retain/reclaim old versions only after proven reader and worker safety.
- Sealing proof checks source writer watermark or per-source monotonic revisions, root scope, actual physical boundary, run/Watcher completeness and membership cardinality **outside** critical writer transaction; a short publish CAS verifies seal and flips authority/revisions. If an OS boundary can change unobserved after sealing, mark source `reconciliation_required` and block irreversible actions until revalidated.

**Hard ownership cuts:** stale row/change set + actual user tags/notes; queued AI/content/dedupe vs independently corrected file; current scan run vs new Watcher, rename and ABA; overlapping enabled/disabled roots; root health degradation vs snapshot-readable records; externally cancelled/partially committed sync; crash before/after publication. No 100k UPDATE-in-transaction, mass FTS trigger update, uncommitted cleanup, or unbounded SQLite writer lock.

## 5. B3 test gates for later Codex implementation (not yet authorized)

| ID | Gate |
| --- | --- |
| P01 | B1 impacted direct SQL/DTO/cursor/read+action entry inventory with source/owner, scope, run/user-data independence; no false production/test labels |
| P02 | E0/E1 immutable user metadata and incarnation with same-path ABA, overlapping roots A/B, disabled/missing/permission root |
| P03 | Reader/list/count/pagination/selection/tag/FTS use one effective snapshot; visible-only global BM25, scope before LIMIT, cursor/cached count revision invalidation |
| P04 | Proof-to-publish run/session/root/Watcher/writer CAS with source correction during staging, no misplaced ROW update under publish lock |
| P05 | Lifecycle SIGKILL before/after pointer commit, stale AI/content/dedupe completion/GC replays, retained historical records |
| P06 | Existing `files.is_stale` direct/mass mutation and external-content FTS trigger coexistence must not bypass the new published visibility authority |
| P07 | Product SQLite version/trigram and exact linked query planner source, paired 0/10k/100k concurrent real user-tag writer and source writer, existing 5000ms timeout unchanged |
| P08 | Hosted Windows 100k writer lock occupancy/reader FTS/identity plus macOS native-backend verification; repeat enough to measure variability and p95 if the contract needs percentiles |
| P09 | Real Windows reparse points/ACL, macOS package/iCloud/file identity, changed directories/root prefix, source deletion and new observation; Owner-native acceptance later and separately authorized |
| P10 | No Global Index, ISG policy, AI consent or public legacy-upgrade behavior implicit authority changes; no CI waiver/extra release gates |

**Known non-negotiable unresolved blockers:** (i) product-linked exact effective-only BM25 FTS plan; (ii) live in-place source attribute update vs E0 immutable published projection; (iii) all writes creating/overriding global `is_stale` beyond `scan.rs` owner; (iv) operation execution and derived job fencing; (v) actual Hosted Windows 100k lock proof. These are why #383 remains DESIGN BLOCKED even though #395 B2 model is accepted.

## 6. Concrete limited next-step decision (no omnibus rewrite)

**B3-F — test-only product-linked FTS & projection options** should be the *next single Codex research contract* after Owner reviews this blueprint. It must exercise the repository's actual bundled SQLite+FTS5 trigram semantics and verify F1/F2 ranking/corpus/parity/EXPLAIN using temporary data without touching production schema or master. First compare correctness for 0/10k/100k, then capture query, rank, staging and concurrent writer costs, with raw evidence; do not commit bulk binary DB snapshots. Two alternatives only, explicit stop if neither qualifies. Product writes remain NO-GO.

**B3-O — source-owner and historical data mapping** can proceed as Owner source-local design in parallel, focused on the above ~10 production choke points, not scanning all 209 IPC registrations. Only after B3-F and B3-O prove a bounded implementable design will Owner authorize Codex Cloud to create staged production PRs. The first production PR must not silently bundle all reader/FTS/scan/AI changes with a versioned schema upgrade: produce an exact dependency-ordered integration/switch-over plan with rollback and feature flags/isolated tests.

**Stop at:** `#381_B3_INTEGRATION_BLUEPRINT_READY_FOR_OWNER_REVIEW`; `#381_PRODUCTION_NO_GO`, `#383_DESIGN_BLOCKED`, `SCHEMA_V38_UNALLOCATED`, `MASTER_UNCHANGED`, no merge/close/release/Codex Review.
