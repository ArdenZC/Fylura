# #380 K2-T0 — Legacy activation isolation feasibility

**Terminal: K2_T0_OS_AUTHORITY_BLOCKED. Production remains NO-GO.**

Eight new bounded characterization cases reproduce noncooperative writes, late launch, WAL/path alias ambiguity, split user corrections and retained native file capability. Their passing test execution does **not** demonstrate continuous legacy exclusion. The missing OS authority is the blocking result, not a reason to implement a repair in this PR.

## Authority and frozen identities

Owner [execution GO 6138888719](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6138888719), [K2 conditional Hybrid decision 6138854189](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6138854189) and [K1 acceptance 6138706099](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6138706099) were reread remotely. Issue bodies/state, PRs and comment hashes are retained in [remote facts](../../tests/issue380_k2_activation/evidence/remote-facts.json). Historical issue-body production instructions are superseded by these restricted Owner decisions.

| Input | SHA / disposition |
|---|---|
| Starting current master | `3275ac7c72df8ae4139a30f16c7f649971324e97` / Tree `009576241cd4e8e2ba12cc644a368314f599faa6` |
| #385 | `8ab50cb627712abef8debfa786d9fb2354ef9581`, OPEN/Draft, merge hold |
| #386 | `d260131f9d6ec354f2361e6c7460c79e62e38391`, OPEN/Draft, independent merge NO-GO |
| #388 Track B | `9e144da261b97a421b6a40f61280e149887f6204`, OPEN/Draft, merge hold |
| #389 K1 | `bbdd3db254582ae622236237b87adb73b097bc42`, Tree `6341a9fdf547fe85331121a412fcb35622415bda`, OPEN/Draft, merge hold |
| Track A | `9e66433fd833e2f52a428976303af10d8a252de9`, accepted frozen evidence |
| #381 S8R | `a48f8849b1ba1baadcf84e217d505e2298cf35a1`, frozen dependency boundary |

No starting drift. #380/#381 remain OPEN. Independent branch `test/issue-380-k2-t0-activation-feasibility` has only the authorized test directory and this report. Final HEAD/Tree, complete diff and CI status are supplied in the delivery/PR record; this document cannot embed its own tree identity. Master, existing PRs, schema, application, Service, IPC, NSIS, defaults, workflows and #381 are unchanged. No v38 allocation.

## What actually executed

Linux x86_64, Python 3.12/SQLite **3.53.1**, uid/euid 1000 in the recorded environment. **8/8 new unittest methods passed**, with explicit pipe barriers, actual independent subprocesses and SQLite transactions. PIDs, program paths, UID, main device/inode/link count, WAL/SHM identity, open-handle identity, exact commands, input, raw stdout/stderr and SQL results are in [results.json](../../tests/issue380_k2_activation/evidence/results.json). The miniature fixture has one representative file, tag, FK, history and FTS plus an atomic commit audit; it is **not Schema37 or a production migration proposal**.

Frozen K1 build/probe shells are read from Git, copied only into ignored task output and reused for the **new activation-record fault cases**. They execute unchanged Database::open, configure_connection, full migrate, normal/QA path and old/new fallback functions with real r2d2/rusqlite **SQLite 3.51.3**. The accepted K1 Tauri path/settings/transport IO substitutions remain explicit. Only build artifact locations change. **No K1 test/actor/run script or its 20-case suite executes; no Track A/B corpus, 5s liveness rerun or #381 inventory rescan occurs.**

Legacy actors are Python behavioral substitutes, not released Zen Canvas binaries. They never read the new activation record, take the new mutex or check generations. The disposable-root nonce only enforces test safety, not upgrade cooperation. One generic script also supplies separately identified snapshot/transform/switch roles; the legacy-writer role only performs normal SQLite writes/reconnects and owned-file actions.

The `opened_main_handle` identity uses a duplicate O_RDONLY descriptor opened at actor startup; it is **not** introspection of SQLite's private fd. Retained SQLite reads/writes independently prove connection behavior. File-action fd identity is measured directly with fstat. Installer and single-instance steps are source-bound models, not execution of NSIS/SCM or Tauri's single-instance implementation.

## T0-01..08 truth table

Each row has actual inputs/output in the named `test_t0_XX_*` case. Expected negative witnesses are assertions, not waived failures. Each fixture is path-validated before first open and deleted on exit.

| Gate | Method / fault point | Actual result and boundary | Safety result / grade |
|---|---|---|---|
| T0-01 | Keep one old process/connection across before-admission → isolation-started → snapshot-complete → new-activation → reconnect; hold new-only flock and change activation record. | Five successful SQL commits persist; snapshot contains only first two. Old connection and later reopen ignore all new flags/lock. | Continuous DB-write exclusion **FAIL/risk reproduced**. ISOLATED_EXECUTED_EVIDENCE; actor is not old product binary. |
| T0-02 | Frozen NSIS stop/close order; real owned Desktop actor exits while independent worker stays alive; model Service STOPPED. | Worker still commits. Service STOPPED, Desktop exit, connection close, worker stop, late-launch exclusion and old-copy exclusion remain six distinct conditions. | Complete writer fence **UNPROVEN**. STATIC_SOURCE_FINDING + isolated model/worker execution; SCM/UAC/NSIS/cross-Session NOT_RUN. |
| T0-03 | Resolve symlink and `sub/../`; hardlink same main inode while old WAL is committed; replace main path with distinct inode in DELETE mode while old connection/duplicate fd remain open. | Symlink/normalized names read WAL commit. Hardlink reads `USER_SEED` while canonical path reads `WAL_COMMIT`, despite equal main inode; alternate WAL/SHM namespace differs. Retained connection reads old data; write fails with **SQLITE_READONLY_DBMOVED(1032)**; new path initially remains `NEW_INODE`, but old actor reconnect then commits `REOPENED_WRITES_NEW_IDENTITY` to that replacement. | Path strings and main inode alone are not complete SQLite physical identity. DBMOVED is a **real local positive**, not proof of closure/lifetime exclusion. POSIX isolated; junction/reparse, Windows handles, active-WAL replacement NOT_RUN. |
| T0-04 | One-time actor check/exit, then late launch, login-item model, second-instance model and byte-identical actor script copied to a different path. Main chmod400 with live WAL, then same owner restores mode/reconnects. | All late processes commit. Existing WAL actor commits with main mode400; same uid can restore mode and commit after reopen. | Persistent launch/access exclusion **FAIL in candidate model**. Real subprocess/mode operations, not product login/single-instance or Windows ACL/UAC qualification. |
| T0-05 | Independent old commit, backup process, late old correction, fixture-only offline SQL transform, pointer switch, first new write. Fail/exit inside transform, before/after replace. | Backup preserves representative rows/FK/history but misses `LATE_USER_CORRECTION`; old/new later acknowledge distinct corrections. SQL transform failure/exit rolls back its transaction and aborted/pre-switch cases keep only the old active writer. Pointer remains whole old/new filename under process exit. Choosing old generation after new first write would lose that acknowledged new correction. | No-silent-fork **FAIL**; activation must BLOCK. Snapshot/pointer/transaction positives are limited. Transform is a model; production migration and power-loss/Windows durability NOT_RUN. |
| T0-06 | Hold real BEGIN IMMEDIATE so old SQLite writer returns BUSY(5), while same actor keeps a real ordinary-file fd. Rename → move → delete → write fd after unlink. | SQL commit fails at actor's100ms timeout; all four file operations succeed. Same file inode remains open with link count0 and content `AFTER_UNLINK`. | DB-write block does **not** revoke native file authority. Linux isolated operations; product Preview/journal pipeline, Windows delete-sharing/macOS native NOT_RUN. |
| T0-07 | Missing/corrupt/generation-mismatched activation record; reuse frozen opener. Old Unknown and new NotSent route with corrupt record; QA legacy profile and normal relaunch path. | Frozen opener creates/migrates v37 despite record faults. Both tested Direct routes actually open/migrate. QA selects old profile; normal resolver ignores experimental activation pointer. SQLite BUSY is separately observed in T0-06; frozen 5s proof is inherited from K1, not rerun. | Mandatory boundary must precede first manager/pool open; current legacy routes bypass a hypothetical later guard. No gate implemented. Source + isolated frozen-code probes; real transport/reinstall/restore/uninstall NOT_RUN. |
| T0-08 | Read only frozen S8R README, Track B fixture source, Track A report and K1 accepted JSON; build bounded dependency table. | Installation activation, DB write generation and domain publication remain distinct. No consumer inventory regenerated. | STATIC_SOURCE_FINDING/dependency contract only. C2/#381 semantic qualification and D4-S9 NOT_RUN/NOT_STARTED. |

Raw [unittest log](../../tests/issue380_k2_activation/evidence/unittest.stdout.txt), [reused shell hashes](../../tests/issue380_k2_activation/evidence/reused-shell-manifest.json), [frozen production source hashes](../../tests/issue380_k2_activation/evidence/source-manifest.json) and [dependency table](../../tests/issue380_k2_activation/evidence/dependencies.json) accompany the structured events. Hash-bound committed evidence identifies the precommit source HEAD honestly; test-input hashes bind the actual uncommitted test code. Final delivery separately records the final-HEAD targeted rerun, not a claimed committed exact-PR-head/native execution.

## Last writer, snapshot and first new writer

The last **observed** old commit is not the last **possible** old commit. T0-01/T0-05 keep the old actor unchanged and synchronize a commit after backup returns. That commit is durable in old WAL and absent from the copied generation. Source pointer switching neither transfers it nor stops later old commits. The first new write then creates two live, acknowledged durable user-state authorities.

A safe Hybrid boundary would need demonstrated exclusion of *all* old writers from before the final committed-WAL snapshot through new writer activation and subsequent lifetime. Snapshot completeness, migration seal and distinct main/WAL/SHM must be verified before activation; a sample audit count or new-only marker is not a permanent fence. The tested audit is a witness, not an activation authority.

Process exits71/72 surround pointer replacement; exit73 interrupts a real uncommitted SQL transform. Failure injection is NOT NULL violation. The SQL rollback and whole-filename outcomes do not prove power-loss durability. The model's successful transform adds only test metadata; it does not qualify any new production migration. After acknowledged new writes, selecting old data is not a lossless rollback; forward recovery or explicit reconciliation would require an Owner contract. Missing/corrupt activation records cannot authorize opening whichever profile exists. No recovery policy is implemented here.

## S / N / H feasibility and OS authority determination

| Candidate | Proven local property | Reproduced gap | Conclusion |
|---|---|---|---|
| S, shared path + stop-world | Real SQLite writer lock temporarily rejects commits. DELETE-mode DBMOVED rejects a moved connection's write. | New-only lock/record ignored; workers and late copies still open/write; lock release or same-owner permission restoration permits writes again. | Current one-time/cooperative S **NO-GO**. A properly privileged native lifetime stop-world is not proved impossible; it is **not demonstrated**. |
| N, separate namespace + snapshot/pointer | Distinct files isolate their own commits; backup/transaction atomicity and process-exit pointer outcomes hold in the model. | Aliases, missing WAL via wrong alias, snapshot-after writes, split corrections, old profile/reopen, retained ordinary-file capability. | Standalone N **NO-GO**. Filename isolation is not complete authority. |
| H, K2 Hybrid | Components remain a conditional design candidate. | No mandatory OS lifetime exclusion of unchanged old same-user writers/relaunch/native actions is present or qualified; no final-writer/no-silent-fork proof. | **OS_AUTHORITY_BLOCKED / automatic migration NO-GO**. Contract refinement may continue only by Owner decision; production GO is absent. |

Three properties must not be conflated: **DB commit exclusion**, **process startup/lifetime exclusion**, and **ordinary-file operation authority**. T0-06 directly separates the first from the third. A process name check, a new-only mutex, activation ledger, user_version or DB filename does not cover all three.

Potential external privileged process enforcement, application-control policy, broker-separated credentials, persistent ownership/ACL changes over the complete DB namespace and authorized filesystem objects, or sandbox/application entitlements are **candidates only**. None is deployed or qualified by this task. Existing descriptors, inherited rights, same-owner ability to restore modes, renamed/copied executables, old login items, cleanup/crash recovery and user consent require live native proof. Do not promote the chmod400 experiment to a complete ACL experiment, or the Python second instance to proof that the actual product plugin is bypassed.

**Determination:** under the tested ordinary same-user model and current source, continuously enforceable exclusion has not been established. If the product promises prevention of *any* old same-user binary operating on personal files, a stronger OS authority over those objects is required; this research cannot certify that promise. K2 Hybrid has enough evidence to identify contract blockers, not enough to begin automatic upgrade or production implementation.

## Bounded cross-domain dependencies

| Domain | Required in a future common migration/activation contract | Frozen evidence / remaining limitation |
|---|---|---|
| C2 Global Index | Complete sealed generation, short pointer CAS, reader/writer participation, compatible DB epoch. | Track B prototype accepted; CTAS/flags are not full relational or consumer proof. |
| #381 Root/Source Manifest | Physical/path/source epochs, seal and effective visibility in a compatible read transaction. | S8R remains frozen; no new audit or production integration. |
| Tags, corrections, history, Consent | Latest user-owned overlay survives snapshot/cutover/recovery; consent predicates remain authoritative. | K1 representative projections; K2 late-correction and two-branch counterexample. |
| IDs, rowid, FK | Stable anchors and associations across add/delete/path reuse; no generation-ambiguous history. | K1 limited preservation and K2 miniature equality, not all production identity cases. |
| Global FTS/BM25/Cursor | Full effective-visible generation corpus, rank/pagination and epoch-bound cursor semantics. | Existing limited FTS evidence; full product qualification absent. |
| GC and Recovery | Reader leases/reclamation, budgets, forward recovery, authenticated active selection and no silent old rollback. | Track B isolated controls; K2 process-exit model does not provide native durable activation. |

Installation activation, DB write generation and C2/#381 publication epochs are linked requirements, not one interchangeable version number. This task changes none of these systems.

## Verification levels, reproduction and CI

| Level / platform | Result |
|---|---|
| STATIC_SOURCE_FINDING | Bounded current/frozen source and accepted evidence read, hashes retained. |
| ISOLATED_EXECUTED_EVIDENCE / Linux | 8 new methods pass; risk witnesses and local protective failures are recorded. Native POSIX operations are actual disposable operations, not full application qualification. |
| HOSTED_WINDOWS_VERIFIED | **NOT_RUN** for K2-T0. Ordinary automatic PR CI is separate; it does not run this harness. |
| OWNER_NATIVE_VERIFIED | **NOT_RUN**. Windows SCM/UAC/NSIS/session/ACL/junction/handle semantics remain NATIVE_UNVERIFIED. |
| macOS native | **NOT_RUN**: app bundle copies, LaunchAgent/login items, sandbox/TCC/entitlements, APFS aliases and native file behavior unqualified. |
| Real power-loss tests | **NOT_RUN**. Process exit is not power loss. |

Run `python3 tests/issue380_k2_activation/run.py`. Required immutable Git objects and previously cached locked Cargo dependencies must be present. The runner builds the frozen probe shell offline, then only this eight-method suite. No fetch, workflow dispatch, prior evidence rerun, performance corpus or production configuration change occurs. Explicit JSON pipe synchronization establishes order; select/process timeouts only bound waits. Schema, snapshots, test copies, mode changes and all file actions live under owned `.tmp-tests/issue380-k2-t0/` roots with nonce and canonical path checks. Fixture paths are checked before connection/file operations; no app data or real user files are touched.

Final-HEAD logs, complete delivery patch/identity and CI observations remain task artifacts; committed evidence is remotely readable. CI's completion, skipped/failed/cancelled reasons are reported in the delivery without rerunning jobs. Natural Draft-PR CI alone is permitted; no manual Hosted/Full Validation/100k/500k/1m/Native Qualification or Codex Review.

**Stop: K2_T0_OS_AUTHORITY_BLOCKED.** #380 remains open, existing Drafts retain holds, #381 production and NSIS/U2 release remain NO-GO. No merge, release, schema allocation, production repair or next-stage work is performed.
