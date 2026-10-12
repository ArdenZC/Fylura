# Root-Scoped Visibility and Evidence Mutation Contract

This is a research-only contract for the D4-S5 SQL prototype. It preserves the accepted D4-S2 physical source model and D4-S3/S4 manifest contracts; it does not change product behavior.

## Effective authority

The current visible relation is the union of independently eligible root memberships:

```text
EffectiveFile(source, path_entry, file, manifest) = DISTINCT
  { member(root, source, path_entry)
    | active_pointer(manifest)
    ∧ manifest_head(manifest, root, generation)
    ∧ generation_member(generation, source, path_entry)
    ∧ manifest_projection(manifest, source, path_entry)
    ∧ root_seal(manifest, root, root_revision)
    ∧ root_authority(root) = root_revision
    ∧ source_seal(manifest, source, path_entry, source_revision)
    ∧ source_authority(source) = source_revision
    ∧ source_revision_current = source_revision
    ∧ run_coverage_complete(root, generation)
    ∧ path_identity_and_physical_identity_match(source, path_entry)
    ∧ active_claim_evidence_if_required(source, path_entry) }
```

`manifest_file_paths` remains a derived path lookup with its uniqueness constraints. It is not independently sufficient to establish current visibility. `d4s5_unqualified_projection_rows` exposes active projection rows that have no eligible root/source relation so tests can distinguish retained history from visible authority.

## Scope revision rules

| Event | Scope advanced | Rows that stay unchanged |
|---|---|---|
| Watcher event, applied watcher revision, root boundary/ownership/health change | That `root_id` once | Other roots and independent sources |
| Source status/confidence/retirement/identity evidence change | That `source_id` once | Other source IDs and root revisions |
| Claim revoke | The claim's `source_id` once, using one `event_id` for the source update and append-only revoke row | Claim row, old Manifest/seal, file row, tags, notes, history |
| File context, user tag, note update | None | Root/source seals and current pointer |
| New complete root generation or Manifest publication | Pointer CAS once; each new root/source seal records the current revisions | Prior published Manifest and its evidence |

A root can only contribute a file while all of its own root evidence is current. A shared source can only contribute while that source's evidence is current. Thus, a valid B root can keep a shared file visible after A's Watcher revision changes, but B cannot rescue a source whose shared physical identity is invalid.

## Cross-transaction cursor

The cursor binds:

```json
{
  "manifest_id": "...",
  "pointer_revision": 4,
  "scoped_root_seal_sha256": "sha256(canonical sorted eligible root/generation/source/path revision vector)"
}
```

Each new read transaction recomputes the values. A mismatch rejects the cursor and restarts the query. It does not attempt to restore an old FTS or BM25 result snapshot. Reads inside one SQLite read transaction keep a consistent snapshot even if a writer commits concurrently; the caller must not reuse that snapshot as current after ending the transaction.

## Claim-revocation entry point

The Harness uses a connection-local event capability and the following service shape:

1. Start `BEGIN IMMEDIATE`.
2. CAS-update `file_sources.source_revision` and its identity confidence.
3. In the update trigger, record one immutable event keyed by `event_id`, then advance the source authority revision once.
4. Append the claim revocation using that same `event_id`; its guard verifies the source event, source revision, and source authority revision.
5. Verify the event and row counts, then commit. On any exception, including commit failure, roll back if the transaction remains active.

The `d4s5_authority_events` primary key rejects event ID reuse. Event DML without the matching service capability fails. A revision row cannot be updated unless the matching event records exactly the next revision. Raw database clients that can install arbitrary application UDFs or drop triggers are outside this isolated prototype's trust claim; the #380 process/database generation boundary must settle that threat model.

## Atomicity observations

- D4-S4's old autocommit sequence was reproduced: the source revision update commits, then an injected failure before the revoke insert leaves zero revoke rows. The global clock has advanced and the current D4-S4 view hides all roots.
- The D4-S5 transaction was interrupted after transaction start, after the source revision update and scope trigger, after the revoke insert, immediately before COMMIT, and at COMMIT through a deferred foreign-key violation. Each case rolled back the source row, scoped revision, event row, revocation row, global diagnostic clock, pointer, current view, claim, and user metadata to the pretransaction values.
- An independent reader sees the old complete state before commit and while the writer's changes are uncommitted. The original reader snapshot remains old after commit; a new transaction and a reopened connection see the source hidden. This is expected SQLite snapshot isolation, not cross-transaction cursor validity.

## Visibility consumers and preserved data

The prototype's File Library and FTS searches both query `d4s5_effective_file_sources`. The FTS index may still contain old terms for a hidden/historical file; user-visible search joins the current effective relation and returns no row for that source. Tags, context, and historical manifests stay attached to the old `file_id`. Reusing the path creates a different `source_id` and `file_id`, so the new incarnation starts without the old user metadata.

No automatic deletion or garbage collection occurs. GC must honor retained manifests, claims, file foreign keys, tags, history, deduplication/content/AI artifacts, outstanding cursors/readers, and the separately authorized source GC contract before a production design can remove bytes or rows.
