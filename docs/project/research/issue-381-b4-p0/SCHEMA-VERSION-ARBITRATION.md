# #381-B4-P0 — Source-verified Schema authority and conflict resolution

| Track / ref | Current state | Source-level Schema mutation | Owner resolution |
| --- | --- | --- | --- |
| master 3275ac7c72df8ae4139a30f16c7f649971324e97 | Schema37, active sole product base | schema.rs:7 version37; latest migration <37 schema automation triggers | source of truth for P0 |
| #383 HEAD 584621c32054e16ac0a61afc669a04380a976b7f | OPEN/Draft/DESIGN BLOCKED | provisional v38 + replace files_au UPDATE trigger; still long bulk-stale txn | **cannot serve as shared Schema38 authority**; do not merge/rebase into P0; retire/reallocate its v38 before any successor |
| #398 HEAD 1acb65fa8623d9f52292cf8c6d1a321e2cbadc32 | OPEN/Draft, #380 IPC P1 | no production Schema file changed at inspection | no current conflict; #380 future P2 must coordinate new master |
| #399 HEAD 93e38861ff39ee0e0bfd4cbd094d999e60a425b0 | test-only FTS | independent bundled 3.51.3 experiment, no product Schema | research baseline only |
| #400 HEAD 609a75a3b81eec13e25e4967dec2fd8ffc97a01c | OPEN/Draft docs | B4-01 logical model only | upstream design source for P0 |

**Owner decision 2026-10-13:** single conditional candidate slot `Schema38 = #381-B4-P0 expand-only identity foundation` *only while master remains the inspected Schema37*. It is **not** a second allocation to the same user_version and not an invitation to merge P0 automatically. #383 provisional v38 is withdrawn as an authorized migration direction; PR #383 itself remains Draft, its diagnostic evidence preserved, and its stale/FTS implementation stays blocked. The #383 FTS trigger optimization must be reconciled against F1 and assigned a subsequent unique version *if still needed*.

**Critical incompatibility:** a throwaway dev DB run from #383 may already have pragma user_version=38 but NO library source foundation. Fast path in master-derived P0 must error read-only rather than silently filling missing tables. Future explicit salvage/migration for developer fixtures is a separately signed task, never automatic overwrite. A newer master Schema38 from another authorized track would require cancel/rebase and new migration-number decision before P0 can proceed.

**No release timetable**, no installed public older clients; this design protects data from prior internal development builds and real current files without inventing customer upgrade-matrix work.
