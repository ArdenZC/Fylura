# #381-D3R bounded SQL-plan comparison

## Input and method

- Production baseline for the old statement: PR source `8dc84751e589c2988fd1c0d3ef8358005afe2f26`, tree `858b10afa4fc7de90e9a3403d6d99c56f7c2866b`.
- Baseline SQL: `UPDATE files ... WHERE is_stale=0 AND id IN (SELECT candidate_id ... stale_eligible=1) AND EXISTS (... candidate_id=files.id AND candidate_path=files.path)`.
- Candidate SQL: `UPDATE files AS file SET is_stale=1 FROM temp.issue381_reconcile_candidates AS prepared WHERE file.id=prepared.candidate_id AND file.path=prepared.candidate_path AND prepared.stale_eligible=1 AND file.is_stale=0`.
- Both variants use the same full project tables, indexes, FTS5 table and associated fixture records. The D2 Schema 37 migration source snapshot is used, then the exact provisional Schema 38 `files_au` migration body is applied, including rowid/name/path guards. The candidate table matches production columns and `WITHOUT ROWID` layout; `candidate_id` is a unique primary key.
- Linux x86_64, rusqlite 0.39.0 bundled SQLite 3.51.3. Three paired runs per size, alternating A/B order, each run starts from a copy of the same database.
- Reproduction command and source are in this directory. Raw results are `output/sql-plan-results.jsonl`.

## Plans and row parity

At 100k, both plans use `idx_files_is_stale` and the temp candidate primary key. The old plan additionally materializes the `IN` list/Bloom filter and performs a correlated primary-key lookup; the set plan removes those steps:

```text
old: SEARCH files USING INDEX idx_files_is_stale (is_stale=?)
     LIST SUBQUERY 1
     SCAN temp.issue381_reconcile_candidates
     CREATE BLOOM FILTER
     CORRELATED SCALAR SUBQUERY 2
     SEARCH prepared USING PRIMARY KEY (candidate_id=?)

set: SEARCH f USING INDEX idx_files_is_stale (is_stale=?)
     SEARCH prepared USING PRIMARY KEY (candidate_id=?)
```

Every run had exactly 10,000 or 100,000 unique, eligible candidates and changed exactly that many file rows. The set statement preserves exact file ID and path matching, `is_stale=0`, eligibility, and a single atomic update. Both variants retained all raw stale FTS terms, performed no stale-only FTS row rewrite, and a normal live-file rename removed the old term and indexed the new term. The existing content-artifact trigger changed the fixture artifact to `stale` in both variants; total change count was 100,002 for 100,000 file rows, including the same two trigger side effects. WAL bytes after commit were identical by size: 7,947,512 bytes at 10k and 77,151,152 bytes at 100k.

## Timings

Milliseconds; median and maximum are over three repetitions. Lock residence is from successful `BEGIN IMMEDIATE` acquisition through `COMMIT`.

| Rows | SQL | UPDATE median / max | Lock median / max | COMMIT median / max |
|---:|---|---:|---:|---:|
| 10k | Old `IN + EXISTS` | 85.348 / 90.549 | 95.348 / 101.323 | 10.522 / 10.756 |
| 10k | `UPDATE FROM` | 83.708 / 85.029 | 94.613 / 96.836 | 11.081 / 11.791 |
| 100k | Old `IN + EXISTS` | 1,296.311 / 1,297.033 | 1,422.115 / 1,422.705 | 125.651 / 125.788 |
| 100k | `UPDATE FROM` | 1,278.120 / 1,296.555 | 1,402.247 / 1,420.618 | 124.107 / 130.901 |

At 100k, set-based SQL reduces median UPDATE time by 18.191 ms (1.4%) and median lock residence by 19.868 ms (1.4%); the maximum lock residence improves by only 2.087 ms, and commit time is effectively unchanged. The plans simplify, but the access path still starts from `idx_files_is_stale`; the timing difference is small and not a stable multi-second improvement.

The prior Hosted Windows D3 run measured 5,284.346 ms from `BEGIN IMMEDIATE` to `COMMIT` and the metadata writer hit the unchanged 5,000 ms busy timeout. Applying the Linux 100k median lock-residence saving predicts about 5,264 ms, still above the required limit. This is only a feasibility estimate; the Hosted Windows run on the final candidate is required to decide the gate.

## Implementation and validation state

- The bounded production substitution uses the set-based statement with the same ID/path, eligibility and stale CAS predicates. It does not change candidate preparation, filesystem boundary revalidation, Watcher revision checks, generation/run/root/session CAS, cancellation handling, invalidation, commit boundaries or SQLite timeout.
- The FTS migration test now asserts raw FTS still contains the stale term while `Database::search_files()` hides stale rows. Production search semantics are unchanged.
- The 100k Windows fixture now seeds and checks 100k preserved tags and user contexts, 100 content hashes cleared, classification history retention, missing fingerprints, stale content artifact, stale duplicate group, stale analysis finding and dedupe rebuild state. Writer failures are captured and logged before the test asserts the existing 5,000 ms limit, so the remaining visibility/WAL checks still run.
- Local `cargo fmt --check` and `git diff --check` pass. The focused migration test could not run in this Linux workspace because `glib-2.0` development files are unavailable; Hosted Windows CI remains the authoritative execution for the production target.
- Hosted Windows repeated 100k checks, final exact-HEAD CI, and final PR disposition are pending.

Schema v38 remains a PR-local candidate only. This experiment uses connection-local temp state and makes no new persistent schema allocation. It does not coordinate or determine #380's eventual schema ordering.
