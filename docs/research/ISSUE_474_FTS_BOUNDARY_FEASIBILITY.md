# Issue #474 — FTS bounded rank boundary feasibility (test-only)

Status: **unqualified research / no production activation**, based on master `df940915084b810101a8a3eb78796cdf8730d55b`. The 1M Windows Release benchmark for #488 identified the two remaining p95 failures: FTS `report` **331.420 ms** and `invoice` **346.993 ms**, 100ms gate unchanged. Extension Prefix was separately merged and is not touched.

## Why this experiment

FTS5 supports mapping weighted `bm25(8,2,1)` into its hidden `rank` and native `ORDER BY rank` sorting. However, the existing global search ordering is **rank ASC, modified_at_fs DESC, id ASC** after **enabled-volume** and **not-stale** filters. SQLite FTS5 native rank ordering alone is **insufficient** if the rank boundary ties, or if prefiltering removes rows. A previous join-heavy native rank-stream candidate already regressed 100k p95; that rejected algorithm is **not retried or deployed**.

The new diagnostic asks a narrower question before designing a new production algorithm: *can a hard-capped 512 FTS-only rank prefix, plus one lookahead, certify the same first 80 active results as the existing query?* It uses FTS-only `SELECT rowid,rank ... ORDER BY rank LIMIT 513`; scores still use `bm25(8,2,1)`, no changes to search SQL/weights/Schema.

Metadata for the first 512 rowids is looked up and filtered by enabled-volume/stale. Rust orders eligible candidates by the exact production secondary keys. **Only** when the query exhausts all FTS rank matches, or the rank of the 80th eligible result is *strictly better* than rank #513, is the frontier certified. In that case, a full `id, weighted rank` vector must match the production FTS response exactly. If the cutoff rank ties, fewer than 80 eligible candidates remain, or too many stale/disabled entries precede active ones, it records `fallback_required=true`. It must **never** claim completeness from a noncertified prefix.

## Evidence capture and gates

Extended existing `global_search_fts_diagnostic.rs` to emit `fts_rank_boundary_preflight` records for `report` and `invoice` on the existing 100k + 262 adversarial-row fixture. Records include ranked prefix count, active/stale/disabled filtering loss, 80th rank, lookahead rank, exact certification/fallback, SQLite query plan, 10-sample (when first sample permits) rank-only latency and source identity. Rank-only latency intentionally excludes metadata hydration, so is **not** a pooled Search p95.

The same established adversarial test already checks both FTS result objects, managed status, enabled/stale, equal-rank stable tie breaks, pagination/candidate window, baseline SQL variant equivalence, and explicit unsafe LIMIT-before-filter counterexample. This addendum only provides mechanism evidence; it does not overwrite its existing assertions or substitute for 30-sample Release performance acceptance.

`global-search-benchmark.yml` gains `mode=fts_boundary_release` (manual `workflow_dispatch` only), which runs **the existing FTS diagnostic test with `cargo test --release`**. The original Debug `fts_diagnostic`, baseline, baseline_release, 100ms gate and all PR normal validation behaviors stay unchanged. Initial intended size is **100000**, not 1M: the diagnostic explicitly asserts 100k to avoid costly or ambiguous new dispatches. Evidence must be read from JSONL and job logs before proposing a production optimization. No modifications to `search.rs`, FTS virtual-table DDL, FTS writes, schema version, ranking, or release gate.

## Decision rules

- **If certified and measurably promising**: propose a separate bounded production candidate only after full hydration/equality tests and same-run Release profile comparisons.
- **If not certifiable / near-tied**: reject this frontier and investigate an index or ranking architecture change separately. Do not add a speculative production fast path with fallback+probe penalty.
- Never treat FTS5 rank-alone truncation, any postfilter-80 shortcut, or an approximate BM25 ordering as a valid equivalence strategy. Release remains **NO-GO** until both FTS p95 cases pass the original 100ms target.
