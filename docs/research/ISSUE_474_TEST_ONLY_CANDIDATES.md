# Issue #474 — test-only extension recency and FTS cost evidence

Status: **READY_FOR_OWNER_REVIEW**. Production activation is not approved or included. The bounded probe improves high-hit `jp*` at both sizes, but zero/sparse queries pay probe-plus-fallback overhead. Native FTS rank retains weighted BM25 work; these measurements do not establish a semantics-preserving 1M solution or prove an architecture change is required.

## Scope and identity

- Base: `b3ace0cda44d0e9db8e67448af88fb056d34232d` (Schema44). Measured test-code HEAD: `1f94a23b912dc82f15e7af792192513456f32242`; tree: `3c1aeca5395afaec510deefd5df32bb48b6338ef`. Both fixtures report a clean working tree and the same code identity. Later commits add only this report/evidence.
- One existing deterministic synthetic fixture per size, 512-row production inserts, real production schema/indexes/FTS triggers and Database pool. 100k then 200k, sequentially, in the same Docker container. No filesystem discovery measured.
- Linux x86_64, kernel 6.18.55, 3 visible processors; Rust 1.97.1 / LLVM 22.1.6; bundled SQLite 3.51.3. `cargo test` default **unoptimized + debuginfo**, no desktop-runtime feature, RUSTFLAGS unset. Container toolchain based on `rust:1-bookworm` digest `sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8`, with GTK/WebKit build libraries. SQLite compile options and database metrics are in each environment record.
- These are Linux test-build paired observations, not Windows/macOS native acceptance, release qualification, or replacements for historical Hosted results. No 1M extrapolation is used for a gate claim.
- Every paired case rotates production / unlimited recency / bounded fallback order over 5 warmups + 30 measured iterations. SQL-tier and public pooled Search are timed independently, including prepare/result mapping; all returned objects are compared outside the timer. Repository Snapshot full-object equality is also checked. VM/Sort observations run separately from latency samples.
- Existing formal benchmark collection, 30 samples, 100ms assertion, and all gates remain unchanged. The official full performance gate was **not run** by this candidate experiment. No production SQL or schema/index/trigger definition changed; no writer cost or migration is introduced.

## Bounded algorithm and correctness argument

`idx_global_entries_modified_at` has only `modified_at_fs DESC`, not an ID suffix. Sorting a raw `LIMIT 1600` by `(mtime,id)` directly can still examine an arbitrarily large tie block. The experiment instead reads at most 1600 raw keys plus one lookahead in index order, rejects uncertifiable ID/mtime types, and discards the entire boundary mtime group. The remaining complete groups are a prefix of the total `(modified_at_fs DESC, id ASC)` order, regardless of physical tie order. Hydration applies the original GLOB, exact-extension exclusion, enabled-volume and stale predicates, all original fields/managed lookup, rank 1.0, and stable order.

If this complete prefix yields `tier_limit` rows, every unscanned eligible row sorts later: the vector is exactly the production tier vector. Otherwise the original production Extension Prefix SQL runs in the same SQLite read transaction. Even an exhausted small fixture falls back if insufficient, rather than returning a partial tier. Only first pages of at most 80 results are eligible; tier limits above 160 and deeper pagination use production immediately. Existing deduplication, 4096 window and offsets stay in their unchanged caller. A pre-existing Repository read transaction is reused; standalone calls open a deferred read transaction covering raw scan, hydration and fallback.

The physical bounded read is intentionally mtime-only; ID ordering is applied to the certified complete groups. The raw budget is 1600 **plus one explicit boundary lookahead**, not an unbounded tie sort. No additional database index is created. The unlimited recency comparator uses the existing mtime index and original filters/stable order, without a raw cap.

Coverage per fixture:

- 12 historical query classes × offsets 0/80/4016/4096 × both experimental variants, comparing full pooled result vectors to production (96 comparisons).
- 11 cases × 35 iterations × 3 variants for both SQL-tier and pooled Search (1,155 complete-vector comparisons per path), plus complete Snapshot results/source health/source revision/index status comparisons.
- High-hit `jp*`, zero, sparse `jso*`, stale, disabled and multiple volumes, managed/unmanaged fields, cross-tier duplicate ID elimination, exact-extension exclusion, tie groups within and above the budget, deep page, 4096 window and beyond-window empty public page.
- Four nullable-ID/malformed-mtime SQL result-or-error comparisons and four pooled comparisons. A selected NULL ID preserves the existing production mapping error; it is not silently dropped.
- Focused WAL interleaving: a writer creates a matching extension after raw scan, before fallback. The in-flight standalone candidate still sees the old zero-result snapshot; the next read sees the write. A caller-owned read transaction similarly remains stable across a writer marking the hit stale.
- Full table/index/trigger SQL signatures are identical before and after the experiment. All assertions passed at both sizes. Raw records retain actual result objects, not just ID/count assertions.

## Extension measurements

All times below are **p50/p95/p99 milliseconds**. P = production, U = unlimited recency, B = bounded+fallback. Cases are sequential mutations on the same fixture, identically visible to all variants. The last exact-extension/tie cases can fill pooled results in an earlier tier; use their SQL-tier measurements to assess Extension Prefix work. A pooled offset of 4096 returns before any tier, while its separately measured SQL-tier diagnostic still requests the 4096-row window.

### SQL-tier

| Size | Case | P | U | B | B SQL fallback |
| --- | --- | --- | --- | --- | --- |
| 100k | high_jp | 7.198/8.923/10.564 | 1.356/1.540/1.636 | 5.543/6.111/7.155 | 0/35 |
| 100k | zero | 0.179/0.263/0.265 | 53.171/56.736/57.162 | 4.978/5.894/6.575 | 35/35 |
| 100k | sparse | 3.057/3.646/3.690 | 2.310/2.578/2.697 | 8.049/8.778/9.352 | 35/35 |
| 100k | deep_80 | 7.602/8.187/8.264 | 2.434/2.724/2.861 | 7.562/8.425/10.435 | 35/35 |
| 100k | window_4096 | 24.237/25.952/26.739 | 50.703/54.499/63.053 | 24.528/29.024/44.220 | 35/35 |
| 100k | beyond_window | 24.151/25.352/25.883 | 49.669/52.490/53.037 | 24.231/25.846/28.477 | 35/35 |
| 100k | managed_and_cross_tier_overlap | 7.175/7.983/8.681 | 1.360/1.599/1.928 | 5.626/6.181/6.217 | 0/35 |
| 100k | stale_disabled_cross_volume | 6.905/7.982/10.563 | 1.366/1.571/2.022 | 5.244/6.115/6.450 | 0/35 |
| 100k | exclude_exact_extension | 6.981/7.453/8.342 | 1.319/1.798/2.083 | 5.155/5.788/5.847 | 0/35 |
| 100k | mtime_tie_over_budget | 7.333/7.821/7.869 | 1.156/1.401/1.467 | 8.719/9.561/10.286 | 35/35 |
| 100k | mtime_complete_tie_group | 7.223/7.809/8.024 | 1.159/1.319/1.337 | 4.093/4.336/4.427 | 0/35 |
| 200k | high_jp | 12.900/14.712/15.510 | 1.267/1.593/2.557 | 5.170/6.594/9.467 | 0/35 |
| 200k | zero | 0.169/0.241/0.255 | 103.171/109.259/112.266 | 4.746/5.653/5.891 | 35/35 |
| 200k | sparse | 5.345/6.376/6.612 | 2.002/2.214/2.437 | 10.023/10.622/10.914 | 35/35 |
| 200k | deep_80 | 13.327/14.733/14.894 | 2.184/2.389/2.599 | 13.405/14.282/14.925 | 35/35 |
| 200k | window_4096 | 29.616/32.320/33.813 | 47.666/59.302/63.412 | 30.056/33.688/34.010 | 35/35 |
| 200k | beyond_window | 29.690/31.845/32.613 | 47.400/49.312/51.670 | 29.747/31.388/32.051 | 35/35 |
| 200k | managed_and_cross_tier_overlap | 13.363/14.021/14.096 | 1.263/1.493/1.547 | 5.254/5.864/7.171 | 0/35 |
| 200k | stale_disabled_cross_volume | 13.241/14.751/15.004 | 1.378/1.630/1.664 | 5.234/6.610/7.671 | 0/35 |
| 200k | exclude_exact_extension | 13.586/14.347/14.666 | 1.411/1.563/1.593 | 4.975/5.622/5.683 | 0/35 |
| 200k | mtime_tie_over_budget | 14.049/14.601/15.005 | 1.165/1.544/1.671 | 15.442/19.050/22.263 | 35/35 |
| 200k | mtime_complete_tie_group | 13.958/15.401/15.915 | 1.083/1.310/1.496 | 3.923/4.236/4.301 | 0/35 |

### Pooled Search

| Size | Case | P | U | B | B SQL fallback |
| --- | --- | --- | --- | --- | --- |
| 100k | high_jp | 7.862/9.226/10.418 | 1.991/2.256/2.511 | 6.111/6.929/7.864 | 0/35 |
| 100k | zero | 0.895/1.059/1.155 | 54.582/56.544/57.425 | 5.852/6.453/6.547 | 35/35 |
| 100k | sparse | 3.751/4.271/4.366 | 2.909/3.451/3.780 | 8.772/9.691/9.969 | 35/35 |
| 100k | deep_80 | 8.372/8.800/8.899 | 3.132/3.512/3.564 | 8.272/8.957/10.074 | 35/35 |
| 100k | window_4096 | 29.486/31.246/31.521 | 55.530/60.712/79.396 | 29.707/32.650/42.430 | 35/35 |
| 100k | beyond_window | 0.028/0.053/0.074 | 0.025/0.029/0.088 | 0.026/0.029/0.063 | 35/35 |
| 100k | managed_and_cross_tier_overlap | 7.945/8.849/9.703 | 1.997/2.528/3.260 | 6.225/6.756/6.962 | 0/35 |
| 100k | stale_disabled_cross_volume | 7.551/9.855/11.314 | 1.991/2.422/2.791 | 5.799/6.398/7.736 | 0/35 |
| 100k | exclude_exact_extension | 1.116/1.317/1.326 | 1.007/1.291/1.390 | 1.091/1.271/1.328 | 0/35 |
| 100k | mtime_tie_over_budget | 1.161/1.381/1.418 | 1.047/1.237/1.539 | 1.113/1.266/1.293 | 35/35 |
| 100k | mtime_complete_tie_group | 1.151/1.309/1.326 | 1.064/1.294/1.606 | 1.128/1.560/2.972 | 0/35 |
| 200k | high_jp | 13.633/16.369/17.268 | 1.898/2.517/5.166 | 5.902/8.314/9.470 | 0/35 |
| 200k | zero | 0.849/1.183/1.341 | 105.358/110.007/111.821 | 5.384/5.791/6.269 | 35/35 |
| 200k | sparse | 5.850/6.359/6.768 | 2.586/2.929/3.143 | 10.689/11.285/11.529 | 35/35 |
| 200k | deep_80 | 14.125/14.867/15.530 | 2.878/3.164/3.198 | 14.199/14.784/15.423 | 35/35 |
| 200k | window_4096 | 35.114/39.522/41.550 | 52.717/58.994/60.943 | 35.402/37.296/38.481 | 35/35 |
| 200k | beyond_window | 0.029/0.032/0.049 | 0.025/0.028/0.046 | 0.026/0.030/0.033 | 35/35 |
| 200k | managed_and_cross_tier_overlap | 14.115/14.822/14.947 | 1.887/2.418/2.684 | 5.972/6.578/6.895 | 0/35 |
| 200k | stale_disabled_cross_volume | 13.749/15.019/15.408 | 2.092/2.676/3.501 | 5.856/6.438/7.849 | 0/35 |
| 200k | exclude_exact_extension | 1.112/1.195/1.359 | 1.041/1.183/1.349 | 1.101/1.266/1.277 | 0/35 |
| 200k | mtime_tie_over_budget | 1.154/1.281/1.325 | 1.060/1.166/1.324 | 1.122/1.358/2.194 | 35/35 |
| 200k | mtime_complete_tie_group | 1.143/1.337/1.364 | 1.062/1.197/1.324 | 1.091/1.259/1.293 | 0/35 |

### Statement work

VM steps / Sort operations below cover top-level Extension Prefix SELECTs, summing raw scan, hydration and fallback when present. Transaction-control statements are excluded. The raw SQL and individual counters/plans are preserved in JSONL. Sort is SQLite operation count, **not sorted rows, bytes, memory or elapsed CPU time**: unlimited recency can perform many small right-part tie sorts. Pooled timing covers all reached tiers, not just these counters.

| Size | Case | P VM/Sort | U VM/Sort | B VM/Sort |
| --- | --- | --- | --- | --- |
| 100k | high_jp | 141,737 / 1 | 12,370 / 80 | 37,227 / 1 |
| 100k | zero | 20 / 1 | 700,017 / 1 | 30,455 / 2 |
| 100k | sparse | 56,584 / 1 | 24,543 / 80 | 89,921 / 2 |
| 100k | deep_80 | 145,257 / 1 | 24,544 / 160 | 145,257 / 1 |
| 100k | window_4096 | 318,441 / 1 | 630,964 / 4096 | 318,441 / 1 |
| 100k | beyond_window | 318,441 / 1 | 630,964 / 4096 | 318,441 / 1 |
| 100k | managed_and_cross_tier_overlap | 141,779 / 1 | 12,372 / 79 | 37,278 / 1 |
| 100k | stale_disabled_cross_volume | 124,699 / 1 | 13,438 / 80 | 35,258 / 1 |
| 100k | exclude_exact_extension | 157,317 / 1 | 14,259 / 80 | 35,163 / 1 |
| 100k | mtime_tie_over_budget | 157,317 / 1 | 18,279 / 1 | 168,532 / 1 |
| 100k | mtime_complete_tie_group | 157,317 / 1 | 14,401 / 1 | 30,850 / 1 |
| 200k | high_jp | 279,777 / 1 | 12,636 / 80 | 37,227 / 1 |
| 200k | zero | 20 / 1 | 1,400,017 / 1 | 30,455 / 2 |
| 200k | sparse | 109,794 / 1 | 23,423 / 80 | 143,131 / 2 |
| 200k | deep_80 | 283,297 / 1 | 25,076 / 160 | 283,297 / 1 |
| 200k | window_4096 | 456,481 / 1 | 631,566 / 4096 | 456,481 / 1 |
| 200k | beyond_window | 456,481 / 1 | 631,566 / 4096 | 456,481 / 1 |
| 200k | managed_and_cross_tier_overlap | 279,819 / 1 | 12,372 / 79 | 37,278 / 1 |
| 200k | stale_disabled_cross_volume | 245,717 / 1 | 13,296 / 79 | 35,328 / 1 |
| 200k | exclude_exact_extension | 310,969 / 1 | 14,888 / 80 | 35,198 / 1 |
| 200k | mtime_tie_over_budget | 310,961 / 1 | 18,324 / 1 | 322,176 / 1 |
| 200k | mtime_complete_tie_group | 310,961 / 1 | 14,334 / 1 | 30,850 / 1 |

High-hit pooled p95 improves 9.226→6.929ms (100k) and 16.369→8.314ms (200k). Unlimited recency is faster on high hits but reaches 110.007ms pooled p95 on the 200k zero-hit case and becomes slower than production at the 4096 window. Bounded fallback limits that scan exposure, but zero-hit pooled p95 worsens 1.059→6.453ms / 1.183→5.791ms and sparse worsens 4.271→9.691ms / 6.359→11.285ms. The 200k over-budget tie SQL p95 worsens 14.601→19.050ms; fallback cost is real. Deep-page P/B use the same production SELECT and VM work; observed timing differences there are not algorithmic improvements. No unconditional production rollout recommendation follows from this data.

## Actual SQL and plans: drift repaired

The duplicated `candidate_plan_sql` builder has been removed. Formal baseline plans, no-result FTS diagnostic tier plans, prefix production-stage SQL, and the disposable-index comparison baseline now obtain SQL through `search.rs::diagnostic_search_tier_sql` / its production FTS accessor. Formal measured public calls and thresholds are unchanged. Records retain actual SQL text, BLAKE3 SQL identity, HEAD/tree, rustc/flags/features and SQLite version. In particular Name Prefix and FTS now show the actual materialized key-only production query, rather than the old full-projection shape.

| Tier/class | Actual SQL BLAKE3 |
| --- | --- |
| exact_basename | `a59ae7c6349bdc48b06011b4dc6bb476f6f869c2a9087e15c40a701181bc21e1` |
| name_prefix | `3897ea6a056eb556c90b3c7aeb0efc4757b7f0fac927679be5ce4b496081e0c6` |
| extension_exact | `abed00956811ce74033f5c02a441f48276179fa5ccfc05383f056889a234b1c4` |
| extension_prefix | `342c73a934c5b6c054ad383b33aa4c5f7f4242d356aa8a5bdbf56d7f640e8fff` |
| fts_substring_report | `53369002834df21d1cd7fc2762ed5fc68b7a27bfab894f00a40b9cb2a275b2c3` |
| punctuation_prefix | `721e2a5138ed5ffca2d4f0a8c2607c0193aa441594fd68b35dff3dda76bb637e` |

### name_prefix

```text
MATERIALIZE candidates
SEARCH ge USING COVERING INDEX idx_global_entries_active_name_order (name_normalized>? AND name_normalized<?)
SEARCH gv USING INDEX sqlite_autoindex_global_volumes_1 (id=?)
USE TEMP B-TREE FOR ORDER BY
SCAN candidates
SEARCH ge USING INTEGER PRIMARY KEY (rowid=?)
CORRELATED SCALAR SUBQUERY 2
SEARCH me USING INDEX idx_managed_entries_global_entry (global_entry_id=? AND enabled=?)
SEARCH ms USING INDEX sqlite_autoindex_managed_scopes_1 (id=?)
```

### extension_prefix

```text
SEARCH ge USING INDEX idx_global_entries_active_extension_order (extension>? AND extension<?)
SEARCH gv USING INDEX sqlite_autoindex_global_volumes_1 (id=?)
CORRELATED SCALAR SUBQUERY 1
SEARCH me USING INDEX idx_managed_entries_global_entry (global_entry_id=? AND enabled=?)
SEARCH ms USING INDEX sqlite_autoindex_managed_scopes_1 (id=?)
USE TEMP B-TREE FOR ORDER BY
```

### fts_substring_report

```text
MATERIALIZE candidates
SCAN global_entries_fts VIRTUAL TABLE INDEX 0:M3
SEARCH ge USING INTEGER PRIMARY KEY (rowid=?)
SEARCH gv USING INDEX sqlite_autoindex_global_volumes_1 (id=?)
USE TEMP B-TREE FOR ORDER BY
SCAN candidates
SEARCH ge USING INTEGER PRIMARY KEY (rowid=?)
CORRELATED SCALAR SUBQUERY 2
SEARCH me USING INDEX idx_managed_entries_global_entry (global_entry_id=? AND enabled=?)
SEARCH ms USING INDEX sqlite_autoindex_managed_scopes_1 (id=?)
```

Bounded high-hit plan: raw `SCAN global_entries USING INDEX idx_global_entries_modified_at` stops at 1601; hydration performs rowid lookups over certified keys, volume lookups, original managed EXISTS and one stable ORDER BY sort. Zero/sparse additionally execute the original extension-index plan. Over-budget ties skip hydration and fall back. Exact statement SQL (including the actual finite rowid set), plans and counters are in each `extension_pair` record.

## FTS read-only cost decomposition

These are overlapping diagnostic drains, **not additive phase durations** or alternative production algorithms. MATCH/BM25/native-rank/association drains visit all 5,000 hits at 100k and 10,000 at 200k; stable sort and production return 80. Stage drains time execution and row decoding after prepare; the existing production wrapper includes prepare and full field mapping. Thus stage/production differences include different materialization/decoding work; do not subtract their percentiles or label the difference an isolated CPU cost. VM counters exclude nested FTS internal statements, so wall time remains important.

| Size | Query | Stage | Rows | p50/p95/p99 ms | VM | Sort |
| --- | --- | --- | --- | --- | --- | --- |
| 100k | report | match_rowids | 5000 | 2.667/3.009/3.069 | 15,009 | 0 |
| 100k | report | match_bm25_8_2_1 | 5000 | 11.433/13.762/15.044 | 25,012 | 0 |
| 100k | report | match_native_rank_8_2_1 | 5000 | 11.083/12.008/12.184 | 20,010 | 0 |
| 100k | report | volume_stale_association | 5000 | 6.930/7.851/7.892 | 60,015 | 0 |
| 100k | report | association_and_bm25 | 5000 | 17.443/18.770/19.391 | 70,018 | 0 |
| 100k | report | stable_sort_80 | 80 | 20.515/21.944/22.240 | 105,101 | 1 |
| 100k | report | actual_production_full_fields | 80 | 25.312/26.958/27.477 | 132,670 | 1 |
| 100k | invoice | match_rowids | 5000 | 3.204/3.662/3.748 | 15,009 | 0 |
| 100k | invoice | match_bm25_8_2_1 | 5000 | 12.838/14.303/16.245 | 25,012 | 0 |
| 100k | invoice | match_native_rank_8_2_1 | 5000 | 12.422/14.300/19.175 | 20,010 | 0 |
| 100k | invoice | volume_stale_association | 5000 | 7.609/8.443/8.788 | 60,015 | 0 |
| 100k | invoice | association_and_bm25 | 5000 | 19.122/20.006/20.386 | 70,018 | 0 |
| 100k | invoice | stable_sort_80 | 80 | 21.862/23.661/23.901 | 105,101 | 1 |
| 100k | invoice | actual_production_full_fields | 80 | 26.897/28.265/31.048 | 132,670 | 1 |
| 200k | report | match_rowids | 10000 | 4.962/5.226/5.318 | 30,009 | 0 |
| 200k | report | match_bm25_8_2_1 | 10000 | 22.581/23.159/24.583 | 50,012 | 0 |
| 200k | report | match_native_rank_8_2_1 | 10000 | 22.272/22.933/23.112 | 40,010 | 0 |
| 200k | report | volume_stale_association | 10000 | 13.164/13.938/15.314 | 120,015 | 0 |
| 200k | report | association_and_bm25 | 10000 | 34.213/35.163/35.251 | 140,018 | 0 |
| 200k | report | stable_sort_80 | 80 | 40.028/41.757/42.408 | 210,101 | 1 |
| 200k | report | actual_production_full_fields | 80 | 48.863/51.046/52.215 | 262,670 | 1 |
| 200k | invoice | match_rowids | 10000 | 6.087/6.469/6.521 | 30,009 | 0 |
| 200k | invoice | match_bm25_8_2_1 | 10000 | 25.476/26.193/26.685 | 50,012 | 0 |
| 200k | invoice | match_native_rank_8_2_1 | 10000 | 25.157/26.573/38.304 | 40,010 | 0 |
| 200k | invoice | volume_stale_association | 10000 | 15.098/15.639/24.324 | 120,015 | 0 |
| 200k | invoice | association_and_bm25 | 10000 | 37.566/38.494/39.697 | 140,018 | 0 |
| 200k | invoice | stable_sort_80 | 80 | 43.390/44.238/59.466 | 210,101 | 1 |
| 200k | invoice | actual_production_full_fields | 80 | 52.108/54.820/55.032 | 262,670 | 1 |

Both BM25 and native rank explicitly use weights **8:2:1**. Every hit is checked for exact numeric equality of native rank and BM25, and the independent stable-sort ID/rank vector equals the complete production result vector ordering. Every measured production execution preserves/compares all result fields; pagination and deduplication remain production-owned. No ranking, tie-break, enabled/stale-before-LIMIT rule or page semantics changes. No rank-frontier, LIMIT-before-filter or generic CTE candidate was run.

MATCH alone is much cheaper than weighted scoring and association+scoring on these fixtures. At 200k, report BM25/native-rank p95 is 23.159/22.933ms and invoice 26.193/26.573ms; native rank does not eliminate scoring cost. Full production FTS p95 is 51.046/54.820ms. Stable sorting still consumes all eligible hits. The data gives Owner a basis to consider retrieval architecture, but it does not prove a qualified 1M algorithm or authorize a ranking/coverage change.

## Reproduction and evidence

Run from a checkout with Rust and the platform Tauri build dependencies. Each command is bounded by the test to 100k or 200k; run sequentially. The `--include-ignored` filter runs the focused snapshot test plus this experiment only.

```sh
ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES=100000 \
ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT="$PWD/.tmp-tests/issue474-100k.jsonl" \
cargo test --manifest-path src-tauri/Cargo.toml --lib issue474_candidates \
  -- --include-ignored --nocapture --test-threads=1

ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES=200000 \
ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT="$PWD/.tmp-tests/issue474-200k.jsonl" \
cargo test --manifest-path src-tauri/Cargo.toml --lib issue474_candidates \
  -- --include-ignored --nocapture --test-threads=1

cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --lib \
  global_index::tests::global_search_ -- --test-threads=1
```

- [100k raw JSONL, gzip](evidence/issue-474-candidates/100k.jsonl.gz) and [200k raw JSONL, gzip](evidence/issue-474-candidates/200k.jsonl.gz): 67 records each, 30 raw timing samples per variant/stage, summaries, full results, actual SQL/plans, counters, identities and completion records.
- [Validation output](evidence/issue-474-candidates/validation.txt): both paired runs passed 2 tests each; focused search regressions passed 17 tests (12 large/manual diagnostics remained ignored); `cargo fmt --check` passed. No Hosted workflow was dispatched, so there is no Hosted run URL to claim.
- Evidence can be inspected with `gzip -dc .../100k.jsonl.gz`; record types are `candidate_environment`, `twelve_class_pagination_equality`, `extension_pair`, `fts_cost_decomposition`, `abnormal_equality`, `candidate_complete`. Result/error equality is asserted by Rust before records are emitted, not inferred from counts.

No merge, Full Validation, 1M dispatch, production Search SQL change, Schema45, Codex Review or Release. Owner decides Extension Prefix activation and whether FTS needs a retrieval architecture change. Existing historical 1M FTS failures and accepted tails remain unresolved; these local observations do not replace them.
