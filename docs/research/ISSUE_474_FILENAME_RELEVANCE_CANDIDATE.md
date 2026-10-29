# Issue #474 — filename relevance candidate

Status: **default-off candidate for Owner comparison; production ranking is unchanged.** This prototype intentionally changes ranking in its opt-in fuzzy tier. It does not claim equivalence to BM25 and is not approved for user-visible cutover.

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
- Same-process Windows Debug comparisons reuse the deterministic production-schema 100k and 200k generator, then redistribute its 5% `report` hits among boundary, substring, and path-only cases through the existing FTS triggers. Five warmups and 30 alternating paired samples cover raw FTS SQL, pooled Search, Repository Snapshot, and 80/4096 windows. Additional paired Search/Snapshot runs cover 63-hit, 1-hit, and zero-hit supported ASCII fallback queries, including the 4096 window for the 63-hit query. Evidence records p50/p95/p99, SQLite VM/sort and allocator high-water counters, Windows process CPU/working-set/private bytes, database/WAL sizes, exact source SHA/tree, and raw JSONL.
- No 500k/1M or Release qualification is dispatched until the 100k/200k correctness and resource results are reviewed. Owner decides whether the proposed UX contract is acceptable and whether one larger native qualification is worthwhile.
- Rollback is immediate: leave the feature absent or unset `ZC_GLOBAL_SEARCH_FILENAME_RELEVANCE_CANDIDATE`; the default BM25 code path, existing FTS triggers, schema, and index authorization remain unchanged.

### Reproduction and manual comparison

From the repository root, set a new, non-existing output path for each run. The test refuses to overwrite existing raw evidence. Keep `TEMP`, `TMP`, and `CARGO_TARGET_DIR` on a non-C: volume for the large fixtures.

```powershell
$env:TEMP = 'F:\issue-474\tmp'
$env:TMP = $env:TEMP
$env:CARGO_TARGET_DIR = 'F:\CargoTarget'
$env:ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES = '100000'
$env:ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT = 'F:\issue-474\evidence\100k-filename-relevance.jsonl'
cargo test --manifest-path src-tauri/Cargo.toml --lib filename_relevance_candidate_paired_100k_200k -- --ignored --nocapture
```

Repeat with `ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES=200000` and a new `200k-filename-relevance.jsonl` output. The golden contract/fallback test runs without `--ignored`:

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
