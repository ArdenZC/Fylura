# ADR-004 — Root-Scoped Visibility Authority

**Status:** D4-S5 research candidate for Owner review. **D4 production remains NO-GO.**

**Frozen parent:** D4-S4 `ecbf19d5fb68b7b59777056e63e26cc95922b439` (Tree `f31ed1f2b8e959e690f427f273aa7a3fa0b0ed28`).

**Scope:** offline, test-only SQLite SQL and Harness under `docs/project/research/issue-381-d4-s5/`. No production code, formal Schema, IPC, installer, master, PR #383, PR #386, or Schema v38 was changed.

## Decision question

D4-S4 seals all current roots against one global `d4s4_authority_clock`. This fails closed, but a Watcher or source evidence change in root A makes the global revision differ from every Manifest seal. The current view then hides independent files in root B until another Manifest publishes. The second issue is transaction atomicity: the S4 claim-revocation example used an autocommit connection and executed the source revision update separately from the append-only revocation insert.

D4-S5 tests whether one effective visibility relation can use root-local revisions for root evidence and source-local revisions for source identity evidence, while preserving a separate membership row per root. It also tests a controlled, explicit `BEGIN IMMEDIATE` transaction for claim revocation.

## Research recommendation

Use **Root Membership + Effective Visibility** as the next production-design candidate:

1. Treat each published `(manifest, root, generation)` membership as an independently sealed fact.
2. Track a monotonic `root_authority_revision` for each root and a monotonic `source_authority_revision` for each source incarnation.
3. Derive current files by unioning only the root memberships whose root and source seals still match their current revisions and whose transitive evidence still matches.
4. Route File Library reads and search through one effective relation. Keep the path projection as an immutable lookup projection, not as the authority.
5. Bind a cross-transaction cursor to the active pointer revision and a deterministic digest of its effective root/source seal vector. Reject the cursor when either changes; do not reconstruct an old FTS snapshot.

This is a research recommendation, not a final production data-model decision. The Harness establishes the relational behavior on small synthetic Schema 37-derived data. It does not prove native file identity, filesystem observation atomicity, production query migration completeness, or acceptable query cost.

## Revision domains and fields

| Evidence domain | Fields / rows that affect it | Revision owner | Current-view effect |
|---|---|---|---|
| Root ownership and scope | `scan_roots.normalized_path`, `source_kind`, `enabled`, `health_status`, `needs_reconciliation` | Root-scoped revision | Invalidates memberships for that root until a valid replacement generation publishes |
| Watcher coverage | `scan_roots.watcher_revision`, `watcher_applied_revision`; the cited run and generation watcher revisions | Root-scoped revision | Invalidates only that root when unapplied events or mismatched coverage exist |
| Source incarnation identity | `file_sources.status`, `identity_confidence`, `source_revision`, `retired_at` | Source-scoped revision | Invalidates every root membership referring to that same source incarnation |
| Claim evidence | Immutable `source_identity_claims`; append-only `d4s4_claim_revocations` | Source-scoped revision, once per logical event | Revocation removes the claim from current trust without deleting the historical claim |
| Path and physical observations | Immutable `file_path_entries` and referenced `file_physical_identity_epochs` | Evidence is immutable; correction adds a new observation/source generation | Exact path-entry and physical identity seal must continue to match |
| Scan coverage | `scan_runs.status`, `coverage_complete`, run watcher snapshot; generation status and members | Immutable after cited publication; replacement uses a new run/generation | Incomplete or changed cited coverage cannot qualify a current membership |
| Manifest publication | Root heads, membership relation, path projection, digest seals, active pointer and pointer revision | Trusted publisher and pointer CAS | New complete manifest switches the current root set atomically; previous manifests remain history |
| User-owned metadata | `files.context`, tags, notes, and other independent user fields | Existing user-data writers | Does not advance source/root authority and is retained when a source is hidden |

The broad Schema 37 `scan_roots.revision` also participates in operational compare-and-swap and lease behavior. It must not be treated as the continuous root visibility version. The prototype uses a separate root-scoped authority row and compares semantic root evidence and Watcher coverage.

## Multi-root and overlap semantics

- **Independent roots and sources:** a change to A's Watcher revision increments A's root revision. B's root seal remains eligible. A source identity update increments only that source's authority revision; a different source in B remains eligible.
- **Overlapping roots for the same file:** both root generations contain the same `source_id` and `path_entry_id`. If A's Watcher evidence changes, A's membership is removed from the effective relation and B's membership may keep the file visible while B's root seal and the shared source/path/physical identity evidence remain valid.
- **Shared source identity failure:** the same source incarnation cannot be trusted by B after its physical/source identity evidence becomes invalid. Every membership referencing that source is removed. The test retains a separate B companion source to verify that only the invalid shared source is hidden.
- **Last membership removed:** a complete successor Manifest may publish replacement generations that omit the source from every root. The file disappears from current reads while its source, old Manifest, tags, notes, and history remain retained.
- **Same-path reuse:** an old source and a different new source at the same path have distinct `source_id` and `file_id`. The new Manifest selects the new incarnation; the old row's tags and note remain on the old `file_id` and are not copied to the new file.

There is no path-based fallback that can promote an old source, a projection row, or another root's membership after the source's own identity seal has failed.

## Read, search, and cursor contract

`d4s5_effective_root_members` is the root-aware relation. It joins the active pointer, root heads, generation members, the corresponding immutable path projection, source identity, physical/path evidence, cited run coverage, root seal, and source seal. It compares the current root revision only with that root's seal, and current source revision only with that source's seal. It does not read the D4-S4 global clock.

`d4s5_effective_file_sources` is a distinct-source projection over that relation. In a production design, File Library reads and FTS search must both join this same effective relation. A search that uses the FTS table alone or a legacy union projection can expose a root membership that lost authority. The Harness verifies this with old union rows retained after root invalidation, current search of the invalid source returning zero, and search of the still-valid root returning one row.

A cursor token contains the active Manifest ID and pointer revision plus a digest of the sorted eligible `(root_id, generation_id, root_revision, source_id, path_entry_id, source_revision)` vector. A later read transaction recomputes this token. If it differs, reject and restart the query. SQLite readers already in a transaction continue to see their original MVCC snapshot; this is internally consistent. A long-lived reader must not be described as current after it ends, and a cursor cannot replay stale FTS membership across a new transaction.

## Controlled source-claim revocation

The research entry point uses `isolation_level=None` and explicitly performs:

```sql
BEGIN IMMEDIATE;
UPDATE file_sources
   SET identity_confidence = 'path_only',
       source_revision = source_revision + 1
 WHERE source_id = :source_id
   AND source_revision = :expected_source_revision;
-- a trigger records one unique (event_id, source) authority event and advances
-- d4s5_source_authority.revision exactly once in this same transaction.
INSERT INTO d4s4_claim_revocations(claim_id, revoked_at, reason)
VALUES (:claim_id, :now, :reason);
COMMIT;
```

The transaction rejects a stale source revision CAS. The append-only claim is not deleted. The original Manifest and its seal, file row, tag, and user context remain. A root event uses a different event ID and advances only that root's revision. The same logical claim revoke uses one source event ID across the source update and revoke insertion, so the D4-S5 source revision advances once.

D4-S4's global diagnostic clock increments twice for the two separate SQL row changes in a successful revoke. It is deliberately not a visibility authority in this candidate. A production transition must replace the global clock as an effective-read gate rather than combine two competing current-visibility definitions.

Failure injection after `BEGIN IMMEDIATE`, after the source update and its authority trigger, after the revoke insert, before commit, and during a deferred-FK `COMMIT` failure leaves source revision, confidence, scoped revision, event, revocation, pointer, current view, claim, and user metadata unchanged. An independent reader sees the precommit snapshot while the writer holds uncommitted changes; after commit, that same snapshot remains internally consistent, while its next transaction and a reopened connection see the revoked source hidden.

## Publication protection

The test-only `AFTER UPDATE OF status ON visibility_manifests` trigger creates root/source scope seals only while both D4-S4 and D4-S5 publication capabilities are active. It runs in the same trusted D4-S4 publication transaction as the new Manifest status and active pointer CAS. The pointer and seals therefore do not become current separately. Seal rows and authority events are immutable; direct edits are rejected by test triggers.

The SQL does not claim SQLite computes SHA-256. S4's trusted Python publication oracle validates relation-set digests and transitive evidence inside its `BEGIN IMMEDIATE` publication transaction. The D5 triggers enforce scope revision/event correspondence and immutability. A production publication API still needs a documented least-privilege connection boundary and an audit that every read path consumes the effective relation.

## Rejected or deferred alternatives

- **One global authority clock:** safe against stale visibility but causes a root-local change to hide unrelated roots. D4-S4 reproduced this false hide; D4-S5 replaces it in the research view.
- **Per-root revision without per-source revision:** unsafe for one source shared across overlapping roots. A root B seal cannot make an invalid shared source identity trustworthy. D4-S5 therefore has two independent revision domains.
- **Trusting the union Manifest path projection:** unsafe because the projection can remain historically correct while one contributing root or its source evidence has lost current authority. D4-S5 computes membership from root heads and generation members, then checks the projection row.
- **Cross-transaction cursor replay:** rejected because it would require reconstructing old FTS/rank state. Stale cursor tokens are rejected.
- **Deleting or rewriting claim/source history:** rejected; revocation is an append-only event and publication changes by a new Manifest/generation.

## Failure and recovery cases covered

| Case | Expected result | Harness evidence |
|---|---|---|
| Root A Watcher revision changes | A hidden; unaffected B visible/searchable; old cursor rejected | `root_scoped_false_hide_regression` |
| Independent source A identity becomes conflicted | A hidden; independent B remains visible/searchable | `independent_source_identity_scope` |
| Shared source identity becomes conflicted in overlapping roots | That source hidden from A and B; independent B companion remains | `overlapping_shared_source_identity` |
| Root A loses eligibility in overlapping roots | B remains only while its root and shared source/path/physical evidence still qualify | `overlapping_root_and_last_membership` |
| Last root generation omits source | Source hidden; unrelated B source stays visible; historical state retained | `overlapping_root_and_last_membership` |
| Old path reused by a new file incarnation | New source visible; old source/user data/history do not transfer | `same_path_new_incarnation_isolation` |
| Claim revoke fails at any transaction phase | No partial source revision, authority, or revoke state | `claim_revoke_failure_injection` |
| Reader races with commit / process reconnects | Precommit reader snapshot consistent; later transaction and reopen see committed state | `claim_revoke_reader_commit_reconnect` |

## Preconditions before any production GO

1. Owner accepts the source/root authority boundaries and the shared-source overlap semantics.
2. Audit and migrate every File Library, FTS, pagination, tag, history, deduplication, content-analysis, classification, AI, Organization, Watcher, Scanner, and Global Index bridge read to the single effective visibility contract. No current result may be authorized only by `files.is_stale` or a path projection.
3. Prove publication, source mutation, root mutation, Watcher, scanner generation, cancellation, and cursor CAS behavior under service concurrency and process restart.
4. Validate existing Schema 37 rows, old source/file foreign keys, metadata retention, and path reuse on a migration candidate. Coordinate database/client generation isolation, installer ordering, and old-client rejection with #380 before assigning a schema version. Do not allocate v38 here.
5. Run broad FTS/BM25 query-plan verification on SQLite 3.51.3, then separately gate Windows/macOS native identity, WAL/GC/disk-space recovery, and production query performance.
6. Demonstrate that application process trust and database-file access assumptions match the actual #380 service/IPC boundary.

Until those gates are met, this ADR supports Owner review of offline relational evidence only. It does not authorize D4 production implementation.
