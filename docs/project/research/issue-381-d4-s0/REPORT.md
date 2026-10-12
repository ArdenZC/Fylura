# Zen Canvas #381 — D4-S0 design feasibility

## Recommendation

**`#381-D4-S0 DESIGN_UNPROVEN` — do not start D4 production implementation from this evidence.**

The isolated Harness demonstrates that a single SQLite manifest pointer can atomically switch a simplified per-root visibility snapshot while existing read transactions keep seeing the old snapshot. It does not demonstrate a shared production visibility authority. Current consumers use `files.is_stale`, raw FTS, related status tables, and a separate Global Index authority; `files` has no durable root ownership relation. A tombstone pointer would leave those consumers inconsistent, and a file projection would require write propagation and would not, by itself, version tags, history, content artifacts, duplicate state, or AI state.

The measurements establish useful bounds for Owner design. They do not select either architecture. D4-S0 made no production edits, assigned no Schema version, and did not modify PR #383.

## Frozen inputs and reproducibility

| Input | Revision |
|---|---|
| Frozen master | `3275ac7c72df8ae4139a30f16c7f649971324e97` / tree `009576241cd4e8e2ba12cc644a368314f599faa6` |
| PR #383 D3R candidate | `584621c32054e16ac0a61afc669a04380a976b7f` / tree `8e72e9db9e4fae09b158d165762bba057e0b543d` |
| Research branch base | frozen master above; local branch `research/issue-381-d4-s0` |
| Master schema | version 37; copied `schema.rs` SHA-256 `c5ba26c0b0337c5e4c656c6440a54ff34c73a173e9d314c4afbec1d89be639b7` |
| PR candidate schema | D3R currently carries a Schema 38 migration candidate for `files_au`; this remains uncoordinated and is not used by the D4 harness |

The full Harness used the copied project schema, indexes, triggers, FTS, scan tables, tags, fingerprints, duplicate records, content artifact, and analysis finding. It ran three repetitions per model at 10k and 100k missing-candidate rows. Each scale also has the same number of live survivor rows plus one control row. Candidate and survivor rows are real SQLite records with related data; root directories are real Linux directories, while candidate leaf files are absent as expected for stale cases. No main database schema migration or `user_version` change was made.

Reproduction command, from the research worktree:

```sh
ZC_ISSUE381_D4_OUTPUT_DIR=target/issue-381-d4-s0-output \
  cargo run --offline \
  --manifest-path docs/project/research/issue-381-d4-s0/harness/Cargo.toml \
  -- --d4-s0
```

The run writes three repetitions per model/scale, then the failure contracts. Source inputs, exact tool versions, and evidence hashes are in [INPUTS.json](INPUTS.json), [ENVIRONMENT.txt](ENVIRONMENT.txt), and [MANIFEST.sha256](MANIFEST.sha256). Raw measurements are in [results.jsonl](evidence/results.jsonl), failure tests in [failure-contracts.json](evidence/failure-contracts.json), and the schema dump in [schema37-sqlite-schema.sql](evidence/schema37-sqlite-schema.sql).

## Authority and consumer audit

The complete reviewed reader/writer matrix is [consumer-matrix.csv](consumer-matrix.csv). It covers every production Rust file found with a direct `is_stale` reference, the shared query helpers that carry stale scope without repeating the field, and test-only occurrences.

The current managed-file authority is the `files` row and its `is_stale` bit. File Library query builders, search and pagination, tags, dedupe, analysis, content, classification, organization, watcher, scanner, and proposal code either filter or serialize that bit. The by-ID File Library detail path is an exception: it loads the file without `is_stale=0`, then loads its tags, findings, content artifact, and duplicate membership. Tag usage counts likewise count raw `file_user_tags` links. A pointer cannot make those paths agree unless they all resolve effective visibility from the same published manifest.

There is no `scan_root_id` on a `files` row and no durable root-to-file ownership table. Root scope is mostly inferred from paths; content artifacts do retain an optional root ID, but that is not ownership for the base file. Managed-scan admission rejects concurrently active overlapping root leases, yet this does not establish historical ownership for existing rows, shared/nested-root visibility, or an atomic publication rule for multiple roots. Current session projection aggregates status; it is not a single visibility pointer across the session's roots.

The Global Index has its own `global_entries.is_stale`, FTS, counters, and worker state. Some bridges also check `files.is_stale` (including managed AI enqueue and source resolution). These are separate authorities; D4-S0 did not alter or reconcile them.

### Direct stale dependencies and trigger effects

- `files.is_stale=0` appears in common scoped-file SQL and direct File Library, legacy search, watcher, content, dedupe, AI, classification, analysis, organization, and rule-proposal queries. D4 overlay rows would still have `is_stale=0`; every one of those reads would need an effective visibility join.
- Schema 37 `files_au` rebuilds raw `files_fts` on every update. D3R changes this trigger through a provisional versioned Schema 38 migration so only index-content/rowid changes rewrite FTS. Under an overlay, the `files` row is not updated, so the base FTS still contains candidate terms; public search must filter through the manifest. That raw retention is not itself a user-visible leak if every search path joins the effective set.
- `content_artifacts_file_changed` fires after updates to `path`, size/mtime, directory, `is_stale`, or content hash and marks matching artifacts stale. Overlay publication does not run that trigger, so artifacts remain `current` unless their visible status is versioned or resolved through the manifest.
- `active_duplicate_membership` depends on duplicate-group status, not `files.is_stale`. D3R stale invalidation marks associated groups stale in the same transaction. A pointer-only tombstone leaves the group active and the duplicate flag visible.
- `file_user_tags`, classification history, feedback, fingerprints, operation history, and evidence are separate rows. The current stale path retains history; an overlay must hide active results/counts without deleting historical records or exposing stale associations as live.

## Model comparison

The Harness modeled:

1. **Staged tombstone overlay:** write candidate IDs into durable test-only chunks, then atomically switch a singleton pointer to a manifest that identifies the operation/root. Readers anti-join the published tombstones.
2. **Versioned file projection:** materialize a per-manifest copy of live file rows and generation-specific trigram FTS, then switch the same kind of pointer. This experiment intentionally does not claim a complete snapshot of all associated tables.

Values below are medians of three repetitions unless a max is shown. All times are Linux local measurements using bundled SQLite 3.51.3 and the unchanged 5,000 ms busy timeout.

| Scale/model | Stage or projection work | Final pointer transaction lock | Metadata/tag writer wait (max) | WAL after publication | File count / stale FTS count / narrow page median |
|---|---:|---:|---:|---:|---:|
| 10k tombstone overlay | 23.4 ms candidate chunks | 38.0 ms (max 38.3) | 25.5 / 40.4 ms | 3.54 MB | 30.6 ms / 502 ms / 49.7 ms |
| 10k versioned projection | 320 ms initial projection; 246 ms generation stage; 259 ms retry stage | 15.4 ms final publish | 336.0 / 334.9 ms | 28.7 MB | 0.86 ms / 13.2 ms / 58.6 ms |
| 100k tombstone overlay | 261 ms candidate chunks | 405.7 ms (max 410.5) | 336.5 / 334.4 ms | 4.54 MB median | 297 ms / 5,617 ms / 490 ms |
| 100k versioned projection | 5,024 ms initial projection; 3,212 ms generation stage; 3,282 ms retry stage | 150.8 ms final publish (max 151.9) | 634.9 / 335.8 ms | 345.8 MB | 8.64 ms / 170 ms / 578 ms |

The overlay final lock includes final scan/root/session/revision/candidate checks and a repeated filesystem boundary validation. At 100k, the final fence took 287–292 ms, the in-transaction filesystem recheck 116–119 ms, pointer update 0.14–0.15 ms, and `COMMIT` about 0.04 ms. Candidate preparation and filesystem preflight happened before the final transaction. This is substantially shorter than D3R's rejected single `UPDATE`, but it is only a Linux Harness result, not a production Windows claim.

Versioned projection staging used 101 short chunks at 100k, with a maximum chunk transaction of 109 ms and approximately 689–719 ms of FTS work. The ordinary writers completed while staging. Their changes incremented the query revision, so the first publish was rejected; the Harness rebuilt the projection and retried. This is the required fail-closed behavior, but it adds another full projection pass unless normal writers are explicitly routed into active and staging projections. The 345.8 MB WAL measurement includes the baseline and candidate projection/FTS work in this disposable database.

At 100k, both models published exactly 100,000 effective stale candidates and exposed 100,001 live rows. An old reader transaction saw 200,001 rows and 100,000 stale-term FTS matches both before and after pointer commit; a new reader saw the new mapping. The raw Schema 37 FTS table still contained 100,000 stale matches in both models. Effective stale search returned zero; a live query returned 100,000; the measured specific page returned one row. File count and broad stale FTS counts are intentionally measured separately from the narrow ranked page.

The broad FTS rank sensitivity experiment is in [broad-ftsrank-diagnostic.json](evidence/broad-ftsrank-diagnostic.json). On a 20,001-file fixture, a true `SELECT ... ORDER BY bm25 LIMIT 25` for the broad `needle` match took 103.8 s without overlay and 52.1 s with overlay under the observed bundled-SQLite plans. This is not the exact weighted/quoted File Library query and must not be read as a production estimate. The same overlay query on host-system SQLite 3.53.1 took 19.4 ms median and its plan also included the temporary ORDER BY B-tree. The product-linked Harness uses bundled SQLite 3.51.3; this large runtime difference is unexplained, so the system result is retained only as an environment contrast and is not used as the primary metric.

## Visibility, retention, and recovery results

- All source rows remained present. At 100k, 100,000 candidate tag links, 100,000 candidate content hashes, 10,001 fingerprint rows, one classification-history row, two duplicate members, one active analysis finding, and one current content artifact remained in the fixture. No historical row was deleted.
- These statuses expose a central limitation: because the Harness publishes only visibility, the hidden candidates still have content artifact status `current`, duplicate group status `active`, content hashes/fingerprints present, and raw tag usage count 100,000. The Harness preserved them safely, but it did not make those derived states uniformly stale/hidden. This fails the full product visibility contract until Owner chooses and specifies how those associated states participate in publication.
- Staged tombstone chunks remained after closing and reopening the DB before publication; the old manifest remained selected. The test rejected Watcher revision, root generation/revision, run revision/cancellation, session revision/cancellation changes, and filesystem reappearance.
- A Linux symlink parent was rejected. A constraint failure rolled back the pointer. Killing a child after an uncommitted pointer write left the old pointer selected and `PRAGMA integrity_check` returned `ok`.
- A multi-root mapping test switched two roots with one pointer and showed old/new reader snapshots. This proves the SQLite pointer mechanism in the Harness, not session orchestration or product recovery ownership.
- `filesystem_to_sqlite_toctou_closed` is explicitly `false`: a filesystem observation and a later SQLite commit are not one atomic operation. Final in-transaction rechecks narrow but do not eliminate the race with a change not yet reported by Watcher. This Linux test does not cover Windows Junctions/reparse points, macOS provider behavior, permission races, mount changes, or cross-volume edge cases.

See [failure-contracts.json](evidence/failure-contracts.json) for the per-fence result values.

## Schema coordination with #380

No Schema version is allocated by D4-S0. Frozen master is Schema 37. PR #383's D3R candidate currently uses a PR-local Schema 38 migration for the conditional FTS trigger; #380 may also need 38. Owner coordination is required before either track publishes a version assignment. A shared migration plan must:

1. choose one combined 37→38 migration order and one owner for that version;
2. make old Schema 37 databases upgrade once to the combined migration, with idempotence checks for both tracks;
3. decide whether staged state is forward-only and how old binaries are prevented from opening a DB with a published manifest;
4. specify rollback before stage rows/manifests are written, after abandoned staging, and after pointer publication; deleting tables on rollback must not destroy active generation state or historical metadata;
5. define mixed-version app/database compatibility and migration crash recovery.

This report does not assign 38 or choose a rollback policy.

## D4 production GO/NO-GO

**NO-GO / design remains unproven.** Before production work, Owner needs to decide:

- the canonical file-to-root ownership and overlap rule, including multi-root session publication;
- whether the generation is only a visibility overlay or a complete snapshot of active derived state;
- how File Library detail/search/pages, tags, active duplicate membership, content artifacts, classification, analysis, proposals, watcher/scanner writes, recovery, and Global Index bridges all resolve the same committed generation;
- whether regular writers are dual-written, invalidate-and-restage, or block publication, with durable operation ownership and GC rules;
- what historical readers see and how a crash resumes or abandons each staged generation.

The tombstone overlay has low staging/WAL cost but pushes a visibility predicate into many readers and does not refresh derived state. The file projection gives a fast indexed live-file count but costs about 329 MB WAL at 100k in this fixture, must rebuild after a concurrent write, and is incomplete without versioned/consistent associated tables. Neither has demonstrated a full production contract. D4-S0 therefore stops before D4 production development.

## Scope and execution limits

Only the isolated test-only Harness and research records in this branch were changed. The master worktree and D3R worktree remained clean at their supplied revisions. No Windows/macOS CI or general CI was run; no Codex Review, PR state change, merge, issue close, or release was performed.
