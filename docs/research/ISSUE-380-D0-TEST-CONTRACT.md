# #380 Rebuild Admission 与持久化状态架构可行性报告

**阶段：#380-D0 TEST-ONLY EXECUTABLE CONTRACT**
**结论：#380-D0 TEST CONTRACT READY_FOR_OWNER_REVIEW**
**决策依据：** [Owner 决议 #6110020867](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6110020867)

本报告验证 A+B+C 的测试原型。它没有修改生产 Coordinator、Service、Repository、Schema 37 或 IPC v3，也没有修复运行中的 #380 缺陷。测试通过只证明隔离状态机和 SQLite 原型满足本文契约。

## 基线和范围

| 项目 | 值 |
|---|---|
| Repository | ArdenZC/Zen-Canvas |
| 测试分支 | test/issue-380-d0-executable-contract |
| D0 起始 HEAD | 3275ac7c72df8ae4139a30f16c7f649971324e97 |
| D0 起始 Tree | 009576241cd4e8e2ba12cc644a368314f599faa6 |
| 上游 | origin/master（创建隔离 worktree 时 fetch 后确认） |
| 原工作分支 | fix/issue-380-service-busy-admission，保持干净 |
| 生产 Schema / IPC | Schema 37 / IPC v3，未改 |

精确的 PR HEAD、Tree、变更文件和 Draft PR 检查结果随 PR 元数据及最终交付记录。报告不能嵌入其自身提交的 HEAD/Tree 哈希，因为加入哈希本身会改变该 Tree。

仓库内仅增加 src-tauri/tests/issue_380_d0_test_contract.rs 和本报告。SQLite fixture 使用唯一临时文件，Drop 清理数据库、WAL、SHM、journal；不读取或写入真实用户数据库。

## 代码证明的当前调用顺序

| 顺序 | 位置 | 当前行为和持久化边界 |
|---|---|---|
| 1 | src-tauri/src/global_index/commands.rs:182-190 rebuild_global_index_source | Tauri 命令调用 Coordinator::rebuild(source_id)。 |
| 2 | src-tauri/src/global_index/coordinator.rs:218-256 Coordinator::rebuild | 先发现来源；目标 Rebuild 时先写 REBUILD_REQUIRED 并调用全卷 stale；无目标时对 list_global_volumes() 返回的每卷执行同样写入；最后才 start()。这些副作用早于工作调度和 Service admission。 |
| 3 | src-tauri/src/global_index/coordinator.rs:447-574 run_index_cycle | worker upsert 来源；跳过 enabled=false 的来源，但第 2 步已对其 stale。之后先写 SYNCING 或 INDEXING，再决定是否走后台 admission/调用 provider。 |
| 4 | src-tauri/src/global_index/coordinator.rs:685-715 run_with_background_admission | 获得的是进程内 WorkScheduler lease/coalesce key，不是持久化 operation 所有权或跨进程 CAS。 |
| 5 | src-tauri/src/global_index/coordinator.rs:82-89 和 Windows provider | trait 默认 rebuild() 委托 start_initial_index()；WindowsGlobalIndexProvider 未覆盖 rebuild()，因此当前 Coordinator 的 Rebuild 路径发出 StartInitialIndex，不是 IPC Rebuild。 |
| 6 | src-tauri/src/global_index/windows/mod.rs:300-312,333-413,425-453 | call_stream 逐帧交给 apply_event；Entries、Checkpoint、SourceState 等事件立即写当前 sink。IPC event frame 中的 request_id 在 call_stream 被丢弃，response 的 protocol/request ID 也未在此验证。流中断或 callback 错误会先写 UNAVAILABLE，然后 Direct fallback；之前已经应用的事件不会自动回滚。Service 的 typed error 走 Ok(Err(error))，不经过 Direct fallback 分支。 |
| 7 | src-tauri/src/global_index/windows/service_host.rs:195-259,262-278 | Service 用进程内 AtomicBool busy CAS；busy 时立即返回 index_service_busy。成功取得 busy 后才加操作锁、校验 source snapshot、设 Service 内部状态为 indexing 并运行 provider；响应在 provider 完成后才发出。没有 durable operation/result。 |
| 8 | src-tauri/src/global_index/coordinator.rs:315-342,574-656 | sink 的 checkpoint/state 写 source health；provider 返回普通错误（包括当前 busy 解码出的 Provider 错误）后，Coordinator 将非 degraded 来源写成 ERROR。因此旧实现可能先出现 SYNCING，然后写 ERROR。 |
| 9 | src-tauri/src/global_index/repository.rs:262-338,341-367 | entry batch 在一个 SQLite transaction 内 upsert entries 并调度 Managed/AI 工作，FTS/count 由触发器维护；这只是单批事务。mark_global_entries_stale_for_volume 的 stale、Managed、AI jobs/items、analysis、count 是六条独立语句，没有包住全套的事务。 |

### 当前操作所有权和允许写入的时点

当前没有一个可恢复的持久化 Rebuild owner。Coordinator 的 Scheduler lease 只覆盖本地 worker；Windows Service 用进程内 busy 标志和操作 mutex；二者都没有持久化 operation ID、source revision 或可以 fencing 旧 writer 的 CAS。updated_at 以秒为粒度，不是操作归属证明。

当前 active DB 在 Service 有 admission 结果之前已被 Coordinator 改成 REBUILD_REQUIRED 并 stale；随后进入 SYNCING；Service event 又会把部分 entry、checkpoint、health 写到 active tables。SQLite 单批 upsert 的事务无法回滚 Rebuild 的此前 stale 或其他已提交批次。FTS 与 count 触发器跟随各批次提交，所以中断时会保留部分发布结果。

Disabled 也有顺序差异：全量 Coordinator::rebuild(None) 在 admission 前 stale 所有已存来源；worker 随后只跳过 disabled 来源。目标 disabled 来源没有统一的、无副作用的拒绝边界。

### 证据分类

- **代码已证明：** 上表中的调用顺序、Service 进程内 busy、普通忙碌错误被写成 source error、全量 rebuild 对 disabled 来源先 stale、response/event request identity 未在客户端验证、当前 stale 六语句不在一笔 transaction 中、v37 会拒绝打开未来 schema。
- **合理推断：** Service 通信中断可能发生在部分事件已经写入后；直接 fallback 因而可能在同一来源重复执行 Rebuild。单调操作 revision、写入 fencing 和完整 staging publish 可建立更可靠的 owner 边界。
- **尚未验证：** 生产 Service 是否能用现有 v3 帧形状稳定提供“任何 active 写入之前”的 admission 事件；Service 正在运行时 DB 写锁竞争、完整 19 字段 GlobalEntry staged merge 的 100k/500k 成本；Windows Service 重启时在途 provider 是否能保证停止发送旧事件。

## A、B、C 方案评估与组合

| 方案 | D0 结论 | 真实能力和边界 |
|---|---|---|
| **A：调整 admission 与 rebuild 顺序，保留 Schema/IPC** | 必要但单独不安全。 | 将 REBUILD_REQUIRED、stale、SYNCING 和任何 active entry 写入移到可信 admission 之后；ServiceBusy 只结束/排队该 operation，不改来源 health。若只用内存锁，进程重启、并发旧回包和 stale writer 仍没有 durable CAS。IPC v3 只有在首个 accepted event/ack 的 request ID、protocol、source 身份和顺序均被验证后才可复用；当前实现没有这些验证。 |
| **B：最小 operation/intent/revision 持久化** | 必要且可与 A 组合。 | 每来源 revision + durable intent/phase + CAS claim 可提供唯一 owner、重复请求 coalesce、旧 revision replay 拒绝和恢复判定。Busy 后的 queued intent、Unknown tombstone、取消/完成需持久化。B 本身不能保留 active 旧索引，因为如果运行中逐批写 active 表，崩溃仍留下混合代。 |
| **C：分阶段可恢复 Rebuild** | 作为安全目标需要，与 A+B 组合。 | 新 generation 写入独立 stage；旧 active entries 与 Managed/AI 关联继续可查。只有 operation revision、来源 enabled/identity 和取消标志仍匹配时，才将 entry/FTS/count/health/checkpoint/Managed/AI 的发布放进一笔受 fencing 的事务。SQL 失败或进程在未提交事务中退出时旧代保留。C 单独无法判断哪个 Service 请求拥有 stage。 |

三者不互斥：D1 应是 **A 调整调用顺序 + B durable owner/CAS + C stage 和原子发布**。A 单独或 A+C 而无 operation revision 都无法安全 fencing 并发旧 writer；A+B 而继续逐批改 active 不能满足崩溃保留旧关联数据的目标。

### D0 可执行契约与回归矩阵

下表的“旧失败”列是对当前生产代码的只读路径证明/既有诊断测试证据；本阶段没有把生产代码替换成旧版做红测。新通过列是隔离原型的可执行测试，不代表生产缺陷已修复。

| 场景 | 当前/旧行为（失败依据） | D0 新契约（通过测试） |
|---|---|---|
| Service busy | admission 前已经 stale，worker 设 SYNCING，busy 被解码成 Provider error 后写 ERROR；既有 coordinator_recovery_tests.rs 保留 busy 特征测试。 | service_busy_is_typed_rejection_without_health_or_index_side_effects：只将 op 回到 queued；entries、Managed、AI、FTS、count、checkpoint、source health 相同。 |
| request/protocol/source/event 顺序不匹配 | call_stream 未验证 response identity，event request ID 被忽略；生产路径可把不可信 event 交给 active sink。 | malformed_or_mismatched_admission_frames_are_unknown_and_side_effect_free、busy_frame_mismatch_is_unknown_and_direct_fallback_is_limited_to_proven_not_sent：变为 Unknown，不写 active projection；发送后未知不 Direct、不 replay。 |
| transport 中断/Direct fallback | run_with_fallback 在流错误时先写 UNAVAILABLE 再 Direct，之前流事件可能已提交。 | transport_interruption_after_send_is_unknown_without_fallback_or_projection_changes：Unknown 保留 active projection；只有明确 ProvenNotSent 且策略允许时才可 Direct。 |
| accepted 后进程退出/重启 | 当前 active 表可能已收到部分批次；stale 和已提交批次没有整体回滚。 | process_exit_during_publish_transaction_recovers_the_old_active_projection、restart_marks_accepted_incomplete_operation_unknown_but_keeps_old_projection：子进程在 WAL 写事务中退出，SQLite 恢复旧 projection；重启把在途 op 标成 Unknown，stage 可保留供诊断。 |
| 发布中的 SQL 错误/各阶段失败 | stale SQL 分句且当前无 publish generation boundary。 | every_publish_failpoint_rolls_back_entries_managed_ai_fts_count_and_checkpoint、sqlite_write_error_rolls_back_partial_publish_and_retains_stage_for_recovery、staging_write_failure_rolls_back_the_batch_without_touching_active_projection。 |
| 并发、重复请求、旧 revision | 没有持久化 revision/CAS；秒级 updated_at 不能 fence 旧写者。 | duplicate_same_source_intents_coalesce_and_revision_replay_is_fenced、两项 racing_* 测试：同源只建一个 intent/revision，异源各自 claim，旧 revision 不能 claim/stage/publish。 |
| disabled、来源变化、取消 | 全量 rebuild 在 worker 跳过 disabled 前已 stale；没有 revision/identity publish fence。 | full_rebuild_enumeration_skips_disabled_sources_and_targeted_rebuild_is_a_noop、source_disable_and_identity_change_fence_staged_publish、explicit_cancel_removes_stage_and_fences_publish_without_touching_active_data。 |
| 成功发布 | 当前是多笔批量更新，旧 generation 不会保持到完整扫描完成。 | independent_oracle_matches_sqlite_and_active_projection_changes_only_at_publish：FTS 搜索、entry_count、Managed 关联、AI 状态、用户修正、checkpoint 和 source state 与独立 in-memory Oracle 一致。 |

共 **18 个测试**，包括上表测试及 1k/10k 有界数据库测试。Oracle 不调用 SQL helper；SQLite fixture 和 SQL 完全隔离。

## 隔离 SQLite 原型与事务边界

测试专用 d0_operations 持久化 source、revision、request ID、protocol、source identity snapshot、phase；部分唯一索引限制每来源一个 queued/dispatching/staging/unknown operation。d0_stage_entries 与 d0_stage_checkpoint 在独立的短事务写入。没有写正式 Schema 或正式测试 fixture。

拒绝、busy、unknown、stage、cancel、重启恢复时，测试比较完整的 fixture projection：active Global Entries、Managed link、AI analysis/jobs/items、FTS hits、entry_count、journal ID/cursor、last full/incremental timestamps、source status/error/updated_at。成功 publish 将 stale/upsert、FTS 触发器、Managed reconcile、AI stale/fingerprint reconcile、source health/count/checkpoint 和 operation completion 一起包在 BEGIN IMMEDIATE transaction；failpoint 或真实 RAISE(ABORT) 时 transaction drop 回滚。独立子进程在多表 DML 后、COMMIT 前 exit(73)，父进程重开连接并检查 SQLite WAL recovery。

此 fixture 有意只建用于验证事务的代表性列，不是 Schema 37 的克隆：Entry 简化为 ID/来源/name/path/extension/fingerprint/stale，AI job/managed 列也少于生产表。因此它验证的是隔离 SQLite 的事务原理，不是所有生产行字段的完整转换或 500k 数据容量。

## 有界性能与 WAL 结果

环境：仓库指定 Rust 1.97.1、rusqlite 0.39 bundled SQLite 3.51.3；SQLite WAL、synchronous=NORMAL、wal_autocheckpoint=0，单独保留 keeper connection 读取 WAL 文件。计时从 operation/stage 调用开始或 publish 开始（含连接/设置和 BEGIN IMMEDIATE 等待）直到返回；Synthetic entry 只在临时库中。publish 在每个规模先执行一次 AfterEntryMerge 失败回滚，再成功发布。

| synthetic 条目数 | stage ms | 失败 publish rollback ms | 成功 publish ms | stage 后 WAL bytes | rollback 后 WAL bytes | 成功后 WAL bytes / 总增长 bytes |
|---:|---:|---:|---:|---:|---:|---:|
| 1,000 | 95 | 1,537 | 917 | 201,912 | 201,912 | 861,112 / 861,112 |
| 10,000 | 651 | 4,308 | 7,413 | 1,837,552 | 4,960,512 | 7,481,952 / 7,481,952 |

fixture 的 10k publish 在本次运行约 7.4 秒，失败回滚中 WAL 文件保留了约 4.96 MB 未提交 frames；重新读取 active projection 仍与回滚前逐值相同。这是写入锁成本信号，不代表 500k 线性外推。原型完整行比生产 GlobalEntry 窄，也没有真实 AI/Managed 数据规模、其他读写连接或生产 hardware。没有运行或自动启动 100k/500k benchmark。D1 前 Owner 仍需设定可接受的最大 writer stall/WAL/磁盘预算，并决定是否先做 generation-pointer 替代原子大 merge。

## v38 候选最小生产形状（尚未授权实施）

1. global_volumes.operation_revision INTEGER NOT NULL DEFAULT 0：每个来源单调递增；更新 revision 必须检查来源仍 enabled 且 stable volume ID、mount path、provider 身份未变化。
2. global_index_operations：operation ID、source FK、revision、命令、当前 IPC request ID、protocol version、expected source identity、phase (queued/dispatching/staging/unknown/cancelled/completed)、outcome/error 和时间戳；唯一 (source_id, revision)、唯一 request ID，以及只允许一个 active op per source 的 partial unique index。Unknown 不可自动重复派发。
3. global_index_rebuild_stage：operation ID + stable global_entry.id 主键，存放完整生产 GlobalEntry 字段（platform_file_id、父 identity、normalized name/path、extension、directory/size/timestamps/attributes/hidden/system/provider/last_seen 等），不能用 D0 简化 struct 直接上线。
4. global_index_rebuild_stage_checkpoint：按 operation 保存 journal ID/cursor；checkpoint 在发布前不得写回 active global_volumes。

**发布不变量：** 所有 stage 写仅由匹配 op ID + revision 的 staging owner 接受；每个 batch 验证 source identity/enabled/current revision；publish 再次 CAS 并在一笔事务内完成 active entry merge/stale、Managed 关联、AI jobs/items/analysis、FTS/count 触发器、source health/checkpoint 和 operation completed；提交前 active 旧代有效，任何错误/进程退出都回滚全部 active 写。未知 Service outcome 不重播、不 Direct fallback；busy 是 typed rejection，回到 queued 或等待显式下一次请求，不写 source ERROR/SYNCING。全量跳过 disabled source，目标 disabled 拒绝且无 active side effect。取消必须先 fence 后续 event/stage/publish writer；Service 仍运行时不得仅凭本地 phase=cancelled 释放 owner。

### v37 升级、备份和回滚

src-tauri/src/db/schema.rs 当前 CURRENT_SCHEMA_VERSION=37；migrate() 对高于当前版本直接拒绝，已有测试 database_rejects_schema_37_as_a_future_version 证明 v37 binary 不开 v38 数据库。旧版本迁移使用 BEGIN IMMEDIATE 包住 migration SQL 和 user_version 更新；v37 当前分支需要加入 37→38 additive migration。操作 revision 默认 0，旧 active rows、IDs、FTS、Managed/AI 不回填、不 stale；operation/stage 初始为空。

升级前需要应用停写或使用 SQLite online backup API 生成一致快照（WAL 开启时不能只复制 main db 文件而忽略 WAL）。migration failpoint 必须验证新字段/表和 user_version 同时回滚。v38 运行后回到 v37 app **必须恢复完整 v37 backup**；单独把 user_version 改回 37 或删除几张表不是安全降级，会丢失新版本活动数据。升级/回滚恢复演练应作为 D1 发布门槛。

附加表、索引、stage payload 与 FTS publish 会增加 DB/WAL 和磁盘峰值；D0 只量到简化行的 10k。若完整字段 staging 的磁盘占用或单一原子 merge 的锁时间超过 Owner 定义预算，替代设计是版本化 generation 表和每来源 active_generation pointer：stage 新 generation 后只原子切换 pointer，旧 generation 延后清理。该方案能缩短 publish 锁，但会扩大 query/schema 改动、同时存两代数据，并要求 Managed/AI/FTS 查询严格 join active generation；需另做 prototype/测量和 Owner 批准。

## 建议 D1 实施边界与 Owner 决策

D1 获准后只按 A+B+C 实施：

- src-tauri/src/global_index/commands.rs：保持命令校验/入口，不在 command 层直接写 Rebuild status。
- src-tauri/src/global_index/coordinator.rs：重排 Coordinator::rebuild、run_index_cycle、run_with_background_admission；intent/claim → verified Service admission → stage → fenced atomic publish；移除 admission 前全卷 stale；明确 disabled 和 Unknown 路径。
- src-tauri/src/global_index/windows/service.rs 与 windows/mod.rs：IPC v3 请求/事件/response 的 request ID、protocol 和 source identity 检查；event/request 顺序验证；区分 typed busy、proven-not-sent、unknown；只允许 owner stage sink 收 event；未知结果禁止 Direct fallback/replay。
- src-tauri/src/global_index/windows/service_host.rs：若现有 v3 frame 可承载首个 admission 事件，则在 provider/任何数据事件之前发送；Service 先做 busy acquire 与 source identity validation。当前 response-only 行为是在 provider 完成之后，不能充当 admission ack。
- src-tauri/src/global_index/repository.rs：stage batch、CAS、cancel fencing、atomic publish；在一笔 tx 中处理完整 entry/Managed/AI/FTS/count/source health/checkpoint。不要复用六条无事务 stale SQL作为 Rebuild staging。
- src-tauri/src/db/schema.rs 及 schema migration tests：按 Owner 批准后实现 v38，验证 v37 populated upgrade、migration rollback、v37 future-schema refusal 和 backup restore。
- 新增 D1 的 Coordinator/Service/Repository 整合测试和完整生产 GlobalEntry 字段测试；保持现有 #377/#380 diagnostic tests 和 CI 门槛不变。

仍需 Owner 明确：IPC v3 的首帧 admission 是否采用 SourceState(indexing) 等已有 frame 语义、如何验证真实 source identity；Unknown 的人工恢复/取消是否允许，以及何时能释放唯一 source owner；接受的最大事务锁/WAL/磁盘预算；10k 结果是否足以进入 D1，或先批准 generation-pointer prototype。D0 未授权 Schema v38、IPC v4、生产实现、Draft 转 Ready、合并或 Issue 状态变更。

## 执行和 CI

- 指定工具链 Rust 1.97.1 已安装在 /tmp，未进入仓库。
- 直接在 Linux 环境运行 cargo test --manifest-path src-tauri/Cargo.toml --test issue_380_d0_test_contract -- --nocapture 未到测试编译：宿主缺少 glib-2.0 >= 2.70 pkg-config 开发文件（Tauri 依赖链）。
- 同一仓库测试文件通过仅含 rusqlite = 0.39 bundled 的 /tmp 临时 harness 编译运行：18 passed、0 failed、0 ignored；含进程退出 crash、失败回滚和 1k/10k 指标。harness 不在仓库 diff 中。
- 未运行完整本地 CI、没有触发 CI Full Validation/100k/500k benchmark。Draft PR 的普通 PR CI 检查状态在创建 PR 后记录在 PR checks 和最终交付中；如 PR CI 与本地隔离 harness 有差异，以 CI 结果为准。
