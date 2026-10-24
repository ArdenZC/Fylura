# Issue #364: high-hit search path attribution

Scope: test-only research against `master@af316d41f4db2c8a2f237f67cb87263f0c12afc9`
(tree `6433e0dc031e2109eb9cf8dfac9c591d79df7c89`). No production
`search.rs`, repository, schema, index, threshold or workflow change.

## Outcome and acceptance

Determine which work the actual pooled search executes and where its time goes;
independently explore Name Prefix and FTS candidates. Preserve complete ordered
result objects, weighted BM25, early active/enabled filters, managed projection,
dedup, the 4096 candidate cap and pagination. Results are exploratory evidence,
not a 100ms performance qualification or production authorization.

The existing natural PR benchmark supplies its single 100k fixture. The new
profiler reuses that database, schema, indexes, triggers and generator. It builds
no extra large fixture, runs no 500k/1M diagnostic or Full Validation, and does
not dispatch workflows. Existing 500k evidence remains historical.

## Measurements

`global_search_path_stages.rs` attaches SQLite STMT/ROW/PROFILE tracing to one
test-owned production pooled handle and invokes the real
`Database::search_global_entries`. Seven other connections are temporarily held
so the production checkout must select the traced eighth handle. Setup and
teardown are outside the request timer. This controls handle identity, not a
connection-pool contention benchmark.

Each request emits its own timeline, first-row timestamps, returned row counts,
VM steps, sort counts and automatic reprepare counts. FTS internal statements
are included in the parent's interval and are not double counted. Trace
intervals include intervening Rust mapping; they are not pure SQLite CPU time.
The same-request remainder contains checkout, SQL construction/prepare,
reset/finalize, dedup and paging. It must not be labelled pool wait alone.
Tracing/string copying introduces observer overhead, especially for tiny queries.
A companion call clocks checkout, the actual production on-connection route
and release separately, executing the public wrapper's exact operations without
reimplementing the route. Its results must equal the traced public call; its
stages belong to that companion call, not to the earlier public timeline.

Separate, explicitly labelled SQL controls time prepare, bind, every step,
full result mapping, Rows drop and Statement finalize in a single execution.
They use both the exact pooled handle after the actual request and the existing
inspection connection. The latter is where the historical SQL-tier probes ran.
They expose a measurement boundary worth checking: the old raw SQL helper stops
its clock before Statement drop, whereas production returns after that drop.
This is a hypothesis, not evidence that finalize explains the 1M gap.

Ten paired exploratory samples alternate original/candidate order; every raw
sample is retained, alongside p50/p95/p99 summaries. No subtraction of
independently sampled percentiles is used for attribution. Actual production
requests, correctness replays and separate SQL controls are distinctly labelled;
control executions are not evidence of repeated SQL in a production request.

## Original and candidate SQL

All original SQL comes from the current production diagnostic accessors, which
already return key-only MATERIALIZED CTEs for both Name Prefix and FTS. Each
summary emits the full original SQL, candidate SQL, bound parameters and both
EXPLAIN QUERY PLAN results. There is no frozen copy of an obsolete baseline.

- Name Prefix: replace only `INDEXED BY idx_global_entries_active_name_order`
  with the existing `idx_global_entries_modified_at` inside the key-only CTE.
  Keep predicates, both orders, projection and limit unchanged. This can avoid
  sorting all high-hit prefix matches by scanning recency order and stopping
  after enough matches. The index lacks the ID tie-breaker, so SQLite must still
  order equal timestamps. Sparse, old or absent matches can scan the entire
  recency index: this is not an admissible blanket replacement.
- FTS: replace only `AS MATERIALIZED` with `AS NOT MATERIALIZED`. Keep FTS-first
  joins, `bm25(..., 8.0, 2.0, 1.0)`, all filters, limits and both total orders.
  This allows a coroutine on the tested SQLite version; it cannot eliminate
  evaluating weighted BM25 for high-hit candidates or the ranking sort.
- Extension Prefix: identity SQL control only. Its range spans distinct
  extension values, so the extension-first index does not give global recency
  ordering. Full projection/managed evaluation and a temp sort remain plausible
  costs. No historical extension CTE is selected or recommended.

## Correctness and limits

The 12 existing query classes plus Name Prefix/FTS no-result controls compare
all result fields and exact BM25 values. Limits 1/80/200/4096 and full route
pages at offsets 0/40/4016/4096 are compared against production. Only one selected
tier changes in the candidate replay. A rolled-back savepoint on the same
fixture adds disabled-volume and stale matches, timestamp ties and enabled/
disabled managed entries/scopes. Both independent candidates must match the
production route on all twelve classes and all four offsets. An ordinary
512-row regression exercises the tracer and the same adversarial checks.

The existing benchmark oracle and gates remain unchanged. Ten samples are too
few to qualify a tail-sensitive production cutover. A SQL-only win does not
establish a pooled candidate win; the production route remains unchanged.

## Source evidence and decision boundary

Accepted 1M source: [Actions 40062023810](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40062023810),
[artifact 12547254467](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40062023810/artifacts/12547254467).
It confirms 12/12 correctness and eight pooled query classes above the official
100ms gate. Raw/pool P95s include Name Prefix 75.620/160.757ms, common prefix
76.251/169.105ms, report 175.851/264.366ms, invoice 200.267/291.105ms, extension
prefix 75.094/158.662ms. These are reference observations, not additive stages.

Historical 500k original/CTE pairs are retained in
[evidence/issue-363](evidence/issue-363/). Those compare pre-cutover SQL with the
now-production CTE; they cannot qualify either new candidate. Across archived
runs the pooled 100ms pass count varied 4/12, 10/12, 4/12, and extension-prefix
regression recurred. Do not rerun them to manufacture a larger pass count.

Hosted stage measurements and final natural CI disposition will be linked from
the Draft PR after completion. No merge, Codex Review, Release, threshold
relaxation or production adoption is authorized by this research.
