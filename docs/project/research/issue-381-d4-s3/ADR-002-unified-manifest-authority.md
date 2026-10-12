# ADR-002 — Unified Manifest Authority and Published-Object Protection

Status: candidate for Owner review. D4 production implementation remains NO-GO. No production database schema or migration is proposed as approved in this ADR.

## Context

D4-S2 accepted the source-incarnation physical model as feasible but did not prove that the derived `manifest_file_paths` projection was complete or immutable. Its Schema 37 candidate permits a caller to supply root generations and path projections independently. The constraints prevent duplicate paths and duplicate source entries, but do not prove equality between a selected generation's members and the projection. Published generation members, root heads, projections, generation metadata, and manifest rows also lack a complete DML immutability contract.

D4-S3 starts at D4-S2 HEAD `0a29c5b0114744f810de90ad31146394ce7be399`, Tree `7de3c7478868dd17b53884e9481d1d6ce60b7b99`. The only changed paths in this successor are below `docs/project/research/issue-381-d4-s3/`.

## Proposed authority definition

For a candidate manifest `M`, let `H(M)` be its root heads. A selected head is eligible only when it refers to the exact root/generation pair, the scan run is `completed` with `coverage_complete=1`, and the generation is `complete` or already sealed as `published`. Its stored member count and deterministic member digest must match the actual member rows.

The expected current-member relation is the distinct union over all selected root generations:

```text
E(M) = DISTINCT(source_id, path_entry_id, path_key, path_text)
       from H(M) -> source_generation_members -> file_path_entries
```

The published path projection is:

```text
P(M) = (source_id, path_entry_id, path_key, projected_path)
       from manifest_file_paths where manifest_id = M
```

Publication requires both `E(M) EXCEPT P(M) = empty` and `P(M) EXCEPT E(M) = empty`. Counts and SHA-256 summaries are recorded for both sorted relations. Canonical summary input is UTF-8 JSON arrays with no insignificant whitespace; rows are sorted by their canonical JSON bytes. The root-to-generation map and each generation's member tuples use the same deterministic encoding.

The relation is keyed by source incarnation and path-entry identity, not by a path string. Multiple roots may report the exact same `(source_id, path_entry_id)` and that tuple is deduplicated. If one source resolves to more than one distinct current path entry across the selected roots, publication fails closed; resolving such a conflict needs a new identity decision and a new generation, not a projection tie-break.

Every selected member must also pass these checks:

- `source_id` exists and is `observed`; identity confidence is `native_verified`, or `user_claimed` with an explicit `source_identity_claims` record for the exact path entry.
- `(source_id, path_entry_id)` resolves through the composite foreign key. The path entry is `verified`, has nonempty evidence digest and physical identity reference, and its physical identity epoch has nonempty platform file identity and epoch data.
- The generation member's identity digest equals the referenced path entry's evidence digest. The projection's path key and path text equal that same path-entry row; a caller cannot substitute an independently supplied path.
- The projection has no extra source, path entry, key, or path value. A valid foreign key alone does not make a projection authoritative.

These checks establish database evidence consistency. They do not independently prove that the current file system still has the recorded object; the native Scanner must obtain and bind current observation evidence before creating the generation. This Harness uses synthetic identity tokens.

## Publication and immutability contract

The supported publication sequence is one SQLite write transaction:

1. Prepare the candidate manifest, root heads, path projection, and generation member snapshots.
2. Re-read the candidate inputs in that transaction. Verify run completion/coverage, per-generation member count and digest, root-map digest, source/path identity joins, no source path conflict, and both directions of the set comparison. SQLite's writer lock prevents another SQLite writer from changing these rows between the final Oracle and commit.
3. Seal each selected complete generation as published without changing its source data.
4. Transition the ready manifest to published. The SQL trigger independently checks generation eligibility, identity joins, and both set differences.
5. Insert the initial pointer or update it with compare-and-swap: target manifest is published, its parent equals the old pointer manifest, and its revision is exactly old revision plus one.
6. Commit. Readers either observe the previous pointer and all previous rows or the new pointer and all new rows.

The research SQL rejects DML changes to published `source_generation_members`, `source_generations`, `manifest_root_heads`, `manifest_file_paths`, and `visibility_manifests`. A generation may transition from `complete` to `published` only without changing its generation identity, root/run, revision, member count/digest, or completion time. A published manifest is immutable even after a newer pointer replaces it. The pointer is the sole active-publication switch; it cannot be deleted or pointed at an unpublished/non-child manifest.

The 16 mutation cases in the Harness cover insert/update/delete attempts on those five published-object tables, direct published-row insertion, and a pointer revision that does not satisfy CAS. The protections are SQL DML guards on the compatible database process; they do not defend against an actor who can replace or drop database schema objects. #380's client/database-generation isolation must prevent an older writer from bypassing the new contract.

## Cursor and historical-read rule

Every cross-transaction list/search Cursor carries at least `(manifest_id, manifest_revision)` plus the query/sort parameters and last key. Before executing a continuation, the service compares both values to the active pointer. Any mismatch returns a stale-cursor response and requires a fresh first page. It does not replay an older FTS state across transactions.

Multiple pages in one short SQLite read transaction use that transaction's pinned WAL snapshot and see one pointer, one path projection, and one `files`/FTS state. A read transaction is not to be held across user interaction. Once it ends, its Cursor is subject to the active-pointer check.

Historical data remains available through a separate read-only history API keyed by immutable `source_id` and `path_entry_id`. That API reads retained `file_path_entries` and associated historical metadata directly; it does not promote an old path into the current File Library and does not promise historical FTS/BM25 replay.

## Rename, `files_fts`, and query scope

For a verified rename, append a new immutable `file_path_entries` row, prepare the new root generation and projection, and update the retained `files` row's current `path`/`name` in the same publication transaction as the new Manifest pointer. The existing FTS update trigger removes old name/path tokens and indexes the new values in that transaction. The new Manifest points at the new path entry. The old Manifest and old path entry remain as historical records. The test confirms that a reader already pinned to the old WAL snapshot still sees its old FTS term; after commit, a fresh transaction finds the new term and not the old term.

The existing external-content `files_fts` indexes every row in `files`, including rows hidden by the active Manifest. Applying the active-visibility join after FTS matching filters visible rows but does not change the FTS5 BM25 corpus statistics. The small plan probe found two raw `txt` matches and one active match. Therefore BM25 cannot yet be claimed to rank only active-manifest content. Before production GO, the FTS design must either provide an active-manifest-scoped corpus/rank calculation or explicitly obtain Owner approval for different ranking semantics, with consistency tests on SQLite 3.51.3. D4-S3 did not build a replacement FTS index or run a large benchmark.

## Metadata, tags, history, and downstream relationships

The path projection controls which source incarnations appear as current File Library/list/search results. Stale or unpublished sources and their `files` rows remain stored. Tags, notes, classification history, fingerprints, duplicate groups, content artifacts, analysis findings, and AI state keep their Schema 37 `files.id` foreign-key anchor and are never reassigned by path equality. D4-S2's 15 assertion groups are rerun unchanged and continue to check path reuse, old row/rowid, tags, history, content/FTS, deduplication, AI retention, FKs, and transaction/process recovery.

Publication does not itself authorize destructive cleanup or mutate user metadata. Any future metadata/tag/AI write contract must bind its operation to the stable source/file identity and source revision; it must not update an old incarnation because a new file reuses the same path. FTS-visible values that change during rename must commit atomically with the new pointer. Reader cursors bound to the prior pointer expire. Watcher/Scanner changes invalidate a prepared generation through their root/session and watcher revisions; publication retries by preparing a new candidate rather than merging a stale projection.

Generation GC remains blocked by a published Manifest reference, a reader on its pinned snapshot/lease, or retained history/reference data. GC must be explicit and retention-aware; ordinary root removal or path reuse does not delete member history, tags, or associated rows. This D4-S3 Harness does not qualify a production GC algorithm or lease lifecycle.

## #380 shared migration prerequisites

No Schema version is allocated by this ADR. The possible version 38 remains unassigned and requires coordination with #380 before any migration can be approved.

Production migration is blocked until the #380 Owner-approved installer, IPC, service, and database-generation design establishes that all writers for an upgraded database use the compatible generation. This ADR does not invent or implement the old-client upgrade gate. The coordinated migration must:

- agree on the single version and ordering across #380 and #381 before release work;
- stop mixed-generation writers using the separately approved #380 mechanism before changing Schema 37 physical tables, FTS, or Manifest authority;
- run the physical rebuild, association/FK preservation, FTS recreation, authority backfill, and journal/trigger setup atomically or with an explicitly recoverable migration journal;
- validate `foreign_key_check`, SQLite integrity, FTS integrity, retained row/rowid and association digests, and any recovered active pointer before opening the upgraded database to normal services;
- preserve a recoverable Schema 37 database until the coordinated upgrade is committed and validated; failed migration leaves the old generation usable and must not expose a partial Manifest; and
- define rollback and backup restore compatibility before assigning the version.

## Rejected publication shortcuts

- Caller-supplied projection without the two-way Oracle: reproduces omission, wrong-path, and phantom-member failures.
- Relying on `manifest_file_paths` uniqueness constraints: uniqueness is not completeness or identity provenance.
- Mutating an already published generation or projection in place: old readers and old Cursors can observe membership changes without pointer revision.
- Replaying previous-page FTS/BM25 after a cross-transaction Manifest change: SQLite's old FTS snapshot is not a durable per-Manifest rank snapshot.
- Treating a database-level path string as source identity: path reuse can transfer the old source's user data and old identity evidence.
- Extending an unpinned read transaction across UI interaction: it can retain WAL pages and does not define a durable history API.

## Remaining proof gaps and production gate

The research Harness proves the specified relational invariants on a synthetic fixture over the full captured Schema 37 DDL. It does not prove native filesystem observation correctness, Windows reparse/ACL/volume identity, watcher completeness, cross-process generation isolation, a full-user-database migration, long-lived reader/GC behavior, space exhaustion recovery, or exact SQLite 3.51.3 FTS/BM25 behavior. SQL triggers also cannot stop a schema owner from dropping those triggers.

Production remains NO-GO until Owner accepts the physical/authority contract, #380's database-generation migration dependency is concretely resolved, exact-product SQLite 3.51.3 FTS semantics are validated, native source evidence and revisions are wired through every writer/reader, and focused platform/migration/performance qualification is authorized.
