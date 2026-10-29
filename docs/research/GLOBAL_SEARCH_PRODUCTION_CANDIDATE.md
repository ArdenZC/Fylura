# Global Search production candidate: Stage A shadow qualification

## Decision

**NO-GO for Stage B or production routing.** This branch keeps the SQLite search result authoritative and leaves the shadow worker disabled by default. The Windows and Apple Silicon macOS 100k/500k/1M matrix completed, but `pdf` and `Month-03` candidate windows diverge from FTS/BM25 at the 4,096 window on every native size; `Month-03` also returns a different first page. Candidate-only p95 exceeds 100 ms for high-hit and zero-hit queries at 1M. The full existing route also exceeds the historical 100 ms p95 gate for high-hit queries on Windows at 500k and both platforms at 1M. These correctness and latency failures block Stage B.

## Scope and safety

- The original candidate qualification was based on master `df940915084b810101a8a3eb78796cdf8730d55b` (tree `fa65a8fd1fe6f45a774024f575010c1928292ae4`). This PR now also includes master `53efe62e75c291e365058365426ac788b93688d8` (tree `8224f300bf790fd2b6c3f439c77f2b7de8756aaa`) for the latest Global Index authorization schema.
- #495 remains a separate research Draft PR. Its static view did not provide a durable generation, bounded incremental feed, or production resource proof. This branch does not cherry-pick its branch or dependency changes.
- The main schema migration chain remains untouched. The optional feed is additive and self-versioned, so it does not claim or change `PRAGMA user_version`; this avoids collision with the independent #489 authorization migration. When the optional `scope_authorization_state` column exists, an invalidation trigger is installed and validated.
- SQLite remains the durable source of file and source facts. Installation does not scan or copy indexed rows. A bounded feed records up to 100,000 events; saturation invalidates incremental use until a full snapshot safely resets the feed.
- `ZEN_CANVAS_GLOBAL_SEARCH_SHADOW=1` is required to start the worker. The worker builds an immutable compact view, applies at most 5,000 generations per batch with a 50 ms work target, and refuses publication on gaps, source changes, budget failure, or stale generations. The SQLite route still produces every visible result. Exact-name, name-prefix, exact-extension, extension-prefix, and BM25 SQL remain unchanged.
- The sidecar tables and triggers are installed when the database opens, even while the worker is off. The PR's Windows/macOS ingestion smoke jobs passed at 1k synthetic rows, but each used one repeat and no no-feed control; marginal trigger write cost is still unqualified.
- No test accesses a user disk or indexes a real volume. Scale fixtures write synthetic rows through the real schema and FTS/change triggers in a temporary database.

## Correctness evidence

Targeted Rust view tests: **9 passed, 0 failed** (plus one ignored scale qualification). Change-feed tests: **6 passed, 0 failed**. `cargo check --lib`, formatting, governance checks, and Clippy passed for the changed modules. The repository's exact Linux `desktop-runtime --all-targets -- -D warnings` command is blocked by existing OS-gated unused-variable and dead-code warnings in `fs_safety` and `startup_upgrade`; Clippy passes when only those two Linux-only warning classes are allowed. On the original candidate source, Windows and macOS full Rust quality, native service qualification, Release builds, performance profile, and quality aggregate all passed in CI. The first macOS Rust test attempt ended with a process-level `SIGBUS` and no failed assertion; its full rerun passed, including Rust tests, Clippy, native lifecycle, and race checks. After syncing the latest master, the search-view tests passed **9/9** and change-feed tests passed **6/6** locally; current-head CI and native requalification are pending. The tests cover independent ASCII substring oracles, stable Top-K and the 4,096 window, punctuation/path/extension cases, insert/delete/rename/move/stale/source disable, actual repository writes, transaction rollback, feed saturation and gaps, the optional authorization trigger, low-memory refusal, database reopen/rebuild, worker refresh behavior, and a reader holding the prior immutable generation while an update is built.

### Evidence source identity

- The original native matrix, PR CI, and ingestion smoke ran on `85775b9b42a72276ac94c2caba040c535135a082` (tree `f3e841c620204fb9ae4b385fb6e1556e6d8be610`), based on master `df940915084b810101a8a3eb78796cdf8730d55b`.
- This PR is now synchronized with master `53efe62e75c291e365058365426ac788b93688d8`. The merged candidate code and schema-compatible tests are at `ad8dc00096e2826e517b456f6a054be81dd6aad4` (tree `6dc9b73e0cd926481370bef3abd63fe1e1c170b8`).
- The original metrics remain attributable to their recorded source. The current integration tip will receive fresh CI and scale evidence before final qualification.

An earlier full Linux `cargo test --lib -- --test-threads=1` run, before the final matcher normalization and worker refresh test, reported **1,106 passed, 54 failed, 36 ignored**. Failures were in AI/provider and native file-workspace/managed-worker suites outside the changed search modules; this is not final-head regression evidence. Final-head Windows and macOS full Rust quality both passed in the PR CI run.

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

## Windows and macOS Release qualification

The branch-scoped [native qualification workflow](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803125504) passed all six Windows/macOS runs at 100k, 500k, and 1M rows. Each fixture used the application schema, FTS/change triggers, synthetic rows, 5 warmups, and 30 measured samples. It records B-only, FTS-only, full SQLite search, and snapshot endpoint p50/p95/p99 for `report`, `invoice`, `pdf`, `jp`, `Month-03`, zero-hit, Chinese, and accented queries.

| Runner / rows | B `report` p50/p95/p99 ms | B `invoice` p50/p95/p99 ms | FTS-only `report` / `invoice` p95 ms | Full SQLite `report` / `invoice` / `jp` p95 ms | Snapshot `report` / `invoice` / `jp` p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Windows / 100k | 9.42 / 9.48 / 9.58 | 9.76 / 9.87 / 9.87 | 39.3 / 44.3 | 35.7 / 40.3 / 29.0 | 16.7 / 18.3 / 13.5 |
| macOS / 100k | 10.08 / 12.77 / 13.29 | 12.43 / 13.97 / 16.13 | 17.2 / 21.3 | 18.2 / 17.0 / 15.4 | 17.6 / 15.3 / 11.5 |
| Windows / 500k | 47.43 / 48.11 / 49.14 | 49.03 / 50.10 / 55.61 | 194.7 / 193.4 | 174.7 / 176.9 / 142.7 | 75.9 / 92.2 / 64.6 |
| macOS / 500k | 43.18 / 53.97 / 57.29 | 45.85 / 50.88 / 54.99 | 52.7 / 59.3 | 42.8 / 47.1 / 48.1 | 44.5 / 44.8 / 70.6 |
| Windows / 1M | 95.73 / 106.41 / 109.59 | 98.55 / 106.21 / 113.65 | 389.6 / 382.2 | 340.2 / 352.0 / 288.0 | 157.6 / 175.6 / 135.1 |
| macOS / 1M | 89.58 / 108.17 / 114.49 | 101.74 / 120.70 / 141.96 | 120.5 / 138.3 | 118.1 / 110.0 / 91.0 | 100.8 / 109.4 / 89.1 |

`report` and `invoice` had matching FTS-window membership and order at each size. `pdf` membership/order differed at all six sizes, although its first production page matched because the unchanged exact-extension tier wins. `Month-03` membership/order and first page differed at all six sizes. Zero-hit membership was equal, but B-only p95 rose to 172.9 ms on macOS and 193.9 ms on Windows at 1M. At 1M, B-only `report` and `invoice` p95 exceeded 100 ms on both platforms. The full SQLite route exceeded 100 ms for `report`/`invoice` on both 1M platforms and for `report`, `invoice`, and `jp` on Windows at 500k. The snapshot endpoint stayed below 100 ms for those Windows 500k cases, but exceeded 100 ms for the listed Windows 1M queries and macOS 1M `report`/`invoice`.

The resource accountant reported the following view and process measurements. “Two views” includes a replacement full view while the old view remains retained. Process RSS includes SQLite/FTS page residency and the test process and is not attributable to B alone; the RSS sampler can miss short peaks (macOS 100k post-build RSS exceeded its sampled build peak). Incremental CPU had one sample at every size and is not a useful peak measurement.

| Runner / rows | Accounted B active / two views MiB | Build ms | RSS after build / sampled build peak / sampled update peak MiB | Idle CPU average | 512-event apply ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Windows / 100k | 12.38 / 24.97 | 215 | 27.3 / 34.8 / 59.9 | 2.03% | 3.28 |
| macOS / 100k | 12.38 / 24.97 | 115 | 138.4 / 23.3 / 245.9 | 0.66% | 2.19 |
| Windows / 500k | 54.51 / 109.22 | 1,866 | 71.3 / 112.8 / 216.5 | 0.57% | 3.18 |
| macOS / 500k | 54.51 / 109.22 | 444 | 586.4 / 570.8 / 1,021.7 | 0.77% | 1.93 |
| Windows / 1M | 108.88 / 217.97 | 6,748 | 124.7 / 212.2 / 313.8 | 0.99% | 3.30 |
| macOS / 1M | 108.88 / 217.97 | 2,518 | 1,113.7 / 1,108.0 / 1,988.9 | 1.05% | 2.01 |

The 1M two-view accountant total of 217.97 MiB is below the 256 MiB limit, leaving about 38 MiB headroom; it is a code-level accounting result, not private-RSS attribution. At 1M, the standalone macOS process reached about 1.11 GiB RSS after build and a sampled 1.94 GiB during incremental update, while Windows was about 125 MiB after build and 314 MiB during update. Long-idle samples lasted 30 seconds. These standalone measurements do not establish application startup RSS, private bytes, long-duration stability, or worker CPU.

Raw Debug and Release logs, including all p50/p95/p99 query results and correctness comparisons, are attached to each matrix job: [Windows 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803125504/artifacts/12868076042), [macOS 100k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803125504/artifacts/12866804179), [Windows 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803125504/artifacts/12867558093), [macOS 500k](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803125504/artifacts/12867637231), [Windows 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803125504/artifacts/12868113479), and [macOS 1M](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803125504/artifacts/12868012045).

The full [PR CI run](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803153773) passed after the macOS Rust job rerun. The automatic [Windows/macOS ingestion smoke run](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40803153821) also passed; it is a 1k-row, single-repeat fixture and does not isolate marginal Change Feed trigger cost.

## Remaining qualification gates

- Resolve FTS/BM25 membership, order, and pagination mismatches for `pdf` and `Month-03` before enabling Stage B.
- Preserve the 100 ms p95 gate. The current full route misses it for high-hit Windows 500k and both 1M native fixtures; B-only p95 also misses it at 1M and for Windows 500k zero-hit. Improve the candidate generation and ranking path without degrading exact-name, prefix, or extension tiers.
- Measure marginal Change Feed trigger cost against a no-feed control at realistic write volumes on Windows and macOS; the automatic 1k smoke is not sufficient.
- Capture application startup RSS, private bytes during build/update, worker CPU, and longer idle stability. Current RSS is from a standalone synthetic process and includes SQLite/FTS page residency; incremental CPU had too few samples.
- Keep the default SQLite route and opt-in shadow-only worker until correctness, resource attribution, and latency gates pass. Do not enable B for user-visible routing.

## Enable and rollback

The default remains SQLite-only. To collect opt-in Stage A evidence in an isolated environment, set `ZEN_CANVAS_GLOBAL_SEARCH_SHADOW=1`; the worker still never supplies visible results. To roll back, unset the variable and retain SQLite as the route. The sidecar is disposable derived state; removing its tables/triggers requires a future explicit migration and is not part of this candidate.
