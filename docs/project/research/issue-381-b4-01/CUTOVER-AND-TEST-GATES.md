# #381 B4-01 — Gate-by-gate acceptance and rollback ledger

This checklist accompanies OWNER-IDENTITY-SCHEMA-CONTRACT.md. It is a **design-stage stop/go contract**, not a list of tests already executed. No Schema version allocated.

| Gate | Before permitting the next phase | Automatic NO-GO evidence |
| --- | --- | --- |
| G00 source truth | Exact new master HEAD/Tree, table/FK/trigger inventory, affected callsite map, precise old/new authority read semantics | Historical frozen IDs misrepresented as production callers |
| G01 baseline | Read-only backup; integrity/FK/FTS checks; counts and stable hashes of legacy files, tags, fingerprint, content, duplicate, plans, operation journals | Missing user facts, corrupt FK, unexplained count drift |
| G02 identity | Cross-platform source UID/incarnation vs path alias; rename/hardlink/replace/low-confidence proof; unresolved legacy facts retained | Same path replacement inherits tags/AI/operation authority |
| G03 E0/E1 | Immutable file text/path/mtime+root union, disabled/degraded roots, Watcher/source rev, candidate stage seal; no E0 text mutation | E0 fresh query silently reads E1 text or root A deletes root B |
| G04 F1 corpus | Actual bundled FTS5 trigram effective-global BM25 float bits, ties, scope before LIMIT, short-query legacy branch, cursor/query revision | F0 physical BM25 substitution or post-LIMIT underfill |
| G05 writer coverage | Every affected Scan/Watcher/operation/restore/tag/AI/dedupe/content owner holds and checks fencing token with source UID and revisions | Any legitimate source writer can UPDATE global files/stale independently and claim new authority |
| G06 cross-file durability | Slots sealed/fsynced/bound to main pointer; fault matrix slot-before-manifest, before/after COMMIT, disk-full, power failure, missing slot, backup/restore | Pointer references missing/partial slot while rendering READY/actionable |
| G07 monotonic readers | Real pooled read transaction pins matching slot/metadata epoch; reader expiration and GC preserve all in-flight refs | Old cursor replay, pinned reader reads changed text, Windows busy handle unlinked |
| G08 no old data deletion | file_user_tags, file_fingerprints, duplicate_group_members, content items/artifacts, organization snapshots and journals retained exactly or explicitly adjudicated | CASCADE deletes or unexplained loss during migration/backfill |
| G09 scaled writers | Product-bundled 100k Hosted Windows competing source/tag writers and pooled reads, real SQLite_BUSY vs Service Busy; unchanged 5000ms | Long publication lock or unbounded GC/FTS trigger rewrite |
| G10 native lifecycle | Windows reparse/NTFS locks/delete-on-close and macOS APFS/provider/iCloud permission/identity behavior under real native acceptance | Assumption from Linux identity, POSIX unlink, power-loss or copy costs |
| G11 rollback | Rollback to proven intact previous slot only via revision CAS; post-commit faults preserve newer metadata/user writer, no blind restore | Any recovery overwrites newer observed UID or user correction |

## 14 exact regression groups for later staged Codex PRs (NOT run)

- \`B4-T01\` empty first bootstrap + old Schema37 backup integrity.
- \`B4-T02\` 100k repeatable batch migration without 100k SQLite writer transaction.
- \`B4-T03\` legacy file row with tag, note, fingerprint, content artifact, organization snapshot survives unknown UID binding, without automatic inheritance.
- \`B4-T04\` verified same object rename/hardlink alias vs proven same-path replacement new incarnation.
- \`B4-T05\` unresolved/conflicting OS identity fails closed but preserves legacy facts.
- \`B4-T06\` root A/B overlap and disable/degrade/permission/Watcher loss.
- \`B4-T07\` scan E1 batch-upserts cannot mutate E0 published metadata/FTS/cursor.
- \`B4-T08\` concurrent tag/restore/operation/AI/copy-phase writer wins source CAS; stale seal/publish rejected.
- \`B4-T09\` source delete/recreate/path reuse; no old tag, preview, action or AI authority leaks.
- \`B4-T10\` independent visible-only trigram BM25 float bits/IDs/pagination/legacy short LIKE across global and scoped roots.
- \`B4-T11\` old E0 readers pin slot while E1 publishes; cursor/count cached state revokes correctly.
- \`B4-T12\` SIGKILL, power cut, disk full and corrupted/missing slot before/after pointer; true no-ghost action policy.
- \`B4-T13\` bounded GC/reclamation with Windows open slots, macOS filesystem semantics, boot repair; no deletion of user facts.
- \`B4-T14\` 100k Hosted Windows real competing writer/source/tag, pooled query/FTS, disk/memory and long-lived reader bounds; percentiles only from enough samples.

## Finite dependency plan

\`Owner B4-01 source/schema decision\` → \`Codex B4-P0 expand-only schema & migration (explicit Owner version GO)\` → \`Codex B4-P1 F1 slot writer/read pin\` → \`Codex B4-P2 root/UID+Managed callers and async workers cutover\` → \`Owner B4-P3 Windows/macOS native qualification\` → \`independent final safe cleanup GO\`.

At each phase: independent Draft PR, exact HEAD/Tree/Diff/raw tests, normal CI, explicit remaining assumptions. Never merge automatically, backfill in user native Owner acceptance PC or run Codex Review. Full product #381 remains NO-GO until all contracts are green.
