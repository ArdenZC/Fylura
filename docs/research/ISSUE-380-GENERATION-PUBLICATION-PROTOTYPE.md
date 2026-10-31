# #380 Generation Publication：Owner 决策输入

终态：**#380-GENERATION_PROTOTYPE_READY_FOR_OWNER_DECISION**。

**工程结论：C2 值得进入下一轮设计冻结；C1、C2、C3 均未获得生产 GO。C2/C3 的现有读取器整合明确 BLOCKED。** 实验读取器可实现跨表原子可见，但未修改的 Schema 37 Global Search/FTS、Managed/AI 和 File Library 不跟随新 generation。新 namespace ID 的 anchor 还会提前暴露给旧读取器；未参与 revision 的 metadata writer 会丢失或遮蔽用户修正。发布 pointer 快不能解除这些阻断。

## 合同、基线与交付边界

- Owner 合同：[Issue #380 comment 6137395908](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6137395908)，Track B，仅测试研究。
- 已阅读 D0：[PR #385](https://github.com/ArdenZC/Zen-Canvas/pull/385)，候选 `8ab50cb627712abef8debfa786d9fb2354ef9581`，tree `84405758645a126f7f8d19f3397c106c604fef2b`；Owner 验收为 reference-model evidence，未授权 merge。其 7.413s/10k 不构成本实验的预算或可直接比较的基准。
- 起始 remote master / HEAD：`3275ac7c72df8ae4139a30f16c7f649971324e97`；tree `009576241cd4e8e2ba12cc644a368314f599faa6`，fetch 后一致。
- 独立分支 `research/issue-380-generation-publication`，独立 worktree；不复用 Track A、#381 或 #386 分支。
- 只新增 `research/generation_publication/`、本报告及 JSON 证据；未修改生产数据库、Schema、Repository、Coordinator、Global Search、IPC、installer、CI routing 或 #381 PR。没有创建 Schema v38 migration。
- 所有 DB 在唯一临时目录，关闭后删除任务自己的 DB/WAL/SHM；不枚举、打开或修改用户数据库/文件。

## Schema 37 的对应范围

[schema37.py](../../research/generation_publication/schema37.py) 从当前 `src-tauri/src/db/schema.rs` 提取真实建表 SQL、索引、触发器及相关 additive columns。包括完整 19 列 `global_entries`、29 列 `files`、25 列 `file_fingerprints`、40 列 `operation_logs`，完整 Global Volumes、Managed Scope/Entry、AI analysis/jobs/items、File Library tag/query/view、classification history/feedback、Analysis Run/Finding/Evidence/Decision 和相关 scan/dedupe/organization 表。原始表保留生产 NOT NULL/CHECK/FK 和 FTS/count/canceled-terminal 触发器。测试显式检查九张版本投影表的列序列与对应原表相同。

这是 **Schema 37 相关域的隔离结构切片，不是生产 migrate() 执行或完整数据库升级测试**；automation/content/rule 等不相关域未建。`user_version=37` 仅标明切片来源，不能交给 app 当正式数据库。`rp_*` 表是完整列的实验投影，CTAS 不继承原表 CHECK/FK；真实 FK anchors 继续保留在原表。投影写入只经过测试 helper，因此不能把这些投影误认为可上线的关联 schema。所有 Python 文件及 Schema 源 SHA-256 均记录在证据中。

每个规模有一个 volume、一个 managed scope；每个 file 都关联 Managed、AI analysis/job/item、fingerprint、tag、classification history/feedback、operation log 和 Analysis finding/evidence/decision。AI 用户修正全部存在，半数 job 已 canceled，另一半 running；每两条 namespace entry 共享一个 physical identity，代表 hardlink。未指定的字段使用生产默认值。新投影统一替换父目录、增加 size/mtime、最后一行 tombstone、使旧 AI/fingerprint/finding 失效；用户修正、取消终态、tags、历史与决策保留。0 行仍有来源健康/checkpoint。

这些是确定性的合成行，不是真实扫描、实际 AI provider 或 native identity 验证。IDs 预先分配，不执行生产 blake3 identity 算法。源码 `global_index/models.rs::stable_entry_id` 证明：`files.id`、namespace entry ID 和 physical identity 不可互换；Windows entry identity 包含 physical ID + parent ID + leaf name，rename 可能产生新 namespace ID，macOS 的 physical ID 分支不同。这里的稳定 ID 通过测试只覆盖父目录路径变化、hardlink 和旧引用保留；新增/替换/Windows rename 的完整映射策略仍 BLOCKED，不能以路径或 physical key 重分配 `files.id`。

## 三种机制及正确性结果

| 方案 | 测试实现 | 可证明内容 | 尚未解除的约束 |
|---|---|---|---|
| C1：active 大事务 merge | 先写完整 stage；BEGIN IMMEDIATE 内更新九张 active 表，原 FTS/count 触发器参与；同事务完成 checkpoint/health/op | 完整投影原子发布；所有 active 表 SQL failpoint 回滚；WAL snapshot reader 保留旧读；旧 FK/用户历史不被删除 | 扫描规模增长仍是长 writer；缓存用户状态要求所有 writer 参与 fence。提交后不提供独立持久旧代读取，旧读依赖原有 read transaction |
| C2：完整 generation + pointer | 每批最多 250 行，写所有九张完整投影及独立 trigram FTS；seal 后以 source/op/metadata revision fence 切换 pointer | 专用读取器在同一 transaction 捕获 active generation；Managed、AI、files、source/count/checkpoint 和 FTS 同代；generation lease 阻止旧代 GC | 旧生产读取器/FK anchors 不跟随 pointer；新增 anchor 提前可见；所有查询与 metadata writer 的版本合同尚不存在 |
| C3：小事务版本写入 + 一致性视图 | 只追加变化的行版本；已提交 watermark 以下每个 ID 取最新版本，FTS 同样过滤 superseded 版本；未变行复用旧版本 | 批量提交过程中旧视图不变，最后 watermark 与 op 原子提交；跨表一致读；旧版本引用安全 | 必须保留 watermark 与版本索引，本质仍需版本化读协议；原 active 表分批更新本身不安全；读/GC 更复杂 |

C3 **不是**把 active 表分批改完再设 READY。负向对照实际提交一条 active entry 更新，文件/AI/checkpoint 尚旧，投影同时不等于完整旧代或新代。追加版本使用 `(stable key, generation)` 索引；缺少这个索引的早期实验读发生扫描膨胀，已在最终代码中纠正，早期中止结果未作为性能证据。

[13 个 unittest 方法](../../research/generation_publication/test_prototype.py) 全部通过，包含 12 个 C1/C2/C3×0/1/1k/10k 单元格及参数化故障矩阵：

| 要求 | 执行证据 / 判定 |
|---|---|
| Global Entries、FTS、Global Search | 完整列投影比较，trigram MATCH、stale 排除、mtime DESC/ID ASC；原 FTS external-content integrity-check。专用 SQL 通过；生产 tier/punctuation/IME 搜索链没有执行，C2/C3 整合 BLOCKED |
| Managed、File Library、历史 | 同 generation 比较完整 Managed 与 files 投影；完整 history/feedback/log/tag/evidence/decision 表逐行保持；FK check 与 integrity_check 通过；未调用生产 Library Query V2，仍需整合证明 |
| AI jobs/items、analysis、用户修正 | canceled 保持 terminal；running 失效；classification_json/user_corrected 不删除；Analysis finding 失效但 evidence/decision 保留 |
| 健康、count、checkpoint、revision | 与独立 Python Oracle 比较；禁止取消、旧 revision、disabled、stable volume/mount/provider 改变或 metadata revision 变化的发布 |
| 取消/旧 stage writer | 首批提交后 cancel 增 revision；下一 batch 与 publish 被 fence；可见旧投影保留 |
| SQL 失败 | C1 九张 active 表分别注入真实 missing-table SQL 错误及 pointer 错误；C2/C3 pointer 错误；stage 用 RAISE(ABORT)，未完成 manifest 拒绝发布 |
| 崩溃、WAL | 每策略子进程分别在 stage 首批后、publish 多表写后/指针写后未提交、提交后 `_exit(73)`；重新打开验证 old/old/new；未完成 operation 转 unknown + 新 revision，禁止重播 |
| 跨表一致读、GC | 显式 read transaction 跨 publish 仍读旧代；durable generation lease 阻止 GC，释放后回收且 active/FTS/用户历史正确。C3 保留未变行的旧版本；原 FK anchors 永不删除 |
| 并发用户修正 | writer 在自己的事务更新 correction + metadata revision，随后 publish 必须拒绝。反例：legacy writer 不增 revision，发布成功但晚到修正在实验视图中消失，BLOCKED |
| 新 identity anchor | 新增原表 namespace anchor 后，专用 C2 仍旧代一行，legacy reader 已看两行，证明纯 pointer + 旧 FK 方案不足，BLOCKED |

Oracle 是独立的逐行 Python transformation，不调用发布 SQL；并非真实 provider 或 production event stream。GC 当前为一笔事务，只证明测试规模的安全性；大规模分批 GC、崩溃 lease 回收、Unknown liveness、backup 的版本兼容以及旧 app 的读取隔离仍需 Owner 冻结合同。lease 不会因进程死亡自动删除，安全代价是可能长期保留旧代。原表 tombstone/anchors 的最终回收也未解决。

## 测量方法和实际结果

原始证据：[linux-bounded.json](evidence/issue-380-generation/linux-bounded.json)，[recovery-and-busy.json](evidence/issue-380-generation/recovery-and-busy.json)。Linux x86_64，3 vCPU，Python 3.12.14，SQLite **3.53.1**；WAL、synchronous=NORMAL、wal_autocheckpoint=0、busy_timeout=5000ms，batch=250。不是 D0 的 rusqlite bundled SQLite，也不是 Windows/macOS native acceptance。

每次写事务记录 BEGIN IMMEDIATE 请求→返回的等待、返回→COMMIT/ROLLBACK 返回的持锁区间、outcome 和单独 ROLLBACK 成本。持锁计时包住整笔事务的 SQL、索引/FTS、Python 参数准备、提交和 writer 同步 hook；是 SQLite API 边界的保守持锁包络，不是 OS lock trace，不能当单条 SQL 时间。专门 writer 在 publisher 获锁后发起 BEGIN IMMEDIATE，另一个独立 connection 持续写 app_settings，每 10ms 尝试一次；用户修正 fence 则用独立正确性测试。无额外重试。只跑每个配置一次，无统计分位或线性外推。

| 行数 | 方案 | stage 总耗时 ms | 成功 publish 持锁 ms | publish contender 提交耗时 ms |
|---:|---|---:|---:|---:|
| 0 | C1 | 1.29 | 0.401 | 1.120 |
| 0 | C2 | 1.37 | 0.075 | 1.075 |
| 0 | C3 | 1.41 | 0.128 | 1.162 |
| 1 | C1 | 1.96 | 0.797 | 1.165 |
| 1 | C2 | 2.24 | 0.091 | 1.117 |
| 1 | C3 | 0.73 | 0.072 | 1.126 |
| 1k | C1 | 30.07 | 171.429 | 178.985 |
| 1k | C2 | 36.11 | 0.110 | 1.125 |
| 1k | C3 | 35.34 | 0.105 | 1.143 |
| 10k | C1 | 289.78 | 1699.162 | 1731.118 |
| 10k | C2 | 364.38 | 0.175 | 1.160 |
| 10k | C3 | 360.07 | 0.110 | 1.160 |

10k 的成功 publish 请求总耗时分别为 **1700.438 / 1.375 / 0.261ms**，与上表持锁时间分开。C2 的 1.375ms 包含 BEGIN 等待。三者 publish contender 均 committed；12 个实验的背景 writer 共 **513 次 committed、0 BUSY/其他错误**。这只证明本次 workload 未超 5s，不保证 writer 公平性或长期高负载 liveness。

| 10k 指标 | C1 | C2 | C3 |
|---|---:|---:|---:|
| stage 写事务数 | 322 | 322 | 243 |
| stage 最长持锁 ms | 2.479 | 7.444 | 7.591 |
| stage 累计持锁 ms | 267.618 | 342.254 | 305.115 |
| 失败 publish 持锁 ms | 1423.060 | 0.135 | 0.128 |
| 失败路径总耗时 ms | 1424.310 | 1.402 | 0.403 |
| 单独 ROLLBACK ms | 0.330 | 0.010 | 0.008 |
| publish 后 WAL MiB | 118.99 | 102.96 | 97.47 |
| publish 后 DB+WAL+SHM MiB | 119.24 | 103.18 | 97.69 |
| 已观测峰值 DB+WAL+SHM MiB | 130.81 | 114.91 | 115.55 |
| 安全 GC 最长持锁 ms | 105.502 | 126.005 | 172.941 |
| TRUNCATE checkpoint ms | 35.925 | 40.547 | 36.868 |
| checkpoint 后 main DB MiB | 77.87 | 75.84 | 74.03 |
| 完整 MATCH 查询耗时 ms | 10.662 | 11.901 | 14.875 |

峰值由 10ms 采样与阶段边界观测共同记录，属于已观测下界；原始 JSON 有各阶段 db/WAL/SHM 字节数及每笔事务。fixture baseline 每方案约 58.64 MiB，包含历史关联和测试用旧投影/索引；C1 也保留相同研究基础设施，因此不能把这些绝对数当生产数据库增量。失败写产生的 WAL、未 checkpoint 的 seed、独立 FTS 和 GC 写均包含在峰值；GC 逻辑删除不收缩 main DB，未执行 VACUUM。最终 parent peak RSS 约 290 MiB，未代表所有子进程总和。

C1 的失败路径先完成多表/FTS写，再错误回滚；单独 rollback 虽仅 0.330ms，但已经占锁 1423ms、写入 WAL，不是“失败免费”。C2/C3 的发布只改 pointer/op，小 rollback 不抵消 staging 的 I/O/磁盘和回收成本。C3 在此全量变化场景仅复用部分 Managed/canceled job 版本，磁盘收益有限，读和 GC 更贵。

真实 5s busy 合同控制：[verify_recovery.py](../../research/generation_publication/verify_recovery.py) 使用预先建立的独立连接。刻意持锁 50.187ms 时 writer 在 53.600ms committed；刻意持锁 5250.191ms 时 writer 在 **5006.963ms 返回 SQLITE_BUSY(5)**。这是 timeout contract 控制，人工持锁时间不混入 workload 性能表，也不等同 ServiceBusy。

1k 真实 WAL crash 后关闭原 connection，再 reopen + 完整跨表投影验证：C1/C2/C3 分别 **35.755 / 38.405 / 43.345ms**（包含读取和校验成本，不声称纯 recovery latency）。成功发布后 SQLite online backup 分别 **4.197 / 3.990 / 5.206ms**；restore projection/历史/完整性检查通过。没有用忽略 WAL 的 main-file copy，也没有运行 v37→v38 migration 或旧 binary rollback 演练。

## 有界执行、可复现性与验证

正确性顺序严格为 0、1、1k、10k，之后才开始测量。运行器默认只包含这四档；100k 必须显式 `--bounded-100k`，500k/1m 无入口。**本交付未执行 100k**：生产可见性/metadata writer 的正确性仍 BLOCKED，继续放大不能解除该 gate。未自动触发 Full Validation、performance CI、Codex Review 或 native Owner 主机测试。

资源保护：Linux 1.5GiB address-space hard cap、启动至少 3GiB 空闲、每 fixture 1GiB DB+WAL+SHM 采样上限，测量阶段 120s SQL progress 截止、正确性子进程 180s timeout、crash 子进程 30s。采样/SQL deadline 从 fixture 初始化后开始，不能声称覆盖 seed 的整个 I/O 生命周期；100k 若另行执行应先加强初始化期硬盘/时间保护。当前所有小规模实验无 resource_errors，数据库临时目录已清理。

```bash
python3 -m unittest discover -s research/generation_publication -p test_prototype.py -v
python3 research/generation_publication/measure.py --output /tmp/new-bounded-evidence.json
python3 research/generation_publication/verify_recovery.py --output /tmp/new-recovery-evidence.json
DOCS_DIFF_BASE=3275ac7c72df8ae4139a30f16c7f649971324e97 npm run test:docs
git diff --check
```

measure.py 自行执行完整正确性 gate，失败时不进入测量，已有输出文件拒绝覆盖。最终已执行：13/13 tests，12 个配置全部测量，三个 backup/recovery 检查及两个 writer 控制。普通 PR CI 若路由通过也不代表 Python 实验在 Windows/macOS 自动运行；该测试没有加入 CI。最终 commit/tree 与 Draft PR 由 PR 元数据和交付记录提供；证据 source_sha256 将实测精确绑定到代码文件，报告不嵌入自身变化造成的自引用 hash。

## Owner 可直接决策的建议

1. **保持 D1-B/C、Schema v38、#381 整合的 production HOLD。** 本任务没有解除这些门槛，也没有修复 #328 native 历史锁原因。
2. **优先冻结 C2 的完整读写合同，再决定是否授权生产实现。** 明确 stable identity anchor 与版本 payload 如何关联；所有 Global Search tier、FTS rowid、Library Query V2、Managed/AI/readiness、history resolve、source health/checkpoint 在同一 snapshot/generation 下如何读；旧 desktop/Service 的能力 gate 必须由 Track A 独立决议。
3. **每个 metadata writer 必须有可验证的参与方式。** 应选择共享 revision + publish CAS/rebase，或把用户状态从 scanner generation 中分离并定义 snapshot overlay；普通旧 writer、发布后的用户修正、enabled/consent flip、AI completion 都不能被快照遮蔽。当前 prototype 的 participating-writer fence 只证明“冲突拒绝”，不提供完整 rebase/liveness 实现。
4. **Owner 定义阶段预算而非 pointer SQL 预算。** 必须分别限制 stage 单批/累计锁、publication、GC、checkpoint、WAL 和双代/anchors 磁盘峰值，定义 busy、Unknown、取消和 lease 崩溃回收。C1 本次 10k 未超 5s，仍不支持更大 Rebuild；C2 的短发布也不保证 staging/GC 能永远满足 5s writer 合同。
5. C3 保留为备选：若实际 rebuild 低变化率带来足够磁盘收益才值得承担版本选择/索引/GC 成本；当前证据不足以优于 C2。先补齐新增/删除/identity replacement、真实 production query 和并发状态 writer 的整合正确性，再安排一次受保护 100k 及 Windows/macOS 定向验证；不外推或自动开启 500k/1m。

**终态可交付，架构可决策；生产整合仍 BLOCKED / NATIVE_UNVERIFIED。** Review、migration 分配、Ready、merge、Issue 关闭与发布均由 Owner 决定。
