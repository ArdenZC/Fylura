# Issue #474 — filename relevance candidate

Status: **default-off candidate for Owner comparison; production activation is NO-GO on the 100ms p95 gate.** At 1M entries/50k hits, candidate Search p95 is 120.654ms first page and 153.439ms at offset 4016. Production ranking remains unchanged. This prototype intentionally changes ranking in its opt-in fuzzy tier and does not claim equivalence to BM25.

## Product search contract

### Tier order

For both the current route and the candidate, preserve the existing fast tiers in this order:

1. Exact filename.
2. Filename prefix.
3. Exact extension.
4. Extension prefix.
5. Fuzzy matching (the existing production punctuation-prefix route continues to handle punctuation-shaped input).

The candidate changes only tier 5 for supported queries. Its proposed fuzzy relevance order is:

1. **Filename word or phrase boundary:** the complete query occurs in the basename, with a boundary before and after it.
2. **Filename substring:** the query occurs elsewhere in the basename or in the indexed extension field. Exact/prefix extension tiers still run earlier.
3. **Path-only substring:** the query occurs in the path and not in the basename or extension.

Within a relevance class, sort by `modified_at_fs DESC, id ASC`; SQLite's existing NULL ordering for `modified_at_fs` is retained. Directories participate by basename and path. Hidden/system entries keep current behavior. Enabled-volume and non-stale filters apply before counting, ranking, and limiting. Cross-tier duplicate IDs are removed by the existing caller.

### Supported queries and fallback

The experimental order applies only to ASCII letters/digits and single internal ASCII spaces, with at least three characters. Matching remains literal against the existing trigram FTS index with ASCII case-insensitive behavior. A phrase with spaces is a contiguous phrase; it receives boundary priority only when the complete phrase is bounded on both sides.

Boundary characters are defined portably as ASCII letters, digits, and underscore. Every other code point, including CJK characters and punctuation, is a delimiter around a supported ASCII query. Query text containing non-ASCII characters, accents, combining marks, punctuation, tabs, repeated spaces, or fewer than three characters uses the existing production route. If the eligible FTS hit count is zero or below 64, use the existing BM25 route as well. This keeps Chinese/Unicode, punctuation, short, zero-hit, and sparse behavior unchanged in this prototype. The 64-hit crossover is a proposed experiment parameter, not a production threshold.

### Complete retrieval, paging, and resource boundary

For supported queries with at least 64 eligible hits, the candidate reads the complete FTS match set, classifies every eligible row, and orders the full set by the new relevance contract before applying the existing de-duplicated window and page. It must never take a newest-first prefix and call it a complete result. The existing page sizes, offsets, 4096 candidate-window cap, fields, managed flags, and source snapshot transaction remain authoritative.

The candidate reuses the current `global_entries_fts` trigram table and its existing incremental triggers. It adds no table, index, trigger, schema version, background worker, or file-fact authority. It uses a key-only SQLite candidate set and the database's temporary rank sort; only the selected window is hydrated with full fields and managed flags. Default builds continue to use the exact current BM25 route. The experimental route requires both the `global-search-filename-relevance-candidate` Cargo feature and `ZC_GLOBAL_SEARCH_FILENAME_RELEVANCE_CANDIDATE=1`; unset either and the current route remains active. No setting, UI control, default index authorization, or release routing is changed.

## Comparison examples

Observed 1-based positions from the independent 74-hit golden fixture are below. These are controlled product examples, not a relevance quality claim about a user's corpus. Within each category the proposal orders by modification time descending, then stable ID ascending.

| Query / example | Current order | Proposed order |
| --- | --- | --- |
| `report` — `Quarterly Report 0.pdf` | BM25 #20 | Filename boundary #26 |
| `report` — `中文report中文.pdf` | BM25 #1 | Filename boundary #15 (CJK delimiters count as boundaries) |
| `report` — `Dossier-report-final.pdf` | BM25 #41 | Filename boundary #21 |
| `report` — `Team Report Archive` (directory) | BM25 #2 | Filename boundary #9 |
| `report` — `Annual reporting guide 0.pdf` | BM25 #45 | Filename substring #51 (`report` is followed by a word character) |
| `report` — `Subreport archive 1.pdf` | BM25 #6 | Filename substring #50 (preceded by a word character) |
| `report` — `Supportreport scan 2.pdf` | BM25 #10 | Filename substring #49 |
| `report` — `pre_report_suffix.pdf` | BM25 #7 | Filename substring #52 (underscore is a word character) |
| `report` — `summary file 21.pdf` under POSIX `/Users/owner/Archive/Reports/` | BM25 #74 | Path-only #53 |
| `report` — `summary file 0.pdf` under `C:\Global\Archive\Reports\` | BM25 #62 | Path-only #74, after every filename match despite its newer timestamp |
| `umen` — `manual-file-0.file`, extension field `document` | BM25 #11 | Filename/extension substring #65 |
| `pdf` — `summary file 0.pdf` | Exact-extension #44 | Same exact-extension #44 |
| `do` — `manual-file-0.file`, extension field `document` | Extension-prefix #66 | Same extension-prefix #66 |
| `数据库` — `档案-数据库-历史.pdf` | SQLite BM25 fallback #1 | Same full result and order #1 |
| `café` — `Café résumé notes.pdf` | SQLite fallback #1 | Same full result and order #1 |
| `needle` — `Monocle needle notes.pdf` (one eligible hit) | BM25 fallback #1 | Same full result and order #1 |
| `annual.report` — embedded punctuation example | No result in this fixture | Same punctuation fallback; no result |
| `zzznomatch` — zero eligible hits | No result | Same zero-hit fallback |

The contract test also verifies two same-name files in different directories remain distinct and use the stable ID tie-breaker. The fixture contains 27 filename-boundary, 25 filename-substring/extension, and 22 path-only `report` hits; a deliberately newer path-only row proves the category order is applied before recency.

## Acceptance and rollback

- An independent Rust oracle classifies the complete FTS match set and compares all candidate result fields, relevance order, and page slices. It does not use BM25 as the candidate oracle.
- Focused cases cover ties, duplicate names/paths, directories, extension-only hits, stale/disabled rows, managed flags, Windows-style and POSIX-style paths, fast-tier precedence, Unicode/punctuation/short queries, sparse/zero fallback, and offsets 0/4016/4096.
- Rollback is immediate: leave the feature absent or unset `ZC_GLOBAL_SEARCH_FILENAME_RELEVANCE_CANDIDATE`; the default BM25 code path, existing FTS triggers, schema, and index authorization remain unchanged.

### Windows qualification and decision

These are same-build Windows x86_64 Debug runs with the `desktop-runtime` and candidate Cargo features, SQLite 3.51.3, source `08d8b07f5058cc51e8b282b8aed85079fb045a50`, and a clean tree. Every pair used five warmups and 30 alternating old/candidate samples. The fixture has a 5% `report` hit rate and deliberately produces different old/new ID orders. `80@4016` measures the page beginning at offset 4016 and therefore requires the complete sorted candidate window through 4096.

| Entries / `report` hits | Page | Raw FTS SQL p95/p99, BM25 → candidate | Search p95/p99, BM25 → candidate | Snapshot p95/p99, BM25 → candidate |
| --- | --- | ---: | ---: | ---: |
| 100k / 5k | 80@0 | 20.285/21.035 → 13.252/13.526 ms | 20.519/20.898 → 14.876/15.873 ms | 19.933/20.479 → 14.237/14.476 ms |
| 100k / 5k | 80@4016 | 38.891/39.010 → 29.007/29.540 ms | 40.974/41.150 → 34.483/34.841 ms | 41.587/42.090 → 34.825/36.085 ms |
| 200k / 10k | 80@0 | 37.039/37.483 → 23.675/26.104 ms | 37.306/37.401 → 24.705/24.999 ms | 37.651/37.860 → 25.747/25.933 ms |
| 200k / 10k | 80@4016 | 56.020/56.445 → 43.060/44.357 ms | 61.778/61.957 → 48.502/49.217 ms | 61.187/62.096 → 48.836/49.475 ms |
| 1M / 50k | 80@0 | 193.953/196.441 → 125.915/129.045 ms | 188.027/188.587 → 120.654/121.197 ms | 184.427/185.524 → 121.577/121.687 ms |
| 1M / 50k | 80@4016 | 235.843/243.573 → 154.081/154.541 ms | 222.397/224.888 → 153.439/154.302 ms | 221.405/223.243 → 153.818/156.281 ms |

The candidate returns the exact independently computed new order and page at all three scales. It reduces high-hit latency and CPU in this synthetic fixture, with SQLite allocator high-water about 5.3 MiB for the first page and 5.8 MiB through offset 4016 at 1M; the BM25 route used about 5.5 MiB and 6.0 MiB. At 1M, pooled Search process CPU across the 30 measured calls fell from 5,578 to 3,516 ms at offset 0 and from 6,594 to 4,500 ms at offset 4016. These are process boundary counters, not SQL-attributed CPU. The candidate's VM-step count is higher because it computes the explicit categories for every FTS match.

**Production activation is NO-GO against the 100ms p95 goal.** At 1M entries and 50k eligible hits, candidate Search p95 is 120.654 ms on the first page and 153.439 ms at offset 4016; p99 is 121.197 and 154.302 ms. Snapshot p95 is 121.577 and 153.818 ms. The candidate is materially faster than BM25, but it does not meet the target. This is evidence about this implementation and synthetic workload, not a claim that all BM25 queries or real corpora have the same behavior.

Sparse and zero-hit queries preserve full BM25 result objects and order. In the 1M run, candidate-route Search p95 was 2.949 ms for 63 hits at offset 0 (BM25 1.481 ms), 3.209 ms at offset 4016 (1.614 ms), 1.436 ms for one hit (1.027 ms), and 1.026 ms for zero hits (0.615 ms). The added probe therefore costs roughly 0.4–1.6 ms on these low-hit cases. The 1M synthetic database used 1,381,838,848 bytes for the main file and 286,298,832 bytes for WAL at capture; the candidate adds zero persistent indexes, triggers, or file facts, and the schema and existing FTS trigger SQL were unchanged.

The native 1M result gives Owner a concrete product/performance choice: retain BM25 for high-hit queries and accept that this workload exceeds the target, or approve a separately budgeted derived filename-ranking index to pursue the proposed relevance order below 100ms. The latter needs explicit memory, write-amplification, startup/idle CPU, generation-consistency, and rollback limits before implementation. No such index or default-on route is included here. No 500k run or further optimization loop is planned in this candidate.

Raw JSONL evidence for all three same-SHA runs is committed beside this document:

- `evidence/issue-474-filename-relevance/100k-windows-desktop-runtime.jsonl` — SHA-256 `D604CE6BEF17D424AF639CAA02C61995DCDC8F2FEE70C959DC3E419E868FB211`.
- `evidence/issue-474-filename-relevance/200k-windows-desktop-runtime.jsonl` — SHA-256 `3A775C20E67FC8A23BDE1003EDD75163935A32FD45F018BAB2DD032C8FA21E5A`.
- `evidence/issue-474-filename-relevance/1m-windows-desktop-runtime.jsonl` — SHA-256 `85CF250F01CE406F5D2EDBA86C24CD97E4872ED93524D04EFAD0A939F7CAC48F`.

The harness used the existing deterministic global-search fixture and existing FTS update triggers. It is a Windows-native database qualification, not an installed-app GUI acceptance; macOS validation is delegated to the pull request's native CI. Benchmark results are Debug observations, not Release forecasts.

### Reproduction and manual comparison

From the repository root, set a new, non-existing output path for each run. The test refuses to overwrite existing raw evidence. Keep `TEMP`, `TMP`, and `CARGO_TARGET_DIR` on a non-C: volume for the large fixtures.

```powershell
$env:TEMP = 'F:\issue-474\tmp'
$env:TMP = $env:TEMP
$env:CARGO_TARGET_DIR = 'F:\CargoTarget'
$env:ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES = '100000'
$env:ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT = 'F:\issue-474\evidence\100k-filename-relevance.jsonl'
cargo test --manifest-path src-tauri/Cargo.toml --features 'desktop-runtime,global-search-filename-relevance-candidate' --lib filename_relevance_candidate_paired_100k_200k -- --ignored --nocapture
```

Repeat with `ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES=200000` and a new output path. After 100k/200k correctness and resource checks pass, the same harness can perform the single 1M qualification by setting `ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES=1000000`. To reproduce the committed Windows runs, include `--features 'desktop-runtime,global-search-filename-relevance-candidate'`. The golden contract/fallback test runs without `--ignored`:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib filename_relevance_contract_oracle_and_fallbacks -- --nocapture
```

For a manual desktop comparison, build the frontend if needed, then launch once with the candidate feature and environment opt-in. Unset the environment variable to compare the production BM25 order in the same database:

```powershell
npm run build:frontend
$env:ZC_GLOBAL_SEARCH_FILENAME_RELEVANCE_CANDIDATE = '1'
cargo run --manifest-path src-tauri/Cargo.toml --features 'desktop-runtime,global-search-filename-relevance-candidate' --bin zen-canvas
```

This command does not enable the candidate in a default build or alter index authorization.
