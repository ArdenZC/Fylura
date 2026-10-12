# D4 Unified Visibility — validation contract proposal

**Purpose:** define the evidence required before any production implementation can pass Owner review. These are future test contracts only. D4-S1 did not build or run these tests.

## Authority oracle

For a pinned manifest `M`, an independent test oracle computes `effective_visible(source, M)` from the manifest’s enabled root heads and complete generation-membership rows. Every production read API and UI action must match that oracle. No test may use `files.is_stale` as its expected-value source.

For each case, capture both the logical result and a retained-state checksum/count across:

- `files` row and source identity/incarnation;
- root/run/session and manifest pointer;
- `file_user_tags`, user tags, notes/feedback/history;
- file fingerprints, duplicate groups/members, analysis findings/evidence;
- content artifact/catalog/FTS;
- classification/AI jobs and results;
- Global Index entry/FTS/managed bridge rows;
- WAL size and `PRAGMA integrity_check` for failure recovery.

## Test matrix

| ID | Setup and interleaving | Required result |
|---|---|---|
| AUTH-01 | One complete root generation contains a source; open page/count/detail/search/selection/path/tag/content/duplicate/AI/Organization APIs. | All active surfaces expose the same source under the same `manifest_id`; each cursor/token reports that revision. |
| AUTH-02 | Stage a generation that omits a previously present source but do not publish. | Old manifest remains active; every API still sees prior visibility; uncommitted/staged FTS hits do not appear in user results or counts. |
| AUTH-03 | Publish an omitted source in a complete root generation. | New reads agree that it is unavailable; old open SQLite read transaction remains on the old snapshot; by-ID active detail and actions reject; history remains accessible explicitly. |
| AUTH-04 | Source exists under two roots; publish absence in only one root. | Source remains globally visible through the other root, is absent from root-scoped results for the changed root, and user metadata remains attached once. |
| AUTH-05 | Parent/child roots have separate generations; parent full scan omits child content or nested request is `covered`. | Child head is unchanged unless an explicit child generation is part of the same publication. No path-prefix inference deletes child membership. |
| AUTH-06 | Same path is presented with different platform volume/file identity from two roots. | Conflict fails closed; no identity merge, stale publication, tag transfer, or content reuse. |
| LEG-01 | Upgrade synthetic Schema 37 DB with non-stale rows: unique prefix, overlapping/nested roots, outside-root, prior exclusion-shaped path, missing identity, tags/history/content/FTS. | Every row/association remains present. Ambiguous rows stay `legacy_unverified`; no row becomes stale from prefix inference. |
| LEG-02 | Upgrade rows already marked stale, then create a new file at the same path with different identity. | Prior row and history stay retained; new incarnation receives no automatic tags/AI/content/fingerprint inheritance. |
| ROOT-01 | Root disabled/removed while a source has memberships in zero, one, or multiple other roots. | One manifest change; history retained; only sources with another effective membership remain active. Removal is distinct from missing. |
| ROOT-02 | Root path/volume changes, then scan fails, is cancelled, or succeeds. | Failure/cancel preserves old head; success changes that root’s head atomically; no implicit path-based transfer to a different source. |
| CAS-01 | Change run/root/session revision, lease token, cancel flag, watcher revision, coverage error, source identity or candidate digest between preflight and publish. | Pointer CAS rejects the entire candidate. The old manifest remains selected and no side-state invalidation is partially visible. |
| CAS-02 | Stage root A and root B; mutate a disjoint root C and race another publication. | Publisher either rebases the immutable root-head map after rechecking A/B or rejects; no lost C head and no partial A/B update. |
| CAS-03 | Concurrent user tag add/remove and metadata write in both orders around publication. | Changes serialize. A stale manifest-bound selection rejects or refreshes; successful user edits persist and are not overwritten by scanner/stale publication. |
| CAS-04 | Scanner or watcher observes source identity/path update while content/classification/AI/dedupe output is in flight. | Output publishes only after job/source/manifest CAS and identity recheck; otherwise it is retained as superseded and retried only from a new eligible snapshot. |
| WATCH-01 | Watcher exact delete, replacement, duplicate event, out-of-order event, overflow/gap, and event after preflight. | Only durable, correctly owned events update membership; a gap blocks reconciliation publication and retains old head with degraded health. Duplicate/out-of-order events are idempotent. |
| FS-01 | Recreate a missing path with same identity and then with a different identity before/during/after candidate publish. | Detectable change rejects the candidate or is repaired by a later watcher generation; different identity is a new incarnation. Action API rechecks the native identity immediately before filesystem mutation. |
| FS-02 | Symlink and platform-specific Junction/reparse/mount/ACL/volume boundary changes under an active root. | No traversal outside the authorized root; unknown boundary or identity never authorizes destructive stale. Test separately on Linux, Windows and macOS before platform claims. |
| DER-01 | After visibility loss, query tag usage, content/FTS, duplicate groups/counts, findings, classification, AI history, Organization and Global Index. | Active results consistently filter by manifest; retained metadata and history remain unchanged; global stale state is not changed by managed publication. |
| DER-02 | Change file `name/path/rowid` and perform file insert/delete while visibility remains live. | Existing FTS INSERT/DELETE/rename/path behavior remains correct; hidden rows never leak through raw MATCH APIs; conditional trigger behavior is covered by the coordinated migration. |
| FAIL-01 | Inject failures at every stage chunk, manifest-map insert, pointer CAS, pointer write, trigger, commit and response serialization. | Before commit: old pointer and data are intact. After commit: new pointer is authoritative and recovery marks publication complete. `integrity_check=ok`; no mixed root map. |
| FAIL-02 | Kill process after each durable stage state and restart through main startup recovery. | `recover_interrupted_scan_runs`, domain recovery and watcher recovery have a deterministic owner/status result; no orphan is guessed to be published. |
| FAIL-03 | Fill disk during staging and pointer transaction; grow WAL while a reader pins an old manifest; crash during chunked GC. | Old published state remains usable; no history is deleted for space; publication pauses/fails with a durable diagnostic; GC is idempotent and cannot delete referenced data. |
| CURSOR-01 | Hold page cursor and detail/Organization plan across pointer flip and GC. | Cursor/plan either reads a leased old manifest consistently or returns a stable stale-token response; no mixed-page results. GC waits for lease release/expiry policy. |
| IPC-01 | Run an old client binary/Service process against a post-migration DB, including same-path binary replacement and stale open handle. | #380 generation gate rejects access before mutations. If this cannot be demonstrated, production GO is blocked. |
| FTS-PLAN-01 | Capture EQP for exact SQLite 3.51.3 linked-library ranked, unranked, count and cursor query forms with effective-membership filtering. | Record source ID, compile options, query text, match binding, plan nodes/join order/temp sort. Result IDs satisfy the authority oracle. This is plan-only; no large timing run. |
| FTS-QUAL-02 | Later Owner-authorized exact-build benchmark after plan review. | Meets separately approved latency/rank semantics and does not rely on host SQLite 3.53.1 numbers. Not part of D4-S1. |

## Measurement gates for a future implementation

- Use the actual combined migrated Schema, indexes, triggers and data relationships. Include 10k and 100k root-membership changes and retain all prior rows/associations.
- Keep SQLite `busy_timeout=5000 ms`. Measure stage-chunk lock times and Writer waits, final publication transaction, COMMIT, WAL growth, disk peak, query plans and recovery time separately.
- Concurrent ordinary metadata and tag writers must complete under 5 seconds while staging and publication; publication must never partially commit after a CAS failure.
- Exact counts and deterministic digest are required for each published root generation. Count-only candidate equality is insufficient.
- Run platform-specific file identity/boundary validation only after a separate authorized test environment is available. Linux tests do not stand in for Windows/macOS.

## Stop conditions

Stop and report `DESIGN_AUTHORITY_UNRESOLVED` if the implementation cannot distinguish source incarnation from path reuse, cannot define multi-root ownership without path inference, or cannot make every reader/writer use the same manifest. Stop production work if any test exposes old/new mixed visibility, unowned stale deletion, metadata loss, old-client writes, unexplained product FTS regression, or GC that can collect a pinned/published generation.
