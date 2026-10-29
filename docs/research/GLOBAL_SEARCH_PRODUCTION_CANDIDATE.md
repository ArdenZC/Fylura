# Global Search production candidate: Stage A shadow qualification

## Decision

**NO-GO for Stage B or production routing.** This branch keeps the SQLite search result authoritative and leaves the shadow worker disabled by default. The prior Windows and Apple Silicon macOS 100k/500k/1M matrix found `pdf` and `Month-03` candidate-window differences from FTS/BM25 at all sizes; `Month-03` also returned a different first page. Candidate-only p95 exceeded 100 ms for high-hit and zero-hit queries at 1M. The full existing route also exceeded the historical 100 ms p95 gate for high-hit queries on Windows at 500k and both platforms at 1M. The exact follow-up native run is pending; these correctness and latency failures continue to block Stage B.

## Current remediation snapshot — 2026-10-29

This section supersedes older implementation/status statements below where they conflict. The follow-up starts from PR head `d2e2a24b2c2e370ec82934b63b9e69c9d48c6e3a`, merged with master `0626e2f6` in local merge `5878469d2a3dd9438cb0824b0bd95b489c0b556f`. The final follow-up SHA, exact-head CI, and paired native write-cost artifacts will be recorded after delivery.

### Search contract and router

The existing SQLite order remains the only user-visible contract: exact name → name prefix → exact extension → extension prefix → FTS/BM25 or punctuation-prefix. The shadow view only observes a production page and cannot provide visible results. Candidate B still chooses `modified_at DESC, id ASC`; matching a few fixture queries is not evidence that this order is interchangeable with BM25.

| Query/shape | Production SQLite route | Shadow action | Result authority |
| --- | --- | --- | --- |
| `report`, `invoice` | FTS/BM25 only when earlier tiers did not fill the page | Eligible only for a first page of at least 80 rows, entirely ASCII letters, and a current view with at most 100,000 indexed entries; candidate IDs are compared with SQLite | SQLite |
| `pdf` | Exact extension when that tier returns rows | Skip | SQLite |
| `Month-03`, path punctuation | Punctuation-prefix tier | Skip | SQLite |
| `jp` | Extension-prefix / SQLite route (two-character query) | Skip: below Candidate B's three-character minimum | SQLite |
| Chinese, accented Unicode | Existing FTS route when applicable | Skip: outside ASCII candidate contract | SQLite |
| Zero-hit, sparse page, offset page, or limit below 80 | Existing indexed route | Skip before the candidate scan, using the production route and returned row count | SQLite |
| Duplicate names with rows from an earlier tier | Existing tier precedence with stable ID de-duplication | Skip because an earlier tier contributed | SQLite |
| Corpus above 100,000 entries, including 500k and 1M | Existing indexed route | Skip before candidate traversal | SQLite |

The route uses the actual deepest production tier and whether an earlier tier contributed; it does not guess from query spelling. It does not scan Candidate B first to classify a zero or sparse query. The independent SQL search tests preserve exact IDs and ordering for the synthetic cases, including duplicates and the 4,096 bound. The candidate oracle remains diagnostic only. No trigram or other second in-memory retrieval index was added: narrowing substring membership cannot preserve BM25 top-K order or remove SQLite's scoring work, and the extra memory/update cost would not authorize visible routing. The shadow probe is bounded to 100k entries; higher-volume and ambiguous classes stay on SQLite before candidate traversal. This is a safe routing boundary, not a fix for the separately measured SQLite latency misses at 500k/1M. A future retrieval index needs a proven BM25-preserving design and native resource evidence before it can help production.

### Default write path and worker recovery

Ordinary `Database::open()` now removes only this feature's stale write triggers and does not install the optional sidecar triggers. `ZEN_CANVAS_GLOBAL_SEARCH_SHADOW=1` explicitly installs the feed and starts the shadow worker; a normal reopen disables those triggers but retains the disposable sidecar tables. The test-only A/B/C fixture entry point selects feed and worker state without changing normal migration behavior. SQLite remains the source of truth, and the main schema migration version is unchanged.

The worker applies bounded batches and retries transient `UpdateBudget` and SQLite errors after 100 ms exponential backoff capped at 5 seconds, up to five retries. Memory-budget refusal, feed/source invalidation, and exhausted retries disable the worker; SQLite continues returning the complete page. The explicit `get_global_search_view_diagnostics` command returns feed/view generations and lag, saturation/fallback reason, memory accountant used/peak, refresh/build/update counts, comparison equality, route skips, retry/exhaustion, memory refusal, and database errors. It reports aggregates only and is queried on demand, so search requests do not pay diagnostic SQL work.

The write benchmark measures paired A (no feed), B (feed/worker off), and C (feed/worker on), with D-style high-frequency rename/move/delete/restore transactions, at 100k, 500k, and 1M. It verifies trigger/sidecar presence, records seed and transaction timings, update p50/p95/p99, feed saturation, worker catch-up, WAL growth after a clean checkpoint with auto-checkpoint disabled, process I/O counters, CPU, and RSS. WAL growth is SQLite logical log bytes, not physical-device bytes. Process I/O counters are process-wide, not attributable to this database.

Default-off no longer adds the feature's trigger work. Feed-enabled write overhead is an explicit Stage A cost, not a reason to enable the worker. A dedicated 30-minute opt-in worker idle benchmark records generation, memory-accountant, RSS, and process CPU stability on Windows and Apple Silicon macOS; application private bytes, low-memory OS pressure, volume/source lifecycle write cost, and production performance still require separate qualification. Stage B stays NO-GO until semantic, latency, and resource gates pass.

## Scope and safety

- The original candidate qualification was based on master `df940915084b810101a8a3eb78796cdf8730d55b` (tree `fa65a8fd1fe6f45a774024f575010c1928292ae4`). This PR includes master `53efe62e75c291e365058365426ac788b93688d8` for the latest Global Index authorization schema and was later synced through master `a0716c0adfe0677dcb6e554d835f63c30a3a88db`; the current follow-up also merges master `0626e2f6`.
- #495 remains a separate research Draft PR. Its static view did not provide a durable generation, bounded incremental feed, or production resource proof. This branch does not cherry-pick its branch or dependency changes.
- The main schema migration chain remains untouched. The optional feed is additive and self-versioned, so it does not claim or change `PRAGMA user_version`; this avoids collision with the independent #489 authorization migration. When the optional `scope_authorization_state` column exists, an invalidation trigger is installed and validated.
- SQLite remains the durable source of file and source facts. Installation does not scan or copy indexed rows. A bounded feed records up to 100,000 events; saturation invalidates incremental use until a full snapshot safely resets the feed.
- `ZEN_CANVAS_GLOBAL_SEARCH_SHADOW=1` is required to start the worker. The worker builds an immutable compact view, applies at most 5,000 generations per batch with a 50 ms work target, and refuses publication on gaps, source changes, budget failure, or stale generations. The SQLite route still produces every visible result. Exact-name, name-prefix, exact-extension, extension-prefix, and BM25 SQL remain unchanged.
- Historical state before this follow-up installed sidecar triggers on ordinary opens. The current follow-up disables owned write triggers on ordinary opens and installs them only for explicit shadow runs. Prior Windows/macOS 1k smoke runs were not a no-feed comparison; local paired write-cost evidence is below and native paired runs are pending.
- No test accesses a user disk or indexes a real volume. Scale fixtures write synthetic rows through the real schema and FTS/change triggers in a temporary database.

## Correctness evidence

Final local follow-up validation: 15 search-view tests passed, 7 change-feed tests passed, and the explicit trigger lifecycle test passed 1/1. The three ignored search-view cases are manual scale, paired-write, and 30-minute idle qualifications. The idle harness passed a 1-second smoke with stable generation, accounted memory, build count, and retries; the native 30-minute run is pending. `cargo fmt -- --check`, `npm run test:governance`, and the `npm run dev:preflight -- --plan` selection passed. `cargo clippy --all-targets -- -D warnings` passed after platform-gating the Windows/macOS-only startup-dialog helpers and correcting Linux-only unused local names. The current Windows/macOS CI and native qualification are pending after the follow-up push.

### Evidence source identity

- The original native matrix, PR CI, and ingestion smoke ran on `85775b9b42a72276ac94c2caba040c535135a082` (tree `f3e841c620204fb9ae4b385fb6e1556e6d8be610`), based on master `df940915084b810101a8a3eb78796cdf8730d55b`.
- The schema-compatible view and feed tests are at `ad8dc00096e2826e517b456f6a054be81dd6aad4` (tree `6dc9b73e0cd926481370bef3abd63fe1e1c170b8`). The PR incorporates master through `a0716c0adfe0677dcb6e554d835f63c30a3a88db` in merge commit `be88572b59e865bcc61a5f343bc641dd43ed7834` (tree `e9ffd63bdf5913432ed28097721ec14275636b1a`); the merged base changes do not alter the search implementation or native matrix source.
- The schema-44 legacy fixture fix is `bdad78555d1f35b334a9d1f2a2d66c922c2c61d4` (tree `00b4c5160764de654f5cc01f226eb98127c06880`). The current PR code tip is `2e6e56b75edc6526f2f77713155333fa8e5d0f4b` (tree `9f1981bfb67c280ada245a928567a684b729d119`); it ports the Windows watcher test reliability fix from `d377886c38e79a75f952f998782b87d034d931a2` and uses the Clippy-clean wake-count comparison.
- The latest native matrix run [40822120096](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096) passed all six Windows/macOS 100k/500k/1M jobs on `228fd1d0573e1b339c6d01391ea37c9604bdfca6` (tree `1cda86ad3d232c623b13a62f07e03548cfea992d`). The PR head adds only test-fixture corrections and documentation after that matrix source; search-view, benchmark, and production code are unchanged.

An earlier full Linux `cargo test --lib -- --test-threads=1` run, before the final matcher normalization and worker refresh test, reported **1,106 passed, 54 failed, 36 ignored**. Failures were in AI/provider and native file-workspace/managed-worker suites outside the changed search modules; this is not final-head regression evidence. PR CI run **40832014785** passed on the master-synced source at `35971d3f9a0fadd2d8fd9dd43dac9cba48b1561f`, including both-platform Rust quality and the Windows watcher durable event regression. Paired ingestion run **40832014818** passed Windows/macOS 1k-row smoke fixtures; the configured large paired A/B/C/D experiment was skipped for pull requests, so the feed write-cost gate remains open.

## Linux Release performance evidence

Machine: Linux x86_64 container, Rust 1.97.1, Cargo Release test profile. Fixture: 100,000 synthetic entries in the application's schema with FTS and change-feed triggers. Each query used 5 warmups and 30 timed samples. p95 and p99 are nearest-rank percentiles. Measurements separate candidate view only, the existing FTS-only reference tier, the full production SQLite route, and the backend snapshot endpoint.

| Query | B view p50 / p95 / p99 ms | FTS-only p50 / p95 / p99 ms | Full SQLite p50 / p95 / p99 ms | Snapshot endpoint p50 / p95 / p99 ms | 4,096-window equality (membership / order / first page) |
| --- | ---: | ---: | ---: | ---: | --- |
| `report` (5,000 matches) | 10.3 / 15.8 / 17.6 | 17.1 / 18.2 / 18.5 | 13.9 / 15.2 / 15.3 | 11.6 / 12.2 / 13.0 | yes / yes / yes |
| `invoice` (5,000 matches) | 11.4 / 16.9 / 30.0 | 21.6 / 25.0 / 25.4 | 17.9 / 20.0 / 23.1 | 16.0 / 23.1 / 31.2 | yes / yes / yes |
| `pdf` (20,625 matches) | 10.7 / 11.3 / 11.3 | 45.0 / 63.5 / 65.0 | 0.61 / 0.98 / 1.05 | 0.95 / 1.63 / 2.42 | no / no / yes |
| `Month-03` (100,000 matches) | 14.9 / 20.9 / 24.2 | 226.1 / 283.2 / 315.6 | 0.45 / 0.61 / 0.64 | 0.67 / 0.87 / 0.92 | no / no / no |
| zero-hit query | 16.2 / 19.5 / 20.3 | 0.068 / 0.217 / 0.237 | 0.60 / 0.89 / 0.91 | 0.82 / 1.14 / 1.31 | yes / yes / yes |

`jp`, Chinese, and accented Unicode queries fall back to SQLite; they are outside the current shadow candidate's 3–64 byte ASCII query contract. Their full SQLite p95 values were 21.8 ms (`jp`), 0.77 ms (Chinese), and 5.49 ms (`résumé`); snapshot endpoint p95 values were 22.6 ms, 1.03 ms, and 5.25 ms.

Every measured Release p95 remained below the existing 100 ms threshold. The decisive failure is correctness: `pdf` and `Month-03` candidate windows omitted IDs present in SQLite FTS/BM25's 4,096 window; the `Month-03` first page also differed. The `pdf` first page happened to match because the unchanged exact-extension tier wins, but later pages are not safe to route through B. The FTS-only figures also show why the full route must remain intact: exact extension and punctuation-prefix tiers avoid the expensive FTS path.

One 512-event incremental update batch applied in 2.59 ms. The in-code memory accountant reported 12,984,384 bytes active and 13,080,371 bytes peak. Building a second full view while the old one was retained raised accounted use to 26,186,513 bytes; after dropping the old view it returned to 12,984,384 bytes, then to zero after dropping both views. View construction took 185 ms.

The process sampler observed 159.3 MB peak RSS during view construction (132.7 MB above pre-build RSS), 151.7 MB peak during the 30-second idle interval, and 274.7 MB during the later query/update workload. These process figures include SQLite page residency, FTS pages, and the test process, so they cannot be attributed to B alone. The idle average was 1.34% with a noisy 56% sampled peak; incremental CPU had too few samples to report. Native application RSS, worker CPU, and long-duration stability remain unqualified.

## Linux Release paired Change Feed write-cost evidence

Machine: Linux x86_64 container, Rust 1.97.1, Release test profile. One 100k synthetic fixture was seeded independently for each A/B/C scenario in the same benchmark process and database schema. Before measuring, the benchmark disabled WAL auto-checkpointing, truncated/checkpointed the seed WAL, and verified a non-busy checkpoint with zero pending frames. Each update group used 5 warmups and 30 measured transactions. WAL figures are append-only logical SQLite WAL bytes for that group. The process I/O counters reported zero throughout this run despite measured WAL growth, so they are unavailable evidence here; this report does not interpret zero counters as zero writes.

| Scenario | Feed / worker verified | Seed seconds | Single update p50/p95/p99 ms | 512 updates p50/p95/p99 ms | 5,000 updates p50/p95/p99 ms | Peak process RSS MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| A, no feed | 0 triggers, no sidecar | 8.34 | 0.08 / 0.22 / 0.24 | 30.34 / 38.31 / 45.78 | 305.91 / 523.82 / 607.80 | 123.0 |
| B, feed, worker off | 7 triggers, sidecar present | 9.04 | 0.14 / 0.26 / 0.29 | 33.66 / 39.96 / 50.90 | 353.17 / 566.22 / 656.13 | 129.1 |
| C, feed and worker on | 7 triggers, sidecar present | 9.14 | 0.22 / 0.34 / 0.37 | 33.59 / 39.57 / 53.05 | 352.04 / 547.41 / 652.62 | 165.7 |

| Scenario | WAL growth for 1 / 512 / 5,000 updates MiB | D-style mixed transaction p50/p95/p99 ms | D WAL growth MiB | D wall time ms |
| --- | ---: | ---: | ---: | ---: |
| A, no feed | 1.90 / 35.06 / 300.40 | 2.97 / 3.98 / 12.65 | 22.35 | 346 |
| B, feed, worker off | 2.48 / 38.65 / 314.92 | 3.52 / 4.26 / 13.51 | 26.16 | 405 |
| C, feed and worker on | 3.00 / 38.47 / 321.90 | 3.80 / 4.74 / 14.32 | 26.31 | 437 |

Against A, B's write p95 was 20.8%, 4.3%, and 8.1% higher for one, 512, and 5,000 updates; C was 58.3%, 3.3%, and 4.5% higher. The one-row p95 changes were 0.05 ms for B and 0.13 ms for C, and are sensitive to scheduling noise. WAL growth was 30.6%, 10.2%, and 4.8% higher for B, and 58.0%, 9.7%, and 7.2% higher for C. The B 5,000-update group saturated its bounded feed; C did not. C's worker completed 1 full build and 205 incremental batches, ended at generation lag 0, used 15,581,432 accounted bytes (18,822,034 peak), and recorded no retries or errors. The D-style workload ran 100 transactions of 50 operations, alternating rename, move, delete, and restore patterns. Process RSS includes the database and test fixture, so it is not attributed to the worker. These Linux measurements are a single-run screening result; native repeated A/B/C/D evidence follows in the Windows/macOS workflow. Raw logs: [100k paired writes](evidence/linux-x86_64-release-write-cost-100k.log).

## Linux Release 500k resource and query preflight

The 500k synthetic fixture completed the Release qualification on the same Linux container before the gated native 1M jobs. The bounded view used 57,155,648 bytes active (54.5 MiB); building a replacement while the old view remained retained peaked at 114,541,321 bytes (109.2 MiB), then returned to 57,155,648 bytes and reached zero after both views were dropped. View build took 658 ms and the 512-event delta applied in 1.82 ms. The code accountant stayed below the 160 MiB active-view and 256 MiB total limits.

| Query | B view p95 ms | FTS-only p95 ms | Full SQLite p95 ms | Snapshot p95 ms | Membership / order / first-page check |
| --- | ---: | ---: | ---: | ---: | --- |
| `report` (25k matches) | 51.10 | 88.68 | 75.99 | 74.98 | yes / yes / yes |
| `invoice` (25k matches) | 61.17 | 95.62 | 88.40 | 78.84 | yes / yes / yes |
| `pdf` (103,125 matches) | 51.30 | 198.86 | 0.61 | 1.25 | no / no / yes; exact-extension production tier wins |
| zero-hit ASCII query | 101.95 | 0.09 | 0.63 | 0.75 | yes / yes / yes |

The zero-hit probe exceeded 100 ms but is skipped by the current router before Candidate B traversal; the production SQLite route returned in 0.63 ms. `Month-03` used the punctuation-prefix route and returned in 0.43 ms; two-character `jp`, Chinese, and accented queries remained SQLite-only. The 100k shadow-probe cap keeps all these 500k Candidate B measurements diagnostic and prevents production queries from traversing the 500k view. Whole-process RSS was 680 MiB after the build and peaked at 1,218 MiB during the later update workload; those values include the SQLite database, FTS cache, and fixture and are not attributable to B. The 30-second no-worker idle sample averaged 1.12% process CPU with a noisy 30.2% sampled peak. This preflight clears the view-accounting gate for the staged matrix but does not certify private RSS, OS low-memory behavior, or production routing. Raw log: [500k view preflight](evidence/linux-x86_64-release-view-preflight-500k.log).

## Prior Windows and macOS Release qualification

The prior [native qualification workflow](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096) passed all six Windows/macOS runs at 100k, 500k, and 1M rows. Each fixture used the application schema, FTS/change triggers, synthetic rows, 5 warmups, and 30 measured samples. It records B-only, FTS-only, full SQLite search, and snapshot endpoint p50/p95/p99 for `report`, `invoice`, `pdf`, `jp`, `Month-03`, zero-hit, Chinese, and accented queries. This is prior-head evidence. The follow-up workflow runs 100k/500k preflight jobs first, gates the 1M Release jobs on those results, and adds paired A/B/C writes plus a 30-minute worker-idle run; those jobs are pending.

| Runner / rows | B `report` p50/p95/p99 ms | B `invoice` p50/p95/p99 ms | FTS-only `report` / `invoice` p95 ms | Full SQLite `report` / `invoice` / `jp` p95 ms | Snapshot `report` / `invoice` / `jp` p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Windows / 100k | 7.17 / 7.39 / 7.40 | 7.51 / 7.68 / 7.72 | 29.2 / 29.9 | 26.5 / 26.6 / 23.4 | 12.1 / 11.6 / 8.7 |
| macOS / 100k | 11.26 / 19.32 / 19.50 | 12.80 / 16.67 / 17.29 | 20.7 / 26.3 | 21.3 / 18.6 / 18.4 | 18.1 / 19.7 / 11.3 |
| Windows / 500k | 49.08 / 51.19 / 56.49 | 50.42 / 52.81 / 56.41 | 199.4 / 196.3 | 178.4 / 181.3 / 146.3 | 82.7 / 89.3 / 68.1 |
| macOS / 500k | 44.64 / 47.53 / 48.48 | 49.13 / 51.25 / 52.14 | 55.1 / 83.6 | 48.5 / 51.2 / 53.2 | 44.7 / 49.4 / 48.9 |
| Windows / 1M | 97.66 / 110.18 / 112.60 | 100.60 / 108.46 / 110.53 | 385.2 / 388.6 | 362.6 / 356.6 / 288.7 | 153.2 / 176.4 / 139.6 |
| macOS / 1M | 90.13 / 114.58 / 115.16 | 98.69 / 104.18 / 106.39 | 107.9 / 122.9 | 108.2 / 192.9 / 100.2 | 116.8 / 126.0 / 94.4 |

`report` and `invoice` had matching FTS-window membership and order at each size. `pdf` membership/order differed at all six sizes, although its first production page matched because the unchanged exact-extension tier wins. `Month-03` membership/order and first page differed at all six sizes. Zero-hit membership was equal, but B-only p95 rose to 185.3 ms on macOS and 202.4 ms on Windows at 1M; it also exceeded 100 ms on Windows at 500k. At 1M, B-only `report` and `invoice` p95 exceeded 100 ms on both platforms. The full SQLite route exceeded 100 ms for `report`, `invoice`, and `jp` on Windows at 500k and 1M; on macOS 1M it exceeded 100 ms for `report`, `invoice`, and `jp` (the latter by less than 1 ms). The snapshot endpoint exceeded 100 ms for `report` and `invoice` on Windows 1M and macOS 1M.

The resource accountant reported the following view and process measurements. “Two views” includes a replacement full view while the old view remains retained. Process RSS includes SQLite/FTS page residency and the test process and is not attributable to B alone; the RSS sampler can miss short peaks (macOS 100k post-build RSS exceeded its sampled build peak). Incremental CPU had one sample at every size and is not a useful peak measurement.

| Runner / rows | Accounted B active / two views MiB | Build ms | RSS after build / sampled build peak / sampled update peak MiB | Idle CPU average | 512-event apply ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Windows / 100k | 12.38 / 24.97 | 179 | 27.6 / 33.4 / 59.6 | 1.98% | 2.62 |
| macOS / 100k | 12.38 / 24.97 | 97 | 138.3 / 22.6 / 245.6 | 0.71% | 2.26 |
| Windows / 500k | 54.51 / 109.22 | 2,048 | 71.7 / 112.7 / 218.0 | 1.66% | 3.21 |
| macOS / 500k | 54.51 / 109.22 | 494 | 585.1 / 563.7 / 1,019.1 | 0.70% | 2.19 |
| Windows / 1M | 108.88 / 217.97 | 6,308 | 124.7 / 214.4 / 313.9 | 1.93% | 3.15 |
| macOS / 1M | 108.88 / 217.97 | 1,635 | 1,112.6 / 1,109.2 / 1,985.2 | 0.78% | 2.09 |

The 1M two-view accountant total of 217.97 MiB is below the 256 MiB limit, leaving about 38 MiB headroom; it is a code-level accounting result, not private-RSS attribution. At 1M, the standalone macOS process reached about 1.09 GiB RSS after build and a sampled 1.94 GiB during incremental update, while Windows was about 125 MiB after build and 314 MiB during update. Long-idle samples lasted 30 seconds. These standalone measurements do not establish application startup RSS, private bytes, long-duration stability, or worker CPU.

Raw Debug and Release logs, including all p50/p95/p99 query results and correctness comparisons, are attached to each latest matrix job: [Windows 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12875682172), [macOS 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12875452278), [Windows 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12875234258), [macOS 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12876010827), [Windows 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12876253497), and [macOS 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12875852830). The original matrix at [40803125504](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803125504) remains available as historical comparison.

The full [PR CI run](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803153773) passed after the macOS Rust job rerun. The automatic [Windows/macOS ingestion smoke run](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803153821) also passed; it is a 1k-row, single-repeat fixture and does not isolate marginal Change Feed trigger cost.

## Remaining qualification gates

- Resolve FTS/BM25 membership, order, and pagination mismatches for `pdf` and `Month-03` before enabling Stage B.
- Preserve the 100 ms p95 gate. The existing full SQLite route missed it for high-hit Windows 500k and both 1M native fixtures. Historical full-scan B-only p95 also missed it at 1M and for Windows 500k zero-hit; the current router skips B above 100k. Improve the SQLite production baseline and any future candidate path without degrading exact-name, prefix, or extension tiers.
- Complete the gated native 100k/500k/1M paired Change Feed write measurements and report feed cost, saturation, and worker catch-up on Windows and macOS.
- Complete the native 30-minute idle run, then capture application startup RSS/private bytes, worker CPU attribution, OS low-memory behavior, and volume/source lifecycle write cost. Current process RSS includes SQLite/FTS page residency and is not a B-only attribution.
- Keep the default SQLite route and opt-in shadow-only worker until correctness, resource attribution, and latency gates pass. Do not enable B for user-visible routing.

## Enable and rollback

The default remains SQLite-only. To collect opt-in Stage A evidence in an isolated environment, set `ZEN_CANVAS_GLOBAL_SEARCH_SHADOW=1`; the worker still never supplies visible results. To roll back, unset the variable and retain SQLite as the route. The sidecar is disposable derived state; removing its tables/triggers requires a future explicit migration and is not part of this candidate.
