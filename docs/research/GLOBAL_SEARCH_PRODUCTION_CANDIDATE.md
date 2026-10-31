# Global Search production candidate: Stage A qualification

## Decision

**`STAGE_B_NO_GO`.** SQLite remains the only user-visible search route; the shadow worker is opt-in and cannot supply results. The exact c204590 source head passed PR CI, ingestion smoke, and the staged Windows/macOS 100k/500k/1M search and write matrix. Both platform 30-minute idle runs also passed. The evidence still blocks Stage B: the `pdf` Candidate B window differs from FTS/BM25 membership and order at every tested size, the historical `Month-03` comparison also differs (the current router skips punctuation queries), and the full SQLite route exceeds the unchanged 100 ms p95 gate for high-hit queries on Windows at 500k and both platforms at 1M.

## Current remediation snapshot — 2026-10-29

This section supersedes older implementation/status statements below where they conflict. All code qualification in this report used PR source head `c20459063b4956bd3ee696cbd9920640b26fba1c` (tree `0b4e2545fd70abbd3cd5aa3b0b8e14a78ccfcdd8`), including master `0626e2f6`. The later report/evidence commit changes documentation only and did not rerun the 1M source qualification.

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

Default-off no longer adds the feature's trigger work. Feed-enabled write overhead is an explicit Stage A cost, not a reason to enable the worker. The 30-minute opt-in idle test passed on Linux, Windows, and Apple Silicon macOS with stable generation, memory-accountant use, and retries. Application private bytes, low-memory OS pressure, real volume/source lifecycle write cost, and production performance still require separate qualification. Stage B stays NO-GO until semantic, latency, and resource gates pass.

## Exact-head CI and native qualification

Tested source: c20459063b4956bd3ee696cbd9920640b26fba1c, tree 0b4e2545fd70abbd3cd5aa3b0b8e14a78ccfcdd8. Exact-head [PR CI](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865803635) and [Windows/macOS ingestion smoke](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865803647) passed. The [staged native run](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219) passed its 100k/500k preflights, gated 1M jobs, and 30-minute Windows/macOS idle jobs. The final report/evidence commit changes docs only; it did not retrigger native 1M jobs.

### Search view latency and correctness

Each platform/size search qualification passed in Debug and Release at 100k/500k, and Release at 1M. Candidate p95 is the diagnostic view-only comparison; the full SQLite p95 is the existing production search route.

| Runner / rows | Candidate report / invoice p95 ms | Full SQLite report / invoice / jp p95 ms | pdf FTS-window membership / order |
| --- | ---: | ---: | --- |
| Windows / 100k | 9.88 / 10.25 | 37.15 / 37.42 / 27.95 | no / no |
| Apple Silicon macOS / 100k | 9.24 / 10.02 | 10.29 / 8.70 / 45.56 | no / no |
| Windows / 500k | 53.67 / 54.37 | 161.02 / 159.27 / 141.84 | no / no |
| Apple Silicon macOS / 500k | 44.59 / 54.62 | 56.01 / 57.66 / 53.15 | no / no |
| Windows / 1M | 56.93 / 62.41 | 213.05 / 228.01 / 180.28 | no / no |
| Apple Silicon macOS / 1M | 120.28 / 175.47 | 152.00 / 280.36 / 138.18 | no / no |

The report/invoice candidate windows matched SQLite FTS membership and order in these runs. The pdf candidate window differed in membership and order at all six platform/size combinations; the production query remains fast because the existing exact-extension tier wins and the shadow observer skips it. The current router also skips punctuation queries such as Month-03 before Candidate B. The historical direct comparison found a Month-03 candidate-window and first-page mismatch. Candidate probing is capped at 100k entries, so the 500k/1M candidate-only values are diagnostic and are not used by production routing.

At 500k, the Windows full SQLite report/invoice/jp p95 values exceeded 100 ms; at 1M, all three exceeded 100 ms on both platforms. The 100 ms p95 gate is unchanged. At 1M the macOS candidate-only report/invoice p95 also exceeded 100 ms, and the Windows zero-hit candidate p95 was 124.97 ms; the router skips corpora over 100k and skips empty/sparse pages before candidate traversal.

The memory accountant reported 12.38 MiB active at 100k, 54.51 MiB at 500k, and 108.88 MiB at 1M. Those values stayed below the 160 MiB active-view and 256 MiB total limits. Whole-process RSS includes the SQLite database, FTS cache, fixture, and test process: at 1M it was 132 MiB after the Windows build and peaked at 437 MiB during update; macOS was 1.16 GiB after build and peaked at 2.04 GiB during update. These process values are not attributable to Candidate B alone and do not replace application private-memory or low-memory OS qualification.

### Paired Change Feed write cost

Each row reports p95 milliseconds for 1 / 512 / 5,000 updates, measured with 5 warmups and 30 samples. A has no feed; B installs the feed with the worker off; C installs the feed and runs the worker. D-style mixed transactions contain rename, move, delete, and restore patterns.

| Runner / rows | A p95 ms | B p95 ms | C p95 ms | D mixed p95 A / B / C ms | C final lag; builds / incremental batches; accounted active / peak MB |
| --- | ---: | ---: | ---: | ---: | --- |
| Windows / 100k | 0.40 / 54.81 / 723.37 | 0.52 / 54.08 / 777.04 | 0.66 / 91.38 / 938.57 | 5.17 / 8.23 / 11.43 | 0; 1 / 208; 15.58 / 18.82 |
| macOS / 100k | 0.22 / 57.38 / 548.50 | 0.36 / 53.73 / 617.07 | 1.26 / 50.20 / 592.64 | 6.14 / 5.43 / 5.34 | 0; 1 / 205; 15.58 / 18.82 |
| Windows / 500k | 0.41 / 45.34 / 537.94 | 0.49 / 92.44 / 662.49 | 0.51 / 76.08 / 575.17 | 5.59 / 6.84 / 6.48 | 0; 1 / 205; 59.75 / 62.99 |
| macOS / 500k | 0.28 / 81.93 / 445.00 | 0.30 / 147.61 / 561.05 | 1.14 / 45.34 / 353.98 | 6.66 / 6.30 / 5.09 | 0; 1 / 205; 59.75 / 62.99 |
| Windows / 1M | 0.30 / 28.08 / 954.11 | 0.25 / 25.92 / 947.56 | 0.27 / 30.26 / 834.18 | 2.85 / 3.34 / 3.23 | 0; 1 / 205; 116.77 / 120.01 |
| macOS / 1M | 0.22 / 32.24 / 1,811.82 | 0.32 / 52.68 / 1,767.42 | 1.21 / 46.24 / 1,375.26 | 5.79 / 4.53 / 7.20 | 0; 1 / 205; 116.77 / 120.01 |

B saturated its bounded feed during the 5,000-update group at every size and platform. C did not saturate; all C runs ended at lag 0 with one full build, 205 or 208 incremental batches, and no retries or database errors. C's 5,000-update catch-up p95 was 31.29 ms on Windows and 104.59 ms on macOS at 1M. The 5,000-update WAL growth, which was the same on both platforms for a given fixture size, was A/B/C 300.4 / 314.9 / 321.9 MiB at 100k, 356.0 / 369.5 / 375.1 MiB at 500k, and 721.3 / 734.1 / 741.5 MiB at 1M.

The benchmark checkpoints the seed WAL and disables auto-checkpointing during measurement. WAL growth is append-only logical SQLite log bytes, not physical-device writes. Process I/O counters and RSS are process-wide and include the fixture; they are not database- or worker-attributed. The 5,000-update groups are synthetic bulk updates; the D-style group does not qualify real volume disable, source revoke, or restore lifecycle cost.

### Thirty-minute worker idle stability

All three runs used 100k synthetic entries and an opt-in worker for 1,800 idle seconds. Feed/view generation, lag, build count, incremental count, accounted memory use, and retry counts stayed unchanged; every test passed.

| Platform | Average / peak sampled process CPU | Accounted memory used / peak MB | RSS before → after; sampled peak MB | Startup catch-up ms | Process I/O counters |
| --- | ---: | ---: | ---: | ---: | --- |
| Linux x86_64 | 1.08% / 23.60% | 12.98 / 13.08 | 169.32 → 46.09; 169.41 | 141.47 | unavailable |
| Windows | 1.53% / 30.97% | 12.98 / 13.08 | 85.09 → 16.86; 85.33 | 235.28 | nonzero |
| Apple Silicon macOS | 0.60% / 35.23% | 12.98 / 13.08 | 160.73 → 56.71; 160.86 | 134.69 | nonzero |

The CPU and RSS figures describe the whole benchmark process, not worker-only attribution. The Linux source was equivalent to c204590; the intervening code correction is macOS-only. Its raw log is [here](evidence/linux-x86_64-release-worker-idle-30m.log).

### Native artifact links

- Search view: [Windows 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12894931200), [macOS 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12894738262), [Windows 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12895166494), [macOS 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12895810193), [Windows 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12895202730), [macOS 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12896041266).
- Paired writes: [Windows 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12894431265), [macOS 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12895041958), [Windows 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12894621607), [macOS 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12894384455), [Windows 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12896246592), [macOS 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12895159976).
- Worker idle: [Windows 30m](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12896002854), [macOS 30m](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40865795219/artifacts/12897016421).

## Scope and safety

- The original candidate qualification was based on master `df940915084b810101a8a3eb78796cdf8730d55b` (tree `fa65a8fd1fe6f45a774024f575010c1928292ae4`). This PR includes master `53efe62e75c291e365058365426ac788b93688d8` for the latest Global Index authorization schema and was later synced through master `a0716c0adfe0677dcb6e554d835f63c30a3a88db`; the current follow-up also merges master `0626e2f6`.
- #495 remains a separate research Draft PR. Its static view did not provide a durable generation, bounded incremental feed, or production resource proof. This branch does not cherry-pick its branch or dependency changes.
- The main schema migration chain remains untouched. The optional feed is additive and self-versioned, so it does not claim or change `PRAGMA user_version`; this avoids collision with the independent #489 authorization migration. When the optional `scope_authorization_state` column exists, an invalidation trigger is installed and validated.
- SQLite remains the durable source of file and source facts. Installation does not scan or copy indexed rows. A bounded feed records up to 100,000 events; saturation invalidates incremental use until a full snapshot safely resets the feed.
- `ZEN_CANVAS_GLOBAL_SEARCH_SHADOW=1` is required to start the worker. The worker builds an immutable compact view, applies at most 5,000 generations per batch with a 50 ms work target, and refuses publication on gaps, source changes, budget failure, or stale generations. The SQLite route still produces every visible result. Exact-name, name-prefix, exact-extension, extension-prefix, and BM25 SQL remain unchanged.
- Historical state before this follow-up installed sidecar triggers on ordinary opens. The current follow-up disables owned write triggers on ordinary opens and installs them only for explicit shadow runs. Prior Windows/macOS 1k smoke runs were not a no-feed comparison; the completed native paired A/B/C write-cost matrix below is the marginal-cost evidence.
- No test accesses a user disk or indexes a real volume. Scale fixtures write synthetic rows through the real schema and FTS/change triggers in a temporary database.

## Correctness evidence

Targeted local validation reported 15 search-view tests passed (3 ignored manual benchmarks), 7 change-feed tests passed, and the ordinary-open trigger lifecycle test passed 1/1. A 1-second idle smoke also passed. These Linux tests used source-equivalent code before the macOS-only binding correction; exact-head CI then passed on c204590, including Windows/macOS Rust quality, Release compilation, and performance groups. `cargo fmt -- --check`, `npm run test:governance`, and the `npm run dev:preflight -- --plan` selection passed. Exact-head Windows/macOS ingestion smoke passed. The complete native matrix and long-idle results are recorded below.

### Evidence source identity

- The original native matrix, PR CI, and ingestion smoke ran on `85775b9b42a72276ac94c2caba040c535135a082` (tree `f3e841c620204fb9ae4b385fb6e1556e6d8be610`), based on master `df940915084b810101a8a3eb78796cdf8730d55b`.
- The schema-compatible view and feed tests are at `ad8dc00096e2826e517b456f6a054be81dd6aad4` (tree `6dc9b73e0cd926481370bef3abd63fe1e1c170b8`). The PR incorporates master through `a0716c0adfe0677dcb6e554d835f63c30a3a88db` in merge commit `be88572b59e865bcc61a5f343bc641dd43ed7834` (tree `e9ffd63bdf5913432ed28097721ec14275636b1a`); the merged base changes do not alter the search implementation or native matrix source.
- The schema-44 fixture and Windows watcher fixes listed above belong to the historical qualification chain. Current code qualification is c20459063b4956bd3ee696cbd9920640b26fba1c, described in the exact-head section above.
- A prior native matrix run [40822120096](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096) passed its six Windows/macOS 100k/500k/1M jobs on `228fd1d0573e1b339c6d01391ea37c9604bdfca6` (tree `1cda86ad3d232c623b13a62f07e03548cfea992d`). It is historical baseline evidence; the staged c204590 results above supersede it for this follow-up.

An earlier full Linux `cargo test --lib -- --test-threads=1` run, before the final matcher normalization and worker refresh test, reported **1,106 passed, 54 failed, 36 ignored**. Failures were in AI/provider and native file-workspace/managed-worker suites outside the changed search modules; this is not final-head regression evidence. PR CI run **40832014785** passed on the master-synced source at `35971d3f9a0fadd2d8fd9dd43dac9cba48b1561f`, including both-platform Rust quality and the Windows watcher durable event regression. Paired ingestion run **40832014818** passed Windows/macOS 1k-row smoke fixtures; its large paired A/B/C/D experiment was skipped for pull requests. The exact-head native paired write-cost matrix is reported above.

## Linux Release performance evidence

The Linux Release query, resource, and write logs were produced from source equivalent to c204590. The intervening code correction is macOS-only; the native exact-head results above are the current Windows/macOS qualification.

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

The process sampler observed 159.3 MB peak RSS during view construction (132.7 MB above pre-build RSS), 151.7 MB peak during the 30-second idle interval, and 274.7 MB during the later query/update workload. These process figures include SQLite page residency, FTS pages, and the test process, so they cannot be attributed to B alone. The idle average was 1.34% with a noisy 56% sampled peak; incremental CPU had too few samples to report. The 30-minute synthetic worker-idle stability test passed on all three platforms above; application private-memory attribution, worker-only CPU, and OS low-memory behavior remain unqualified.

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

Against A, B's write p95 was 20.8%, 4.3%, and 8.1% higher for one, 512, and 5,000 updates; C was 58.3%, 3.3%, and 4.5% higher. The one-row p95 changes were 0.05 ms for B and 0.13 ms for C, and are sensitive to scheduling noise. WAL growth was 30.6%, 10.2%, and 4.8% higher for B, and 58.0%, 9.7%, and 7.2% higher for C. The B 5,000-update group saturated its bounded feed; C did not. C's worker completed 1 full build and 205 incremental batches, ended at generation lag 0, used 15,581,432 accounted bytes (18,822,034 peak), and recorded no retries or errors. The D-style workload ran 100 transactions of 50 operations, alternating rename, move, delete, and restore patterns. Process RSS includes the database and test fixture, so it is not attributed to the worker. These Linux measurements are a single-run screening result; the completed native A/B/C evidence is summarized in the exact-head section above. Raw logs: [100k paired writes](evidence/linux-x86_64-release-write-cost-100k.log).

## Linux Release 500k resource and query preflight

The 500k synthetic fixture completed the Release qualification on the same Linux container before the gated native 1M jobs. The bounded view used 57,155,648 bytes active (54.5 MiB); building a replacement while the old view remained retained peaked at 114,541,321 bytes (109.2 MiB), then returned to 57,155,648 bytes and reached zero after both views were dropped. View build took 658 ms and the 512-event delta applied in 1.82 ms. The code accountant stayed below the 160 MiB active-view and 256 MiB total limits.

| Query | B view p95 ms | FTS-only p95 ms | Full SQLite p95 ms | Snapshot p95 ms | Membership / order / first-page check |
| --- | ---: | ---: | ---: | ---: | --- |
| `report` (25k matches) | 51.10 | 88.68 | 75.99 | 74.98 | yes / yes / yes |
| `invoice` (25k matches) | 61.17 | 95.62 | 88.40 | 78.84 | yes / yes / yes |
| `pdf` (103,125 matches) | 51.30 | 198.86 | 0.61 | 1.25 | no / no / yes; exact-extension production tier wins |
| zero-hit ASCII query | 101.95 | 0.09 | 0.63 | 0.75 | yes / yes / yes |

The zero-hit probe exceeded 100 ms but is skipped by the current router before Candidate B traversal; the production SQLite route returned in 0.63 ms. `Month-03` used the punctuation-prefix route and returned in 0.43 ms; two-character `jp`, Chinese, and accented queries remained SQLite-only. The 100k shadow-probe cap keeps all these 500k Candidate B measurements diagnostic and prevents production queries from traversing the 500k view. Whole-process RSS was 680 MiB after the build and peaked at 1,218 MiB during the later update workload; those values include the SQLite database, FTS cache, and fixture and are not attributable to B. The 30-second no-worker idle sample averaged 1.12% process CPU with a noisy 30.2% sampled peak. This preflight clears the view-accounting gate for the staged matrix but does not certify private RSS, OS low-memory behavior, or production routing. Raw log: [500k view preflight](evidence/linux-x86_64-release-view-preflight-500k.log).

## Historical Windows and macOS baseline before this remediation

The prior [native qualification workflow](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096) passed all six Windows/macOS runs at 100k, 500k, and 1M rows. Each fixture used the application schema, FTS/change triggers, synthetic rows, 5 warmups, and 30 measured samples. It records B-only, FTS-only, full SQLite search, and snapshot endpoint p50/p95/p99 for `report`, `invoice`, `pdf`, `jp`, `Month-03`, zero-hit, Chinese, and accented queries. This is prior-head evidence; the exact c204590 follow-up data is in the native qualification section above.

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

The prior 1M two-view accountant total of 217.97 MiB is below the 256 MiB limit, leaving about 38 MiB headroom; it is a code-level accounting result, not private-RSS attribution. At 1M, the prior standalone macOS process reached about 1.09 GiB RSS after build and a sampled 1.94 GiB during incremental update, while Windows was about 125 MiB after build and 314 MiB during update. Those samples lasted 30 seconds. The current 30-minute synthetic idle results are reported above; neither test establishes application startup RSS, private bytes, worker-only CPU, or low-memory behavior.

Raw Debug and Release logs, including all p50/p95/p99 query results and correctness comparisons, are attached to the prior matrix run: [Windows 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12875682172), [macOS 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12875452278), [Windows 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12875234258), [macOS 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12876010827), [Windows 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12876253497), and [macOS 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40822120096/artifacts/12875852830). The original matrix at [40803125504](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803125504) remains available as historical comparison.

The historical [PR CI run](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803153773) passed after the macOS Rust job rerun. The automatic [Windows/macOS ingestion smoke run](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803153821) also passed; it is a 1k-row, single-repeat fixture and does not isolate marginal Change Feed trigger cost. Current exact-head CI and ingestion smoke are linked in the section above.

## Remaining qualification gates

- Resolve Candidate B's FTS/BM25 membership and order mismatch for `pdf`, and preserve pagination and the historical `Month-03` behavior, before enabling Stage B.
- Preserve the 100 ms p95 gate. The existing full SQLite route missed it for high-hit Windows 500k and both 1M native fixtures. At 1M, candidate-only p95 also exceeded the gate for macOS report/invoice and Windows zero-hit; the router skips views above 100k and skips empty/sparse pages before traversal. Improve the SQLite production baseline and any future candidate path without degrading exact-name, prefix, or extension tiers.
- Capture application startup RSS/private bytes, worker-only CPU attribution, OS low-memory behavior, and real volume/source lifecycle write cost. The current synthetic workloads and whole-process RSS do not provide those application-level resource and lifecycle results.
- Keep the default SQLite route and opt-in shadow-only worker until correctness, resource attribution, and latency gates pass. Do not enable B for user-visible routing.

## Enable and rollback

The default remains SQLite-only. To collect opt-in Stage A evidence in an isolated environment, set `ZEN_CANVAS_GLOBAL_SEARCH_SHADOW=1`; the worker still never supplies visible results. To roll back, unset the variable and retain SQLite as the route. The sidecar is disposable derived state; removing its tables/triggers requires a future explicit migration and is not part of this candidate.
