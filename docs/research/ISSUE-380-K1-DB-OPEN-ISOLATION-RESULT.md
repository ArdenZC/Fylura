# #380 K1 DB-open generation isolation — test-only result

**#380-K1_DB_OPEN_ISOLATION_TEST_ONLY_READY_FOR_OWNER_REVIEW**

**Architecture/production result: K1 NO-GO for both S and N as currently demonstrated.** Twenty passing characterization methods reproduce the risks and limited SQLite safety properties; they do not qualify old/new compatibility, production migration or rollout. No winner is forced.

## Authority, identity and scope

Read Owner [K0/K1 authorization 6138255026](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6138255026), [Track A acceptance 6137956459](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6137956459) and [Track B acceptance 6137994083](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6137994083). Remote facts, comment bodies/hashes and source reference trees are frozen in [remote-facts.json](../../tests/issue380_db_open_generation/evidence/remote-facts.json).

| Input | Frozen HEAD | Use |
|---|---|---|
| Production master | `3275ac7c72df8ae4139a30f16c7f649971324e97` | New independent branch base; old DB path/open/migration |
| Track A | `9e66433fd833e2f52a428976303af10d8a252de9` | Extraction/IO-boundary method; no retrospective test rerun |
| Track B #388 | `9e144da261b97a421b6a40f61280e149887f6204` | Only frozen fixture insertion/seeding and backup reference |
| D1-A #386 | `d260131f9d6ec354f2361e6c7460c79e62e38391` | Frozen new fallback function |
| #381 D4-S8R | `a48f8849b1ba1baadcf84e217d505e2298cf35a1` | Frozen consumer boundary; README read, no inventory rescan |

Master tree is `009576241cd4e8e2ba12cc644a368314f599faa6`; remote master and #385/#386/#388 heads did not drift at start. #380 remains OPEN; all three reference PRs remain OPEN/Draft. #388 C3 10k lock controlling result remains **0.128199 ms**, not 0.110 ms; K1 does not rerun or extrapolate that measurement.

Branch: `test/issue-380-k1-db-open-generation`. Tracked changes are only `tests/issue380_db_open_generation/**` and this report. No production Schema/Database/Repository/Coordinator/Service/IPC/NSIS/Search/Managed Scan/#381/workflow edits or v38 allocation. Final HEAD/Tree, complete diff and exact-head automatic CI are supplied in the Draft PR/delivery record; a file cannot embed its own commit/tree without changing that identity.

## Executed code and truth boundaries

The Rust probe executes **unchanged frozen Database::open, configure_connection, database_path, open_database and the full schema.rs migrate implementation**, including the real r2d2 pool, rusqlite, schema_automation and schema_automation_triggers. The full production migration creates the fixture, rather than Track B's partial schema installer. Only Track B's frozen `insert`/`seed` functions insert three representative rows with two shared physical identities and associations.

Harness substitutions are explicit:

- Tauri AppHandle/path acquisition is a supplied disposable app-data root. Both normal and native-qa builds execute the frozen path function; this is not the real Tauri application.
- Default settings JSON is an empty scan-folder policy (`defaultScanFolders: []`), so migration cannot discover user folders. This substitutes settings input, not migration SQL/version guards. Database wake/cache field types have minimal shells; unrelated Database methods are omitted.
- Python actors are independent processes with actual SQLite connections and legacy-table SQL. They model retained Desktop/worker connections; they are **not old Desktop binaries or SCM services**. Frozen current Service owns provider/PipeSink and does not call Database::open itself; its persisted events normally reach the Desktop-owned SQLite sink.
- Old/new fallback function bodies execute unchanged with typed IO outcomes injected at the service-call seam. Direct invokes the real frozen open/migrate probe; no named-pipe transport is exercised.
- Installer stop/close/replace/restart checkpoints, an active-DB pointer and coherent promotion are isolated control models. POSIX symlinks/flock/fsync/rename are real operations in disposable roots; they do not establish Windows path, mutex, loaded-image or installer behavior. Owned-file rename demonstrates retained process capability, not execution of the product's Preview/journal/confirmation pipeline.

[Source manifest](../../tests/issue380_db_open_generation/evidence/source-manifest.json) records immutable blob hashes, extracted function hashes and first lines. No frozen function behavior is patched. Test-input hashes and raw child commands/stdout/stderr are in [results.json](../../tests/issue380_db_open_generation/evidence/results.json). Historical precommit run HEAD identifies master; input hashes bind the uncommitted harness actually executed. Final delivery separately records the exact final-HEAD rerun. The initial diagnostic logging/fixture errors were harness errors and are not compatibility results; no risk assertion was removed or weakened.

## K1-01..K1-09 coverage matrix

All executed rows use Linux x86_64, Rust 1.97.1/rusqlite 0.39 **SQLite 3.51.3** and Python 3.12.14 **SQLite 3.53.1**. Three-row fixtures have the full frozen migrated schema; projection checks cover 17 populated domain tables plus both FTS indices. Commands/fault parameters and actual outputs are keyed by the `test_k1_XX_*` method in structured evidence. The Python worker uses a 100 ms test timeout; the frozen Rust opener retains production's **5000 ms** timeout.

| ID | Exact source / injection / steps | Expected and actual output | Conclusion / unverified boundary |
|---|---|---|---|
| K1-01 | lib.rs:135/148 → connection.rs:82/213 → schema.rs:36 → main.rs:106. Fresh root; then drop one existing index and reopen at v37. | Frozen open returns v37 and persisted tables before hypothetical post-open reject. Same-version reopen recreates `idx_global_volumes_mount_path`. | STATIC_SOURCE_FINDING + ISOLATED_EXECUTED_EVIDENCE: gate is too late. Real Tauri/Windows startup NOT_RUN. |
| K1-02 | Old actor opens first; synchronized writes before/after Rust reopen and reconnection. Hold read transaction across migration/new commit. Separate adversarial user_version=1000 marker. | `BEFORE_OPEN`, `AFTER_NEW_OPEN`, `AFTER_RECONNECT` commit. Held snapshot remains old until rollback. Rust rejects future marker, retained connection still commits `HELD_IGNORES_FUTURE`. | S does not revoke held read/write authority. Marker is a disposable negative control, not allocation of a new schema. Old binary/native handles NOT_RUN. |
| K1-03 | Frozen ServiceRuntime/PipeSink and retained desktop-worker actor. Model Service stop; repeat Rust open and replay writes with same worker PID. | `REPLAY_BEFORE_SWITCH` / `REPLAY_AFTER_SWITCH` commit. Worker survives modeled control event. Source shows Service host has no Database::open. | A stop indication alone is not SQLite writer exclusion. Actual SCM stop/join/pipe replay NOT_RUN. |
| K1-04 | Final NSIS preparation source order plus four DB-open launch checkpoints. Spawn a real late old actor at each; hold a new-only POSIX flock separately. | Late actor opens/writes fixed DB at stopped/closed/replaced/restarted checkpoints. `IGNORES_NEW_LOCK` commits while new-only lock is held. | S startup exclusion unproved. Control order is modeled; Windows Find/Kill/SCM/mutex/file replacement NOT_RUN. |
| K1-05 | Fixed path; `sub/../` and directory symlink; generation-labelled file symlink; commit into WAL with keeper open, copy main versus SQLite online backup. | Aliases modify same database/inode. Main-only copy lacks `COMMITTED_WAL_ONLY`; backup contains it. | N requires physical namespace separation and a WAL-aware snapshot. Windows reparse/hardlink/UNC/case/ACL/handle semantics NOT_RUN. |
| K1-06 | Snapshot → old late write → pointer switch; old/new actors after cutover; child exits 71/72 around pointer replace and 73/74 around SQL commit, WAL/DELETE. Real 36→37 control and injected index-name collision. | Snapshot retains `AT_SNAPSHOT`, old file gets `LATE_OLD_WRITE`. `OLD_BRANCH`/`NEW_BRANCH` diverge; old actor renames owned file. Pointer selects one complete filename; uncommitted SQL rolls back, committed SQL survives. Valid control upgrades to37 preserving projection; injected failure leaves36 and unchanged projection/pointer. | N file separation and SQLite atomicity are limited positives, not no-loss cutover. Process exit is not power loss. Windows atomic replace, coherent installer rollback and product migration NOT_RUN. |
| K1-07 | Second-process BEGIN IMMEDIATE while frozen Rust opens; release then retry. Execute old/new fallback bodies for busy/sqlite_busy/unknown/not_sent. | Rust returns `database is locked` at about5s, later opens; old writes after release. Old Direct opens/migrates for unknown/not_sent/sink failure. New does so only for NotSent; busy, SQLite sink failure and Unknown do not Direct. | BUSY is liveness, ServiceBusy is request rejection, Unknown is ambiguity. None supplies pre-open DB authority; new NotSent still bypasses a hypothetical later gate. Real pipe classification NOT_RUN. |
| K1-08 | Normal versus native-qa path builds; QA override and alias; relaunch with experimental active pointer; restore an older complete backup after new write. NSIS recovery source inspected. | Normal ignores QA env; QA selects profile path, alias resolves old path. Frozen normal relaunch ignores pointer. Restored copy lacks newer active update. | Profile, reinstall and restore path choice needs generation authority; SQL backup is not recovery policy. Actual uninstall/reinstall/NSIS/Owner profile NOT_RUN. |
| K1-09 | Compare complete `rowid,*` projections before/after online backup, FK/integrity checks, tag/correction/history/AI/Managed and FTS rowid hits. Successful migration control also compares projection. | Exact projection equality, FK=[], integrity=ok, stable IDs/rowids/associations and FTS hits retained. Independent later user correction is isolated and hence forks. | Snapshot preservation proved for representative three-row corpus, not production semantic C2/FTS/BM25/cursor/#381 seals or scale. Native product NOT_RUN. |

First DB-open boundary is the pool manager's actual SQLite open/configure before `Database::open` returns. WAL configuration may change persistent journal state before migration; at a fresh database the first application table DDL is `CREATE TABLE files` inside migrate. At schema37, ensure routines perform DDL and settings-root backfill before return. main then recovers/prunes before runtime/domain admission. Neither a Service admission result nor a later C2 domain-publication pointer authorizes those earlier actions.

The migration failpoint uses a disposable backup: reconstruct empty v36 automation tables from frozen production DDL and remove only v37 additions, with an uninjected successful 36→37 control. A view occupies `idx_automation_intents_active`; unchanged migration returns `there is already a table named idx_automation_intents_active`. The transaction rolls back its committed schema-version update and domain rows remain unchanged. This fixture construction is not a proposed production downgrade/rollback.

## S versus N

| Property | S: fixed path + stop-world | N: separate generation namespace + switch |
|---|---|---|
| Verified positive | SQLite read snapshots remain consistent; ordinary locks serialize writes while held. | Physically distinct files isolate writes; SQLite backup includes committed WAL and preserves representative rows/IDs/FTS/FKs; POSIX pointer replacement is whole-file across process exits. |
| Reproduced counterexample | Held/reconnected old writers, late process launch, new-only advisory lock ignored, same-path reopen migrates before later rejection. | Aliased namespaces are not distinct; main-only copy loses WAL; late old writes miss snapshot; after switch old/new data forks and old file-action capability remains. |
| Unproved assumption | Every legacy connection/worker terminated, no relaunch during or after cutover, mandatory pre-open authority shared by unchanged old code, real SCM/handles/Direct excluded. | Quiescent no-loss final snapshot, physical-path authority, persistent exclusion of old app/file actions, coherent recovery/profile/reinstall/uninstall selection, durable power-loss-safe activation. |
| Production result | **NO-GO** with current one-time close/new-only rules. | **NO-GO** despite limited namespace/backup positives. |

P0 blockers: pre-open and pre-migration authority is absent; legacy actors ignore new cooperative flags and domain pointers; snapshot/cutover cannot account for late writes; old DB state and filesystem actions remain a competing durable authority. P1 blockers: path aliases/profile/restore ambiguity, crash/power-loss/Windows activation proof and coherent rollback after post-cutover writes. The accepted C2 domain pointer must remain separate from DB_generation/process authority; K1 proves neither #381's seals nor C2 complete generation publication.

Owner recommendation: require one coherent **pre-open activation, lifetime writer/native-action exclusion, final snapshot/catch-up, path identity and rollback** contract before production GO. A new mutex, user_version refusal or changed filename alone cannot satisfy it. N's physical separation is useful defense in depth but is not selected as a complete solution. Owner must decide the enforceable OS/process authority boundary for unchanged old clients, when all old writers are demonstrably quiescent, how divergent post-cutover data is handled without loss, and which generation restore/uninstall/reinstall can activate. No new implementation or research stage is authorized by this report.

## Verification levels and reproduction

| Grade | K1 status |
|---|---|
| STATIC_SOURCE_FINDING | Completed for bounded startup/service/path/installer/fallback owners, with hashes. |
| ISOLATED_EXECUTED_EVIDENCE | **20/20 methods passed**, 0 failed; negative witnesses observed in disposable fixtures, independent processes and connections. |
| HOSTED_WINDOWS_VERIFIED | **NOT_RUN** for K1. Ordinary automatic PR CI is a separate routing/governance result, not execution of this harness. |
| OWNER_NATIVE_VERIFIED | **NOT_RUN**; Windows SCM/NSIS/token/handles/native filesystem/path admission remains NATIVE_UNVERIFIED. |

Run `python3 tests/issue380_db_open_generation/run.py`. It builds both frozen Rust probes with `cargo build --offline --locked`, then runs the bounded Python suite. Frozen Git objects must be available and locked crates cached; the runner never fetches or dispatches workflows. Initial dependency-cache preparation used crates.io; acceptance runs are offline. No 100k/performance/Full Validation/Native Qualification is launched.

[Raw unittest log](../../tests/issue380_db_open_generation/evidence/unittest.stdout.txt), [structured events/projections](../../tests/issue380_db_open_generation/evidence/results.json), [source hashes](../../tests/issue380_db_open_generation/evidence/source-manifest.json) and build command/compiler/binary records reside in the evidence directory. Every child is synchronized through JSON stdin/stdout; timeouts bound waits, rather than sleeps establishing order. SQL/file actions are confined to disposable `.tmp-tests/issue380-k1/` roots; QA environment is removed unless explicitly injected. Fixtures are deleted on close; shared caches and real databases are untouched. Final-HEAD rerun logs and delivery patch/identity remain in ignored output artifacts.

Draft PR natural CI may run; no manual Hosted, Full Validation, Codex Review, merge, issue close, release or continuation into production is performed. #385/#386/#388 holds, #381 production hold, Schema v38 unallocated and NSIS/U2 release NO-GO remain unchanged.
