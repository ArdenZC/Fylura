# #381-B3-F feasibility and source-integration decision

## Decision scope

**F1 is the preferred feasible test-only candidate for Owner review. F2 proves exact ranking arithmetic, but its fresh-reader generation semantics remain BLOCKED when the physical index is mutated before publication. F0 is rejected.** This conclusion is bounded to the paired fixtures and measured single-run costs. It does not repair #381, unblock #383, allocate v38, authorize B3-O/B4 implementation, or qualify Windows/macOS application behavior.

Owner [6143106540](https://github.com/ArdenZC/Zen-Canvas/issues/381#issuecomment-6143106540) explicitly authorizes these new test/research files and the isolated Rust manifest/lock. Latest fetched master: `3275ac7c72df8ae4139a30f16c7f649971324e97`, tree `009576241cd4e8e2ba12cc644a368314f599faa6`. B3 blueprint #396 is `ba3985d93a0c0627720d478988d25a704e8b5757`; accepted B2 #395 is independent, not the branch base. #383's historic Hosted Windows 5284ms writer failure remains separate and DESIGN BLOCKED. Product is unreleased; no public old-binary migration prerequisite is introduced. All production files and frozen research branches remain unchanged.

## Actual source and build parity

The product locks rusqlite **0.39.0**, r2d2_sqlite **0.34.0**, libsqlite3-sys **0.37.0**. The isolated crate uses those exact versions and bundled features, plus product-locked serde/JSON/sha2/cc. Every resolved registry `(name, version, source, checksum)` matches the product lock. The downloaded SQLite crate SHA-256 matches the lock; amalgamation/header hashes and feature graph are retained. Product rusqlite owners are only the application and r2d2_sqlite; both declare bundled. The extra C auxiliary includes the header supplied by `DEP_SQLITE3_INCLUDE` from that same libsqlite3-sys build. No system SQLite, toolchain-file edit, custom SQLITE/LIBSQLITE build options, or dependency/version substitution is used.

Native connections actually report **SQLite 3.51.3**, source ID `2026-03-13 10:38:09 737ae4a34738ffa0c3ff7f9bb18df914dd1cad163f28fd6b6e114a344fe6d618`; compile options include FTS5. Real trigram DDL, ASCII case, CJK/supplementary Unicode, quoted punctuation and short-term behavior are exercised. This is source/feature equivalence and native library behavior on Codex Linux, **not full product application-build or Windows/macOS binary equivalence**. Product target compilers, pool/IPC integration, full schema and native physical identity remain unqualified.

The fixture copies the exact two-column external-content trigram declaration and three INSERT/DELETE/**AFTER ANY UPDATE** triggers from `db/schema.rs:2763-2818`. It deliberately uses a reduced physical `files` fixture plus independent epoch/root/index structures. In particular, same-path historical UID rows are representable here; production `files.path UNIQUE`/path-shaped IDs/FKs need a separately approved representation. This is not a migration test.

The production parser at `db/queries/mod.rs:176-195` quotes up to 12 whitespace-separated phrases joined by AND. Library V2 (`library/mod.rs:1406-1455`) does **not** add LIKE fallback. Legacy `files.rs:2422-2483` explicitly adds literal escaped LIKE for fewer than three non-whitespace characters, with rank 1,000,000. Both branches are modeled separately: Library short trigram queries stay empty; legacy LIKE runs over the correct published candidate's name/path and is compared across candidates. There is no claim that LIKE and trigram are equivalent. Probe relevance ordering uses the actual ascending rank / mtime DESC / name COLLATE NOCASE ASC / UID ASC tuple from `library/mod.rs:1688-1703`; varying fixture mtimes exercise real ties.

## Fair paired corpus and independent references

Each native run loads the same portable fixture into the physical source index, prepares a single published root-union membership/statistics authority, and evaluates F0/F1/F2 on the same query/scopes. A source UID/observation/physical fixture key is independent of path; root claims are explicit and overlapping claims deduplicate UID. Disabled roots never grant scope; reconciling B remains a labeled read-only snapshot source in this research contract, not a grant of current physical/action permission or a claim about current product health guards.

The score corpus is always **global effective-visible documents**, even for A/B queries. Scope filtering occurs before LIMIT/page slicing; candidate retrieval is unlimited. Native small reference builds an independent visible-only trigram index from fixture-label unions, with UID/mtime unindexed and name/path weights 6.0/1.5. It independently orders by the production tie tuple. It does not reuse model membership SQL or F1 slot preparation. A separate Python string/overlapping-phrase BM25 formula uses only portable fixture labels/text/mtime and independently verifies every full result, exact float bits and order, including the entire 10k/100k outputs. It imports no SQLite, model SQL, C metrics or native scorer. Large native reference index rebuilding is intentionally absent.

F0 uses the real all-physical FTS, native global BM25 and outer effective-member/scope filtering. IDs remain correct when filtering precedes LIMIT, but scores differ from the reference. Hidden high-frequency/long text alters corpus length and df. An actual raw top-two physical page followed by filtering underfills. Negative outcomes are recorded as `expected_rejection_observed`, not a success for the broken design.

## F1 — prebuilt immutable effective-only index slots

F1 creates a **separate disposable SQLite file per generation**, with only effective documents and frozen name/path/mtime/identity/root projections. Old slots open read-only and keep actual SQLite read transactions. Expensive DDL/build/integrity checks happen outside the main source writer reservation. Initial construction is O(N). Incremental root changes use SQLite online backup to copy the prior slot, then apply changed document inserts/deletes/updates; the copy is **O(database bytes)** and the root-claim projection is O(claims). Neither is presented as O(1). Root withdrawal preserves a second root's shared claim. Native index document churn and copy/prepare/storage costs are recorded separately.

The shared membership/statistics ledger stages in 1000-claim source transactions. A sealed proof binds source writer/text/root revisions, exact payload digest, N and token total. Members cannot be changed once sealed. Publication runs four fixed statements (`BEGIN IMMEDIATE`, constant-size indexed authority CAS/pointer+revision, one generation-state update, COMMIT); it does not rebuild/walk the index or source files. Cancelled, stale or forged READY/digest/stat authority rejects with rollback. Current cursor version/query/scope/generation/rank digest validates against **fresh current authority**, never an old captured index. An unrelated A rank mutation invalidates B even when B IDs stay unchanged; conservative global expiry may also invalidate harmless changes.

Reader registration serializes with pointer/GC authority. Old pins retain their actual immutable data; expiry uses a monotonic persisted logical test clock, so replaying an earlier clock cannot resurrect a retired reader. This clock is a deterministic fixture mechanism, **not a proposed product TTL policy**. Actual pin lifetimes in milliseconds are recorded. GC unlinks only private unreferenced index slots and removes obsolete ledger membership in at most 1000-row batches; repeated cleanup preserves the current epoch and all physical sources/user facts. Main writer occupancy for ledger GC is measured separately. File deletion is outside a held SQLite writer transaction. Windows open-file semantics, disk/pinned-snapshot bounds and real recovery policy need future review.

Six actual child-process SIGKILL barriers cover partial slot build, ready-before-publish, before pointer COMMIT, after COMMIT, GC before unlink and GC after unlink. Recovery returns exactly the old or complete new published slot; abandoned stage never becomes READY. Both candidates and a fresh small native reference check the recovered rank. These are process-crash tests, not power-loss, disk-full, corruption, migration or cross-file durability qualification.

## F2 — effective statistics and exact custom native BM25

F2 uses the **all-physical index only** for unlimited candidates. Its persisted effective N/token-total/member mask comes from source `_docsize` and the shared immutable epoch ledger; it does not derive statistics by rebuilding F1 or the reference index. The test-only C FTS5 auxiliary obtains actual trigram phrase instances and per-column counts using `xInstCount/xInst`, document length using `xColumnSize`, and effective per-phrase df via cached `xQueryPhrase` traversal filtered by the global member mask. It computes the exact SQLite BM25 formula with 6.0/1.5, k1=1.2, b=0.75 and SQLite's nonpositive-IDF floor. Diagnostic records expose phrase token count, df, frequencies and dl. Cached phrase scans still cost O(physical phrase hits); the planner does not expose this internal work.

**Actual result:** exact native/reference float-bit and complete-order parity in the tested matrices, including large independent checks. This is not a close-score substitution. Internal `_docsize` encoding, extension registration/caching, Unicode/phrase edge cases beyond the fixtures, concurrent provider/pool ownership and maintaining incremental effective statistics remain integration gates. The auxiliary builds a per-query bitmap and full ranked vectors; no production p95/memory/cursor-cost claim is made.

**Concrete generation blocker:** rename a visible source's name/path in the physical table while E0 is published. The all-file index no longer supplies a candidate for an E0 term, even though E0 membership/statistics are retained. A deliberately unguarded F2 query actually omits the old visible result. F1's immutable slot and an already-pinned F2 SQLite read transaction both retain E0. A **fresh F2 request** rejects via text-revision/corpus proof until refresh. Even hidden-only text writes conservatively trigger this guard. Thus F2's rank mathematics is proven in the fixtures, but it is **BLOCKED as a continuously readable fresh-request replacement** without immutable all-observation candidate rows or a separately bounded reader/snapshot strategy. Exact statistics alone cannot retrieve a term that the mutable index has erased. No F0/global-BM25 fallback is adopted.

## Explicit matrix and measurements

| ID | Fixture / assertions |
|---|---|
| B3F-C01-one, C10 | One/empty index, native capability, weights, short Library vs literal legacy LIKE; 0-corpus competing writers use persisted empty-corpus user state because no file/tag FK target exists |
| B3F-C01 | Shared fair small dataset, repeated trigrams, multiple weights, ASCII case, CJK/supplementary/accent/combining Unicode, punctuation, three-row pages and exact stable ties |
| B3F-C02 | A/B overlap, disabled D including a shared claim, reconciling B read label, withdrawal of A keeps B; explicit scope before page slicing |
| B3F-C03 | Hidden long/frequent text and df adversary; F0 wrong score and post-LIMIT underfill; F1 global effective corpus invariant |
| B3F-C04 | Actual visible in-place candidate omission if F2 guard ignored; preserved pinned E0; same-path new physical UID with retained old notes/tags and no inheritance |
| B3F-C05 | Exact existing AFTER ANY UPDATE stale-bit churn with name/path fixed; physical SQLite/WAL/index writes, zero shadow writes, unchanged source/user hashes |
| B3F-C06 | B IDs unchanged but global rank changes through A; actual old/legacy-version/scope-tampered cursor rejection |
| B3F-C07 | Cancel, forged READY/digest and source-writer CAS failures retain pointer; no unconditional restoration |
| B3F-C08 | Live-pin GC held; expired pin and earlier-clock replay rejected; bounded/idempotent cleanup, physical sources retained |
| B3F-C09 | Six deterministic subprocess SIGKILL/recovery traces; exact old-or-new, F1/F2/independent native rank and retained sources |
| B3F-C10/C11/C12 | Paired 0/10k/100k, deterministic 20% hidden ratio (large), 10 A-only claim withdrawals, initial/copy/delta preparation, query/plans, DB/WAL/btrees, actual pin duration, 1000-row GC batches, two actual competing writers at unchanged 5000ms |

Queries/plans/logs are for the **exact probe statements**, with MATCH-driven native scans and Rust scope/rank/page materialization. They are not claimed to be complete production CTE/pool/IPC plans. EXPLAIN QUERY PLAN has ordinary FTS5 virtual-table limitations and cannot price auxiliary phrase scans or Rust work. Query timing includes full candidates, UID/document-map materialization, filtering and sorting; small diagnostic metrics queries run separately after the measured query. Peak VmHWM is the actual cumulative native process maximum across both variants/preceding phases, not a per-variant allocation attribution. One raw sample per query/phase/size is reported; there are no p95/p99 claims or 500k/1m runs.

## Owner decision table

| Dimension | F1 | F2 |
|---|---|---|
| Exact globally effective BM25 | Proven in this matrix, native rank | Proven in this matrix, exact custom native API statistics/math |
| Source changes needed | Versioned immutable relevant file projections, per-root union, index slot resolver and consumer guards | Effective membership/statistics ledger, native API scorer/provider ownership, immutable all-observation candidate retrieval or fresh-request snapshot strategy |
| DDL/layout | Separate slot files in probe; actual product storage/version/FK/rowid design unapproved | Shared epoch/stats plus physical index in probe; private docsize/native-extension contract unapproved |
| Preparation | Full index build initially; whole-slot copy plus index document delta on root churn | N/length/member staging; per-query cached df traversals; no effective-only index required |
| Writer reservation | Fixed main publication; shadow build locks its own file; main ledger/GC bounded batches | Same fixed authority publication; physical AFTER ANY UPDATE churn remains |
| Mutable prepublish attributes | Old slot retains exact name/path/mtime | Fresh request blocks; ignoring proof demonstrably loses candidates; existing SQLite reader can retain E0 |
| User metadata | Physical UID facts retained; private slot removal never deletes user rows | Physical UID facts retained; same-path replacement inherits none |
| Cursor/p95/memory | Full-result digest and global expiry correctness technique; copy/old slots and unknown pool/UI cost remain | Full candidates, bitmap/df scans and digest cost; main WAL pin growth and fresh-reader gap remain |
| Crash/GC | Linux process interruption and monotonic lease expiry demonstrated | Shared epoch stats recovery demonstrated; main physical versioning/source durability unresolved |
| Windows/macOS/native | Not run; open-file, physical identity/permissions, performance and full application integration remain independent gates | Same unknowns plus native scorer build/registration/pool behavior |
| Disposition | **FEASIBLE TEST-ONLY / preferred for Owner** | **EXACT RANK PROBE / GENERATION-INTEGRATION BLOCKED** |

Immutable source attributes and their historical/current presentation, every affected reader/writer/direct ID route, operation/action eligibility, real Watcher/source fences, existing `files` IDs/FKs/rowids/FTS/backup/migration, pooled query budgets and Hosted/native 100k fairness still need separate Owner integration reviews. The isolated result supplies evidence for that decision only. Stop OPEN/Draft; no production GO, merge, issue close, release or Codex Review.
