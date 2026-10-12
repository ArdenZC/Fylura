# Zen Canvas #381-D2 — Bounded Performance Design Feasibility

**Disposition:** `#381-D2 DESIGN FEASIBILITY READY_FOR_OWNER_REVIEW`
**Production status:** NO-GO pending Owner review and D3 authorization. D2 changed no product source, migration, IPC, CI, or PR state.

## Decision summary

On this isolated Linux host, **C-full** (current full `files_au` trigger + out-of-lock candidate preparation + exact in-transaction fencing + one set-based stale UPDATE) and **C-scoped** (the same fenced/bulk path plus the name/path-scoped FTS trigger) both kept the two competing writers under the existing 5,000 ms timeout at 100k rows. C-full is still close to the limit: max writer wait **3,849 ms**. C-scoped has more room: max writer wait **1,634 ms**.

The 100k A baseline also passed on this host, but only narrowly: the maximum competing writer wait was **4,311 ms**, leaving about **689 ms** before the configured timeout. The longest A writer-lock residency was **4,446 ms**. This does not supersede the Owner’s Windows evidence for the accepted PR head: CI run [38165968538](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38165968538) measured **30.352 s** of writer-lock residency and a competing writer timing out at 5,000 ms. This D2 host is not Windows and cannot requalify that result.

The least risky D3 candidate to validate is the **combined** C-scoped path: retain every durable CAS and publication invariant; prepare an exact candidate snapshot before `BEGIN IMMEDIATE`; recheck that snapshot, root/run/session/generation/revisions/Watcher/cancellation, root identity and live filesystem boundaries after acquiring the writer; publish with one set-based UPDATE plus the existing atomic downstream invalidation/revision work. The trigger optimization must be a versioned migration if adopted. Coordinate its schema version with #380; D2 did not use or reserve v38.

If D3 on the supported Windows runner still exceeds the 5 s writer budget, stop production changes and take the staged-publication design in §8 to Owner. Do not commit partially visible stale chunks or increase busy timeout.

## Frozen inputs and scope

- Repository: `ArdenZC/Zen-Canvas`.
- Branch: `codex/issue-381`.
- Exact input HEAD: `b79b97a2afc6c40ec57719325f86c486666d0bee`.
- Exact input tree: `8761bf387723d8e8da8afac313fae600da968cff`.
- PR #383 remained OPEN/Draft at that same head. No commits were pushed and no GitHub comment was posted.
- Owner D2 authorization: [comment 6115104109](https://github.com/ArdenZC/Zen-Canvas/pull/383#issuecomment-6115104109).
- The accepted prior performance failure remains recorded at [CI run 38165968538](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38165968538); it was not rerun.
- Production code references used for the comparison: [`schema.rs`](../../../../src-tauri/src/db/schema.rs#L2801), [`reconcile_missing`](../../../../src-tauri/src/db/queries/scan.rs#L1668), [`invalidate_stale_files_in_transaction`](../../../../src-tauri/src/db/queries/dedupe/mod.rs#L1749), and the production FTS query shape in [`fts_benchmark.rs`](../../../../src-tauri/tests/fts_benchmark.rs#L24).

The D2 run used disposable harness code and copied migration inputs in `/workspace/issue381-d2`; the reviewable report, harness source, catalog dump and result files are now checked into `docs/project/research/issue-381-d2/` on the D3 branch. The production worktree was clean at the frozen input head during D2.

## Harness and Schema 37 provenance

The independent Rust harness copies and executes the branch’s actual `schema.rs`, `schema_automation.rs`, and `schema_automation_triggers.rs` migration code. `PRAGMA user_version` was 37; the resulting SQLite catalog dump contains **65 tables, 107 indexes, 26 triggers, and 1 view**, including the real `files_fts`, `files_au`, File Library indexes, scan ledgers, tags, content artifacts, dedupe, and analysis tables. The schema catalog is preserved in [`schema37-sqlite-schema.sql`](schema37-sqlite-schema.sql).

The harness calls the copied production `schema::migrate` directly. The full Tauri crate could not be linked on this host because GLib development files are absent; therefore this was not an invocation of `Database::open`. The migration source itself compiled and ran. The only migration host stubs are the app-settings key/default JSON and path/time helpers; the settings stub has no default roots, after which the harness inserts its own real Schema 37 root/run rows. No tables or indexes were simplified or hand-created.

The harness uses `rusqlite 0.39.0` with bundled SQLite and reported runtime SQLite **3.51.3**. Production branch Schema 37 source SHA-256 values match the copied harness inputs:

- `schema.rs`: `c5ba26c0b0337c5e4c656c6440a54ff34c73a173e9d314c4afbec1d89be639b7`
- `schema_automation.rs`: `9b9aa8b284ad3054adf70dff77afda09a4147abd1ac20653449f7a430b8cb7f3`
- `schema_automation_triggers.rs`: `ef75b080127eece3dba5b97f134c18e3d26502dc0a0ea88ebd8237d0cc395140`

Host: Linux 6.18.44, x86_64, three visible vCPUs on a virtualized AMD EPYC 9V45 host, overlay filesystem. The app’s SQLite settings were mirrored: WAL, `synchronous=NORMAL`, `temp_store=MEMORY`, foreign keys on, and **5000 ms** busy timeout. No threshold or timeout was changed.

## Dataset and compared variants

Each 10k/100k fixture was seeded once through the real Schema 37 catalog and copied for each variant/replicate. Candidate file paths are genuinely absent below a real temporary root with 128 real parent directories. Every candidate has a `file_user_tags` assignment, user-authored file metadata, and a non-empty content hash. 10% have real `file_fingerprints`; the fixture also has a current content artifact, a duplicate group/member pair, an active analysis finding, one classification-history row, and durable root/session/run rows with generation/revision/Watcher fields. Database indexes and triggers remain enabled during seeding and measurement.

| Variant | FTS trigger | Candidate preparation / final fence | Stale row SQL |
| --- | --- | --- | --- |
| A | Original unconditional `files_au` | Candidate query + two filesystem observations inside writer transaction | Production-shaped 400-ID loop |
| B | `WHEN old.name IS NOT new.name OR old.path IS NOT new.path` | Same as A | Production-shaped 400-ID loop |
| C400 | Scoped FTS | Candidate query and first boundary observation outside; exact DB snapshot/CAS/Watcher/root/leaf/directory recheck inside | 400-ID loop |
| C-scoped | Scoped FTS | Same fenced preflight/recheck as C400 | One `UPDATE files ... FROM stale_candidate_snapshot` |
| C-full | Original unconditional FTS | Same fenced preflight/recheck as C400 | One set-based `UPDATE ... FROM` |

The final group has two replicates for every variant at both 10k and 100k: **20 measured cases**. Each case starts two actual competing SQLite writers after the reconciler holds `BEGIN IMMEDIATE`: a normal `files.context` metadata update and a `user_tags.updated_at` update. Both use a separate connection and the production 5000 ms timeout. A separate reader samples the published stale count during the write transaction.

C does not trust its preflight filesystem observations. In the publication transaction it compares the exact staged candidate ID/path set with the current eligible DB set, checks run/root/session/generation/Watcher/cancel state, then rechecks every candidate leaf, root identity, and each unique real parent chain before updating. This reduces lock time but cannot make filesystem state and SQLite COMMIT one atomic primitive; see §7.

## Performance results

Values are median of two runs unless a range or maximum is shown. Times are milliseconds. WAL figures are median file growth over the measurement transaction.

| Variant | 10k BEGIN→COMMIT | 100k BEGIN→COMMIT | 100k range | 100k max competing writer | 100k WAL growth |
| --- | ---: | ---: | ---: | ---: | ---: |
| A full FTS + 400 IDs | 348 | 4440 | 4433–4446 | 4311 | 169,430,912 B |
| B scoped FTS + 400 IDs | 158 | 2231 | 2200–2263 | 2186 | 75,886,280 B |
| C400 scoped + preflight + 400 IDs | 148 | 2148 | 2145–2150 | 2235 | 75,886,280 B |
| C-scoped + preflight + set update | 133 | 1526 | 1484–1568 | 1634 | 75,886,280 B |
| C-full + preflight + set update | 302 | 3583 | 3477–3689 | 3849 | 167,063,972 B |

All 20 cases marked the exact expected 10k or 100k stale rows. All ordinary metadata and tag writers committed under 5000 ms. Maximum wait across all cases was **4311 ms** (A, 100k), below the configured 5000 ms timeout. Reader observations were exclusively `{0, full_count}`; no partial stale count was observed.

### What the measurements isolate

- A/B differ only in the disposable `files_au` body. At 100k, paired A-minus-B UPDATE plus downstream invalidation time was **2073 ms** and **2178 ms** in the two replicates (median **2126 ms**). This is the measured estimate of unconditional FTS-trigger work; it is not an instrumented timing inside SQLite’s trigger VM.
- The trigger runs not only on `is_stale=1`, but also on the later production `content_hash=''` invalidation UPDATE. A therefore performs two unnecessary FTS delete/reinsert passes per candidate. B preserves the separate `content_artifacts_file_changed` trigger; the content artifact still becomes stale.
- Median 100k WAL file growth fell from **169.43 MB** (A) to **75.89 MB** (B), a **55.2% reduction**. C-full remains at **167.06 MB**; C-scoped is **75.89 MB**.
- C-scoped reduced median 100k lock time **31.6%** vs B by replacing the 400-ID loop with one set-based update while keeping the exact in-transaction recheck. C400 by itself did not materially beat B: its exact snapshot recheck costs about as much lock time as moving the first query/path observation out saves.
- C-scoped’s 100k preflight median was **179 ms** outside the writer lock: 42 ms candidate SQL, 97 ms leaf/parent boundary validation over 100,000 leaves and 128 distinct parents, and 40 ms to prepare the connection-local candidate snapshot. Final in-lock filesystem validation was **98 ms**; exact DB snapshot/fence checks add to that. Its median stale UPDATE was **980.7 ms**, related invalidation **228.7 ms**, and COMMIT **105.4 ms**.
- C-full shows bulk SQL and fenced preflight can help with the existing FTS trigger on this host, but the writer still waits up to **3849 ms** and WAL remains near A. C-scoped has materially safer local margin.

SQLite rusqlite does not expose per-trigger FTS time or per-statement WAL-write time. Therefore the report records A/B paired trigger delta, WAL byte growth, each top-level UPDATE/invalidation duration, COMMIT duration, and full transaction duration. WAL I/O time is not claimed as a separately timed subphase; measuring it exactly would require VFS-level instrumentation. Directory timing is reported as combined leaf `symlink_metadata` checks plus unique parent-chain checks, with counts for both.

100k phase detail (medians across two untraced runs):

| Phase | C-scoped | Notes |
| --- | ---: | --- |
| Candidate SQL before writer lock | 41.8 ms | Exact eligible ID/path set |
| Filesystem preflight before writer lock | 97.4 ms | 100k leaf checks; 128 unique parent chains |
| Connection-local candidate snapshot preparation | 40.1 ms | Before `BEGIN IMMEDIATE` |
| Total preflight | 179.2 ms | Outside main-database writer lock |
| Final filesystem revalidation under writer lock | 97.8 ms | Leaf, root identity, and directory-chain checks |
| Stale UPDATE plus scoped trigger work | 980.7 ms | One set-based UPDATE; trigger runtime is included |
| Related stale invalidation SQL/triggers | 228.7 ms | Content, fingerprints, dedupe, analysis, revisions |
| COMMIT | 105.4 ms | Separate commit timer |
| WAL file growth | 75,886,280 bytes | Between before/after snapshots |
| Full `BEGIN IMMEDIATE`→`COMMIT` | 1,526.2 ms | Complete writer-lock interval |

The paired A-minus-B differential estimates the unconditional FTS trigger contribution by phase: stale UPDATE **1,267.6 ms** median (replicate deltas 1,180.9/1,354.4 ms), downstream invalidation **858.0 ms** median (892.1/824.0 ms), total **2,125.6 ms** median. This is differential execution time, not a SQLite-internal trigger timer. The only direct WAL syscall-time breakdown is the intrusive `strace` sample below; the normal runs separately report WAL growth and COMMIT time. rusqlite does not expose an uninstrumented, exclusive VFS WAL-write timer. Directory timing combines leaf `symlink_metadata` calls and unique parent-chain checks; counts are retained in the raw JSONL.

Raw measurements: [`results.jsonl`](results.jsonl) (hash recorded in `MANIFEST.sha256`). The final 20-case run used frozen D2 `src/main.rs` SHA-256 `85954540d0e6cbfacd041e4f2a03b1d3bcfc99fffc27dcf4d67d8e67b0f4621b`. The checked-in reproduction copy changes only output-directory selection from the original fixed path to `ZC_ISSUE381_D2_OUTPUT_DIR` (default `target/issue-381-d2-output`); measured SQL and benchmark/failure logic are unchanged. Its current source SHA-256 is `b960f0bff1e576778593de145c64226addcf97ecce7a8dc37f06bf3c2d029916`, also recorded in the checked-in manifest.

### Supplemental WAL syscall trace

A single C-scoped 100k diagnostic invocation was run under `strace -f -yy -T` to collect WAL write/sync syscall durations. For the measured `single-case` WAL, it observed **89,391 `pwrite64` calls** (201,530,880 returned bytes, summed syscall time **1.5178 s**) and **4 `fsync` calls** (summed **0.0250 s**), for **1.5428 s** of summed selected WAL syscall time. This is an instrumented syscall sum, not an exclusive WAL phase timer; it can overlap other phases and `strace` materially perturbed the run. Under tracing the full transaction took **11.641 s**, final in-lock filesystem checks took **3.094 s**, COMMIT took **3.123 s**, and both competing writers hit the existing 5000 ms timeout at 5.032 s and 5.044 s. This run is diagnostic-only and is excluded from the 20-case performance gate; the unfiltered compressed raw trace is not checked into this branch (21 MiB); its SHA-256 values and machine-readable syscall totals are in [`wal-syscalls-summary.json`](wal-syscalls-summary.json). [`single-case.jsonl`](single-case.jsonl) preserves the measured record. The trace is diagnostic-only and is not required to reproduce the 20-case gate. The separate untraced C-scoped cases are the relevant writer-liveness results.

## Correctness and failure evidence

- Exact stale counts: 10,000/10,000 and 100,000/100,000.
- File Library search shape (`files_fts MATCH` joined to `files` with `is_stale=0`) returned zero stale hits after publication; the underlying FTS term remains present for stale files, as the stale flag owns visibility.
- Every tag assignment and tested user metadata value remained unchanged; classification history remained present.
- Existing content artifact trigger moved the current artifact to stale; fingerprints were marked missing; duplicate group and its active analysis finding became stale. No history or row was deleted.
- Normal name/path rename removed old FTS terms and added new terms. Separate conditional-trigger contract confirmed insert→search, rename old→new, and delete→no result: [`fts-contract.json`](fts-contract.json).
- Transaction rollback left 0 stale rows, the artifact current and run revision unchanged. A child process killed with SIGKILL after uncommitted stale UPDATE recovered with 0 stale rows and `integrity_check=ok`.
- Watcher revision change, cancellation, and root generation replacement all rejected publication with 0 stale rows.
- Replacing the root inode, replacing a directory with a symlink, or making a candidate leaf reappear after preflight each rejected publication with 0 stale rows.
- All failure cases passed: [`failure-contracts.jsonl`](failure-contracts.jsonl) (SHA-256 `967b2461b71962da6adf782b262057e05a98f91dce34461065fa9ba84d14da03`).

These are Linux boundary tests. Windows reparse-point handling, Windows source identity APIs, and macOS provider/file identity semantics were not exercised here.

## TOCTOU boundary

The C experiment does not assert that an SQLite transaction freezes the filesystem. It checks root device/inode and the current root/leaf/directory chain after acquiring the SQLite writer and immediately before SQL publication. A filesystem actor can still mutate the namespace between the last OS check and SQLite COMMIT. The production D3 implementation must use the existing platform-safe root/source identity and reparse/symlink verification mechanisms, recheck at the publication point, and fail closed when its proof is unavailable. A platform-native validation is required before claiming this closes the Windows failure.

## D3 scope and migration decision

Recommended smallest production experiment for Owner authorization:

1. Keep the scan generation, run/root/session revision CAS, Watcher revision, cancellation, exact candidate membership, source identity, boundary recheck and one-transaction atomic publication.
2. Move only immutable candidate collection/first-pass path observations outside the writer transaction; retain exact current-candidate and filesystem boundary rechecks inside it.
3. Replace 400-ID updates with one set-based update from a connection-local candidate snapshot. Keep stale rows and all existing content/dedupe/analysis/library invalidations in the same transaction.
4. In a coordinated, versioned migration, narrow `files_au` to actual `name/path` changes while preserving insert/delete triggers and rename equivalence. Existing `CREATE TRIGGER IF NOT EXISTS` does not update already-installed trigger bodies, so this cannot be shipped as an unversioned source-only trigger edit.
5. Run D3 on the supported Windows runner with the prior 100k scenario and both competing writers. Keep the 5000 ms timeout and existing performance gate. The D2 local numbers are feasibility evidence only.

**Schema answer:** C preflight + set-based SQL alone uses a TEMP table and does not need a persistent schema change. Adopting B for existing installations **does require a versioned migration**. If D3 requires staged publication, that would require a durable publication ledger/schema change. Coordinate either version with #380; D2 did not choose or consume v38.

If C-scoped or C-full misses on Windows, proposed staged fallback for Owner design review: create an operation-scoped per-root stale batch with run/session owner, lease token, generation, root revision, Watcher revision, source identity, candidate count/hash and lifecycle state. Populate its candidate rows in bounded durable chunks while the batch is invisible. One short final CAS transaction marks the fully validated batch published; File Library/FTS/content/dedupe/AI visibility predicates consume the same published overlay so no user can observe half a generation. Existing `is_stale`, FTS, fingerprints, duplicate groups, content artifacts and analysis findings can then be materialized idempotently behind that authoritative overlay. Before publish, crash residue is invisible and resumable/abortable; after publish, the atomic pointer is the durable truth and startup/cross-process recovery can resume materialization. Ownership must be root-scoped with generation/Watcher CAS and lease recovery; multi-root sessions retain per-root publication boundaries. This is architecture only, not D2 implementation authorization, and would require a coordinated schema migration plus audit of every File Library, content, dedupe and AI reader.

## Reproduction

From the repository root (the output path is configurable):

```sh
CARGO_TARGET_DIR=.tmp/issue-381-d2-target ZC_ISSUE381_D2_OUTPUT_DIR=.tmp/issue-381-d2-output cargo run --offline --release --manifest-path docs/project/research/issue-381-d2/harness/Cargo.toml -- --perf
CARGO_TARGET_DIR=.tmp/issue-381-d2-target ZC_ISSUE381_D2_OUTPUT_DIR=.tmp/issue-381-d2-output cargo run --offline --release --manifest-path docs/project/research/issue-381-d2/harness/Cargo.toml -- --contracts
CARGO_TARGET_DIR=.tmp/issue-381-d2-target ZC_ISSUE381_D2_OUTPUT_DIR=.tmp/issue-381-d2-output cargo run --offline --release --manifest-path docs/project/research/issue-381-d2/harness/Cargo.toml -- --fts
```

The benchmark command writes the 20-case JSONL and the Schema 37 catalog dump. `--contracts` and `--fts` write separate result files and do not delete the performance JSONL. The WAL syscall sample was captured separately with this diagnostic command (it is intentionally not part of the performance gate):

```sh
CARGO_TARGET_DIR=.tmp/issue-381-d2-target cargo build --offline --release --manifest-path docs/project/research/issue-381-d2/harness/Cargo.toml
mkdir -p .tmp/issue-381-d2-output
ZC_ISSUE381_D2_OUTPUT_DIR=.tmp/issue-381-d2-output strace -f -yy -T -e trace=pwrite64,pwritev,write,writev,fsync,fdatasync -o .tmp/issue-381-d2-output/wal-syscalls.strace .tmp/issue-381-d2-target/release/issue381-d2-harness --single 100000 C
```

Inputs and harness hashes:

- Harness `src/main.rs`: `85954540d0e6cbfacd041e4f2a03b1d3bcfc99fffc27dcf4d67d8e67b0f4621b`
- Harness `Cargo.toml`: `86231cf8e227652d3b3990f8079f7d1b54dc806c2f63cebe79f6b3383ac7eecc`
- Harness `Cargo.lock`: `98673caf0348328bf23217e95cbe61ad3617039d919eabb23afb61de09178a00`
- Schema dump: `954d93d368e382d87dbd1e2cdf3719ff73de20010918ec633108434557c8ce5b`

The migration source copies are inside `harness/src/db/production_schema/`; their SHA-256 values are listed above.

## Final state

- No production files, schema, IPC, CI, or migration version changed.
- No CI was run for D2; the known failed exact-head CI was not rerun.
- PR #383 remains OPEN/Draft at `b79b97a2afc6c40ec57719325f86c486666d0bee`; no merge, issue closure, review request, or release was performed.
- **`#381-D2 DESIGN FEASIBILITY READY_FOR_OWNER_REVIEW`** — stop pending the main conversation’s decision on D3 authorization.
