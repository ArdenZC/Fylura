# Issue #366：macOS Hosted managed-scan admission 超时调查

**调查结论：两个阶段化 macOS D2 样本确认单次 SQLite `BEGIN IMMEDIATE` 占用 8.827–9.148 秒墙钟；SQLite 的 5 秒 busy timeout 是累计请求睡眠预算，不是墙钟截止时间。D3 在四轮 Native macOS Hosted CI 中以 9.035–9.232 秒通过，距原 10 秒 watchdog 仅 0.768–0.965 秒。D2 已由 Owner 接受 test-harness 修复；本次只把同一 test-only fixture 方案用于 D3：先完成 durable admission、claim 和 batch 数据初始化，再验证池内每个连接的 5 秒默认值，最后仅对该测试数据库设为 500ms。两个测试都保留 10 秒 watchdog 和原子性断言。OS 额外墙钟的精确分摊仍未测得；本次 exact-head CI 在以下代码及报告提交时待验证。**

- 起始 `origin/master`：`58062c5c356969f332f19c7458028bf2e097595e`
- 首次阶段诊断代码头：`3c96555aa9046665451985f14e3e9df9ce6096be`
- 分支：`investigate/issue-366-macos-managed-scan-busy-timeout`
- Draft PR：[ #367 ](https://github.com/ArdenZC/Zen-Canvas/pull/367)
- Issue：[ #366 ](https://github.com/ArdenZC/Zen-Canvas/issues/366)
- Owner 授权：[Issue comment 6100291859](https://github.com/ArdenZC/Zen-Canvas/issues/366#issuecomment-6100291859)
- D2 接受及 D3 补充要求：[PR review comment 6101274015](https://github.com/ArdenZC/Zen-Canvas/pull/367#issuecomment-6101274015)

PR #367 和 Issue #366 保持 OPEN；PR 保持 Draft。D2 逻辑不重查、不重写，仅修正其过时注释。本轮只修改 D3 test-only fixture 与本报告；未修改生产 SQLite busy timeout、SQL、scan admission、scheduler、Schema、IPC、Global Search 或其他 track。没有运行 Codex Review、500k/1m benchmark 或 Full Validation。

## 1. 范围与证据等级

目标测试是：

`db::queries::scan::tests::managed_scan_admission_fails_closed_after_busy_timeout_without_partial_authority`

本报告区分三类证据：

1. **历史失败记录**：原始 macOS job 在无阶段日志时超出 10 秒 channel receive deadline；这些失败保持原样，没有被后续成功覆盖。
2. **本轮 Hosted 实测**：两个 macOS arm64 Hosted run 对 D2 生产默认 `busy_timeout=5000ms` 执行目标测试，获得 method、SQLite transaction、线程 CPU、send/receive 的共同时间线；另有四轮 Native macOS CI 为 D3 在原 5000ms 默认值下留下独立 wall-time 记录。D3 修复后的 exact-head CI 结果将在对应 PR 证据评论记录。
3. **源码可证实机制**：仓库锁定的 rusqlite 和 bundled SQLite 源码说明 timeout 是按累计请求 sleep 时长工作，而不是严格的 monotonic 墙钟截止时间。该源码机制不能单独证明 Hosted 内核每次 sleep 的实际时长。

## 2. 实际代码路径

在基线 master `58062c5c356969f332f19c7458028bf2e097595e` 中：

- [`src-tauri/src/db/connection.rs`](../../../src-tauri/src/db/connection.rs) 的 `configure_connection` 对每个 SQLite 连接调用 `conn.busy_timeout(Duration::from_secs(5))`；连接池是 r2d2 SQLite pool。
- [`src-tauri/src/db/queries/scan.rs`](../../../src-tauri/src/db/queries/scan.rs) 的 `Database::admit_managed_scan` 先解析 roots、计算 canonical request hash、取得池连接，再开始立即事务。该同步 admission 路径不调用 `WorkScheduler`。
- `begin_managed_scan_write_transaction!` 调用 `transaction_with_behavior(TransactionBehavior::Immediate)`。
- 锁定的 `rusqlite 0.39.0` 在 `Transaction::new_unchecked` 中把 `Immediate` 映射为一条 `BEGIN IMMEDIATE`，经 `conn.execute_batch(query)` 执行。此 admission 路径没有应用层循环或重试。
- 测试中的 `HeldManagedScanWriter` 使用一个池连接持有真实 `BEGIN IMMEDIATE`，正常通过路径在读出 root/session/run authority 计数后释放并 join writer，再 join contender。holder 另有 20 秒安全释放。
- 测试对 contender 结果 channel 保持原有 10 秒 receive deadline；检测到 timeout 时仍失败，先释放 writer，再仅额外等待 3 秒以记录 contender 后续结果，不把超时转换为成功。
- D3 `managed_scan_batch_write_returns_busy_atomically_and_succeeds_after_release` 先建立 managed root、durable admission、queued-run claim 和待写 `InsertFileRequest`，之后才调用既有 `#[cfg(test)]` pool helper。helper 逐一借出池内所有连接，验证每个连接当前 `PRAGMA busy_timeout=5000ms`，再为该 D3 数据库全部连接设置 500ms；随后才开启真实 held writer。该测试的 10 秒 result-channel watchdog、20 秒 holder safety release、BUSY/rollback/无部分写入/版本不变、释放 writer 后重试成功及线程 join 均保留。
- `scripts/performanceManifest.mjs` 将该 ignored 测试列为 `workspace_foundation_sqlite_admission_timeout`；macOS Native Performance job 使用精确测试名执行它。

## 3. 历史 Hosted 时间线

| CI run / job | 真实结果 | 证据边界 |
|---|---|---|
| [38026682784](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38026682784)，job [114138992324](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38026682784/job/114138992324) | D2 在约 10.16 秒以 `timed out waiting on channel` 失败。 | 未记录 pool、BEGIN、SQLite 错误码或发送阶段。日志 SHA-256：`c8ee58d00da4d3a8c34910210b65d322322c65f42075c68163b9c0cbef9823e9`。 |
| [38056013797](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013797)，job [114224702884](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013797/job/114224702884) | D2 返回 plain `SQLITE_BUSY 5/5`，耗时 9,219ms，无部分 authority，测试通过。 | 证明受控 writer contention 可以返回真实 BUSY；不能解释两个失败。日志 SHA-256：`105a4d825a294620db1916dfba58a18c8e52a584527d31867b8e55756f242180`。 |
| [38064775222](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38064775222)，master job [114250183816](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38064775222/job/114250183816) | D2 在约 10.21 秒以相同 channel timeout 失败；master run 失败。 | 无阶段标记，不能反推 timeout 瞬间 contender 在哪一段。日志 SHA-256：`bee0ca31c6d550e7e913282e60b06e97b3f29910d1ec7115cbc51c06a27014b4`。 |
| [38068879734](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38068879734)，Native job [114262149859](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38068879734/job/114262149859)，attempt 1 | pool 0ms、busy timeout 5000ms；D2 返回 `BUSY 5/5`，8,807ms，无部分 authority。 | 首个阶段化 Hosted 样本；仍未测线程 CPU。日志 SHA-256：`bfba60e956ef8b6ecd29cb98c8a8e6690356dd177f9aea6b31ca83800a80f910`。 |
| 同一 run attempt 2，job [114264864189](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38068879734/job/114264864189) | pool 0ms；D2 `BUSY 5/5`，9,751ms，无部分 authority，距 10 秒 deadline 仅 249ms。 | 真实成功记录，不是失败，也不覆盖历史失败。日志 SHA-256：`9f06e5e51e34800ccf7a56b9e9268c274f54e39d4e5f402844dd14b8651c3bd1`。 |
| [38070488344](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38070488344)，Native job [114266812181](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38070488344/job/114266812181) | 精确当时 PR 头 `e7b8b4c…`：pool 0ms，`BUSY 5/5`，9,050ms，无部分 authority。 | 成功没有解释原失败的完整根因。日志 SHA-256：`159db331121e9b16f333fcb1472900a8f3da0cb292b52eb82c8e74ac4fdb0852`。 |

### D3 原始 5000ms fixture 的 Hosted 观测

下列四次均为真实 Native macOS Hosted D3 PASS 结果；它们不是失败记录，也未因后续 fixture 调整而删除。四次均在 10 秒 channel watchdog 内返回 plain BUSY 并完成其测试断言，但只剩 768–965ms 余量：

| CI run / job | D3 真实结果 | 到 10 秒 watchdog 的余量 | 证据 |
|---|---:|---:|---|
| [38070488344 / 114266812181](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38070488344/job/114266812181) | 9,035ms | 965ms | `SQLITE_BUSY 5/5`。 |
| [38073369114 / 114275927570](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38073369114/job/114275927570) | 9,232ms | 768ms | `SQLITE_BUSY 5/5`。 |
| [38074989532 / 114280260772](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38074989532/job/114280260772) | 9,194ms | 806ms | `SQLITE_BUSY 5/5`。 |
| [38076196367 / 114283657838](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38076196367/job/114283657838) | 9,051ms | 949ms | `SQLITE_BUSY 5/5`、`rollback_atomic=true`、`after_release_success=true`。 |

这四次 D3 都不是已证实的生产错误；但它们与已测得的 5000ms SQLite timeout 可消耗 8.8–9.75s wall 的 Hosted 现象一致，故 10 秒 watchdog 余量不足。D3 test-only timeout 降为 500ms 后，保留原 10 秒 watchdog 和完整语义检查，避免通过放宽测试阈值掩盖调度差异。

## 4. 本轮精确阶段观测

首个诊断构建 CI [38073102567](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38073102567) 在 Native job 启动前失败：`wait_until_held() -> Instant` 的表达式仍以分号结尾，Rust 报 `expected Instant, found ()`。这是本次诊断代码的编译错误，不是 SQLite 失败。删除该分号后，Windows 与 macOS release compile 均通过。这个失败已保留，未 rerun 掩盖。

修正后的 exact-head CI [38073369114](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38073369114)，macOS Native job [114275927570](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38073369114/job/114275927570)，使用 synthetic request key `issue366-admission-timeout-01a12701-1b4c-7f92-97b9-47842e77b83a`，原始日志 SHA-256：`98dc2e7cb341c24d2b3d41b28c047781ca75e137c7451ac0e40eaad2b10b5dcf`。

所有时间来自同一 monotonic test origin；微秒数据按原始日志记录：

| 阶段 | elapsed / wall | 说明 |
|---|---:|---|
| writer 的 `BEGIN IMMEDIATE` 成功，barrier 可用 | test origin +73,398µs | 写锁已真实持有。 |
| contender 发 start signal | +73,671µs | `send_ok=true`。 |
| admission call 开始 | +73,764µs | signal 后 93µs。 |
| `admit_managed_scan` 方法进入 | +73,782µs | call 后 18µs。 |
| root 解析 / request hash | 118µs / 98µs | 不是主要耗时。 |
| pool connection | 40µs wall、40,917ns thread CPU | `PRAGMA busy_timeout=5000`；排除池等待消耗数秒。 |
| `BEGIN IMMEDIATE` 调用边界 | +74,196µs | admission method 总计约 403µs 后进入。 |
| 10 秒 channel receive 开始 | +74,267µs | 本次样本比事务调用边界晚约 71µs；总比 method call 晚约 503µs。 |
| `transaction_with_behavior` 返回 | +9,222,331µs | `begin_wall_us=9,148,114`；线程 CPU `2,318,417ns`；plain `SQLITE_BUSY` primary/extended `5/5`。 |
| `admit_managed_scan` 返回 | +9,222,781µs | 整个 admission call wall `9,149,017µs`，thread CPU `3,212,208ns`。 |
| result send 返回 | +9,223,055µs | send 自身 wall `120µs`。 |
| 主线程收到结果 | +9,223,094µs | channel wait `9,148,826µs`；send-start 至 receive `159µs`。 |
| 测试最终结果 | `elapsed_ms=9149`，通过 | `partial_authority=false`；plain `BUSY 5/5`；writer 与 contender 均按正常路径清理/join。 |

本次 BEGIN 的线程 CPU 约占 BEGIN 墙钟的 **0.025%**。这证明时间主要消耗在 SQLite 调用内部的非 CPU 等待；它不区分线程处于 `nanosleep`、内核 sleep overshoot 还是 runner 未及时调度。SQLite 返回到 admission return 约 450µs；result send/receive 又不到 0.3ms。因此结果 channel 交付没有吞掉数秒，也没有在 SQLite 返回后等待调度的证据。

本样本在 10 秒 channel deadline 前约 851ms 返回；先前 9,751ms 样本仅余 249ms。历史失败日志没有这些 marker，不能证明其失败时也已经进入 SQLite 或已经返回 BUSY。

第二次有效阶段观测来自 exact PR-head run [38074989532](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38074989532)，macOS Native job [114280260772](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38074989532/job/114280260772)，synthetic request key `issue366-admission-timeout-01a1270e-29cf-7df2-9db4-6a6b6870c71c`：

| 阶段 | elapsed / wall | 说明 |
|---|---:|---|
| writer lock 建立 / start signal / admission call | +59,376µs / +59,536µs / +59,573µs | signal 到 call 37µs。 |
| method 进入 / pool connection | +59,578µs / +59,759µs | root 解析 63µs、hash 32µs、pool 21µs wall / 22,375ns thread CPU。 |
| 配置与 deadline 起点 | pool PRAGMA `5000ms`；`BEGIN IMMEDIATE` 边界 +59,770µs；channel wait +59,621µs | watchdog 比 BEGIN 早 149µs；计时原点差不足 0.2ms。 |
| `transaction_with_behavior` 返回 | +8,887,093µs | `begin_wall_us=8,827,310`、thread CPU `1,658,875ns`、SQLite primary/extended `5/5`。 |
| admission 返回 / send 返回 / receive | +8,887,272µs / +8,887,343µs / +8,887,491µs | send-to-receive 186µs；channel wait 8,827,869µs。 |
| 测试结果 | `elapsed_ms=8828`，通过 | plain `SQLITE_BUSY 5/5`，`partial_authority=false`；清理/join 通过。 |

第二样本 BEGIN 线程 CPU 约占墙钟的 **0.019%**；它以 8.827s 返回真实 BUSY，距 10s watchdog 约 1.173s。两个新样本中 watchdog 与 SQLite 事务起点相差仅 `-149µs` 与 `+71µs`，因此这两次成功观测不支持 timer 起点错位造成秒级差异。

## 5. SQLite busy timeout 源码核对

依赖锁定为 `rusqlite 0.39.0` 与 `libsqlite3-sys 0.37.0`，项目启用 bundled SQLite。代码证据在 Cargo registry 对应锁定源码：

- `rusqlite-0.39.0/src/transaction.rs::Transaction::new_unchecked`：`Immediate => "BEGIN IMMEDIATE"`，随后执行单条 SQL。
- `libsqlite3-sys-0.37.0/sqlite3/sqlite3.c::sqliteDefaultBusyCallback`（约 186623 行）：按 `delays[]`/`totals[]` 累积请求 sleep 时长，达到 `busyTimeout` 后停止继续重试。
- 同一 amalgamation 的 `sqlite3_busy_timeout` 文档说明 handler 会多次 sleep，直到**累计睡眠至少达到给定毫秒数**。这个描述不是严格墙钟截止时间。
- `unixSleep`（约 46348 行）在 `nanosleep` 分支调用 `nanosleep(&sp, NULL)`，随后返回请求的 `microseconds`；它不测量并返回实际经过的 monotonic 墙钟时间。`libsqlite3-sys` build 脚本设置 `HAVE_USLEEP`，没有设置 `HAVE_NANOSLEEP=0`；macOS 构建走默认 nanosleep 分支。
- 代码里没有证据显示 application 在这次 admission 周围重启多个独立 5 秒 timeout；观察到的延迟位于单条 `BEGIN IMMEDIATE` 调用本身。

源码能解释“5,000ms 是 SQLite 请求 sleep 的累计目标，而不是承诺 5,000ms 后按墙钟返回”。两个 Hosted BEGIN 分别耗时 9.148s 与 8.827s，线程 CPU 分别约 2.3ms 与 1.7ms；额外墙钟来自 SQLite 调用中的非 CPU 等待，而不是计算、池等待或 channel handoff。

**仍未实测的部分：**本次没有逐次量取 SQLite 每个 `nanosleep` 的请求时长与实际返回时长，也没有 macOS scheduler trace。因而约 `9.148s - 5.000s = 4.148s` 的额外墙钟，尚不能在内核 sleep overshoot 和 Hosted runner 调度延迟之间精确分摊。将全部差额直接归因于某个 macOS bug 或 GitHub runner 缺陷都超出证据。

## 6. 假设判定

| 候选原因 | 本轮证据 | 结论 |
|---|---|---|
| 连接池等待 | 40µs wall。 | 排除为本次 9 秒延迟来源。 |
| admission 前的 root/hash 工作 | 合计为亚毫秒级；call 到 BEGIN 边界约 0.4ms。 | 排除为主要延迟来源。 |
| WorkScheduler / channel deadlock | admission 路径不调用 WorkScheduler；SQLite 返回后 send/receive 少于 0.3ms。 | 本次成功样本不支持；原始失败无法回溯到具体阶段。 |
| SQLite 内部锁等待 | 两次 `BEGIN IMMEDIATE` 分别 wall 9.148s、8.827s，均返回真实 `BUSY 5/5`。 | 近 9 秒耗时直接位于 SQLite transaction call 内。 |
| 5 秒配置被多次 application retry | 源码路径只有一次 Immediate transaction call，没有应用层重试。 | 未发现多次独立 5 秒预算；SQLite 内部会重复调用 busy callback。 |
| OS sleep / 调度 | BEGIN CPU 2.318ms/1.659ms 对比 9.148s/8.827s wall。 | 强烈支持非 CPU 等待；尚不能分解实际 sleep overshoot 与 runner 调度。 |
| send/receive 竞态 | 两次 send-to-receive 分别 159µs、186µs；deadline 起点与 BEGIN 相差不足 0.2ms。 | 未见秒级 handoff 延迟或秒级 timer 起点错位；历史失败仍无阶段数据。 |
| 部分 authority / 生产 fail-closed 错误 | 两次 D2 均 plain BUSY，root/session/run 计数均为 0。 | 两次受控锁争用均无部分授权；不是所有生产时序的证明。 |

## 7. 是否存在可以证明的测试层缺陷

本轮修复过两类不同问题：第一轮诊断补丁的一个 Rust 返回类型编译错误；第二轮 exact-head CI 中，Windows 与 macOS Rust Quality 在非 `performance-test-tauri` 编译配置下将诊断专用 fallback variable 和 elapsed helper 当作 warning-as-error。后者是本 PR 诊断代码的 cfg/lint 缺陷，不是原始 timeout 根因；已最小收窄 helper cfg 并删除非诊断分支的无用变量，待 exact-head CI 验证。

原始 harness 的时间余量问题有直接支持：SQLite 配置是 5,000ms requested-sleep 预算，但两次实测事务墙钟达 8.827–9.148s；更早成功样本达 9.751s，而失败 watchdog 是 10s。两次成功样本的阶段时间又排除了 pool、预处理、timer 起点偏移和 result channel 交付是秒级来源。历史失败没有 stage marker，所以不能证明两次失败都在 SQLite BEGIN 内超时；不过现有 10s wall deadline 与 SQLite 非严格 wall-clock timeout 的组合留下的余量过小，足以构成真实测试稳定性缺陷。

Owner 已接受 D2 的 test-only fixture 修复；本次不重新调查或改写其逻辑。只将 D2 注释更正为“先验证池内 5000ms 默认值，再在该测试 fixture 使用 500ms”，避免暗示 D2 仍等待生产 5 秒。

本次 D3 修改按 Owner 明确顺序进行：完整完成 admission、claim、构造待写文件条目后，使用既有 `Database::set_test_busy_timeout_for_all_connections(5_000, Duration::from_millis(500))`；此辅助函数会先持有池内所有连接并分别验证 `PRAGMA busy_timeout=5000ms`，然后逐连接设置和复验 fixture 的 500ms。真实 writer lock 在该步骤之后取得。D3 的 lower-bound elapsed 断言由 4 秒改为 fixture timeout 的一半（250ms），继续证明操作确实等待锁；upper bound 仍为 10 秒。

D3 锁住 writer 时仍要求 `SQLITE_BUSY` primary/extended code `5/5`；然后验证失败批次在 `files` 和 `scan_seen` 中均没有部分写入，并确认 scan run/session revision 及扫描计数不变。释放并 join writer 后，用相同 revision/token 再次持久化成功、推进 finalization 并 settle run，最后 join contender。10 秒 channel watchdog、20 秒 holder safety release 和 Holder 的 Drop 释放/join 仍保留。新增日志显式记录 `production_default_ms=5000`、`fixture_busy_timeout_ms=500`，结果日志包含 rollback/after-release 成功标记。

上述差异仅属于 ignored test fixture 与其报告。生产 `configure_connection`、SQLite busy policy、SQL、schema、scheduler、IPC 与扫描行为均没有变更。本次代码及报告提交时，新 exact-head Hosted CI 尚待执行；终态 CI 与 D2/D3 实测会通过 PR #367 的最终证据评论提供，避免把旧 D3 PASS 误当成新 fixture 验证。

## 8. 改动范围

本 PR 现有修改仅包括：

- `scan.rs` 的 `#[cfg(all(test, feature = "performance-test-tauri"))]`、synthetic `issue366-` 限定阶段日志：method/root/hash/pool/BEGIN entry-return、SQLite primary/extended error、macOS thread CPU。
- D2 测试的 writer barrier、start signal、channel wait、admission call、send/receive 与 timeout cleanup 诊断时间戳；Owner 已接受 500ms test-only fixture，本次只修正文案。保留 10s channel deadline、20s holder safety release 和所有 fail-closed assertions。
- `connection.rs` 新增 `#[cfg(test)]` pool helper：逐一借出整个连接池，校验每个连接的默认 `busy_timeout=5000ms` 后才应用测试 fixture 的 500ms 覆盖；release 配置不变。
- D2 与 D3 ignored test 均只在各自的 test database pool 上使用 helper 验证 5000ms 默认值并覆写成 500ms。D3 在 admission/claim/data 初始化后、held writer 开始前设置；仍验证真实 `SQLITE_BUSY 5/5`、无部分写入、revision 不变、释放 writer 后成功以及线程/连接清理。D3 10 秒 watchdog 未变，仅将 4 秒最小耗时改为 250ms fixture 预算的一半。
- 诊断 helper cfg 修正非 performance Rust Quality 构建的 warning-as-error。
- Native macOS performance 源文件路由与合同测试，确保 `scan.rs` 变更会选中已存在 Native job。
- 本中文调查报告。

未改 release-mode SQLite/scheduler/admission 行为、Schema、IPC、搜索或 AI。

## 9. 本地检查与 exact-head CI

本地 Codex Cloud 执行：

- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`：通过。
- `npm run test:governance`：通过。
- `node scripts/checkPerformanceArchitecture.mjs`：通过，3 个文件、30 个测试。
- 为执行 D3 的 release ignored test，已将 GTK/WebKit 原生开发依赖（约 68 MB 下载）缓存至共享 `/workspace/zen-canvas-native-deps/archives`，并解包至 `/workspace/zen-canvas-native-sysroot`；不修改容器系统包。`pkg-config` 现可找到 GLib 2.84.4、GDK/GTK 3.24.49、WebKit2GTK 2.54.0。
- 本地 D3 exact 命令 `cargo test --release --locked --manifest-path src-tauri/Cargo.toml --features performance-test-tauri --lib db::queries::scan::tests::managed_scan_batch_write_returns_busy_atomically_and_succeeds_after_release -- --exact --ignored --nocapture --test-threads=1` **未能在 Linux 编译完成**：应用的 `ai/settings.rs` 使用 `keyring::Entry`，但仓库仅在 `cfg(target_os="windows")` / `cfg(target_os="macos")` 声明 keyring target dependency；Linux build 因 `keyring` 未链接退出 101。该阻塞与 D3 patch 无关；没有改业务/Cargo target 配置来绕过它。目标测试由 macOS Hosted Native CI 执行。
- Linux 结果不作为 macOS/Windows 性能证据。此次针对测试源文件的 Windows/macOS release 编译与 Native macOS D2/D3 实测均由本次 exact-head Hosted CI 验证，并在 PR 最终证据评论报告。

CI：

- [38073102567](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38073102567)：诊断补丁编译错误；Native macOS 被取消/未执行。保留失败。
- [38073369114](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38073369114)：Native macOS job `114275927570` 成功运行 D2；Windows/macOS release compile、Windows Global Index service qualification、Frontend/format、六个 scoped 100k shards、Performance profile 均通过。
- 同一 run 的 Windows Rust Quality job `114275286600` 有一个无关测试失败：`file_workspace::change::tests::change_arriving_during_refresh_supersedes_page_and_is_not_lost` 在 `src/file_workspace/change.rs:590` 创建 fixture root 时返回 Windows `Access is denied`。本调查未修改或重跑该用例；run 的 Windows Quality aggregate 因此失败。
- 报告编写时，该 run 的 `Rust quality (macos-latest) (merge_integration)` job `114275927656` 仍为 queued，终态 **NOT VERIFIED**；不据此声称完整 CI aggregate 成功。
- 该 CI 使用 `extended` profile，未启用 Full Validation；Native job 中 10k mixed-filesystem classifier、100k macOS bookkeeping 以及 Workspace Foundation suite（含 D2）均通过。没有执行 500k 或 1m。
- [38074989532](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38074989532)，PR head `5efcc9b…`：第二次 Native macOS job `114280260772` 成功，D2 使用原生产 5000ms 默认值并再次以 `SQLITE_BUSY 5/5` 在 8.828s 内 fail closed、`partial_authority=false`。同 run 的 macOS/Windows release compile、Windows Global Index、Frontend/format、六个 scoped shards、Performance profile 均通过；Windows 与 macOS Rust Quality 因本 PR 诊断代码在无 performance feature 构建下的两个 warning-as-error 而失败。失败具体为 fallback diagnostic key unused 与 `issue366_elapsed_from_test_start_us` dead code；此修复属于 test-only instrumentation cfg，已纳入当前工作树，待新的 exact-head CI 验证。
- 本次 exact-head CI 将验证 D2/D3 池内逐连接 5000ms 默认检查和 500ms fixture override、D2 无部分 authority、D3 `BUSY 5/5` 与 rollback/after-release 成功，以及适用的 Windows/macOS Rust Quality、Native macOS Performance 和 Performance Profile。报告随代码提交时该 run 尚未启动；完成后的准确 run/job/测量值通过 PR #367 顶层证据评论追加，保持此报告提交与受测代码 head 一致。
- Codex Review：未运行。PR 仍 Draft。

## 10. 未验证事项与 Owner 验收

- 历史失败 `38026682784` 与 `38064775222` 没有原始阶段数据；无法判定其 timeout 瞬间是尚在 BEGIN、刚从 SQLite 返回还是 contender 尚未调度。
- 两个 8.827–9.148s 样本确认耗时在 BEGIN 内；无法给出每次 nanosleep 的实际返回时长，也没有 macOS scheduler trace。额外约 3.827–4.148s 仍不能在 sleep overshoot 与 runner 调度之间精确分摊。
- 原始失败 run 没有阶段 marker；两次新观测都成功返回 plain BUSY，因此不能声称直接复现了旧失败的确切阶段。修复针对已经证实的测试余量问题，不改变 production timeout。
- 本阶段已经完成授权范围内的两次生产 5s 配置 macOS 阶段观测。修复后的 exact-head CI 是对 test-only 500ms fixture 的验证；不再额外启动独立 targeted run。
- Windows 本机验收、实际用户负载和 production SQLite 行为没有在本任务范围内修改或声称已验证。

**建议 Owner 决策：**审阅 test-only 500ms fixture 修复与 exact-head CI。若需精确分解剩余墙钟差，需另行授权 macOS Hosted syscall/scheduler 级采样；当前没有 production SQLite policy 变更建议。

## 当前状态

**本报告描述 D3 test-only fixture 提交时的状态：D2 已获 Owner 接受；D3 新增的 500ms test-only fixture 尚待本次 exact-head Hosted CI 实测。** 原始历史失败、D3 四次约 9 秒的先前 PASS 和 D2 先前/修复后测量均保留；CI 完成态与本次 D2/D3 最终数字在 PR #367 顶层证据评论记录。PR #367 保持 OPEN/Draft，Issue #366 保持 OPEN；不合并、不标记 Ready。OS 额外墙钟的精确分摊仍未测量。
