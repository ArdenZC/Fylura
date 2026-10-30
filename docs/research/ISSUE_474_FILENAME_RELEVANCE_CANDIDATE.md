# Issue #474 — filename relevance candidate

Status: **default-off candidate; P0 corrections implemented; correctness and native Release qualification are pending for the corrected source.** Keep PR #510 open/Draft. Do not merge, release, or enable the route by default. No derived ranking index is authorized.

The earlier Windows Debug measurements below are historical observations from source `08d8b07f5058cc51e8b282b8aed85079fb045a50`. They do not qualify the corrected SQL or the current PR head.

## Proposed product contract

The first four production tiers stay in their existing order and implementation:

1. Exact filename.
2. Filename prefix.
3. Exact extension.
4. Extension prefix.

Only the later FTS tier uses this default-off candidate. For supported queries its order is:

1. Filename whole-word or bounded-phrase match.
2. Filename or extension substring.
3. Path-only match.

Within each candidate class, the order is `modified_at_fs DESC, id ASC`. SQLite retains its current NULL ordering for modification time. Files and directories participate equally. Existing hidden/system behavior and complete `GlobalSearchResult` fields remain in force. Enabled, non-coverage-pending volumes and non-stale entries are filtered before sorting and limiting. The existing caller still deduplicates IDs across tiers.

Boundary characters are ASCII letters, digits, and underscore. Every other code point, including CJK characters and punctuation, is a delimiter around a supported ASCII query. Thus `pre_report_suffix.pdf` is a substring match. Accented or non-ASCII query text, punctuation, tabs, repeated spaces, and queries shorter than three bytes stay on the existing SQLite BM25 route.

Supported ASCII queries with one or more internal spaces use a contiguous phrase. Zero, sparse, and dense match sets all use the same candidate ordering. There is no 63/64/65 count threshold. Inserting or deleting a match can insert or remove that row in the order, but it does not change the relative order of unchanged rows.

## Boundary implementation and retrieval

The SQL builds one filename GLOB pattern over `('/' || name_normalized || '/')`. The slash sentinels make a filename start or end an ordinary delimiter. The pattern can match any occurrence with legal boundaries, including a later occurrence after an earlier embedded substring. Supported query text excludes GLOB metacharacters. The classifier uses no recursive CTE and no per-character SQL query; substring checks use `instr` once per field.

The independent Rust oracle scans every Unicode scalar start in each basename, checks the complete query at each start, and independently tests the neighboring characters. It does not reuse the SQL pattern or inspect only the first match.

The candidate reuses the existing FTS5 table and triggers. SQLite ranks all eligible FTS matches by the proposed class and tie-breakers before applying the existing bounded 4096-row window. A wrong newest-first 4096-row prefix is never reclassified. The key-only CTE sorts identifiers, rank, and modification time; only the selected window is hydrated with full result fields and managed flags. Repository Search Snapshot continues to run under its existing SQLite read transaction.

The route still requires both the `global-search-filename-relevance-candidate` Cargo feature and `ZC_GLOBAL_SEARCH_FILENAME_RELEVANCE_CANDIDATE=1`. Default builds and unset environment variables keep the BM25 route. No default index authorization, exclusion policy, schema, persistent index, trigger, worker, or result API changed in this candidate.

## Correctness coverage added

The focused tests compare complete result objects and exact order to the Rust oracle. They cover repeated word and phrase occurrences (`prereport-report.pdf`, `xreporty-report.txt`, a repeated `report archive` phrase), ordinary substrings (`prereport.txt`, `reporting-guide.pdf`), underscore boundaries, accented/CJK neighbors, an empty path, Windows and POSIX paths, directories, duplicate basenames, equal timestamps, NULL timestamps, hidden/system/managed rows, disabled/stale/coverage-pending sources, and the preserved first four tiers.

A separate fixture performs 63→64→65 insertion and deletion transitions, repeats each query, checks multiple pages and tied modification times, verifies stale and volume-enabled updates, and holds a Search read transaction across a concurrent source-status update. The scale harness also checks 80-row first pages, offset 4016, the 4096 boundary, zero and sparse hits, full output fields, schema stability, SQL query plans, SQLite statement counters, and repository snapshot status/revision consistency.

These tests are implemented but their result is not claimed until current-HEAD CI completes. The CI Rust quality jobs run `cargo fmt --check`, Clippy with warnings denied, and the Rust test suite. The focused release workflow runs the contract and 63/64/65 tests on the exact source SHA before measuring.

## Old/new scenario comparison

The correctness fixture emits `filename_relevance_golden_comparison` JSONL with old BM25 and proposed public-result positions, the public fast-tier classification, the independent FTS candidate class and position, rationale, and a stated limitation for each case. These labels distinguish a row that the unchanged fast tiers return first from the class it receives if it appears in the FTS candidate set. It includes more than ten file-manager scenarios:

| Query / file scenario | Proposed class | Why that order helps | Limitation |
| --- | --- | --- | --- |
| `report` — `Quarterly Report 0.pdf` | Filename boundary | Both words are visible in the basename. | No term-frequency or exact phrase scoring within the class. |
| `report` — `中文report中文.pdf` | Filename boundary | CJK neighbors are delimiters under the explicit rule. | No language-specific CJK segmentation. |
| `report` — `Dossier-report-final.pdf` | Filename boundary | Hyphens separate the queried word. | All non-ASCII-word punctuation is treated alike. |
| `report` — `prereport-report.pdf` | Filename boundary | The later bounded match wins over the first embedded occurrence. | Multiple occurrences do not earn extra score. |
| `report` — `xreporty-report.txt` | Filename boundary | The later complete word is found despite an earlier embedded occurrence. | Supported queries use an ASCII-defined boundary. |
| `report` — `Annual reporting guide 0.pdf` | FTS filename substring | A query followed by a word character stays below a whole-word hit. | No stemming or linguistic word forms. |
| `report` — `Subreport archive.pdf` | Filename substring | A query embedded inside a larger word remains searchable at lower priority. | The substring is not discarded. |
| `report` — `pre_report_suffix.pdf` | Filename substring | Underscore is a word character by contract. | Some users may expect underscore to separate words. |
| `report` — `reporting-guide.pdf` | Public filename-prefix tier; FTS class is substring | The existing prefix tier retains priority; its FTS class remains available in the candidate contract. | Prefix results precede fuzzy FTS classes by design. |
| `report` — `reporté notes.pdf` | Public filename-prefix tier; FTS class is boundary | The non-ASCII neighbor is a delimiter under the FTS rule, while the unchanged prefix tier returns this row first. | Accent folding is not supported for query text. |
| `report` — filename with empty indexed path | FTS filename boundary; public prefix tier | Basename relevance does not require a path string; the public route still preserves its prefix tier. | Empty paths still constrain navigation and identity elsewhere. |
| `report` — a Windows path-only hit | Path only | Filename matches appear before path-only matches even when the path row is newer. | Recency still orders rows inside the path-only class. |
| `report` — a POSIX path-only hit | Path only | Both separator styles receive the same relevance class. | Path normalization remains platform-owned. |
| `report` — `Team Report Archive` directory | Filename boundary | Directory names are searchable using the same basename rule. | File versus directory is not an extra tie-breaker. |

The previous golden JSONL positions came from a 74-hit fixture and the flawed first-occurrence oracle. They are obsolete; the corrected fixture and output replace them when the exact-source correctness job runs. The new fixture also verifies that exact name, name prefix, exact extension, and extension prefix retain priority.

## Native Release qualification gate

The corrected implementation needs fresh Windows x86_64 and Apple Silicon macOS Release evidence. The new `filename_relevance_release_preflight` workflow mode runs 100k and 200k fixtures on each native runner, using five warmups and 30 paired samples per route on the same device and database. Only after correctness, latency, disk, SQLite-memory, process-memory, and CPU evidence is reviewed should `filename_relevance_release_million` be dispatched with `entries=1000000`; it runs one 1M paired fixture on each platform.

Each JSONL record captures exact source SHA/tree, runner image/name/CPU, Rust and SQLite versions, p50/p95/p99 for raw SQL, pooled Search and Repository Snapshot, output ID hashes, sort/full-scan/VM-step counters, SQLite allocator high-water, process CPU, and native memory observations. The query plan records FTS access, materialized key sorting, and temporary B-tree steps. Windows reports working-set/private-memory boundary samples. macOS reports CPU and `getrusage` cumulative peak RSS; the system API does not expose per-query private footprint in this harness. The fixture includes cold setup and population timings, while the formal 30 samples are warm-cache steady-state after five warmups; no cold-open latency claim is made.

The production p95 target remains **100 ms** for complete Search and Repository Snapshot. The corrected source has no Release results yet, so Release performance, resource use, and production readiness are currently **unverified**. The old Debug data below is not a forecast and is not a pass.

## Historical Debug evidence — superseded

These same-run Windows x86_64 Debug results came from the prior SQL/Oracle and the 64-hit switch. They only explain why the experiment continued; they cannot qualify the corrected candidate.

| Entries / `report` hits | Page | Raw SQL p95, BM25 → prior candidate | Search p95, BM25 → prior candidate | Snapshot p95, BM25 → prior candidate |
| --- | --- | ---: | ---: | ---: |
| 100k / 5k | 80@0 | 20.285 → 13.252 ms | 20.519 → 14.876 ms | 19.933 → 14.237 ms |
| 100k / 5k | 80@4016 | 38.891 → 29.007 ms | 40.974 → 34.483 ms | 41.587 → 34.825 ms |
| 200k / 10k | 80@0 | 37.039 → 23.675 ms | 37.306 → 24.705 ms | 37.651 → 25.747 ms |
| 200k / 10k | 80@4016 | 56.020 → 43.060 ms | 61.778 → 48.502 ms | 61.187 → 48.836 ms |
| 1M / 50k | 80@0 | 193.953 → 125.915 ms | 188.027 → 120.654 ms | 184.427 → 121.577 ms |
| 1M / 50k | 80@4016 | 235.843 → 154.081 ms | 222.397 → 153.439 ms | 221.405 → 153.818 ms |

The prior 1M Debug candidate exceeded 100 ms p95 on first-page and offset-4016 Search and Snapshot. Its source was `08d8b07f5058cc51e8b282b8aed85079fb045a50`; it is not current-source qualification. The old 1M fixture database measured about 1,381,838,848 bytes main file and 286,298,832 bytes WAL. The old candidate added no persistent objects. Its raw evidence and hashes remain in `evidence/issue-474-filename-relevance/` as history, not as proof for the corrected query.

If the corrected Release candidate still exceeds 100 ms at 1M after this one substantive SQL correction, stop tuning. Report the bottleneck and ask the Owner to decide whether a bounded derived ranking index is worth a separate authorization. Do not add that index, change memory budgets, merge the PR, or enable production search here.
