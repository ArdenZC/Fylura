# D4-S2 physical identity validation contract

**State:** design/test contract only. Production implementation remains NO-GO.

**Frozen input:** Schema 37 at master `3275ac7c72df8ae4139a30f16c7f649971324e97`; D4-S1 ADR at `bb46ba8bd0d4a2a01af976fb9b05d6c4224b4697`.

## Authority oracle

For an active manifest `M`, every visible row must join through exactly one `source_id → files.id` crosswalk, one selected manifest path projection, and at least one complete published root generation that contains the exact same `(source_id, path_entry_id)` evidence. `files.is_stale`, lexical path prefix, FTS rowid, file hash, old `files.id` path text, or Global Index path match cannot independently authorize visibility.

The candidate manifest must satisfy both uniqueness rules before pointer CAS:

```text
UNIQUE(manifest_id, source_id)   -- one path entry per visible source
UNIQUE(manifest_id, path_key)    -- one visible source per normalized path
```

Every path key includes an OS-aware normalization version. If two roots disagree on source or path evidence, or a source has two effective paths, the candidate is rejected and the prior pointer remains selected.

## Validation matrix

| ID | Setup/change | Required oracle |
|---|---|---|
| PHYS-01 | Same-path deleted source and different-identity replacement | Old and new source IDs and file row IDs differ. Old associations remain on old `files.id`; new source receives none. Path-only, name-only, hash-only, or stale-bit equality never links them. |
| PHYS-02 | Verified same-entry rename within one volume | Source ID and FK row remain; a new path-entry record is appended; old path history remains; path projection and FTS update in the same manifest publication. |
| PHYS-03 | Cross-volume move, copy/delete, or identity gap | New source unless a platform-specific contract proves continuity. Matching content hash is insufficient. |
| PHYS-04 | Two hard-link paths with one current native physical epoch | Distinct source IDs and path-entry IDs, same verified physical-identity ID; separate tags/user corrections; dedupe may link them. |
| PHYS-05 | OS file ID reuse after delete, volume detach/re-attach, unsupported identity | New physical epoch or unknown identity. No unique tuple constraint or path fallback can merge sources. |
| PHYS-06 | Symlink, Junction/reparse point, mount, cross-volume boundary, ACL change | Handle-based root and leaf validation; unresolved boundary rejects membership/publication. Platform coverage is separate on Windows/macOS/Linux. |
| ROOT-01 | Same source/path is referenced by roots A and B; publish removal of A only | Source stays visible through B. No side table is invalidated because one membership ended. |
| ROOT-02 | Remove final effective root membership | Source becomes inactive for list, count, FTS/snippet, tags, content, duplicates, analysis, AI, and path action. All rows remain. |
| ROOT-03 | One included root fails/cancels/loses coverage in a multi-root session | No session pointer swap; every prior head remains. An explicit retry subset is a separate operation and response. |
| ROOT-04 | Parent/nested roots and different physical identity at same normalized path | Root membership is explicit. A conflicting candidate fails manifest path uniqueness; path-prefix precedence is never used. |
| LEG-01 | Migrate every Schema 37 `files` row with both `is_stale` values, duplicate legacy paths introduced only after migration, missing fingerprints, old user data | Preserve rowids, IDs, all columns and child values. Every old source is `legacy_unverified`, with no active membership or current path. Live legacy records are read-only compatibility/history items. |
| LEG-02 | Scanner sees a file at an unverified legacy path | Create a new opaque source and file row. Do not attach old tags, notes, classification, history, fingerprint, content, duplicate or AI state. |
| LEG-03 | User selects a specific legacy row and explicitly authorizes association to a native-observed path | Append actor, authorization digest, source/path identity evidence and claim kind. Keep old source only if no verified identity conflict. A manual transfer to a new source is a separate audited copy action. |
| FK-01 | Enumerate/rebuild Schema 37 | Preserve direct FKs from tags, fingerprints, duplicate groups, content artifacts and content runs. After migration: `PRAGMA foreign_key_check` empty. No stale/root operation deletes a parent row. |
| HIST-01 | Old classification history/feedback, findings/evidence, operation log, tags/notes, content, duplicate and Global Index AI rows | Preserve all values. Rows without source FK remain history-only until a separately designed source link exists. Global AI stays on Global Index authority. |
| FTS-01 | Preserve files rowids while rebuilding the table; insert/rename/delete source rows | `files_fts` external-content integrity check succeeds; insert/update/delete semantics remain. Raw MATCH can retain historical candidate terms, while effective search returns only pinned-manifest source IDs. |
| FTS-02 | Compare raw MATCH IDs, active search IDs, snippets, count, cursor and BM25 under pinned manifest | IDs/snippets/count never leak hidden sources. BM25 corpus and IDF must match the approved effective corpus. If the 3.51.3 plan ranks over hidden rows, block release or obtain explicit Owner acceptance of a documented alternative. |
| CAS-01 | Change source/path revision, root/path revision, run/session lease, cancellation or generation during preflight | Pointer CAS rejects the whole candidate. No source, file payload, or side-state partial publication. |
| CAS-02 | Watcher revision/event gap before publication | Reject candidate, retain old manifest, mark reconciliation required. Event after commit degrades root and is repaired through the same source/root ownership path. |
| CAS-03 | User metadata/tag update races publication | Independent domain revisions and SQLite serialization preserve committed user changes. A manifest-bound target selection rejects/refreshes after pointer change. |
| CAS-04 | AI/content/classification/fingerprint/dedupe worker finishes after source or manifest changed | Verify source ID, source revision, input/domain/policy revision, cancellation and active visibility inside its commit transaction. Retain superseded history; do not apply output to a new same-path source. |
| MIG-01 | Force a SQL constraint failure after files rebuild but before migration COMMIT | Full rollback: Schema 37 unique path, rows, FKs, indexes, triggers and FTS remain. No migration marker/crosswalk. |
| MIG-02 | Terminate process after DDL and relation creation but before COMMIT; restart | SQLite presents original Schema 37 or fully committed state only. Verify marker, ID crosswalk digest, row counts, `foreign_key_check`, `integrity_check`, FTS integrity; rerun once idempotently. |
| MIG-03 | Repeat migration after committed marker | No duplicate source IDs or history rows; same DDL/input digest and relation counts. Schema version is coordinated elsewhere. |
| MIG-04 | Old binary/service or IPC opens upgraded DB, including same path/name executable | #380 database-generation gate rejects before mutation. No raw `files.is_stale` fallback or path-as-ID upsert. |
| LEASE-01 | Cursor, analysis/AI job, organization plan, or cross-transaction reader spans pointer flip | It stays pinned to one manifest under a durable lease or returns stable expired-token status. It cannot mix old and new pages. |
| GC-01 | Prune a source whose history, published generation, FTS state, run, job, cursor, or reader is retained | GC refuses deletion. Unknown ownership means retain. |
| GC-02 | Crash/space exhaustion during explicit GC | Authorized chunks are idempotent; published/pinned data remains. Staging pauses instead of deleting evidence. |
| IPC-01 | Upgrade, downgrade, restore, and concurrent client scenarios | #380 exclusive generation handshake precedes migration and all writes. Rollback is a verified whole-database restore with clients drained. |

## D4-S2 Harness scope and evidence

Run the one-time harness with:

```sh
python3 docs/project/research/issue-381-d4-s2/harness/run.py
```

The script constructs the full captured Schema 37 DDL in a disposable database and tests LEG-01 through LEG-03, PHYS-01 through PHYS-04 as fixture identities, ROOT-01/ROOT-02, FK-01, HIST-01, FTS-01, MIG-01 through MIG-03, and source-row deletion protection. It includes hard process exit before COMMIT and an idempotent startup rerun. Raw assertion results and SQL are under this directory's `evidence/` and `harness/`.

This harness uses SQLite 3.53.1 from the workspace Python runtime. It does not claim exact product SQLite 3.51.3 behavior, native inode/file-ID continuity, real filesystem boundary safety, multi-process app/service compatibility, Windows/macOS behavior, or performance. It ran no CI or benchmark. Remaining contract rows are future production qualification gates, not passed tests.

## Required sequence before any production GO

1. Owner accepts source/path/physical identity meanings, explicit legacy claim, path collision and multi-root session semantics.
2. #380 Owner accepts exclusive DB generation/client/IPC gate; both tracks coordinate one unnumbered migration plan and version owner.
3. Inventory all schema FKs, non-FK source references, FTS triggers, search/snippet/count/cursor code, current path-as-ID writers, and Global Index bridge; assign migration owners.
4. Add old-fails/new-passes tests before production edits. Verify real Schema 37 migration idempotency, FTS rowids, complete association retention, all failure modes, and old-client rejection.
5. Validate linked product SQLite 3.51.3 query plans and visible-corpus ranking. No broad benchmark before plan/correctness review.
6. Run platform-native filesystem identity and boundary suites on Windows/macOS/Linux; test Watcher gaps and filesystem/commit race semantics.
7. Only after separately authorized implementation, measure writer waits, migration/WAL/disk, leases, GC, and recovery at representative data sizes. Keep the existing busy timeout.
