# Global Search covering-index production candidate

Owner scope: two covering indexes only; one bounded A0/A1/A2/A3 comparison.
Latest remote master verified 2026-10-26: HEAD
`8ffc7d04a8c392e28299445b845d39e606f43f5c`, tree
`57a6d066b58d53746cf3fc0ad95709bc6d3737f3`, Schema 42.
[Baseline CI 40298939633](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40298939633)
succeeded; its routed Rust/release jobs were skipped. This is not a fresh full
platform qualification. Issue #364 and Draft research PRs #456/#457 remain
background evidence; their SQL candidates are not included.

## Production scope and migration

Schema 43 appends `volume_id` to the active Name order index and adds active
`(volume_id, last_seen_at, id)` Source Facts. The 42→43 migration explicitly
drops/recreates the Name index in the existing BEGIN IMMEDIATE migration
transaction, with user_version updated only in that transaction. Existing rows,
FTS, source/status/search SQL, COUNT(id), result mapping, BM25, filtering,
candidate window, sorting, paging, deduplication and source revision JSON/BLAKE3
remain unchanged. No extension-prefix, FTS or mtime index change.

Migration risk: index creation holds the migration write lock, scans/sorts active
entries and needs additional disk/WAL/temp space. Time and storage depend on the
installed database. DDL failure rolls back the old index and version; retry is
possible after fixing the resource failure. Schema 42 binaries reject version 43;
rolling back an installation requires restoring a compatible pre-upgrade backup,
not manually decrementing user_version. No automatic destructive downgrade.

## Bounded protocol

The existing Rust generator and its production FTS/count triggers populate one
A0 fixture per scale. After closing/checkpointing it, three byte copies are made.
Each copy changes only the two specified indexes, outside query timers:
A0 original; A1 Name only; A2 Source only; A3 both. All keep Schema 43 as a test
container; A0's two index definitions match Schema 42. No fourfold generation.

All twelve original queries, limit 80, five warmups and thirty samples. Actual
on-connection search route, actual pooled search, production Source Health
helper (including JSON/hash), actual Index Status and actual Repository Snapshot
are interleaved across variants with rotating start order each round. Each call
is timed independently; no percentile subtraction. Complete ordered objects,
BM25, source health/revision and status equal A0 on every call. Pagination at
limits/offsets 1/0, 80/40, 200/4016 and 80/4096 is compared separately. SQL-tier
measurements use current production SQL accessors and full result mapper, with
five/thirty samples per tier; they are isolated tier measurements at limit 80,
not a claim that every tier executes in every production request.

Separate real Snapshot PROFILE traces record executed SQL, VM/Sort/Fullscan.
Nested FTS statements are retained, not added to parent totals. Trace is off
for public-call latency. Source/Status EQP is collected after tracing, and every
tier has its own SQL, parameters, plan and counters. Name/Source covering use
is asserted. Extension-prefix keeps its full projection; its plan and wide-row
cost are explicitly retained. dbstat reports per-B-tree bytes. Five rollback-only
256-row insert samples retain production triggers; this is a bounded index
maintenance diagnostic, not sustained ingestion qualification. DDL time includes
drop/build and is excluded from all query latency samples.

Run 100k first; inspect correctness/performance before dispatching a single
1M Windows Hosted run. Do not repeat 1M generation, rerun full CI, relax 100ms,
merge, or run Codex Review. The natural PR CI remains authoritative for normal
required checks. Prior macOS SIGBUS #446 is historical, not assumed to explain
any new failure. Results and remaining bottlenecks will be appended after runs.
