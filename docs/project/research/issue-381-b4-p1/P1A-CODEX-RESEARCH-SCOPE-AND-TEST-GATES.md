# #381 B4-P1A — proposed Codex Cloud test-only F1 durability & pin qualification

> **后续 Owner 状态对账（2026-10-14，历史资料不改写）：** 下文的起始 master `ab3915e3`、`P1A_TEST_ONLY_NOT_STARTED`、`P1A_SCOPE_PROPOSED_NOT_CODE_AUTHORIZED` 等均是**本设计形成时的历史状态**，不能作为最新执行授权或验收结论。
> 当前 master 为 `523db5e5a2e1526cd3e73f49f9842947cfd3e289`（[#410](https://github.com/ArdenZC/Zen-Canvas/pull/410) 已合并，Schema 38 未变；[post-merge CI](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38526386728) SUCCESS）。P1A 隔离测试已在 [Draft #408](https://github.com/ArdenZC/Zen-Canvas/pull/408) 执行，R1–R3 整改复审接受，**PR 仍未合并**；Hosted Windows/macOS 的 [T10-A #411](https://github.com/ArdenZC/Zen-Canvas/issues/411) 已获得真实研究证据但整体 `T10_NATIVE_GATE_BLOCKED`（Windows 目录同步、真实 ENOSPC 和断电证明仍欠缺）。[T10-B 设计 #412](https://github.com/ArdenZC/Zen-Canvas/issues/412) 已建立，[B1 Test-only 故障注入任务 #413](https://github.com/ArdenZC/Zen-Canvas/issues/413) 已授权但截至对账时未启动。**`P1A_FULL_ACCEPTANCE_NO_GO / P1B_PRODUCT_NO_GO / ISSUE_381_PRODUCT_FIX_NO_GO`。**
> 本补充只更新信息时间线，不改变下文 F1 设计候选、源码历史证据或任何实施门禁；不得把旧 Draft 文档合并视为产品功能验收。

**Status:** `P1A_SCOPE_PROPOSED_NOT_CODE_AUTHORIZED`. Owner design candidate, not a Codex execution request and not #381 production GO. **Base:** master `ab3915e3cdcc7c729317498b3b97501862832a91` / Schema38; recheck live master at launch and stop for any schema/architecture change. Dependencies: F1 preferred [#399](https://github.com/ArdenZC/Zen-Canvas/pull/399), source contract [#400](https://github.com/ArdenZC/Zen-Canvas/pull/400), P0 merged [#402](https://github.com/ArdenZC/Zen-Canvas/pull/402).

## Narrow scope

Proposed new isolated test harness under `tests/issue381_b4_p1a_f1/` with executable Rust (matching pinned product rusqlite/libsqlite3-sys and FTS5) plus fixture files and docs under `docs/project/research/issue-381-b4-p1a/`. Optional isolated CI launch follows existing routing (normal PR CI); additional manual performance workflow only with Owner authorization. **DO NOT modify any production code, production schema, Cargo root manifest/lock, CI/workflow, app startup, scanner, watcher, File Library, IPC or platform install.** Reuse #399 as *reference evidence*, not by silently modifying its frozen corpus/results. Include independent oracle and exact product-lock parity proof, not an unverified copy of a helper function.

P1A input is an explicitly labelled synthetic snapshot of `UID+observation+immutable text+root claims`, not real user or product-authoritative index data. P1A's independent Oracle defines correct effective-visible global FTS rank/count/page/cursor. Versioned manifest, SHA-256, FTS integrity, read handles and lease/GC barriers must be observable. The test harness may freely fault disposable files. It **must never** mutate a real user's DB or a production `library_generation_authority` singleton.

## Mandatory negative/positive matrix, all require raw evidence

| Gate | Test scenario and independent assertion |
| --- | --- |
| P1A-T01 | Binary/feature/source hash identifies actual SQLite 3.51.3, product rust/rusqlite lock and trigram support; no system SQLite substitution |
| P1A-T02 | 0, 10k, 100k documents; exact effective-global corpus across overlapping/disabled/degraded roots; UID/observation immutable name/path; dedup/low-value exclusion policy is fixture-labelled, not guessed |
| P1A-T03 | F1 vs independent reference full sorted IDs, BM25 float bits, 6.0/1.5 weights, global effective df, ties, root filter-before-LIMIT, Library short-trigram vs legacy escaped-LIKE distinction |
| P1A-T04 | Publish E0 then stage E1 changed name/path; fresh E0 reader and existing pinned E0 retain E0 text/rank/cursor while candidate E1 remains unpublished |
| P1A-T05 | Reader pin vs retirement/GC race in every interleaving: GC before pin, pin before GC, open failure after registration, released/expired/crashed reader, restarted process and lease replay; no missing-slot-success |
| P1A-T06 | Pooled connections (product max 8), multiple read-only handles, writer cancellation and 5,000ms BUSY boundary: prove no per-request `BEGIN IMMEDIATE` bottleneck or explicitly classify design BLOCKED |
| P1A-T07 | FTS finalized and checkpointed, writable handles closed, `integrity_check`+FTS integrity pass, manifest content digest and *actual final bytes* SHA match on reopen; tampered file/manifest rejects |
| P1A-T08 | Child-process terminate at open/write/seal/sync/manifest/barrier/GC points; explicit old-or-complete-new states only; no half-published pointer or lost tag/user-data fixture |
| P1A-T09 | Model disk-full, partial write and interrupted backup; maintain old slot, classify unavailability; never try to recover by recreating arbitrary old `files_fts` results |
| P1A-T10 | On real Hosted Windows: open-handle delete/rename, reader-pin lifetime and crash/restart; on Hosted macOS: rename/permission/reopen. If runners insufficient mark `ENVIRONMENT_UNAVAILABLE`, not PASS |
| P1A-T11 | Main vs sidecar durability: simulated pointer-before/after-commit, sealed-before-pointer, recovery orphan and pointer-to-missing-slot; fail closed. This is test-only pointer simulation, not product `generation` activation |
| P1A-T12 | Record initial/copy/delta size/latency, WAL, disk amplification, checkpoint duration, p50/p95/p99 with adequate raw samples and actual 100k competing writer occupancy; no invented percentiles |
| P1A-N01 | F0 physical BM25/post-LIMIT underfill proves negative; do NOT silently change rank or accept post-LIMIT filtering |
| P1A-N02 | Mutable F2 fresh-E0 candidate loss remains rejected; no unqualified custom scorer fallback |
| P1A-N03 | Unverified same-path replacement never inherits user tags/operations/action authority; path/size/mtime hashes not promoted to VERIFIED |
| P1A-N04 | Expired/replayed cursor, reused slot ID, falsified manifest/root revision, stale CAS, GC double-deletion fail safely and preserve newer facts |

Every test must report observed **PASS / EXPECTED_REJECTION / FAIL / ENVIRONMENT_UNAVAILABLE** and reason; negative counterexamples must be confirmed rejected, not counted as a product success. Record version and `sqlite_source_id`, actual commands/exits, source SHA-256, OS/FS types, FTS counts/digests, stdout/stderr, exact HEAD/Tree and any CI run/skips. No `PASS` from mocking the file-system semantics that the case is supposed to verify.

## GO / NO-GO after research

If gates are green and Windows/macOS test environment genuinely exercises required file-handle and durability semantics, recommend a **new separate Owner P1B product adapter scope**. If F1 fails cost/GC/reader liveness or OS proof, return a precise `B4_P1A_BLOCKED` issue and compare smaller lifecycle/storage alternatives without altering effective-visible ranking or quietly relaxing 5s writer guard. Neither P1A nor P1B is permission for generation cutover: P2 RootEpoch/UID writer coverage and P3 native qualification remain separate.

Hard limits: One new independent **OPEN/Draft PR**, test-only, no merge/master write/Issue closure/Release/Codex Review, no actual Windows Owner workstation development. Stop at `B4_P1A_TEST_ONLY_READY_FOR_OWNER_REVIEW` only on verified evidence.
