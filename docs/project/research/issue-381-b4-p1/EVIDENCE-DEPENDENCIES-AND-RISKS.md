# #381 B4-P1 — exact evidence ledger, dependency map and unresolved decisions

> **后续 Owner 状态对账（2026-10-14，历史资料不改写）：** 下文的起始 master `ab3915e3`、`P1A_TEST_ONLY_NOT_STARTED`、`P1A_SCOPE_PROPOSED_NOT_CODE_AUTHORIZED` 等均是**本设计形成时的历史状态**，不能作为最新执行授权或验收结论。
> 当前 master 为 `523db5e5a2e1526cd3e73f49f9842947cfd3e289`（[#410](https://github.com/ArdenZC/Zen-Canvas/pull/410) 已合并，Schema 38 未变；[post-merge CI](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38526386728) SUCCESS）。P1A 隔离测试已在 [Draft #408](https://github.com/ArdenZC/Zen-Canvas/pull/408) 执行，R1–R3 整改复审接受，**PR 仍未合并**；Hosted Windows/macOS 的 [T10-A #411](https://github.com/ArdenZC/Zen-Canvas/issues/411) 已获得真实研究证据但整体 `T10_NATIVE_GATE_BLOCKED`（Windows 目录同步、真实 ENOSPC 和断电证明仍欠缺）。[T10-B 设计 #412](https://github.com/ArdenZC/Zen-Canvas/issues/412) 已建立，[B1 Test-only 故障注入任务 #413](https://github.com/ArdenZC/Zen-Canvas/issues/413) 已授权但截至对账时未启动。**`P1A_FULL_ACCEPTANCE_NO_GO / P1B_PRODUCT_NO_GO / ISSUE_381_PRODUCT_FIX_NO_GO`。**
> 本补充只更新信息时间线，不改变下文 F1 设计候选、源码历史证据或任何实施门禁；不得把旧 Draft 文档合并视为产品功能验收。

**Source pin:** [master ab3915e3](https://github.com/ArdenZC/Zen-Canvas/commit/ab3915e3cdcc7c729317498b3b97501862832a91) / Git tree `ca22989646575bba9a7224ce10becbb89c01e0ee`, Schema38.
**Post-merge CI:** [38446614725](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38446614725), push attempt 1 SUCCESS, 19/27 successful, 8/27 conditional skips, 0 failures. Hosted Windows Workspace Foundation `managed_scan_pressure=HARD PASS`, `foreground_wait_ms=760`; observed first-page latency target **TARGET MISSED** at idle P95=193us, pressure P95=741us. This remains an observational performance gap; independent [#404](https://github.com/ArdenZC/Zen-Canvas/issues/404) preserves prior SQLite BUSY(5) cancellation fail.

## Source facts vs assumptions

| Evidence | Exact source/reference | Supported fact | Unsupported inference |
| --- | --- | --- | --- |
| P0 migration | `src-tauri/src/db/schema.rs:7,10-152,181-208,908-1003` | Four dormant tables, four indexes, strict legacy singleton check | Product read-mode generation works after setting a field |
| Product pool | `src-tauri/src/db/connection.rs:56-118, configure_connection` | 8 pooled connections, WAL, synchronous NORMAL, 5s busy, 3GB mmap cap | FULL/fsynced cross-file durability, no read/write contention |
| Library query | `src-tauri/src/db/queries/library/mod.rs:559-576,727-944,1388-1455` | Main read txn, current revision/cursor, physical file+FTS BM25 6/1.5 | Immutable E0 reader pin or globally effective corpus |
| File FTS | `src-tauri/src/db/schema.rs:3047-3110` | External-content trigram, AFTER ANY UPDATE maintenance | Zero churn under stale-bit updates |
| Scanner | `src-tauri/src/db/queries/scan.rs:1475-1705` | Live files upsert and direct global stale flag path | Root overlap atomically versioned and user-safe now |
| Independent B2 Oracle | [#395](https://github.com/ArdenZC/Zen-Canvas/pull/395) | Research fixture semantics, 16 positive + 6 negative | Full native-app integration |
| B3-F feasibility | [#399](https://github.com/ArdenZC/Zen-Canvas/pull/399) `docs/project/research/issue-381-b3f/REPORT.md` | F1 preferred in reduced bundled-SQLite Linux probe, true E0 immutability | Windows handles, crash/power-loss, p95/p99, product SourceUID validity |
| B4-01 architecture | [#400](https://github.com/ArdenZC/Zen-Canvas/pull/400) `OWNER-IDENTITY-SCHEMA-CONTRACT.md`, `CUTOVER-AND-TEST-GATES.md` | Proposed source, epoch, reader and safety invariants | Production design already implemented |
| #383 conflict | [#383](https://github.com/ArdenZC/Zen-Canvas/pull/383) | Old unapproved alternative Schema38 FTS trigger and 100k writer 5284ms > 5000ms | Mergeable or safe to port to new master |
| Global Index Service | [#398](https://github.com/ArdenZC/Zen-Canvas/pull/398), [#403](https://github.com/ArdenZC/Zen-Canvas/pull/403) | Separate #380 Service intent/revision work | Shared F1 catalog / coupling authorized |

## Explicit alternative decisions

- **A: F1 immutable per-epoch slot** — **preferred for P1A feasibility**, provided bounded copy amplification, reader pin/GC and cross-file recovery prove safe. Test-only evidence supports rank correctness; do not prematurely code product.
- **B: F2 mutable physical FTS** — rejected as default current strategy because fresh old readers lose old text candidates. Retain research result and reconsider only with a *different* immutable candidate source and qualified performance.
- **C: F0 existing FTS + post-filter / per-root BM25** — rejected as incorrect global effective-corpus ranking; no silent downgrade.
- **D: Monolithic all-in-one migration in #383** — blocked by data ownership, writer-liveness and Schema38 collision. No cherry-pick, direct merge or auto Schema39 rewrite.

## Unresolved Owner decisions before a P1B production implementation

1. **Registry and schema:** persistent slot states/leases/root writer proofs need a separate Owner-approved migration and cross-Track version allocation beyond live Schema38. No fixed Schema39 number in this PR.
2. **Atomic reader pin:** choose and prove actual multi-process main-pointer/slot pin CAS barrier; #399 per-reader `BEGIN IMMEDIATE` approach is not provisionally accepted for product.
3. **Source eligibility:** P0 has no actual RootEpoch claims, verified attribution/writer coverage or immutable root union. Which P2 writes create the trustworthy corpus and at what authoritative snapshot? Refuse shadow-product equivalence until proven.
4. **Durability:** Windows NTFS rename/delete/share mode, macOS APFS directory durability, SQLite checkpoint/sync and full backup set need real tests. Linux SIGKILL is only partial.
5. **Disk/resource budget:** measure full-slot copy and retained E0 pin growth at 100k, 500k and 1m scale where required; account for watcher churn and real app pool, do not infer p95 from one synthetic sample.
6. **Reader and token compatibility:** bind slot/corpus/rank digest to main transaction, scoped count, tag selection, cursor and text. Do not present results as active until P2 cutover/rollback proof.
7. **Security and governance:** slot contains user-sensitive names and paths even without user content. Explicit access control, path-redacted logs, lifecycle on uninstall, backup/restore consistency and no default indexing of OS/install/dependency trees.
8. **Existing latency debt:** keep [#404](https://github.com/ArdenZC/Zen-Canvas/issues/404) independent; investigate SQL busy owner before any claim that new pin transactions won't worsen it.

## Ordered phase gates

`P0 merged, Schema38 & push CI accepted` → `P1 Owner slot-design gate (this docs-only PR)` → `P1A isolated F1 durability/reader-pin evidence` → `P1B **separately authorized** dormant product slot adapter (if justified; schema version arbitrated)` → `P2 SourceUID/RootEpoch verified writers + effective projection and reader/cursor/operation CAS` → `P3 native cross-platform + crash/performance qualification` → `full #381 product GO only upon independent Owner acceptance`.

**Stop state:** `B4_P1_DESIGN_READY_FOR_OWNER_REVIEW / P1A_TEST_ONLY_NOT_STARTED / P1B_PRODUCT_NO_GO / ISSUE_381_PRODUCT_FIX_NO_GO`. No semantic claim that a docs-only green CI qualifies any product behavior. No additional master, PR or Issue mutation without new Owner authorization.
