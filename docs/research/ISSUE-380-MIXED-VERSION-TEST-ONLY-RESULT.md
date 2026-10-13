# #380 Mixed-Version / Service Admission — test-only result

**#380-MIXED_VERSION_TEST_ONLY_READY_FOR_OWNER_REVIEW**

**Production disposition: NO-GO for mixed-version rollout and destructive Rebuild.** Passing characterization tests reproduce unsafe behavior; they do not certify compatibility or repair #380.

Authority: [Owner parallel decision 6137395908](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6137395908), read as the latest comment on 2026-10-13. Local independent branch: `test/issue-380-mixed-version`. No production/IPC/Installer/Schema edit; #385/#386 and master are unchanged. No Hosted run, Full CI, Codex Review, merge or release was started. The branch remains local to avoid triggering Hosted PR/push workflows.

| Frozen input | HEAD | Tree |
|---|---|---|
| Latest master / Old Desktop / Old Service | `3275ac7c72df8ae4139a30f16c7f649971324e97` | `009576241cd4e8e2ba12cc644a368314f599faa6` |
| #385 D0 reference contract | `8ab50cb627712abef8debfa786d9fb2354ef9581` | `84405758645a126f7f8d19f3397c106c604fef2b` |
| #386 D1-A / New Desktop / New Service | `d260131f9d6ec354f2361e6c7460c79e62e38391` | `4765285668095fb3d7c0dcefd8a8ba5f2bd4fb69` |

“New” means this frozen D1-A candidate, not an implemented D0 A+B+C architecture. Master was rechecked with `git ls-remote`. Final test-only HEAD/Tree/diff identities are in the delivery record; embedding a document's own tree hash would change that tree.

## Complete combination matrix

The ServiceRuntime and client validators execute frozen production function bodies. The host uses a synthetic enumerator; frames are serialized with each generation's real Serde definitions and decoded by the opposite client. “Stream writes” below exclude the separately executed Coordinator Rebuild prefix.

| Desktop × Service | Successful stream | Busy before admission | Rebuild before admission | Post-send disconnect / sink error | Production gate |
|---|---|---|---|---|---|
| Old × Old | Accepted; provider/stale/checkpoint applied | `Provider` error, no stream writes | REBUILD_REQUIRED, stale entries, Managed disabled, AI stale, count 0 | UNAVAILABLE then Direct replay, including real SQLite sink BUSY | NO-GO |
| New × Old | `ServiceAdmissionMissing`; zero stream writes, no Direct | `ServiceBusy`, no stream writes | Same destructive prefix already committed | Unknown/partial-send refuses Direct; missing admission is only the narrow first-event case | NO-GO |
| Old × New | Accepted; new Sources admission ignored by old client | `Provider` error, no stream writes | Same destructive prefix already committed | Old client still replays; request/protocol/source identities remain unguarded | NO-GO |
| New × New | Trusted admission accepted; active status then provider/stale/checkpoint applied | `ServiceBusy`, no stream writes | Same destructive prefix already committed | Unknown forbids Direct, but already applied stale/checkpoint is not rolled back | NO-GO for destructive Rebuild / rollout |

Old busy → source ERROR and new busy/missing-admission → preserved current source state are **Coordinator cycle source findings**, not a full worker/native execution claim. The actual executed Rebuild prefix proves that preserving its *current* state cannot restore pre-Rebuild data.

## Executed evidence and counterexamples

- **13 Rust tests passed; 0 failed/ignored.** Both real Coordinator::rebuild bodies and three real Repository update functions execute against SQLite. All eight desktop × enabled/disabled × targeted/all cases stale before the `start()` seam. Checkpoint, AI summary and user-corrected flag survive; active search visibility, Managed eligibility, AI statuses and count do not.
- **6 Python tests passed; 0 failed**, with bounded subcases: four combinations, 24 SQL failpoints (2 desktops × 2 journal modes × 6 statements), four child exits, two actual writer locks, five installer launch schedules and four recovery source/model cases.
- Both real Service admission CAS implementations admit exactly one of eight concurrent contenders. Busy is a request rejection; SQLite BUSY is a separate failure. Real second-connection writer contention returns SQLite code 5. Python uses a deliberately short **50 ms test timeout**, not a measurement of production's 5 s liveness; Rust sink error is obtained from a real lock with a 20 ms test timeout. D1-A retains Database/SqliteFailure and never Direct-falls back; old code converts the sink error into a failed stream and executes Direct once.
- New client rejects wrong event request IDs before sink writes and rejects source-mismatched admission. Wrong terminal response request/protocol is rejected **after prior valid events were already applied**. Old client accepts those frames. IPC v3 requests have only protocol_version/request_id/command/source; no client-generation/capability requirement. New EntryStale's additive volume_id deserializes in the old client and is ignored. Parsing compatibility is not semantic isolation.
- Two separate child processes execute the identical request through both frozen Service hosts and produce identical successful frames; fresh Service runtimes also execute the same request twice after reset; both also execute duplicate request IDs in one runtime. A fresh new client validator accepts a correctly identified old stream again. Matching a request ID is not durable replay/generation fencing.
- Before-dispatch cancellation returns Paused in the new production test entrypoint; old event application has no per-event cancellation check. Cancelling the Service at its first event causes its real PipeSink to stop with index_paused. A later request runs again. This does not prove SCM worker termination. Disconnect after admission/provider/stale yields old Direct replay and new Unknown with stale retained. Partial request write and post-send failure before the first event are Unknown for new code. Proven NotSent allows Direct in both versions.
- Exact Repository stale SQL executes as six autocommit statements. Injected ABORT at each statement leaves the prior committed prefix, including after connection reopen. The count trigger participates in the first entry-stale statement. A child exits with code 73 after two committed statements; stale entries/disabled Managed remain in WAL and rollback modes. FTS retains its raw token hit while the active `is_stale=0` search join returns no hit. No test adds a wrapping transaction to make the old behavior look safe.

## Installer race boundary

Frozen installer source stops/waits for the exact-owned Service, closes Desktop, replaces files, then converges Service startup. The gate is a FindProcess/KillProcess observation, not a generation attestation. Named-pipe identity compares the client executable path with current_exe; same canonical path does not identify the loaded generation.

The bounded executable interleaving model inserts one canonical Desktop launch at each of five checkpoints and continues installer actions. Launch while Service is stopped can enter the tested NotSent → Direct path. Launch **after the Desktop-close check but before replacement** leaves an old process alongside the new Service after restart. Launch after replacement but before Service startup also sees NotSent. Pre-mutation cancellation has a reversible service-restore source path; after file mutation the partial-failure path keeps a captured repair Service stopped/compensates new service, without proving that a Desktop cannot launch and Direct-index.

These are source-bound model counterexamples, **not executed NSIS, SCM or Windows file-replacement evidence**. The model assumes a launch is allowed at the inspected boundary; actual loaded-image replacement restrictions, same-path launch, service stop completion, cancellation and rollback remain native UNVERIFIED. Those missing guarantees are blocking, not PASS exceptions.

## Reproduce / review

Run `python3 tests/issue380_mixed_version/run.py` from the repository. It reads immutable Git objects, extracts functions without rewriting their bodies, and creates a minimal locked Cargo harness under ignored `.tmp-tests/issue380-mixed-version/`. `cargo test --offline --locked` and Python stdlib SQLite/unittest run there; fixtures are temporary and never use user databases. Missing Git objects or cached locked dependencies fail instead of fetching or changing the tested peers. For a fresh environment, fetch the frozen PR objects and prime the locked dependencies separately before offline execution.

Environment: Linux x86_64, Rust 1.97.1; bundled rusqlite 0.39 SQLite version is printed in the Rust log; Python SQLite **3.53.1**. Initial dependency cache preparation used crates.io; all recorded acceptance tests execute offline. The harness bypasses Tauri platform dependencies, not the selected functions' behavior. No NTFS/MFT provider, named pipe, process token check, real SCM restart, Windows installer, full app migration, production 5 s writer-budget or D0 generation publish acceptance is claimed.

Raw logs, frozen source hashes and structured observations: [evidence](../../tests/issue380_mixed_version/evidence/). Final rerun identity and logs are retained in the ignored output directory. Test glue replaces transport, enumeration, worker scheduling and DB connection acquisition only; each production function slice remains byte-for-byte from its frozen object. Old error enums, stream loop, event handler, host/PipeSink and Rebuild functions retain their actual risk.

Owner production blockers remain: admission-before-destructive-Rebuild; durable ownership/replay/generation fencing; service-side generation/capability rejection or a proven installation/startup exclusion boundary; suppression of Direct during transition; coherent recovery and native evidence. This task supplies decision inputs and implements none of those production policies. #385 merge hold, #386 independent production merge NO-GO, Schema v38 unallocated and NSIS U2 release NO-GO remain in force.
