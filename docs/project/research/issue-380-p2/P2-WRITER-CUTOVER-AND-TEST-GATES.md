# #380-P2 — Exact writer classes and test/implementation gates

**Owner source snapshot:** master `3275ac7c72df8ae4139a30f16c7f649971324e97`, Schema37; #398 unmerged P1 scoped-accepted. This ledger is a source-grounded **priority inventory, not an exhaustive proof** of every indirect SQL writer. Production P2-P1 is NO-GO until affected writer callpaths and SQL triggers are traced.

## Twelve mandatory tests for later production candidates (none executed here)

| Case | Verification / observable failure that must be caught |
| --- | --- |
| P2-T01 | User Rebuild(one): intent durable without changing source status/error/checkpoint/active rows/AI, then ServiceBusy zero side effects. |
| P2-T02 | Rebuild(all): snapshot per-source pending intent, source enabled/disabled/removed changes; coalescing preserves latest generation per source. |
| P2-T03 | Legacy true native RebuildRequired remains a **health fault** independent of user pending; permission/user-corrected status not hidden. |
| P2-T04 | Two independent SQLite writer connections claim same source; exactly one CAS wins; after token epoch increment old one cannot write, including equal-status ABA. |
| P2-T05 | WorkScheduler resource admission precedes ServiceBusy; no speculative INDEXING/SYNCING/READY/ERROR or cleared last_error. |
| P2-T06 | Source disable/permission/topology changes while Service stream in flight; old owner cannot re-enable or claim READY; disabled policy still respected. |
| P2-T07 | Every Sink effect — Batch, EntryStale, VolumeStale, Provider, Checkpoint, State — rejects stale owner within same transaction; no partial managed/AI side effect. |
| P2-T08 | Triggered global_volume entry_count increments and bootstrap recompute under competing owner; not a hidden unfenced entry publication. |
| P2-T09 | Genuine provider/OS/SQLite_BUSY vs request-level ServiceBusy and UnknownOutcome; correct durable health and pending intent without false fallback. |
| P2-T10 | Service partial stream then crash/restart; stale token fenced, new owner can reconcile without replaying unsafe Direct, durable pending survives. |
| P2-T11 | Managed scope/policy, user correction and AI job claim/completion concurrently with scan; no user fact rollback or old-AI reactivation. |
| P2-T12 | Actual P1 plus P2 merged-tree integration test under bundled Rust SQLite 3.51.3, Windows Service runtime and macOS; test SourceHealth DTO/revision, repeated writer contention and unchanged 5000ms timeout. Do not claim P3 full snapshot/100k safety until proven. |

## Writer mapping needed before production owner migration

| Class | Source path/call | Durable effects | Proposed ownership |
| --- | --- | --- | --- |
| INTENT-PUBLIC | `coordinator.rs:218-256` | provider/status/error and whole-volume stale pre-admission | REPLACE with durable user intent; no health/data mutation |
| COORD-TOPOLOGY | `coordinator.rs:447-529` | discover/upsert/provider, legacy-fallback RebuildRequired, Unavailable and missing-source stale | separate topology/revoked-access writer; cannot overwrite newer operation/policy |
| COORD-PREWORK | `coordinator.rs:545-562` | speculative INDEXING/SYNCING and optional last_error clearing | write only after verified Service + DB claim |
| COORD-TERMINAL | `coordinator.rs:574-662` | READY/PAUSED/ERROR/recovery | operation CAS, do not restore old status by blind update |
| SINK-STATE | `coordinator.rs:315-352` | checkpoint state/provider | per-operation same-transaction CAS |
| SINK-ENTRY | `coordinator.rs:303-313,379-382` | Batch/stale and derived invalidation | entry/source ownership + short same-txn CAS, P3 visible atomicity pending |
| REPO-HEALTH | `repository.rs:207-259` | unconditional volume source state/enabled/provider | split health/op/policy/topology writer categories and CAS |
| REPO-BATCH | `repository.rs:262-339` | entries INSERT/UPDATE and AI enqueue transaction | writer CAS inside batch transaction; no prior writer from another generation |
| REPO-MASS-STALE | `repository.rs:341-392` | global_entries, managed_entries, AI jobs/items/state, entry_count across separate statements | pre-admission forbidden; P3 bounded generation publish, not status-only change |
| SCHEMA-TRIGGER | `db/schema.rs:1821-1849,1872-1876` | entry count implicit trigger mutations + startup recompute | preserve correct counter origin/transaction; annotate maintenance writer |
| MANAGED-POLICY | `global_index/managed_scope.rs:25-160+` | managed user scopes, policy, AI status independently | user/policy writer authority, NOT Rebuild owner |
| MANAGED-AI | `managed_worker_hardened.rs:60-120+` | AI eligibility/queue/retry/analysis | per-entry policy/source fingerprint + operation-derived visibility |
| LEGACY-AI | `legacy_queue.rs:14-137` | File Library bridged queue/cancel | user intent independent of source rebuild |
| SEARCH-DTO | `commands.rs:10-46,72-120` and `repository.rs:696+` | reader source health+revision and complete/partial | query-bound consistent snapshot, monotonic operation owner separate from digest |
| WINDOWS-P1 | #398 `windows/mod.rs` and `service_host.rs` | verified Service admission/busy, transient Direct fence | accepted but unmerged; P2 adds persistent ownership, not IPC v4 |

**Evidence type:** current production source statements above reviewed; indirect Triggers/SQL callpaths requiring runtime tests are not certified exhaustive. Never treat test-only diagnostic functions, pre-release compatibility investigations, or unrelated Managed File Library writers as Global Index operation owners.

## Hard gates before implementation permission

1. Live master and product current Schema version, #398 P1 merge/integration-branch decision, #381 B4-P0 v38 version collision resolved. New unique P2 migration number allocated only by Owner on that refreshed tree.
2. Exact minimal DB data representation for intent, source epoch/owner, and crash recovery that **cannot be bypassed** by old repository public APIs and triggered updates; preservation plan for legacy source error/AI user corrections.
3. Clear P2-P0 *dormant* production files allowlist and P2-P1 writer-cutover files allowlist; no broad behavior switch/FTS/managed Library migration.
4. Run T01–T12 on real build after code, exact-head normal CI, actual hosted Windows Service qualification and later Owner hardware acceptance. Missing test is a stated NO-GO, not waived.
5. Every stage stops at OPEN/Draft + exact HEAD, Tree, changed Diff, source line evidence, raw test/CI and unresolved risks; **never automatically merge or release**.

**Recommendation:** Owner should use this finite scope contract and then authorize P2-P0 when schema/parent constraints are resolved, rather than another broad A/B/C architecture study.
