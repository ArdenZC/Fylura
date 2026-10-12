# ADR-003 — Transitive Evidence Seal and Trusted Manifest Publication

**Status:** D4-S4 research candidate; D4 production remains **NO-GO**. This ADR changes no product code, formal Schema version, IPC, installer, master, PR #383, or PR #386.

**Frozen parent:** D4-S3 `ad85df097d7a8425df01e1a95829c38fd17dd4d6`, Tree `58b0e5854237189275d54224d5ed80237d34c526`.

## Decision sought

Treat a published Manifest as authoritative only while its complete transitive evidence seal remains current. A Manifest's relation rows are not enough: the evidence that said a scan covered a root, that a source identity was accepted, and that a path belonged to that root must also be immutable or explicitly version-invalidated.

The research candidate adds an append-only seal per Manifest and a monotonically increasing authority revision. The candidate records root evidence, source/physical/path evidence, claim identifiers, deterministic counts and SHA-256 summaries. A trusted application publisher computes those summaries while holding `BEGIN IMMEDIATE`, then in the same transaction seals the generation, publishes the Manifest, performs pointer CAS and commits. A connection-local capability function makes direct DML status/pointer publication fail closed. It is an authorization hook, not a hash function or a cryptographic boundary.

## Reproduced failures in the prior prototype

Against the exact D4-S3 Schema 37 Harness and guards, but before the D4-S4 protection SQL:

| Post-publication mutation | Prior SQL result | Consequence observed |
|---|---|---|
| `scan_runs.coverage_complete = 0` | Accepted | Run coverage no longer supports publication; raw effective view still showed the source. |
| `scan_runs.status = 'failed'` | Accepted | A published generation now referred to a failed run; raw view still showed the source. |
| `file_sources.status` or `identity_confidence` change | Accepted | S3 identity Oracle reported invalid identity; raw view continued exposing it. |
| `file_physical_identity_epochs.evidence_digest` change | Accepted | S3 Oracle did not compare this digest at all; raw view remained visible. |
| Delete explicit `source_identity_claims` row | Accepted | S3 identity Oracle reported invalid identity; raw view continued exposing it. |
| Change `scan_roots.normalized_path` | Accepted | The path no longer belonged to the root, but root/member Oracle and raw view were unchanged. |

Separately, a valid membership candidate with a deliberately incorrect `member_digest` or `root_map_digest` passed the D3 SQL status and pointer triggers when a caller issued raw SQL. The Python Oracle rejected both candidates, but no SQL-side capability enforced that it had been called.

The exact counterexample rows and the before/after digest values are in `evidence/transitive-evidence-results.json`. D4-S3 evidence remains byte-for-byte untouched.

## Authority and revision rules

1. **Root/run evidence is frozen or invalidated by domain.** A published run's terminal `status`, `coverage_complete`, `stale_reconciliation_allowed`, and `watcher_revision_at_start` cannot be rewritten. A correction is a new scan run. `scan_roots.normalized_path`, `source_kind`, `enabled`, health/reconciliation state, and Watcher revisions are semantic authority inputs. Their compatible service mutation increments the authority revision. `display_name` is presentation metadata and does not invalidate visibility.
2. **`scan_roots.revision` is a publication CAS value, not a continuous visibility epoch.** Schema 37 increments it for operational lease and Watcher lifecycle transitions. Comparing it continuously to a published generation would hide a root during ordinary scan admission. The final publisher compares the candidate's `source_generations.root_revision` to the live value while holding the writer lock. A root configuration or Watcher change advances the separate authority revision. A production design must split semantic root revision from operational lease revision before GO.
3. **Source state is versioned.** A change to `file_sources.status`, `identity_confidence`, or retirement state requires `source_revision = old + 1` through the service evidence-mutation path. The path increments the authority revision. Current reads and cursors bound to the previous authority revision fail closed until a successor Manifest publishes.
4. **Physical identity observations and claims are append-only evidence.** Referenced physical identity epochs cannot be updated or deleted. A new observation creates a new identity/path evidence row. Explicit claims cannot be edited or deleted. Revocation is represented by an append-only event; it bumps `source_revision` and the authority revision and preserves the old claim record.
5. **Published generations, members, heads, projections, Manifest rows, and seal rows retain the D4-S3 freeze rules.** User metadata and tag joins are not authority evidence and remain writable. The test verifies `files.context`, `user_tags`, and `file_user_tags` writes leave the authority revision and current source set unchanged.
6. **Path/name changes are publication writes.** A current source's `files.path`/`name` update requires the publisher capability. The existing Schema 37 `files_au` trigger updates `files_fts` in the same transaction. Notes, context, classification fields, tags, and other file metadata remain independent.

The prototype uses one global authority clock. Any relevant root/source invalidation temporarily hides every current root, including unrelated roots, until a successor Manifest is published. This is intentionally fail-closed but may be too broad for production. A per-root revision map is a later design choice, not authorized here.

## Trusted publication transaction

The only supported entry is the service repository's `publish_visibility_manifest` operation. A caller cannot issue `UPDATE visibility_manifests SET status='published'` or change the pointer through the compatible DB connection because the status/pointer triggers require the private, zero-argument connection capability. The service arms the capability only inside this operation and resets it on every success or error path.

While holding `BEGIN IMMEDIATE`, the publisher must:

1. Re-read the active parent pointer and require the caller's expected pointer revision.
2. Verify each selected root/generation/run identity; root revision CAS; Watcher revision, applied revision, and run-start snapshot; terminal run state and complete coverage; no pending reconciliation; enabled root and compatible root kind.
3. Validate source revision/status/confidence, exact path-entry ownership, physical identity epoch and evidence digest, claim evidence/revocation state, and root boundary using platform-native path semantics.
4. Independently derive the selected member relation and the Manifest path projection. Require both SQL `EXCEPT` directions to be empty, with equal counts and deterministic application-side SHA-256 summaries for members, root map, and source evidence.
5. Seal the root/source evidence snapshots at the current authority revision. Seal each complete generation without changing its inputs.
6. Transition the ready Manifest to published, then insert/update the active pointer with parent/revision CAS.
7. Commit. Any validation, trigger, disk, or CAS failure rolls back seal, status, source path/FTS changes, and pointer together; the prior Manifest remains stored and the pointer remains unchanged.

### What is checked where

| Check | Trusted application Oracle inside the write transaction | SQLite trigger |
|---|---|---|
| SHA-256 over canonical row encodings | Computes member, root-map, source-evidence digests and compares stored values. | No SHA-256 function is assumed. Trigger cannot authenticate the digest bytes. |
| Root/generation/run/source revisions and claim state | Re-reads candidate and live evidence; verifies exact versions and no revocation. | Prevents unauthorized evidence DML or increments authority revision through the private mutation path. |
| Membership and projection completeness | Independently derives expected rows and checks summaries. | Rechecks both `EXCEPT` directions, path/source joins, generation counts and existing D3 invariants. |
| Publication authority | Private repository method controls the capability after Oracle success. | Rejects direct generation/Manifest status and pointer DML without the connection-local capability; D3 CAS triggers still validate parent/revision. |
| Rollback | Owns the single SQLite transaction and always rolls back on error. | `RAISE(ABORT)` aborts the attempted statement; transaction wrapper rolls back the full publication. |

SQLite itself cannot stop a process with unrestricted file access from registering a forged UDF or dropping triggers. The tested entrypoint protects the compatible service connection against accidental/raw SQL calls; it is not an adversarial-local-process security boundary. #380's compatible-client and database-generation isolation is a hard prerequisite.

## Cursor and visibility behavior

A cross-transaction Cursor carries `(manifest_id, pointer_revision, authority_revision, query/sort, last_key)`. Continuation succeeds only if all three authority coordinates still match. Pointer changes reject it as in D4-S3; a root/source/claim invalidation rejects it even if the pointer row itself has not changed. A transaction-pinned WAL reader continues to see its old snapshot only until that read transaction ends.

`d4s4_effective_file_sources` is the test-only authoritative view. It joins the D3 relational view to the current seal, root snapshot, source snapshot, and physical/path evidence. A changed evidence clock yields no current rows. The old D3 `effective_file_sources` view intentionally remains a raw relation and still shows rows after evidence invalidation; all future product consumers must use the authority-filtered view, never that raw view.

## Compatibility and retained state

The S2 physical model keeps `files.id` as the stable FK anchor and uses a separate `source_id` for source incarnation. A new file reusing an old path gets a new ID/source and cannot inherit the old row's tags or associated metadata. D4-S4 does not change that model.

The root/source evidence seal is additional provenance. `files`, FTS, tags, notes, classification history, fingerprints, duplicate groups, content artifacts, analysis findings and AI records remain retained under existing relationships. Staling/removing a root is not deletion or garbage collection. User tags and notes are independent of visibility authority and are not frozen by the seal.

## #380 shared migration requirements

No Schema version is allocated. Schema v38 remains unassigned pending cross-Track Owner coordination. Before production migration:

- #380 must approve the single compatible installer/IPC/service/database-generation gate and prove old writers cannot open the upgraded DB.
- #380 and #381 Owners must agree on version number, ordering, backup/rollback semantics, and one-writer handoff before either Track reserves a migration.
- The migration must atomically create physical incarnation relations, transitive evidence/seal relations, authorization triggers, and compatible FTS state, or expose a recoverable journal state that keeps the old database generation authoritative.
- Every production connection must install the private capability only in the approved service repository; no legacy client or unwrapped connection may publish.
- Before opening the new generation, migration validation must check foreign keys, SQLite integrity, retained association counts/digests, root/source seals, FTS integrity, and pointer recovery. Failure keeps the prior generation usable and must not expose a partial Manifest.

## Remaining risks and production gate

- The synthetic evidence digest is an opaque value. The Harness binds the full row tuple into its source seal but cannot prove a native Windows/macOS file identity or a real evidence digest's provenance.
- Root boundary validation is POSIX-only in this Harness. Junctions, symlinks, ACL changes, mount/volume identity, case folding, and path replacement race remain outside proof.
- One global authority clock invalidates all roots on one evidence change; a safe, efficient per-root design has not been tested.
- Existing production File Library, search, classification, dedupe, content, organization, AI, and Global Index paths read `files.is_stale` or related legacy state directly. None consume the proposed Manifest view today.
- The #380 client/DB-generation gate is not implemented or verified here. No Schema v38 is assigned.
- SQLite 3.51.3 product FTS/BM25, native Windows/macOS behavior, GC/read leases, WAL/space exhaustion, full migration and performance remain separate gates.

**D4-S4 conclusion:** the isolated prototype closes the reproduced SQL DML bypasses on its compatible connection and gives changed evidence an explicit current-visibility/cursor invalidation version. This supports Owner review of a research contract only. **Production GO remains NO-GO.**
