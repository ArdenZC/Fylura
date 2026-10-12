# ADR-001 — Managed File Unified Visibility Authority

**Track:** Zen Canvas #381 Managed Scan, D4-S1

**Status:** Proposed for Owner review; design only

**Disposition requested:** accept, amend, or reject the authority model below

**Production status:** NO-GO. This ADR allocates no schema version and authorizes no implementation.

**Source baseline:** master `3275ac7c72df8ae4139a30f16c7f649971324e97`, tree `009576241cd4e8e2ba12cc644a368314f599faa6`, Schema 37.

## 1. Decision in one paragraph

Use **Root Membership + Effective Visibility** as the canonical managed-file authority. Keep a durable identity for each indexed path-entry, record its membership independently for each scan root and immutable successful root generation, and atomically publish a complete manifest that names the currently effective generation for every root. A single database pointer selects the manifest. Every reader, mutation, trigger effect, and background job must resolve availability from that same manifest snapshot; `files.is_stale` becomes legacy compatibility data and must not authorize product behavior. This is a logical authority proposal, not a complete-file projection. A staged tombstone overlay may be used as an implementation detail only when its candidates are root-owned, identity-bound generation membership changes. Do not choose a versioned copy of every file and derived row as the initial architecture.

The authority answers “which managed source is visible in this committed database snapshot?” It cannot make a filesystem observation and a SQLite commit one atomic operation. A published generation therefore means “the latest complete, fenced observation accepted by the database”; it is not proof that no filesystem event can occur or be delayed immediately after validation. Destructive filesystem actions must still re-open and verify the target identity at the action boundary.

## 2. Context and evidence boundary

Owner accepted D4-S0 research and set production authority to **unproven**. The D4-S0 harness showed that a pointer can atomically switch a simplified visibility set and that old SQLite readers retain their prior snapshot. It did not show a product-wide authority: it used reduced ownership and side-state models, one real Linux filesystem, and a synthetic two-root switch. Do not treat those measurements as qualification of this design.

The D4-S0 observations relevant to this decision are:

| Experiment | 100k result | Use in this ADR |
|---|---:|---|
| Tombstone overlay final transaction | 405.7 ms median, 410.5 ms max; ~4.54 MB WAL median | Bounded pointer transaction is plausible; it says nothing about a complete root-membership ledger or every production consumer. |
| Versioned projection publication | 150.8 ms median, 151.9 ms max; ~345.8 MB WAL | Fast pointer publication has substantial write amplification and the tested projection omitted many related tables. |
| Overlay broad stale-term FTS count | ~5.62 s median | Effective filtering and raw FTS population need separate query-plan review. |
| Broad BM25 diagnostic, bundled SQLite 3.51.3 | 20,001-row fixture: 103.8 s without overlay and 52.1 s with overlay; host SQLite 3.53.1 was ~19 ms on its non-comparable run | Separate blocking query-plan risk; do not infer a design win or run another broad benchmark in D4-S1. |

The experiment preserved base rows and side data, but visible raw tag counts, `current` content artifacts, `active` duplicate groups and current analysis state did not follow the overlay pointer. This is why a pointer by itself is insufficient.

### Frozen inputs

- D4-S0 report and 18-file research-only commit: [`b86fa298441729f7cb0027e076aa3d34a3d3ac04`](https://github.com/ArdenZC/Zen-Canvas/tree/b86fa298441729f7cb0027e076aa3d34a3d3ac04), tree `972a4c96927f898ac8f21612c9af6f678922a339`.
- PR #383 remains OPEN/Draft at D3R HEAD `584621c32054e16ac0a61afc669a04380a976b7f`, tree `8e72e9db9e4fae09b158d165762bba057e0b543d`.
- Master Schema 37 source: `src-tauri/src/db/schema.rs`; input SHA-256 is recorded in [`INPUTS.json`](INPUTS.json).
- Exact Rust stale references and function grouping are in [`source-audit/stale-reference-inventory.csv`](source-audit/stale-reference-inventory.csv) and [`source-audit/function-inventory.csv`](source-audit/function-inventory.csv). The inventory generator reads the frozen Git commit and does not modify production files.

## 3. Source audit: current authority and entrypoints

### 3.1 What currently owns visibility

In Schema 37, `files.is_stale` is the effective managed-file live bit consumed by numerous queries and services. `files` has no `scan_root_id` or durable root-to-file membership relation. `file_fingerprints` can hold platform/volume/file identity evidence, but it is keyed by file and is not root ownership. `scan_roots`, `scan_runs`, `scan_sessions`, `scan_session_roots`, and `scan_seen` provide run and coverage bookkeeping, not a published root membership manifest. Root ownership is consequently inferred from the path and the current scan operation.

The source inventory scanned **247** tracked Rust files and found **262** `is_stale` reference lines across **40** files at the frozen revision. After test-only entries are marked, it groups the production references into **103** functions or schema/type entries: File Library, query/DTO helpers, Scanner/Watcher, dedupe, content, analysis, classification, AI, Organization, rule proposals, schema, and the Global Index. A second literal-SQL scan records **433** mutation lines across **141** production/schema function groups for files, roots/runs/sessions, tags, fingerprints, duplicates, content, analysis, and Global Index tables. See [`source-audit/mutation-reference-inventory.csv`](source-audit/mutation-reference-inventory.csv) and [`source-audit/mutation-function-inventory.csv`](source-audit/mutation-function-inventory.csv). These generators are text heuristics, not Rust/SQL parsers; dynamically assembled SQL, method-level call graphs, trigger bodies, frontend/IPC use, and indirect semantic dependencies are manually reviewed below. The exact generators and frozen line references are included so reviewers can reproduce and correct the grouping.

### 3.2 Function-level readers, writers, triggers, and tasks

This table names the source-level entrypoints that form the visibility boundary. The CSVs are the exhaustive line-by-line companion and include test references separately.

| Surface | Current functions / SQL objects | Current dependency and required D4 contract |
|---|---|---|
| Shared file query scope | `queries::scoped_files_sql`, `scoped_files_sql_with_extra_where` (`db/queries/mod.rs:79-100`); callers `files::search_files_in_scope`, `get_paged_files_in_scope_with_filter`, count/preview helpers; `classification::execute_active_rules_for_scope_with_settings`; AI `collect_selected_ai_classification_targets` | The shared CTE hard-codes `f.is_stale=0`. Replace this semantic boundary with a manifest-bound effective-source relation. All CTE users must reuse the same read transaction and manifest revision. |
| File Library browse/search | `library::query_file_library_v2`, `build_query_parts`, `files_matching_authoritative_query_scope`, `active_library_rows_exceed_deferred_threshold`, summary/page/count/cursor builders | Direct live predicate, FTS rowid join, count caching and pagination must share a manifest snapshot. Cursor/cache keys must include publication/library revision. |
| File Library by ID and actions | `library::get_file_library_detail`, `resolve_file_library_path`, `get_file_library_selection_summary`, `selection_stale_count`, `selection_where`; `files::get_indexed_file_rows_by_ids`, `verify_indexed_file_identity`, `get_indexed_file_naming`, operation preview/restore helpers | `get_file_library_detail` currently reads a raw `files` row by ID and then tags/findings/content/duplicates without `is_stale=0`. Revalidate effective visibility before returning active detail or permitting a path action. Explicit IDs cannot bypass authority. |
| User tags | `list_user_tags`, `create_user_tag`, `update_user_tag`, `delete_user_tag`, `mutate_file_user_tags` (`db/queries/library/tags.rs:4-168`) | Tag usage and expected-usage CAS currently count raw `file_user_tags`; selection mutation uses `selection_where`. Preserve associations, count only effective sources in active UI, and revalidate the manifest at mutation commit. User deletion remains an explicit user action. |
| Scanner | `scanner::recover_scan_state`; `scan::admit_managed_scan`, `claim_queued_scan_run`, `persist_scan_batch`, `reconcile_missing`, `finalize_scan_run`, `upsert_file_rows_tx`, cancellation/retry entrypoints | Run/root/session revisions, lease, coverage errors, `scan_seen`, watcher revision and cancellation already fence stale reconciliation. They do not create durable root membership or one session publication pointer. Staging and recovery must retain those fences. |
| Watcher | `watcher::apply_watcher_exact_mutations_with_retry`; `scan::apply_watcher_exact_mutations`; `watcher::recover_watcher_reconciliation_state`; root revision and watcher queue/apply helpers | Exact file mutations currently alter `files.is_stale` and invalidate related state. Every accepted event must be associated with a root/source and durable sequence. A gap marks uncertainty and retains the last published state until reconciliation. |
| Dedupe | `dedupe::collect_dedupe_candidates`, `save_prehash`, `save_full_hash`, `copy_cached_fingerprint_for_rename`, `invalidate_file_in_transaction`, `invalidate_stale_files_in_transaction`, `recover_dedupe_runs`; SQL view `active_duplicate_membership` | Candidates filter stale rows. Invalidation clears hash, marks fingerprints/groups/findings stale; the view filters group status but not file visibility. Effective duplicate members/counts must be computed against the same manifest; membership loss in one root must not invalidate a source still visible through another root. |
| Content | `candidate_filter`, `candidate_count`, `for_each_candidate`, `load_candidate_by_id`, `revalidate_content_boundary`, `query_content_artifacts`, `get_content_artifact`, `recover_content_runs`, `prune_content_artifacts`; `content_artifacts_file_changed` | Candidate and by-ID gates use `files.is_stale`. `content_artifacts_file_changed` only fires on `files` updates; pointer publication will not fire it. Preserve artifacts/history, but expose active content/FTS only for an effective source and matching identity/revision. |
| Analysis and findings | `list_managed_files_for_analysis`, `get_managed_file_for_analysis`, `authoritative_duplicate_exact_subjects_tx`, `publish_analysis_run`, `recover_analysis_runs`; `analysis::managed_file_identity_snapshot` / `managed_file_identity_matches` | Inputs and output CAS must bind to source identity and manifest. Preserve findings/evidence as history; an old job cannot publish an active finding after source or manifest authority changes. |
| Rule classification and AI | `execute_active_rules_for_paths_with_settings`, `execute_active_rules_for_scope_with_settings`, `execute_rules_on_inbox_with_settings`; `collect_selected_ai_classification_targets`, `select_indexed_file_columns`, AI batch/cancel/commit functions | Candidate filters and returned DTOs consume the stale bit. Results must bind to source/file revision plus effective manifest; stale/unknown sources are not candidates, and old outputs become retained superseded history. |
| Automation | `admit_automation_analysis`; trigger wakeups after scan/file writes | Eligibility joins managed files and Global Index state. A queued job must revalidate both authorities at claim and completion. |
| Organization | `materialize_organization_plan`, `load_indexed_file_by_id`, `load_indexed_files_for_projection`, `resolve_current_assessment`, `validate_edited_filename`, plan recovery/execution | Plan snapshots include `is_stale`, duplicate state, and sometimes Global Index identity. Bind plan and action to a manifest ID; reject/rebuild after authority changes and re-open physical identity before action. |
| Rule proposals | `build_rule_proposal_impact`, `validate_exact_impact_binding`, `recover_rule_proposals` | Candidate count and impact are stale-filtered; proposal snapshots and validation must carry the same manifest revision. |
| Global Index bridge | `enqueue_managed_ai_for_library_files`; `managed_scope::{add_managed_scope, backfill_managed_scope}`; managed AI `claim_next_managed_ai_job`, `validate_managed_ai_job`, `complete_managed_ai_job`, queue recovery | `global_entries.is_stale` is a separate authority. `enqueue_managed_ai_for_library_files` also checks `files.is_stale`. Require a durable source link and a current managed manifest at enqueue/claim/complete; never copy managed stale state into Global Index state implicitly. |
| Frontend and DTO | `FileLibraryList`, `FileLibraryInspector`, `ContentUnderstandingSheet`, `VaultView`, `organizeModel`, `OrganizeSuggestionInspector`, presentation adapters; `FileSummaryDto`, `FileLibraryDetailDto`, `IndexedFile` | UI consumes `isStale`/`is_stale` for missing labels and disables path actions. The future response must carry effective availability and a snapshot/revision token, not serialize the raw legacy column as authority. Browser mock API stale behavior is test/demo-only. |
| Startup/recovery tasks | `main.rs` invokes `recover_dedupe_runs`, `recover_analysis_runs`, `recover_content_runs`, `scanner::recover_scan_state`, `recover_organization_plans`, `recover_rule_proposals`; watcher recovery runs after startup/rebuild; analysis/content/dedupe pruning jobs | Recovery must own stage rows by durable run/session/root IDs, detect pointer already committed, resume idempotently or mark abandoned, and never infer ownership from file path alone. GC/prune must not delete published or reader-pinned evidence. |
| Schema and triggers | `migrate`, `ensure_scan_ledger_schema`, `ensure_watcher_reconciliation_schema`, `ensure_dedupe_schema`, `ensure_content_schema`, `ensure_fts_triggers`; `files_ai`, `files_ad`, `files_au`, `content_artifacts_file_changed`, `active_duplicate_membership`; Global Index FTS/count triggers | `files_ai/ad/au` maintain external-content `files_fts`; Schema 37 `files_au` rewrites FTS on every update. Content invalidation depends on a file UPDATE. `active_duplicate_membership` ignores file visibility. Global Index triggers/counters are an independent subsystem. Trigger behavior must be included in the new contracts, not relied upon as implicit publication. |

The frontend consumer list is traceable in [`frontend-consumers.csv`](source-audit/frontend-consumers.csv). The per-reference CSV includes exact master line numbers for all direct reads/writes, serialized fields, test-only rows, and independent `global_entries` uses.

## 4. Proposed logical data model

This is a logical schema sketch only. Names are descriptive; it neither assigns a migration number nor asserts final physical DDL.

### 4.1 Entities and authority

| Logical entity | Minimum contents | Authority role |
|---|---|---|
| `file_source` | Stable `source_id`; path-entry identity kind; current `files.id` link; normalized path key; identity tuple (`platform_kind`, volume identity, file identity where available); identity confidence; `source_revision`; first/last observation | Identifies one managed path-entry. It is not the content hash or a dedupe physical-object key. Hardlink aliases remain separate path entries even when they share a physical key. |
| `scan_root` | Existing root ID and path; root/path revision; enabled/retired status; volume/root-handle identity; health and watcher sequence watermarks | Defines one independently reconciled scope. A textual path prefix alone is not ownership proof. |
| `root_generation` | Root ID, generation ID, owning run/session, base manifest/root revision, scan revision, watcher revision at start/end, coverage result, observed count and deterministic digest, status | A durable, complete or rejected observation. A partial/error/cancelled generation cannot replace a published head. |
| `root_generation_member` | Root ID/generation, source ID, observed path, source-identity evidence/digest, file facts used for admission, watcher sequence, present state | Positive membership evidence for one complete generation. Old generations remain immutable until eligible GC. Absence is implied only by omission from a coverage-complete published generation; it is never inferred from an incomplete stage. |
| `publication_manifest` / `manifest_root` | Immutable manifest ID and revision; complete map from enabled root IDs to published generation IDs; optional session/root publication provenance | The authoritative root-head map. Manifest rows are immutable after ready. |
| `active_manifest_pointer` | One current manifest ID/revision, updated with CAS | The single atomic visibility cutover. Reader snapshot begins by observing this pointer once. |
| `legacy_source_state` | Existing file ID, legacy stale bit, assignment state (`provisional`, `unassigned`, `retained_stale`), candidate roots, proof state | Makes old data behavior explicit without manufacturing historical ownership or deleting it. |
| `reader/job lease` | Owner kind/id, manifest ID, source/run identity, expiry/heartbeat or durable cursor token | Prevents GC from deleting data still required by cross-transaction readers, jobs, or cursors. Expiration must be conservative and recoverable. |

### 4.2 Effective visibility rule

For a pinned manifest `M`, a source is **managed-visible** if at least one root enabled in `M` names a published, complete root generation that contains that exact `source_id` and whose captured source identity is compatible with the current source identity. Root health such as `missing`, `permission_required`, or `needs_reconciliation` does not erase the last published membership; it makes freshness/action confidence degraded. A full scan with incomplete coverage does not publish. A root removal retires that root’s membership from a new manifest but preserves its generation and history.

At a design level:

```text
effective_visible(source, M) =
    exists root R, generation G:
        M.head(R) = G
        and R is enabled in M
        and G is complete and published
        and G contains source with an accepted identity binding

effective_missing(source, M) =
    source had prior authoritative membership
    and no enabled root head in M contains that same source
    and each removed membership was removed by a complete accepted root generation

legacy_unverified(source) is neither effective_missing nor destructive authority.
```

The migration compatibility view keeps every previously non-stale legacy row discoverable as `legacy_unverified` until a successful root observation or an explicit user assignment supplies ownership proof. It is read-only for destructive operations until an action-boundary identity check succeeds. A row already marked stale stays retained in history and is not revived solely because a path reappears. If the same path now resolves to a different platform identity, create a distinct source incarnation; do not silently attach the old source’s tags, notes, analysis, or content to the new object. Reattachment requires verified same identity or explicit user action.

The effective File Library is the union of managed-visible sources plus the explicitly surfaced compatibility slice for `legacy_unverified` live rows. Root-scoped queries use root membership, not prefix matching. “Missing” and “unmanaged/retired” are distinct states. Neither state deletes the `files` row or associated history.

### 4.3 Ownership and precedence truth table

| Situation | Proposed rule | Visibility/result |
|---|---|---|
| Same path/source observed under two active roots with compatible identity | Store two memberships pointing to one source. No “winning root”; manifest membership is set-valued. | Global File Library shows one source. Removing one root membership does not hide it while the other published membership remains. |
| Parent and nested roots both configured | Root ownership follows the explicit membership rows, not longest-prefix precedence. A parent scan cannot silently delete a child-root membership. For an overlapping session, publish only the effective roots whose coverage contract is explicit; a nested root recorded as “covered” keeps its previous head unless the session explicitly includes it. | A scan that cannot prove child coverage leaves child visibility unchanged and reports reconciliation required. This may retain an absent file until the child root is scanned; it avoids false stale. |
| Two root observations claim one normalized path but platform volume/file identities conflict | Mark both observations conflicted/quarantined; do not merge IDs, transfer tags, or publish a destructive absence. | Preserve prior published state and history; root/source requires reconciliation. |
| Root unavailable, ACL denied, volume detached, watcher gap, or scan incomplete | Do not replace the prior root head. Keep last published memberships and mark freshness/health uncertain. | Existing rows may remain visible with degraded status; no new stale state is inferred. |
| Root explicitly removed/disabled by user | Publish a manifest omitting that root; retain root/run generations. Do not cascade-delete file/history data. | A source remains visible if another enabled root owns it. Otherwise mark unmanaged/retired and hide from active managed actions while retaining history. |
| Root path moved | Increment root path revision; scan new scope as a new candidate generation. Do not transfer membership by path similarity. Keep old membership/history until user-approved retirement and successful replacement. | Old and new paths do not inherit each other’s ownership automatically. |
| Duplicate or nested root selections in one request | Deduplicate identical roots. Treat a “covered” nested request as status only unless its independent root generation is explicitly part of the publication set. | No implicit deletion or ownership transfer from coverage accounting. |
| Legacy live row has exactly one textual root-prefix candidate | Record `provisional` candidate, but do not treat prefix as verified membership or allow stale deletion. | Keep discoverable as `legacy_unverified` until complete scan + source identity evidence accepts membership. |
| Legacy live row has multiple possible roots or no root prefix | Record `unassigned`; retain current active visibility in a compatibility slice and disallow destructive actions until explicit selection or scan proof. | No root may claim it only because a path prefix matches. |
| Legacy row has `files.is_stale=1` | Preserve as `retained_stale`; do not infer that a later same-path file is the same incarnation. | Hidden from current active list, available to history/recovery and explicit reassociation. |
| User explicitly selects file IDs | Resolve only against pinned manifest and current identity. A root-scoped selection requires membership in that root; a global selection can use any enabled root. | Explicit selection cannot override stale/unknown ownership. A changed manifest or identity rejects/rebuilds the selection. |
| Symlink/Junction/reparse point or cross-volume boundary | Root and leaf identity must be checked by opened handles and the configured traversal policy; traversal cannot escape the authorized root by lexical path. | Unknown target/boundary is not new membership and cannot authorize stale deletion. Preserve prior state, mark coverage incomplete. |
| Path reappears between stage and publication | Revalidation or watcher/source revision mismatch rejects the candidate; same-path different identity is a new source incarnation. | No partial stale publish; prior manifest remains active. |

### 4.4 Multi-root sessions

Preferred contract: a session constructs one candidate manifest for its full set of effective roots. It publishes all included root heads in one pointer transaction only if each required root generation is complete, coverage-eligible, still owned by the same run/session, and passes its root/source/Watcher fences. Any required root failure, cancellation, conflict, or missing coverage rejects the entire candidate manifest; the previous manifest remains active. A caller can explicitly retry only the failed/subset roots in a new session. “Covered” nested requests are not counted as completed membership replacement unless the scanner actually produced a root-scoped generation for them.

This all-or-nothing rule is the safer proposal but can delay successful roots because one root failed. Owner must decide whether partial multi-root publication is a product requirement. If partial is allowed, the API must expose a partial manifest result, bind every result/cursor to the new manifest, and prove that failed roots retain their old head; it must not silently report the whole session as an atomic snapshot.

## 5. Candidate comparison

| Candidate | Authority model | Query/read cost | Write/WAL cost | Migration/recovery | Disposition |
|---|---|---|---|---|---|
| **A. Root Membership + Effective Visibility** | Per-source, per-root membership and immutable successful root generations; manifest union defines effective File Library. | One indexed effective-membership test/join in every reader. Root-scoped filters become membership-key filters instead of path-prefix checks. FTS and library queries need join-order/plan validation. | Stage membership records in bounded chunks; pointer publish small. It does not rewrite every file row or every derived table at publication. Exact stage-write/WAL cost is unmeasured. | Requires nontrivial Schema 37 ownership backfill, source-incarnation model, one query helper used everywhere, multi-root manifest writer, stable cursors, old-client gate, and conservative GC. | **Preferred logical authority** because it represents positive ownership, overlap and root removal without duplicating all payload/derived rows. Production still NO-GO pending identity/backfill/search proof. |
| **B. Staged Tombstone Overlay** | Candidate IDs in a stage plus a pointer marks rows hidden; without a positive membership ledger it says only “this operation hid these IDs.” | Every File Library/FTS/ID/detail/tag/action query anti-joins or resolves the overlay. Raw FTS can contain hidden terms; raw side-state counts still leak unless every consumer joins. Large tombstone sets can slow broad FTS count/rank. | Best bounded staging/WAL result in D4-S0: about 4.54 MB and ~406 ms final lock for 100k in the reduced harness. | Does not establish which root owns an old file, whether another root still owns it, or what a root deletion should do. Abandoned overlay rows need ownership and GC. | **Reject as standalone authority.** It can encode a root-owned removal delta inside A only after the positive membership and derived-state contracts exist. |
| **C. Versioned File Projection** | Materialized live files (and potentially FTS/related rows) per generation, with pointer to a full snapshot. | Fast live count/filtered reads if indexes are maintained; no stale anti-join for base rows. FTS per version may improve ranking isolation but multiplies index maintenance. | D4-S0: ~345.8 MB WAL at 100k; initial projection ~5.0 s and rebuild after concurrent writer ~3.2 s. The measured copy omitted tags/history/content/AI/Global Index. | Requires versioning or write-through of all mutable payload and derived state, larger cleanup/backups, pinned reader retention, and migration of all queries to generation tables. Not a full authority until every side table shares generation semantics. | **Reject as initial implementation.** Reconsider only if effective-view query plans or exact FTS ranking semantics cannot meet product targets and after a complete projection design is costed. |

Option A does not imply zero work or a short final commit for every release. It isolates the authoritative visibility state from the materialization policy. If profiling later requires a read projection, it can be derived from A and verified against the manifest; it must not become a second authority.

## 6. Unified visibility, retention, and mutation semantics

| Domain | Active/read contract at pinned manifest | Retention and invalidation contract |
|---|---|---|
| File Library list/count/detail/path | Every page, count, search, preview, by-ID detail, selection, and path resolver checks the same effective-source relation. A response includes manifest ID/revision and effective availability. | No stale transition deletes the base file row. By-ID detail may return history-only data only through an explicitly historical API, not the active detail/action API. |
| FTS | `files_fts MATCH` is candidate generation only; every row is joined to the pinned effective view before returning IDs, snippets, or page results. Cursor binds query fingerprint + manifest/library revision. Raw hidden terms may remain on disk. | FTS insert/update/delete triggers remain correct for actual `name/path/rowid` changes. Visibility changes do not require rewriting the entire external-content index. Search result membership is filtered; exact BM25 corpus/ranking semantics across hidden generations remains a separate decision/risk. |
| Tags and user metadata | Active tag list/usage counts count distinct effective sources. Add/remove/select operations bind to manifest, source revision and expected count; verify again in the same write transaction. | Keep tags, notes, user corrections and history on the prior source. Do not cascade delete on stale/root removal. A new file incarnation at a reused path does not inherit them without verified identity or explicit user transfer. |
| File history, analysis findings, evidence | Active surfaces show only findings whose source identity and authority revision still match. Historical retrieval includes original source/path/manifest and a retained/unavailable label. | Findings, feedback, operations, audit logs and provenance are append-only or retained under existing policy. Visibility loss may mark current projection superseded but does not erase history. |
| Content artifacts and content FTS | Candidate selection, by-ID access, `content_artifact_fts` search, and provider/automation output require effective visibility, matching source identity, content revision and policy. | Keep artifacts/raw text under existing retention and consent policy. A missing or retired source does not make an artifact current or delete it; expose it as retained history. A same-path new incarnation cannot reuse it without validated identity/fingerprint. |
| Dedupe/fingerprints/duplicates | Active candidates and `active_duplicate_membership` resolve only effective members. Duplicate count/reclaimable size must be computed from members visible in the same manifest, not stale `member_count` alone. | Keep hashes/fingerprints/group revisions as history. A source losing one root membership is not invalidated if another root still contains the same source. A physical identity conflict invalidates current use, not stored evidence. |
| Classification/rules/proposals | Candidate sets and exact proposal impacts include manifest ID. Classification/proposal validation rechecks source revision and effective visibility before commit/action. | Keep prior classification, user corrections, learning and proposal decisions. Old worker output is superseded/retained, never applied to a now-hidden or different source incarnation. |
| AI and automation | Managed AI candidate/claim/complete checks current managed manifest and source identity. A job queued under an older manifest must revalidate or reject/requeue. Automation cannot infer managed visibility from Global Index rows alone. | Preserve job history and provider provenance. No model result or user correction is erased by stale membership. AI may operate on unrelated Global Index sources under its own contract. |
| Organization plans/actions | Plan materialization, current assessment, validation and execution bind selected source IDs to the manifest and source revision. On change, reject/rebuild. Before destructive filesystem work, open/recheck target identity and authorized root boundary. | Preserve plan/audit history. A stale source blocks the action; it does not erase the plan record. Global Index assessment is recorded as an independent cross-check, not the managed authority. |
| Global Index bridge | `global_entries` and `global_entries.is_stale` remain a separate authority. Managed actions use an explicit source-link row and require both a live Global Index identity and an effective managed source at enqueue, claim, and completion. | Root membership changes do not mutate global stale state or global FTS. Conversely, Global Index may not resurrect a hidden managed source. Unknown/mismatched bridge identity blocks work. |
| Frontend/IPC | Active DTOs carry `availability`, `manifestRevision` (or opaque token), and effective status. All panels, safe-action gates, selection APIs, and pagination consume that same token. | An old DTO remains renderable as historical data but must not authorize an action. Existing `isStale` may be derived for display compatibility only after every consumer has moved to effective status. |

No automatic stale operation deletes tags, notes, history, content, hashes, duplicate evidence, classification or AI audit data. User-requested delete/tag removal remains an explicit operation with its own confirmation and expected revision.

## 7. Revisions, CAS, invalidation, and retry

### 7.1 Revision domains

| Revision/token | Changed by | Publication/job rule |
|---|---|---|
| `manifest_revision` + `manifest_id` | Every successful active pointer swap | Reader captures once per SQLite read transaction; cursor, selection and long-lived plan carry it. Pointer CAS uses expected parent manifest. |
| `root_revision` / `root_path_revision` | Root enable/retire/path/volume/policy change | Any stage using an older value is invalid and must be revalidated or abandoned. |
| `watcher_revision` / `watcher_applied_revision` | Durable watcher event arrival / successful event application | A candidate may publish only if all events through its captured sequence are represented and no gap exists. Any uncovered change rejects it and leaves `needs_reconciliation`. |
| `run_revision`, `session_revision`, lease token, cancel flag | Scanner/run/session state changes | Preserve existing D3R and Schema 37 CAS rules. Require terminal success, complete coverage, same owner, no cancellation, and exact generation. |
| `source_revision` + identity digest | Source identity/path/observed metadata used for identity and derived output | All preflight members compare ID/path/identity digest and deterministic count/digest, not count alone. A changed source invalidates the affected staged membership or publication. |
| `library_query_revision` | Any mutation changing query-visible file/tag/availability fields | Count cache, cursor and selection fingerprints bind to it plus manifest revision. |
| Domain revision (tag/content/fingerprint/classification/AI/global) | Domain writer only | Job output uses compare-and-swap for its input revision and source identity; the manifest change triggers effective-visibility revalidation, not an unguarded write-through. |

### 7.2 Writer ordering

1. **Scanner:** persist candidate observations and positive membership rows under a durable root/run/session owner in bounded short transactions. Candidate rows are not active membership. Keep source/path identity evidence, count+digest, coverage errors and watcher sequence. Preserve existing scan/run/session revision CAS.
2. **Pre-publish validation:** outside the main DB writer lock, re-open/check root and candidate identities using handle-based OS checks, verify directory/reparse/volume boundaries, and record the exact evidence digest. A changed/missing/reappeared source makes the generation ineligible. This is preparation only, not a claim that the filesystem cannot change again.
3. **Publication transaction:** `BEGIN IMMEDIATE`; verify active pointer parent, each changed root head/revision, run/session/lease/cancel, Watcher coverage, staged row count+digest, source identity/version CAS and root boundary evidence; write the immutable manifest root map; compare-and-swap the single active pointer; mark generation published; `COMMIT`. A single session pointer makes all included roots visible together. Any check failure rolls the whole transaction back.
4. **Post-publish:** reconcile queued Watcher events. If an event raced and was not included, keep the published generation as the last accepted observation, mark root degraded/needs-reconciliation, and publish the event through the same ownership model or rerun. Do not silently patch side tables out of band.
5. **Tag/metadata writers:** independent tag or user metadata updates do not change root membership. They commit under current library/source revision. Selection-bound writes validate the captured manifest and source visibility in the same transaction. A concurrent root publication serializes before or after the mutation; after a manifest mismatch, reject and let the UI refresh rather than mutate a different target set.
6. **AI/content/classification/dedupe writers:** persist only if run/job owner, source incarnation/revision, cancellation state, input domain revision and current effective visibility remain valid. If the manifest changed, re-resolve under the new manifest; publish only if still visible and identity-compatible, otherwise retain a superseded result and retry from a current snapshot.
7. **CAS conflicts:** do not overwrite the current pointer. If only unrelated root heads changed, rebuild/rebase the candidate manifest against the latest pointer and repeat all affected-root checks. If an included root, source, watcher sequence, selected target set or identity changed, discard/rebuild the affected stage. Retries are bounded and durable; no busy-timeout extension or unbounded retry loop is assumed.

The final OS recheck and SQLite commit still have a non-atomic gap. The defined contract is based on accepted observations and durable watcher coverage, with a post-publication repair path. Every file-changing action independently rechecks physical identity and root boundary immediately before acting. If product requirements instead demand “the row is invisible at the exact instant any unreported filesystem delete occurs,” SQLite plus a Watcher cannot prove that requirement; a stronger OS lease/notification guarantee would be needed and is not established here.

## 8. Failure and recovery matrix

| Failure/event | Required authority outcome | Recovery/retention requirement |
|---|---|---|
| Crash before candidate staging | Active manifest unchanged. | No candidate exists; normal scan retry. |
| Crash during chunked staging | Active manifest unchanged; candidate remains owned by run/session and marked incomplete after recovery. | Resume only if durable owner, identity digests and revisions still match; otherwise orphan stage is retained until safe GC. |
| Crash after stage-ready but before pointer transaction | Old manifest remains current. | Re-run final CAS/identity checks; never infer that ready means published. |
| Process killed after pointer update but before commit | SQLite rollback leaves old pointer. | Verify with integrity and pointer reload; recover old run as interrupted. |
| Crash immediately after successful commit, before worker records response | New pointer is authoritative. | Recovery compares active manifest ID to stage and marks publication complete idempotently; do not publish twice or roll back by deleting rows. |
| Scanner cancellation or old generation resumes | No pointer swap. | Reject on cancel/run/root/session CAS; preserve prior membership and history; mark stage abandoned. |
| Watcher event gap/overflow or event during staging | No candidate based on incomplete watcher coverage can publish. | Preserve current head; set reconciliation-required; drain/rebuild. Event loss is not evidence of absence. |
| File/path reappears after observation | Candidate fails identity, watcher, or final boundary validation where observable. | Same identity can be re-enrolled in a later complete generation. New identity is a new source incarnation; no implicit metadata transfer. Residual unreported FS race is disclosed. |
| Symlink, Junction, reparse point, mount or permission change | Unknown boundary cannot publish membership deletion or transfer. | Preserve old head, coverage error, root health degradation; require platform-specific handle verification. Linux Harness does not prove Windows/macOS semantics. |
| Old Schema 37 DB / ambiguous legacy ownership | No destructive backfill. | Preserve all file rows and associations. Live legacy rows remain `legacy_unverified`; stale legacy rows remain history-only; successful scan/user choice establishes future ownership. |
| Same path claimed by two roots with conflicting volume/file identity | Candidate rejected/quarantined; never merge source IDs or move user data. | Preserve both historical assertions and root heads; present explicit reconciliation. |
| Tag/metadata write races publication | SQLite serial order; if selection token changed, mutation rejects. | Keep already committed user metadata. Do not clear it during publication; refresh and retry against current manifest. |
| AI/classification/content/dedupe result races publication | Result CAS/revalidation rejects if source hidden, changed, canceled or identity differs. | Retain as superseded job/history; retry only against a new eligible snapshot. |
| WAL grows because a historical reader pins old pages | No reader sees mixed pointer/table state. | Reader leases/cursors pin manifests logically; WAL pressure pauses staging/publication before exhausting disk. Do not delete referenced generations to reduce WAL. Report pinned reader and require close/retry. |
| Disk full during staging or manifest publication | SQLite transaction rolls back; current pointer unchanged. | Keep old manifest; mark run recoverable/requires space; never partially publish or delete history to free space. Use bounded chunks and reserve-space policy. |
| Crash during generation/manifest GC | Published pointer and referenced data survive. | GC is mark-then-sweep, chunked and idempotent; every chunk rechecks active roots, run owners, history and leases. Unknown lease means retain. |
| Stale cursor or cross-transaction historical reader | Cursor cannot silently cross manifests. | Return a stable stale-cursor response or use a durable lease/snapshot token. Keep referenced generation until lease expiry and successful release. |
| Old or mixed application generation opens migrated DB | Block database/IPC open before it can mutate authority tables. | Require #380 binary/database generation gate; path identity alone is not proof. No fallback to raw `files.is_stale` after a new manifest exists. |
| Raw FTS returns a hidden/new-unpublished term | Active query filters candidate rows by the same manifest before result projection. | Raw index retention is allowed only if no public search, count, preview or cursor leaks the result. Investigate query-plan/ranking behavior separately. |
| Global Index and managed source disagree | Fail closed at the bridge; keep each subsystem’s own status. | Do not mutate global stale state from managed publication. Reconcile only through explicit identity-linked bridge work. |

## 9. SQLite 3.51.3 broad FTS/BM25 blocker and plan-only validation

D4-S0 observed an unexplained ~52–104 second broad BM25 query on the bundled SQLite 3.51.3 fixture versus ~19 ms on host SQLite 3.53.1. The SQL shape and plans were not sufficient to explain the difference; the fixtures/builds are not directly comparable. This remains a separate **search qualification blocker**. It must not be hidden by selecting a fast pointer model or by using a small page-only timing.

Before any new benchmark, run a **plan-only** check against the exact bundled SQLite 3.51.3 product build and the committed schema/query shape:

1. Record the exact app/source revision, `sqlite3_sourceid()`, compile options, schema hash, index definitions, relevant statistics and bound `MATCH` expression. Confirm the loaded library is actually 3.51.3.
2. Capture `EXPLAIN QUERY PLAN` for the exact File Library FTS candidate CTE (`files_fts MATCH`, `bm25`, active-source join, ordering, limit, cursor) and the unranked count/search shape. Also capture the current Schema 37 query for comparison. Use actual bound parameters, not a rewritten approximate SQL string.
3. Record virtual-table index constraints, join order, materialization, full scans, automatic indexes, `USE TEMP B-TREE FOR ORDER BY`, and which predicates are applied before versus after FTS rank. Confirm every returned row is then checked against the same manifest in one read snapshot.
4. Compare plans only between query variants that return the exact same IDs and ranking contract on a small correctness fixture. Do not run a 20k/100k latency campaign in D4-S1. Do not use host SQLite 3.53.1 as a substitute for the linked product library.
5. If EQP is identical but a runtime anomaly remains, stop and produce a separate SQLite/runtime diagnostic plan before any large benchmark. Owner approval is required before benchmarking or changing rank semantics.

**GO requirement:** exact-version query-plan review must show an acceptable, understood plan for page, count and cursor flows; any broad BM25 latency target must be set and measured later under a separately authorized exact-build test. D4-S1 performs no FTS benchmark.

## 10. #380 installer, IPC, and database-generation coordination

Live #380 Owner review at `https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6122891634` accepts #380-S0 research, authorizes one narrowly scoped synthetic NSIS observability run, and keeps U2 production design **NO-GO**. It explicitly says Schema v38 is not allocated in coordination with #381. PR #386 remains OPEN/Draft; master remains the frozen revision above.

The shared gate proposed here is:

1. Keep #380 and #381 migrations as **unassigned fragments**. PR #383’s current Schema-38 FTS-trigger change is a candidate, not an allocation. Neither track can ship a private 37→38 interpretation.
2. Before implementation, both Owners review one combined old-37-to-next-schema migration set: root/source membership and manifest tables, legacy preservation/backfill rules, conditional FTS-trigger change if retained, and any accepted #380 database-generation marker/compatibility work. One migration owner, one combined ordering, one crash-recovery story, one supported-client matrix. Do not assign the version here.
3. Before migration or publication, #380 must establish exclusive DB ownership and a client/database-generation handshake that cannot be spoofed by same executable path. Installer/service/app processes must drain or be rejected; old binaries must not open the post-migration DB and write raw `files.is_stale`/Global Index state. The currently accepted #380 evidence does not yet establish this product gate.
4. Backup and WAL recovery precede migration. A migration crash before manifest publication leaves Schema 37 usable only by compatible clients or resumes the single forward migration. After publication, rollback cannot drop active manifests or history; it must use the coordinated recovery path or restore a consistent verified backup while all clients are stopped.
5. Do not introduce a database-generation IPC/API field or allocate a schema number in this ADR. The #380 track owns its protocol and installer design. #381 must consume the owner-approved generation contract before any root manifest becomes authoritative.

### Shared release order (no version number)

`Owner accepts #380 generation/installer contract` → `Owner accepts #381 authority and migration/backfill contract` → `combined migration diff and compatibility matrix reviewed across both tracks` → `exclusive client drain/backup/upgrade test plan approved` → `implementation on separately authorized branches` → `schema migration + IPC generation checks pass exact-version tests` → `Owner approves a single combined rollout/rollback sequence`.

If either track cannot meet a step, keep both features unshipped and keep DB semantics at Schema 37. No schema slot is reserved.

## 11. Production GO / NO-GO conditions

### Required before a production implementation can receive GO

- Owner accepts the canonical source identity, root membership and nested/multi-root rules, including exact session atomicity and explicit user selection semantics.
- A frozen Schema 37 inventory is reviewed end to end; all 103 production function/schema entries, indirect tag/FTS/duplicate/detail consumers, frontend actions and recovery workers have assigned migration owners and test contracts.
- Legacy migration truth table is accepted. Every row and association remains recoverable; unknown ownership cannot be made stale by inference; backfill is resumable and idempotent.
- Windows/macOS/Linux identity/root boundary contracts cover symlink, Junction/reparse, mount/cross-volume, ACL changes, inode/file-ID reuse, hardlinks, rename and path reuse. Unknown cases fail closed.
- Multi-root manifest publication, stage ownership, crash recovery, watcher event-gap behavior, cancellation, CAS and bounded retry are implemented and tested; failures expose only old or new publication, never a mixed one.
- File Library, FTS, tag counts/mutations, historical details, content/FTS, duplicate membership/counts, analysis/classification, AI, Organization, Global Index bridge, cursors and frontend actions all pass the same manifest contract tests.
- Old-client exclusion, installer handoff, IPC identity and DB-generation isolation are accepted in #380; no raw stale-bit writer can run against a published manifest.
- Stage WAL/disk budget, pinned reader lease, GC and disk-full behavior are measured at representative scale without changing the 5,000 ms SQLite timeout. Measurement must include membership staging, not only pointer time.
- SQLite 3.51.3 exact-build FTS plans and later authorized latency qualification meet Owner-approved query targets; result-set/rank semantics are explicit.
- A combined migration, rollback/forward recovery, backup and release order is approved across #380/#381 without private schema allocation.

### Immediate NO-GO conditions

Any unowned legacy source treated as missing; path-only ownership used for destructive stale; one root hiding a still-live other-root membership; a consumer or action using raw `files.is_stale`; tags/history/derived evidence deleted or shown as active after source loss; old binary can mutate post-migration DB; pointer and candidate digest mismatch; incomplete watcher/coverage generation publishes; new and old reader observe a mixed manifest; GC cannot prove a generation unreferenced; FTS query plan leaks hidden rows or has an unexplained product-scale regression; Windows/macOS boundary claims based only on Linux fixtures.

## 12. Unresolved questions for Owner

1. Accept the proposed stable path-entry source identity distinct from `files.id` and dedupe physical identity? Which native identity tuple is authoritative on each platform, and what exact behavior is permitted for path-only observations?
2. Should non-stale legacy rows with unknown root ownership stay in a visible, read-only compatibility slice until claimed, or should they move to a separate review surface? Either decision must preserve all tags/history and prevent automatic stale.
3. Should an overlapping multi-root session publish all included roots atomically (preferred here) or allow explicit partial success with a visible partial-manifest response?
4. When a root is removed, should its current-only source be hidden as `unmanaged`, or retain a read-only Library row? The proposal retains all history and distinguishes this from missing.
5. Does the product require BM25 scores to be computed over only currently effective sources? If yes, a global external-content FTS table plus post-MATCH membership filter may not meet that semantic contract; an FTS projection or another indexed design may be required.
6. What durable reader/cursor lease lifetime is acceptable, and what disk-pressure action should be exposed before publication is refused?
7. What can #380 guarantee against an old app or Service process with a same-path replaced executable still holding a database handle? The accepted #380 S0 evidence does not answer this.
8. Are post-publication Watcher repair and action-boundary identity checks sufficient given the irreducible FS-to-SQLite race, or does the product require a stronger native notification/lease guarantee?

None of these questions prevents review of the proposed logical authority, but the first, second, third, fifth, seventh, and eighth block production GO until answered.

## 13. D4-S1 recommendation

`#381-D4-S1 READY_FOR_OWNER_ADR_REVIEW`

The root-membership and effective-visibility authority can be specified coherently enough for Owner design review. The research does **not** establish production feasibility, migration safety, FTS qualification, or native platform proof. Do not proceed to production implementation from this ADR alone.
