# Validation contract

## Inputs and command

Run from the repository root:

```sh
python3 docs/project/research/issue-381-d4-s3/harness/run.py
```

The runner opens disposable SQLite databases, loads the complete captured Schema 37 DDL and the committed D4-S2 test-only candidate SQL, and applies only the D4-S3 SQL prototype. It executes no project test suite or CI job.

## Preserved D4-S2 checks

The Harness imports and invokes the committed D4-S2 `run_main_experiment()` without edits. All 15 original assertion groups pass, including:

- default `legacy_unverified` compatibility and explicit legacy claim;
- source crosswalk completeness and immutable path-entry behavior;
- overlapping-root removal and history retention;
- same-path replacement with a distinct source and no tag/content inheritance;
- FTS visibility filtering and content/metadata/tag/history/fingerprint/duplicate/analysis/AI retention;
- same-identity move, hardlink identity, failed path-conflict rollback, unapproved deletion rejection;
- foreign-key, SQLite integrity, and external-content FTS checks; and
- migration rollback, simulated process exit, and startup recovery/retry.

These are the D4-S2 assertions over a synthetic fixture; they are not claims of a real user database migration.

## D4-S3 added assertions

The additional run is counterexample-first and includes:

1. Original-model invalid publication: omitted member, wrong path, extra valid-but-unselected projection, and unverified `path_only` identity.
2. Original-model post-publication `source_generation_members` update and delete without pointer revision.
3. Original-model cross-transaction Cursor reuse after pointer revision change, including list continuation, detail lookup, and old/new FTS term results.
4. Protected-model rejection and transaction rollback for the four invalid publication cases.
5. Valid overlapping-root publication with exact union deduplication, counts, digests, and pointer CAS.
6. Sixteen rejected insert/update/delete or pointer mutations across the five published-object tables and pointer CAS.
7. Rename failure injection: files row, FTS update, new path entry, generation and pointer all roll back when the projection path is wrong.
8. Rename success: pinned old reader remains coherent; new reader sees the new path and FTS token; old Manifest and path entry remain available through a SQLite read-only history connection; stale cross-transaction Cursor is rejected.
9. Small FTS/BM25 query and `EXPLAIN QUERY PLAN` capture with raw versus active row counts.

## Output

`evidence/manifest-authority-results.json` contains assertions and raw row differences, counts, digests, blocked DML messages, FTS query results and plan lines. `evidence/manifest-authority-run.log` is the exact console summary for the final harness invocation. `evidence/SHA256SUMS` covers both, the SQL, runner, docs, input list, and exact D4-S2 raw log archive.

## Limits

- Python SQLite is 3.53.1, not the product's SQLite 3.51.3. The recorded query plan is version-specific.
- No large-scale benchmark, 10k/100k data set, Hosted Windows/macOS run, full CI, or production migration was run.
- Filesystem identity and watcher observations are synthetic. The database proof does not prove actual current disk state.
- The D4-S2 15 assertion groups exercise transaction rollback and process interruption; D4-S3 adds rollback of invalid publication and rename failure. It does not test power-loss behavior on every platform or exhausted disk/WAL.
- SQLite triggers cover normal DML, not a process allowed to replace/drop schema objects.
