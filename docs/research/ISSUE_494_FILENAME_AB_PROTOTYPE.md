# Issue #494 — Global Search filename A/B research prototype and benchmark evidence

Status: **CORRECTNESS FOLLOW-UP FOR OWNER REVIEW**. No production search path, schema, source
authority, index scope or ranking contract is changed by this PR. The only production dependency
change is a `pulldown-cmark` patch update to 0.13.5 in the preview representation crate and both
lockfiles, which clears the newly surfaced RustSec advisory. Draft PR #495 stays draft.

The historical performance tables below are from pre-fix commit
`e2bab5018ca3fa93dc7385dc3d02f96e7afc51aa`. They are exploratory baseline data only; they do not
measure the corrected snapshot-scoped Candidate B implementation or qualify a 100 ms gate.

- Base: `df940915084b810101a8a3eb78796cdf8730d55b` (`origin/master`, #488).
- Experiment commit: this PR head. Test-only module
  `src-tauri/src/global_index/tests/issue494_filename_ab/`.
- Historical baseline: Windows x64, Rust 1.97.1, bundled SQLite, `--release`,
  `--features desktop-runtime`.
- Corrected-code native measurements were run from source SHA
  `be835b09f1c7e8f2c7aca1997db3b76214d9c867`, tree
  `579b050ccf8d1ec5493b5be740b440014e9a93ad`, on Windows and macOS Release at official 100k/1M.
  They follow the exact source's adversarial cases and retain raw JSONL artifacts. The final report
  distinguishes this code SHA from the documentation/CI-only follow-up commit.

## 1. What this PR contains

| Path | Role |
| --- | --- |
| `tests/issue494_filename_ab/mod.rs` | Shared `Class` / `Hit` ordering, folding rules, window helpers, the "recorded window equals production window" guard |
| `tests/issue494_filename_ab/oracle.rs` | Production oracle (five-tier composition), proposed-order oracle, extension oracle, FTS-vs-substring divergence probe |
| `tests/issue494_filename_ab/candidate_a.rs` | Candidate A: full FTS5 posting enumeration, no BM25 ordering, Rust classification, bounded Top-K, index-served extension channel |
| `tests/issue494_filename_ab/candidate_b.rs` | Candidate B: disposable compact view (contiguous folded name bytes, interned directories, stable ids), read-snapshot identity fence, bounded Top-K, SQLite hydration |
| `tests/issue494_filename_ab/fixtures.rs` | Official generator reuse plus a controlled-density fixture (exact 1/40/5 000/50 000 hit groups, path-only, extension, Chinese, punctuation, mixed case, stale rows, a disabled volume); correctness runs seed an immutable fixture image once and copy it to isolated test databases |
| `tests/issue494_filename_ab/cases.rs` | Correctness tests, including cross-connection and same-connection invalidation, stale narrow-cache shrink/grow, and hydration fallback |
| `tests/issue494_filename_ab/scale.rs` | The `#[ignore]`d 100k/500k/1M probe and the JSONL evidence emitter |
| `src/global_index/search.rs` | Test-only `diagnostic_candidate_window()` accessor plus crate-visible page-limit constant for the research SQL fallback |
| `.github/workflows/global-search-benchmark.yml` | Dispatch-only `issue494_ab` / `issue494_ab_density` modes plus a branch-scoped Windows/macOS correctness and measurement job |

The previous single-file prototype (`tests/filename_search_ab.rs`) is replaced, not extended: its
ranking oracle returned the empty set for every query (the extension filter was inverted), its
extension classes were unreachable, and its "generation" fence was an integer supplied by the test.

## 2. Correctness

### 2.1 Oracles and what they actually prove

Three channels are compared, never merged:

- **Production oracle** — re-derives the five-tier composition (exact name → name prefix → exact
  extension → extension prefix → FTS/punctuation) from raw SQLite facts, including the
  `layer_limit` growth, de-duplication, offset-after-dedupe and the 4 096 window. It is asserted
  equal to the real `search_global_entries_on_connection` output on the edge fixture (13 queries ×
  3 page positions) and on the 100k synthetic fixture (9 queries × 2 page positions).
  **Limitation:** BM25 itself is delegated to SQLite in both the oracle and production. The oracle
  owns tier composition, filtering, de-duplication, ordering around the score and pagination; it
  does not re-implement the BM25 formula. This is stated rather than implied.
- **Proposed oracle** — filename exact → prefix/word boundary → inner substring → path-only, ties by
  `modified_at_fs DESC` then stable id. It is **unbounded**: it returns every match, so the tests can
  assert the 4 096 window is a *prefix* of the true order instead of hiding truncation inside the
  oracle.
- **Extension oracle** — extension exact/prefix, reported as its own set. The Owner has not approved
  a priority for this class, so it is never merged into the ranked name/path channel.

Ranking differences between the proposal and production are emitted as `ranking_diff` records. They
are never asserted equal. Missing matches are always a failure.

### 2.2 Correctness case inventory

The cases run with `--test-threads=1`. Their pass/fail state is recorded against the exact HEAD in
§8; this inventory is not a result claim for a later commit.

The Windows/macOS correctness step runs in Release mode, the same profile used by the scale
measurements. Large 100k correctness fixture images are built once per test binary and copied into
fresh databases for each case; tests never share writable database state. The scale probe bypasses
this seed cache, so fixture construction and sampled Candidate B RSS remain unchanged.

| Test | What it pins |
| --- | --- |
| `issue494_production_oracle_reproduces_production_search` | Oracle == production, 13 queries × 3 pages |
| `issue494_production_oracle_holds_on_synthetic_scale_fixture` | Oracle == production at 100k |
| `issue494_oracles_exclude_stale_and_deauthorized_rows` | Stale and disabled-volume rows never reach any oracle |
| `issue494_candidate_a_matches_proposed_oracle_on_edge_cases` / `..._on_density_fixture` | A == proposed oracle, ordered ids |
| `issue494_candidate_a_paginates_without_gaps_or_repeats` | 4 096 window reachable in 64-row pages, no repeats, no gaps |
| `issue494_candidate_b_matches_proposed_oracle_on_edge_cases` / `..._on_density_fixture` | B == proposed oracle, both channels |
| `issue494_extension_channel_is_identical_across_architectures` | A, B and the oracle agree on extension exact/prefix |
| `issue494_incremental_narrowing_equals_full_scan` | 10-step typed prefix: narrowed == full scan == oracle within one view |
| `issue494_cross_connection_data_version_is_not_a_shared_generation` | Per-connection `data_version` behavior and snapshot isolation after an external UPDATE; a view cannot cross snapshots |
| `issue494_same_connection_write_invalidates_a_snapshot_view` | A same-connection UPDATE leaves both old counters unchanged; the next snapshot rejects the view |
| `issue494_snapshot_fence_rejects_commit_before_external_top_k_insert` | COMMIT/ROLLBACK/SAVEPOINT and query-only changes are denied; the snapshot exposes a restricted SQL-read facade with no hook-registration API; after an external insert, the old snapshot stays complete and a rebuilt view includes the new winner |
| `issue494_cross_connection_mutations_are_snapshot_consistent_and_rebuild_completely` | External delete, rename, mtime reorder and inserts preserve the old snapshot result, then a new snapshot rebuild restores the complete ordered page |
| `issue494_hydration_mismatch_falls_back_instead_of_returning_partial_or_misordered_top_k` | If hydration drops/reorders preselected Top-K rows, the page wrapper executes the full production SQL fallback in the same snapshot and the independent oracle checks refill/order |
| `issue494_narrow_cache_is_bound_to_view_identity_across_shrink_and_growth` | Old slot cache is bypassed across rebuilt views; both shrink and growth match the oracle |
| `issue494_budget_degradation_falls_back_instead_of_overrunning` | Impossible RAM budget degrades instead of allocating |
| `issue494_fts_match_set_matches_plain_substring_scan` | Trigram MATCH set == plain Rust substring set on 9 queries at 100k |
| `issue494_ranking_diff_report_is_produced_not_asserted` | Diffs are emitted as evidence, not asserted |
| `issue494_window_and_limit_contract_is_unchanged` | Oracle does not truncate; production returns empty beyond 4 096 |
| `issue494_duplicate_basenames_and_cross_tier_overlap_are_deduplicated` | Duplicate basenames and cross-tier overlap return each id once |
| `issue494_case_and_unicode_semantics_match_the_stored_facts` | Unicode folding reaches the proposal; production only via FTS |
| `issue494_short_cjk_query_is_not_silently_empty` | A 1-codepoint CJK query matches, and A must scan for it |
| `issue494_recorded_window_matches_production_contract` | `MAX_WINDOW` still equals the production constant |

**Duplicate ids are structurally impossible** (`global_entries.id` is the primary key), so the
de-duplication logic is exercised through duplicate basenames, cross-tier overlap and the
extension/name overlap instead.

### 2.3 Findings the audit produced

1. **Extension classes are unreachable in a filename-only ranking.** Every filename ends with
   `.ext`, so a query equal to an extension is always also a name substring. Ranking extension hits
   inside the name channel would fill the page with arbitrary `.pdf` names by mtime and never reach
   the extension class. The experiment therefore keeps extension as a separate channel. **This is a
   product decision the Owner must make**, not something this PR decides.
2. **Production cannot search directory names for queries containing punctuation.** `Month-03`
   contains `-`, so the FTS tier is skipped and the punctuation tier only matches name prefixes. On
   the 1M fixture production returns **0** rows while the proposed order returns **66 667**
   directory matches. The same query shape (`Q3-2026`, `final-v2`) is common in real folders.
3. **`lower()` is ASCII-only.** Production's exact/prefix tiers compare `name_normalized` against
   SQLite `lower(?)`, so `RÉSUMÉ` never reaches them; it is found only through the FTS tier. The
   oracle reproduces this instead of "fixing" it.
4. **Trigram MATCH and plain substring agree here.** 0 divergences in either direction across 9
   queries at 100k, including diacritics, CJK and punctuation. FTS5 trigram does *not* fold
   diacritics (`resume` does not match `Résumé`), so candidate B's plain folding is semantically
   compatible on this corpus.
5. **The production candidate window truncates silently.** At 1M, `pdf` has 181 250 matches but the
   product can only ever expose 4 096 of them; this is existing behaviour, not introduced here.

### 2.4 Independent code re-review

An independent reviewer examined the snapshot facade, guarded `execute` path and SQL fallback
route after these fixes. The reviewer confirmed callers cannot use the facade or returned
statements to write, control the transaction, or change `query_only`; the fallback runs in the same
live snapshot and the page wrapper respects the production 200-row limit. No raw-connection or
hook-registration accessor was found. The review found no correctness blocker; it recommended
making the page-limit contract explicit, which is now documented and enforced.

## 3. Performance

Method on the historical commit: one fixture per size, one process, `--release`, 5 warmups + 30
measured calls, except queries with more than 20 000 oracle hits used only **5 measured calls** to
bound runtime. For those high-hit rows, both p95 and p99 are the sample maximum; they are not stable
tail estimates. Corrected-code native runs use 5 warmups + 30 measured calls for every query,
including high-hit queries. The harness sorts samples and selects index `ceil((n - 1) * q)` (zero
based), without interpolation. Thus p99 is the maximum observed call for both n=5 historical and
n=30 native rows; neither is a stable population-tail estimate. Percentiles are per call and are
never summed across stages. `B filename` is RAM only; historical `B full` adds the old
`data_version`/`MAX(rowid)` check and SQLite hydration; corrected `B full` uses a snapshot identity
fence and SQLite hydration. `A` includes full posting enumeration.

**These are warm-cache numbers.** The historical cold-cache 1M `fts_substring_report` p95 of
331.420 ms is not reproduced or replaced here; the existing 100 ms pooled gate is reported
separately and was not rerun as a release acceptance gate. Same-process and same-fixture timing does
not make the paths equivalent: they differ in
ranking semantics, source-fact work, pooling and result coverage. Treat cross-path ratios as
exploratory signals, not production replacement evidence. Corrected-code native data is summarized
in §3.6 and kept in the native JSONL evidence files.

All values are p50/p95/p99 ms.

### 3.1 1 000 000 rows (official generator)

| Query | Hits | Prod SQL | Prod pooled | Prod Snapshot | A page | A window | B filename | B full page | B full window |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `zzznomatchtoken` | 0 | 0.4/0.6/0.6 | 0.4/0.6/0.8 | 0.4/1.0/1.3 | 0.0/0.0/0.0 | 0.0/0.1/0.1 | 3.9/4.4/4.5 | 4.2/4.6/5.1 | 4.2/4.6/5.1 |
| `Annual Budget 2026.xlsx` | 1 | 0.2/0.3/0.4 | 0.3/0.4/0.4 | 0.3/0.4/0.4 | 0.3/0.5/0.6 | 0.3/0.6/0.7 | 5.1/5.8/6.1 | 6.0/6.9/7.0 | 5.7/6.5/6.9 |
| `Set-0042` | 50 | 0.2/0.4/0.5 | 0.2/0.8/1.7 | 0.3/0.6/0.8 | 6.8/8.2/8.6 | 6.9/7.5/7.6 | 7.5/9.0/9.1 | 8.0/9.5/10.0 | 7.9/8.6/9.5 |
| `Month-03` (path only) | 66 667 | 0.2/0.3/0.3 | 0.3/1.0/1.0 | 0.3/0.4/0.4 | 103.9/104.7/104.7 | 113.3/116.9/116.9 | 19.4/22.6/22.6 | 21.3/24.9/24.9 | 34.6/37.8/37.8 |
| `report` | 50 000 | 97.3/99.8/99.8 | 222.1/226.4/226.4 | 216.4/218.0/218.0 | 57.9/60.7/60.7 | 65.9/70.6/70.6 | 21.8/23.8/23.8 | 21.9/23.1/23.1 | 36.0/36.9/36.9 |
| `invoice` | 50 000 | 104.1/106.5/106.5 | 235.8/242.6/242.6 | 242.3/251.6/251.6 | 61.6/63.1/63.1 | 72.0/76.2/76.2 | 19.5/20.4/20.4 | 21.4/23.9/23.9 | 32.4/35.4/35.4 |
| `IMG_` | 49 999 | 20.9/21.8/21.8 | 24.8/25.6/25.6 | 24.4/28.4/28.4 | 53.3/56.2/56.2 | 60.7/62.6/62.6 | 16.5/17.8/17.8 | 18.9/19.0/19.0 | 30.0/31.2/31.2 |
| `数据库` | 50 000 | 21.0/21.2/21.2 | 23.7/25.3/25.3 | 24.1/26.3/26.3 | 48.6/52.9/52.9 | 54.4/55.9/55.9 | 15.3/15.9/15.9 | 16.1/16.4/16.4 | 29.8/30.0/30.0 |
| `final-v2` | 50 000 | 19.5/19.8/19.8 | 23.7/25.2/25.2 | 23.9/24.0/24.0 | 68.8/71.3/71.3 | 76.3/77.9/77.9 | 16.6/16.8/16.8 | 17.5/18.3/18.3 | 30.7/32.8/32.8 |
| `Résumé` | 50 000 | 20.2/20.5/20.5 | 24.4/26.2/26.2 | 24.3/26.7/26.7 | 59.8/61.0/61.0 | 67.7/71.2/71.2 | 14.9/16.3/16.3 | 16.0/16.3/16.3 | 31.8/33.8/33.8 |
| `meeting-notes.md` | 50 000 | 0.1/0.2/0.2 | 0.2/0.2/0.2 | 0.2/0.3/0.3 | 85.7/89.6/89.6 | 110.0/121.4/121.4 | 16.1/16.5/16.5 | 16.9/18.2/18.2 | 30.3/31.3/31.3 |
| `pdf` (extension intent) | 181 250 | 0.3/0.4/0.4 | 0.4/0.5/0.5 | 0.4/0.5/0.5 | 167.7/173.0/173.0 | 199.8/201.7/201.7 | 41.5/43.0/43.0 | 52.3/52.7/52.7 | 85.9/87.4/87.4 |
| `jp` (2 codepoints) | 81 249 | 5.1/5.7/5.7 | 9.8/10.4/10.4 | 10.7/11.3/11.3 | 596.7/616.5/616.5 | 610.2/619.5/619.5 | 30.5/32.3/32.3 | 36.8/37.9/37.9 | 65.9/77.7/77.7 |

Exploratory comparison with the 100 ms warm p95 target (page = 80 rows, window = the 4 096
candidate window; these historical data do **not** qualify the target). Every listed high-hit row
has only five observations, so its displayed p95 and p99 are the largest of five calls.

| Architecture | Page p95 over 100 ms | Window p95 over 100 ms |
| --- | --- | --- |
| Production pooled | `report` 226.4, `invoice` 242.6 | same queries |
| A | `Month-03` 104.7, `pdf` 173.0, `jp` 616.5 | `Month-03` 116.9, `meeting-notes.md` 121.4, `pdf` 201.7, `jp` 619.5 |
| B | none (worst 52.7) | none (worst 87.4) |

These results are insufficient for architecture acceptance. The structural coverage findings remain
useful, while latency comparisons require 30-call measurements on the corrected HEAD:

- **A showed an exploratory signal on `report`/`invoice`.** The five-call p95 values were 60.7–63.1
  ms versus 226–243 ms pooled. They do not establish a stable tail or qualify the 100 ms gate.
- **A cannot serve the proposed semantics.** The classes the Owner asked for — word-boundary match,
  path-only match, and substring for queries shorter than 3 codepoints — are not expressible as
  SQLite index ranges. A therefore either scans the authorized fact set (`jp`: 616.5 ms,
  `Month-03`: 104.7 ms) or gives up. This is structural, not a tuning problem.

### 3.2 500 000 rows

| Query | Hits | Prod pooled | A page | A window | B full page | B full window |
| --- | --- | --- | --- | --- | --- | --- |
| `report` | 25 000 | 112.6/169.8/169.8 | 27.7/30.9/30.9 | 30.9/31.4/31.4 | 10.6/11.6/11.6 | 24.5/25.0/25.0 |
| `invoice` | 25 000 | 121.9/124.9/124.9 | 35.9/44.1/44.1 | 51.8/61.6/61.6 | 10.8/11.7/11.7 | 25.0/25.9/25.9 |
| `Month-03` | 33 333 | 0.2/0.2/0.2 | 49.6/51.6/51.6 | 54.7/56.5/56.5 | 9.3/10.2/10.2 | 23.2/23.8/23.8 |
| `pdf` | 90 630 | 0.4/0.6/0.6 | 78.4/80.5/80.5 | 102.0/113.4/113.4 | 25.4/28.8/28.8 | 54.4/57.5/57.5 |
| `jp` | 40 629 | 2.8/2.8/2.8 | 287.9/303.3/303.3 | 289.2/299.6/299.6 | 18.1/18.9/18.9 | 43.2/44.6/44.6 |

### 3.3 100 000 rows

| Query | Hits | Prod pooled | A window | B filename | B full page | B full window |
| --- | --- | --- | --- | --- | --- | --- |
| `report` | 5 000 | 23.0/24.9/25.8 | 6.8/8.2/8.6 | 2.3/3.0/3.1 | 2.2/3.0/3.1 | 14.1/16.0/16.1 |
| `invoice` | 5 000 | 24.6/45.9/52.3 | 6.8/7.4/7.5 | 2.0/2.5/2.6 | 2.0/2.6/2.8 | 14.0/17.8/19.4 |
| `pdf` | 18 130 | 0.3/0.5/0.5 | 23.7/34.6/34.8 | 4.4/4.6/4.7 | 4.9/5.3/5.8 | 26.1/28.5/28.6 |
| `jp` | 8 129 | 0.8/1.2/1.4 | 61.6/69.4/73.4 | 3.8/4.6/5.1 | 4.1/5.3/5.4 | 24.5/26.9/28.1 |

### 3.4 Historical examples where production still wins

| Query shape | Production | B | Why |
| --- | --- | --- | --- |
| Exact filename (`meeting-notes.md`) | 0.2 ms | 18.2 ms | A unique index seek beats any scan |
| Exact extension (`pdf`) | 0.4 ms | 52.7 ms | `idx_global_entries_active_extension_order` range |
| No match | 0.6 ms | 4.6 ms | FTS returns nothing; B still scans the whole name blob |

B is not a replacement for the index-backed tiers. Any production adoption has to keep them.

### 3.5 What A actually buys, and what it cannot

On the historical commit, A enumerated every posting without BM25 and ranked in Rust. At 1M with
50 000 hits, its five-call page p95 values were 57.9–63.1 ms versus 97.3–106.5 ms SQL-only and
222.1–242.6 ms pooled for production. The enumeration step alone (join included) was 37.2–47.5 ms
and Rust classification of the 50 000 rows added about 22 ms. These are small-sample exploratory
values: the nominal ratio on this set is not a stable tail estimate or production acceptance. The
enumeration cost can help when FTS scoring dominates, but exact index tiers still outperform it.

**Fairness note.** A was implemented as the literal design in the issue: "FTS5 gives candidates,
then rank in Rust", replacing the whole tier ladder. It therefore also pays enumeration for
extension-intent queries (`pdf`: 173.0 ms) where production answers from
`idx_global_entries_active_extension_order` in 0.4 ms without ever reaching FTS. A production-shaped
A that keeps the cheap tiers in front would avoid that particular regression; it would not avoid
`jp` or `Month-03`, because those need a scan.

**Verdict on A:** no performance acceptance is made from the historical samples. Corrected-code
native 30-call A measurements are now included in §3.6, but they remain warm synthetic research
data. Replacing the BM25 substring tier while retaining index tiers still requires Owner approval
of the ranking semantics and production-shaped qualification. The coverage table documents why
replacing the whole ladder would change results.

### 3.6 Corrected-code native measurements

The corrected Candidate B measurements below are from the exact source SHA/tree recorded above.
Every query has 30 measured calls after 5 warmups. Values are p50/p95/p99 milliseconds using the
order-statistic rule above. The three paths do different work and return different coverage: the
production path preserves existing ranking and may miss the proposed path-only case; B uses the
research ordering and performs SQLite hydration. These numbers do not establish a production win
or qualify the 100 ms gate.

| OS / fixture | Query | Proposed hits | N | Production pooled | B full page | B full window |
| --- | --- | ---: | ---: | --- | --- | --- |
| macOS / 100k | `zzznomatchtoken` | 0 | 30 | 0.3/0.4/0.6 | 0.6/1.0/1.1 | 0.6/0.7/1.0 |
| macOS / 100k | `report` | 5,000 | 30 | 9.3/14.8/17.1 | 3.4/4.9/6.0 | 15.3/22.5/23.6 |
| macOS / 100k | `invoice` | 5,000 | 30 | 12.8/18.6/18.9 | 1.9/2.2/2.3 | 11.5/34.1/42.2 |
| macOS / 100k | `Month-03` | 6,667 | 30 | 0.2/0.2/0.4 | 2.7/3.7/4.5 | 11.4/14.9/17.5 |
| macOS / 100k | `pdf` | 18,130 | 30 | 0.3/0.3/0.4 | 6.5/7.4/7.5 | 21.0/22.8/23.3 |
| macOS / 100k | `jp` | 8,129 | 30 | 0.8/0.8/0.9 | 5.7/8.9/9.8 | 21.5/26.1/26.6 |
| macOS / 1M | `zzznomatchtoken` | 0 | 30 | 0.3/0.6/0.6 | 6.6/10.1/10.8 | 6.5/11.5/12.0 |
| macOS / 1M | `report` | 50,000 | 30 | 85.8/105.2/112.5 | 30.4/38.5/39.2 | 36.0/37.3/39.0 |
| macOS / 1M | `invoice` | 50,000 | 30 | 91.2/103.7/108.2 | 23.2/24.9/25.5 | 32.8/36.3/38.6 |
| macOS / 1M | `Month-03` | 66,667 | 30 | 0.2/0.4/0.4 | 24.5/38.1/63.6 | 36.2/44.6/48.8 |
| macOS / 1M | `pdf` | 181,250 | 30 | 0.3/0.3/0.4 | 59.3/64.4/65.5 | 93.3/118.5/128.5 |
| macOS / 1M | `jp` | 81,249 | 30 | 6.5/7.4/8.9 | 52.1/73.4/74.3 | 77.6/90.7/97.3 |

The Mac results show the cost of broad proposed-order scans at 1M: no-match B takes 10.1 ms p95,
and `pdf` full-window hydration takes 118.5 ms p95. High-hit B page calls are below 100 ms p95 in
these six selected cases, while B's 4,096-row full window exceeds 100 ms for `pdf`. This is a
research workload summary, not a pooled production acceptance result. The complete 13-query tables
for both fixture sizes and both operating systems are regenerable from the committed raw files with
`summarize.py`.

## 4. Resources (candidate B)

The following historical table predates the snapshot fence. Its RSS values are retained only as
context; corrected-code native measurements follow it.

| Rows | Cold build | Estimated heap | Name bytes | Directory bytes | Id bytes | Slot bytes | Distinct dirs | Sampled peak RSS | Sampled CPU peak |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 100 000 | 163 ms | 12.2 MB | 3.0 MB | 2.1 MB | 1.5 MB | 3.8 MB | 34 585 | 88 MB | 200 % |
| 500 000 | 864 ms | 72.5 MB | 15.1 MB | 10.1 MB | 7.6 MB | 15.0 MB | 167 919 | 329 MB | 246 % |
| 1 000 000 | 1 805 ms | 145.0 MB | 30.2 MB | 20.2 MB | 15.3 MB | 30.0 MB | 334 585 | 624 MB | 333 % |

- "Estimated heap" is capacity accounting of the view's own buffers, not RSS.
- "Sampled peak RSS" is the whole test process sampled every 20 ms **around the build only**, and it
  includes the SQLite page cache and the pooled writer used to create the fixture. It is not an
  incremental B-attribution bound.
- The id buffer is 15.3 MB at 1M. Storing `rowid` and hydrating ids would remove it at the cost of
  losing stable-id tie-breaking; that variant was not implemented and is not claimed.

Corrected-code native memory and build measurements:

| OS / rows | Build ms | Estimated view buffers MiB | RSS at build start MiB | Sampled peak RSS MiB | Peak minus start MiB | RSS samples |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| macOS / 100k | 134 | 12.2 | 141.1 | 261.5 | 120.3 | 3 |
| macOS / 1M | 2,148 | 145.0 | 307.9 | 1,457.4 | 1,149.5 | 20 |

RSS is the whole test process sampled every 20 ms around the view build. It includes fixture and
SQLite activity and cannot be attributed to Candidate B; the 1M measurement is a material memory
risk signal, not a B-only cost. The view buffer estimate is not process RSS. Windows measurements
are listed in the same table once the Windows run completes.

## 5. Incremental writes, invalidation and rebuild

The table below is historical pre-fix evidence from `e2bab50`; it prices a full rebuild and does not
describe a supported production update contract.

| Rows | Insert 512 | Delete 512 | Full rebuild after insert | Full rebuild after delete |
| --- | --- | --- | --- | --- |
| 100 000 | 23 ms | 37 ms | 149 ms | 150 ms |
| 500 000 | 23 ms | 156 ms | 798 ms | 789 ms |
| 1 000 000 | 23 ms | 343 ms | 1 802 ms | 1 726 ms |

Corrected-code native write/rebuild calls are single measurements on the official fixture (not
30-call distributions); they price the current full-rebuild research path and do not implement an
incremental patch:

| OS / rows | Insert 512 ms | Delete 512 ms | Rebuild after insert ms | Rebuild after delete ms | Narrow-cache slots |
| --- | ---: | ---: | ---: | ---: | ---: |
| macOS / 100k | 33.0 | 25.9 | 114.6 | 110.5 | 0 |
| macOS / 1M | 21.7 | 241.2 | 1,329.6 | 1,176.8 | 0 |

The official fixture has no density query suitable for measuring narrow-cache slots, hence zero
slots here. The `write_amplification_bytes_per_row` field is based on SQLite file-size delta; zero
means the file size did not change at that sample boundary and is not a measure of bytes written.

- The experiment has **no incremental patch path**. The corrected view belongs to exactly one
  `ReadSnapshot`; after that transaction ends, the view is rejected and the caller must use the
  authoritative SQL path or build a new view. This is deliberately bounded to the research harness.
- `PRAGMA data_version` is not a persistent source generation and is only comparable on the same
  connection. The cross-connection test observes the writer's value stay unchanged on its own
  commit and the reader's value remain pinned while its read transaction is active, then advance
  after that transaction ends. The old SQLite snapshot still exposes its original facts. The
  same-connection test shows an UPDATE after the snapshot ends changes row facts without advancing
  that connection's `data_version` or `MAX(rowid)`, so the old view is rejected by its snapshot
  identity.
- The experiment now holds an exclusive mutable borrow of the reader connection, sets `query_only`,
  and installs an SQLite authorizer that denies writes, `COMMIT`/`ROLLBACK`, savepoints, and changes
  to `query_only`. It flushes cached statements before installing the guard. Callers get a restricted
  SQL-read facade, not `rusqlite::Connection`, so they cannot replace the authorizer. Build,
  selection and hydration run inside that pinned transaction. Another connection may commit while
  the read snapshot stays stable; another snapshot or connection cannot reuse the view. A test-only
  fault injector bypasses the guard solely to exercise hydration fallback. This is not a durable or
  cross-process generation and does not make B a persistent application cache.
- Hydration that drops a selected row or observes a changed rank returns
  `Fence::HydrationMismatch` with empty result vectors. The page wrapper executes the complete
  authoritative SQL lookup under the same pinned `ReadSnapshot`, bypassing the preselected subset;
  it does not return a partial page. It takes a requested page length and clamps it to the existing
  production 200-row limit, so Candidate B's 4,096-row research window is never mistaken for a user
  page. If that snapshot cannot be used, the caller restarts the full lookup under a new snapshot
  rather than returning the old B page.
- `NarrowCache` holds the originating view identity plus the folded query and slots. A different
  rebuilt view bypasses it before slot access; a defensive range check also rejects invalid slots.
  The shrink and growth tests compare the bypass result with the complete oracle.
- Memory pressure: `ZC_ISSUE494_B_MEMORY_BUDGET_BYTES` (default 256 MiB) aborts the build and returns
  no view. This limit counts estimated view buffers only; it is not a process-wide budget and omits
  build-time maps, SQLite pages and allocator overhead. The 1-byte degradation check remains
  covered. The proposed 256 MiB production cap is provisional: native Mac whole-process RSS rose by
  1,149.5 MiB over its pre-build sample at 1M, and the current measurements cannot attribute that
  rise to B or certify a process-wide cap.

## 6. Failures, risks and unresolved limits

1. **No incremental update.** 1.8 s rebuild per write batch at 1M is old exploratory evidence; there
   is no production update implementation or latency claim in this follow-up.
2. **Memory evidence is incomplete.** The corrected view-buffer estimate is 145 MiB at 1M; Mac
   whole-process sampled RSS increased 1,149.5 MiB from build start to peak. That RSS includes
   SQLite and fixture setup and is not attributable to B, so the provisional 256 MiB B-owned cap
   remains unverified. Windows native data is added below when its job completes.
3. **High-hit historical percentiles are weak.** Those rows have five samples, so their p95/p99 are
   the maximum observed call. They do not qualify the 100 ms target.
4. **The corrected native run is warm-cache synthetic evidence only.** The old five-sample rows
   have p95 and p99 equal to the maximum of five calls. Corrected-code queries have 30 samples, but
   p99 is still the observed maximum; neither sample count estimates a stable population tail.
   Full raw samples and summaries are retained for Owner review.
5. **Cold-cache behavior is unverified.** No cold-cache 1M run was performed, so the historical cold
   331 ms is not shown fixed.
6. **A is measured as a whole-ladder replacement on the historical commit.** Its `pdf` row is not
   production-shaped; the structural failures (`jp`, `Month-03`) remain coverage findings.
7. **Synthetic corpus.** 334 585 distinct directories, 16 extensions, fixed-width names. Real disks
   have longer tails; the directory interning ratio is the number most likely to move.
8. **Extension-intent collision** is unresolved and is a product decision, not a defect.
9. **Local Linux validation used a disposable Ubuntu 24.04 container.** After installing the Tauri
   GTK/WebKit build dependencies there, the repository lib test target compiled and all 23
   `issue494_*` adversarial tests passed. That run preceded the isolated `pulldown-cmark` advisory
   patch; the hosted exact-head jobs cover the current dependency locks.
10. **The prior Windows local build note** recorded `rustc` repeatedly dying with
   `STATUS_ACCESS_VIOLATION` and hung while the sandbox denied access to
   `F:\CargoTarget\incremental\*.lock`. Builds had to run with `CARGO_INCREMENTAL=0` and a single
   job. This is a historical environment note, not a current test result.

## 7. Architecture recommendation

**No production adoption is approved by this PR.** Keep the exact-name, name-prefix,
extension-exact and extension-prefix tiers unchanged. Candidate B remains a test-only experiment;
the product ranking and extension-intent decision remain open.

| Decision | Verdict |
| --- | --- |
| A (FTS5 enumeration + Rust ranking, no BM25) | **No decision** — old path measurements are exploratory and do not qualify a replacement |
| B (compact RAM filename index) | **Research only** — snapshot-scoped correctness is covered; persistent invalidation, incremental updates and memory are not production-ready |
| Replace production now | **NO** — no production search, cache, schema, or ranking behavior changes; the preview parser only receives the RustSec patch update described above |

### Executable incremental-update proposal for a separate production gate

1. In a later additive schema migration, maintain a monotone `global_search_generation` and a bounded
   `global_search_changes` feed in the same write transaction as source facts. SQLite triggers must
   cover insert/update/delete on `global_entries` fields that affect membership or rank (id, volume,
   stale state, normalized name/path, extension, mtime) and authorization changes on
   `global_volumes`. A volume disable/removal event invalidates every row for that volume.
2. A worker reads the generation, change feed and current rows in one read transaction. It coalesces
   entry changes by stable id, patches the compact view, checks there are no missing feed generations,
   then publishes the immutable view atomically. Readers keep their current `Arc` until completion.
3. Bound each patch batch to 5,000 changed rows or 50 ms, whichever comes first. If the feed has a
   gap, volume-wide invalidation exceeds the batch, or the patch cannot fit the memory limit, mark B
   unavailable and answer via the current SQL path until a fresh view is ready. Never publish a
   partially applied generation.
4. Keep B limited to the FTS/BM25 substring tier until an Owner approves product semantics. Exact
   name, name prefix, extension exact and extension prefix remain on their existing covering indexes.
   Extension results remain separate until ranking is decided.
5. Begin with shadow comparison against SQL on the same generation and record missing/extra ids and
   order differences. Only after zero correctness divergence and paired native qualification should
   a feature flag allow guarded requests to use B. Generation mismatch, hydration mismatch, budget
   refusal or update lag always falls back to SQL.

### Provisional RAM policy for the production evaluation

- Cap all B-owned memory at **256 MiB total**, including the active view, pending deltas, staging
  allocations and any retired view still held by a reader. Keep a single active view at or below
  **160 MiB**, and a patch/staging allowance at or below **64 MiB**.
- The historical 1M view estimate is 145 MiB, so one view is below the proposed active limit. A
  second full copy would be about 290 MiB before temporary maps and exceeds the total cap; the
  incremental path must patch within the allowance or use SQL fallback. The current buffer estimator
  is not complete enough to certify these caps; native allocation/RSS accounting must include maps,
  allocator overhead and overlap.
- On budget refusal, generation lag or low-memory pressure, use SQL for search and discard an
  over-budget staging view. No ranking or result limit changes accompany this degradation.

### Production migration sequence

1. Owner approves ranking semantics and the 256 MiB B-owned budget proposal.
2. Implement generation triggers and the incremental feed behind a separate schema migration with
   migration/reopen/rollback coverage; do not change the production search path yet.
3. Run SQL-vs-B shadow checks, 100k and 1M paired Windows/macOS Release measurements, cold-start
   and memory-pressure cases, and a real-disk corpus. Bind every artifact to the exact SHA/tree.
4. Enable guarded B only for the FTS substring tier behind an opt-in flag; keep SQL fallback on every
   incomplete or over-budget state. Expand only after native acceptance.
5. Roll back by disabling the flag and returning to SQL. Retain additive generation data until the
   Owner approves a later schema cleanup; this PR has no migration to reverse.

## 8. CI evidence

CI results are recorded only after runs for the final HEAD complete. The previous run
`40650100384` belongs to an earlier commit and is not evidence for this follow-up. The
branch-scoped native job runs the correctness cases and produces separate 100k and 1M JSONL files
on `windows-latest` and `macos-latest`; each record contains its source SHA and tree. The existing
100 ms pooled gate result is reported separately from the research benchmark.

## 9. Reproduction

```bash
# Adversarial Candidate B correctness cases
cargo test --release --manifest-path src-tauri/Cargo.toml --features desktop-runtime --lib \
  global_index::tests::global_search_benchmark::filename_search_ab::cases:: \
  -- --nocapture --test-threads=1

# Scale probe, one size at a time, release only
ZC_ISSUE494_ENTRIES=1000000 ZC_ISSUE494_FIXTURE=official \
  cargo test --release --features desktop-runtime --lib \
  filename_search_ab::scale::issue494_filename_ab_scale_probe \
  -- --ignored --exact --nocapture --test-threads=1

# Controlled density (exact 1/40/5 000/50 000 hit groups)
ZC_ISSUE494_ENTRIES=100000 ZC_ISSUE494_FIXTURE=density \
  cargo test --release --features desktop-runtime --lib \
  filename_search_ab::scale::issue494_filename_ab_scale_probe \
  -- --ignored --exact --nocapture --test-threads=1
```

The branch-scoped Windows/macOS job runs the same official scale command at both 100k and 1M and
uploads SHA/tree-bound JSONL artifacts. The dispatch-only `issue494_ab` and `issue494_ab_density`
modes remain available for further exploration. These are research measurements; the existing
100 ms pooled-search gate is a separate CI result.

Raw JSONL for every number in this report is committed under
`docs/research/evidence/issue-494/`:

| File | Content |
| --- | --- |
| `official-100k.jsonl` | 100k official fixture, 13 queries, fixture + B build + incremental records |
| `official-500k.jsonl` | 500k official fixture |
| `official-1m.jsonl` | 1M official fixture |
| `density-100k.jsonl` | 100k controlled density fixture (exact 1/40/5 000/50 000 hit groups) |
| `summarize.py` | Read-only script that regenerates the tables above from the JSONL |

Each record carries `experiment`, `record`, `os`, `build`, the fixture identity, the query, the
sample count and the raw percentile triples, so a table cell can be traced back to one line.
