# Global Search production candidate: Stage A shadow qualification

## Decision

**NO-GO for Stage B or production routing.** This branch keeps the SQLite search result authoritative and leaves the shadow worker disabled by default. Linux Release candidate latency was below 100 ms at 100k synthetic rows, but high-hit candidate sets diverged from FTS/BM25 at the 4,096 window. Windows/macOS native qualification at 100k, 500k, and 1M rows is still required, and the ranking/window mismatch must be resolved before an Owner production review.

## Scope and safety

- Based on master `df940915084b810101a8a3eb78796cdf8730d55b` (tree `fa65a8fd1fe6f45a774024f575010c1928292ae4`).
- #495 remains a separate research Draft PR. Its static view did not provide a durable generation, bounded incremental feed, or production resource proof. This branch does not cherry-pick its branch or dependency changes.
- The main schema migration chain remains untouched. The optional feed is additive and self-versioned, so it does not claim or change `PRAGMA user_version`; this avoids collision with the independent #489 authorization migration. When the optional `scope_authorization_state` column exists, an invalidation trigger is installed and validated.
- SQLite remains the durable source of file and source facts. Installation does not scan or copy indexed rows. A bounded feed records up to 100,000 events; saturation invalidates incremental use until a full snapshot safely resets the feed.
- `ZEN_CANVAS_GLOBAL_SEARCH_SHADOW=1` is required to start the worker. The worker builds an immutable compact view, applies at most 5,000 generations per batch with a 50 ms work target, and refuses publication on gaps, source changes, budget failure, or stale generations. The SQLite route still produces every visible result. Exact-name, name-prefix, exact-extension, extension-prefix, and BM25 SQL remain unchanged.
- The sidecar tables and triggers are installed when the database opens, even while the worker is off. They add bounded SQLite write work, which still needs native ingestion-cost qualification.
- No test accesses a user disk or indexes a real volume. Scale fixtures write synthetic rows through the real schema and FTS/change triggers in a temporary database.

## Correctness evidence

Targeted Rust view tests: **9 passed, 0 failed** (plus one ignored scale qualification). Change-feed tests: **6 passed, 0 failed**. `cargo check --lib`, formatting, governance checks, and Clippy passed for the changed modules. The repository's exact Linux `desktop-runtime --all-targets -- -D warnings` command is blocked by existing OS-gated unused-variable and dead-code warnings in `fs_safety` and `startup_upgrade`; Clippy passes when only those two Linux-only warning classes are allowed. Windows/macOS CI will run the unmodified native lint gate. The tests cover independent ASCII substring oracles, stable Top-K and the 4,096 window, punctuation/path/extension cases, insert/delete/rename/move/stale/source disable, actual repository writes, transaction rollback, feed saturation and gaps, the optional authorization trigger, low-memory refusal, database reopen/rebuild, worker refresh behavior, and a reader holding the prior immutable generation while an update is built.

An earlier full Linux `cargo test --lib -- --test-threads=1` run, before the final matcher normalization and worker refresh test, reported **1,106 passed, 54 failed, 36 ignored**. Failures were in AI/provider and native file-workspace/managed-worker suites outside the changed search modules; this is not final-head regression evidence. The PR's Windows and Apple Silicon CI lanes will run the full suite against the final head.

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

## Remaining qualification gates

- Run the branch-scoped `Global Search view native qualification` workflow at 100k, 500k, and 1M rows on Windows and Apple Silicon macOS. It is limited to candidate code and database changes and does not run for ordinary documentation changes.
- Resolve high-hit membership/order mismatch before enabling any Stage B route; preserve complete pagination and the unchanged 100 ms gate.
- Review native Release p50/p95/p99 for candidate-only, FTS-only, full SQLite search, and the snapshot endpoint, plus fallback and 4,096-boundary correctness.
- Capture native application startup RSS, peak RSS/private bytes during view build and updates, idle CPU, update-worker CPU, long-idle resource stability, and prior-view release. The current standalone Rust process sample is preliminary and does not qualify these application-level gates.
- Confirm search-change trigger write cost on both native platforms and inspect CI/clippy results for the PR head.
- Improve candidate generation and ranking parity before considering Stage B. The current view scans all indexed rows per query and has not been measured above 100k in this environment; do not enable it for user-visible routing.

## Enable and rollback

The default remains SQLite-only. To collect opt-in Stage A evidence in an isolated environment, set `ZEN_CANVAS_GLOBAL_SEARCH_SHADOW=1`; the worker still never supplies visible results. To roll back, unset the variable and retain SQLite as the route. The sidecar is disposable derived state; removing its tables/triggers requires a future explicit migration and is not part of this candidate.
