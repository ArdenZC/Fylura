# Zen Canvas — ISG-00 低价值文件索引治理：Owner 源码研究、成本模型与实测合同

> **版本**：2026-10-11 · Owner Research R1（静态研究已完成；成本实测未完成）
> **源码基线**：`ArdenZC/Zen-Canvas master@df37531f785193a948409e2f8cb36369561c6a6f`
> **合并后 CI**：[38111501260](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38111501260) **SUCCESS**；此前 #335 已 MERGED，#329 CLOSED
> **跟踪**：[#370](https://github.com/ArdenZC/Zen-Canvas/issues/370) / [#371](https://github.com/ArdenZC/Zen-Canvas/issues/371)；关联 [#328](https://github.com/ArdenZC/Zen-Canvas/issues/328)、[#340](https://github.com/ArdenZC/Zen-Canvas/issues/340)
> **职责**：ChatGPT/Owner 负责研究、方案、源码复核、实验设计和解释；Codex Cloud 仅负责必要测试/benchmark/诊断代码和执行验证。
> **证据边界**：这是基于真实代码、已保存的 CI/benchmark 任务材料及官方 API 的**静态研究和实测设计**，不是已采集 Zen 真实设备的资源改善数字；不授权直接更改生产默认索引。

## 1. Executive Decision

**建议把全盘元数据索引中低价值子树的成本治理列为 P0 产品架构工作；执行顺序为「先数据安全和覆盖语义 → 合成成本基线 → 独立策略内核 → 共享写入接入 → 平台读属性前移 → 历史数据安全迁移 → UI/灰度」。** 不将 AI 资格筛选替代索引治理，也不引入全新的扫描器、二级搜索库、定时 watcher 或 AI Queue。

**当前能证明的事实**：Zen 已有多条文件来源，Windows MFT 枚举经临时 staging/目录解析/`symlink_metadata`/sink 入库；Global Index 写入触发 SQLite/FTS 并可能触发 Managed AI 入队；Managed Scanner 的目录名剪枝并非 Global Index 过滤；Schema 37 存在针对 `global_entries` 的级联依赖；现有 Windows 基线“输出 0 项即异常”不容许未经区分的全部策略排除。**当前不能证明的事情**：低价值文件占机器总量的百分比，节省的 RSS/CPU/IO/Token 绝对值，macOS Spotlight gather 下降，或 Windows MFT 原生枚举条目下降。

## 2. 权威和真实调用链（截至基线的有证据主要入口，不冒充全仓枚举证明）

| 域 | 当前源码与行为 | 主要成本或风险 | 需要进一步证明 |
|---|---|---|---|
| Windows 初次索引 | `global_index/windows/mft.rs`：`FSCTL_ENUM_USN_DATA` → `stage_mft_records` 写临时 SQLite → `load_staged_directories` / `resolve_directory_paths` → `stream_staged_entries` → 普通文件 `symlink_metadata` → `GlobalIndexSink` | 原生枚举量、staging 写放大、父链与路径解析、per-file stat、sink 批处理 | 精确阶段计数和时间分解、目录路径决策缓存的命中率 |
| Windows 增量 | `global_index/windows/usn.rs`：USN journal 变更、文件身份/父链查询、stale/upsert、日志断档后的恢复重建 | 目录移动后覆盖变化、过时 status、失去父锚点 | 排除→包含、包含→排除、目录 rename、USN gap |
| Windows Service | `global_index/windows/service.rs` / `service_host.rs`：版本化 IPC v3、服务忙锁、Service/Direct | LocalSystem 权限边界、不同策略版本 | 正常并发 busy 与误重复调度分离；两路径同收录结果 |
| macOS 初次索引 | `global_index/macos/spotlight.rs`：`NSMetadataQuery` gather → 枚举结果 → `metadata_item_to_entry` → 批处理 | Spotlight 结果处理、文件属性/父身份、Zen 自身数据库开销 | path-first 排除是否有效；Spotlight gather 自身不是 Zen 能承诺的收益 |
| macOS 更新 | Spotlight 更新、`macos/fsevents.rs` 缺口补偿 | 移入/移出排除域、云占位/权限失真 | 原生证据，目前 UNVERIFIED |
| Global Index 持久化 | `global_index/repository.rs::upsert_global_entries_batch` UPSERT `global_entries`，调用 `enqueue_ai_jobs_for_entry_with_scopes`；`db/schema.rs` 管理 FTS5/trigram、索引及触发器 | 每条入库行引发 B-tree/FTS 和 managed 状态的持久成本 | 真正每条写入的计时、WAL/FTS 字节和队列数量 |
| 普通查询 | `global_index/search.rs` + `global_index/tests/global_search_benchmark.rs`，候选窗口 4096、结果/分页独立控制，已有 12 类查询 | 对应覆盖变更后的搜索召回损失/误排、查询 p95 | 不能将「排除了文件」产生的更快查询等同于不损害产品 |
| Managed Scanner | `scanner.rs`, `path_filter.rs`; `db/queries/mod.rs` / `watcher.rs` 使用名字剪枝 | 已经存在的误排用户目录风险（见下） | user-chosen root 遇到同名组件的确切行为及回归 |
| Managed AI | `managed_scope.rs`, `repository.rs`, `legacy_queue.rs`, `db/queries/organization/mod.rs`、既有 worker | 初始 backfill 数量上限不等于增量零成本；不能全量交给模型 | 各 producer 是否都经 ADR-0013 资格准入，需精确入口测试 |

**补充架构边界**：ADR-0012 明确 File Library Query V2、BrowseService、Legacy Store 与 Global Search 各自权威；ADR-0013 明确 `Indexed ≠ Managed ≠ AI Eligible ≠ AI Analyzed ≠ Mutation Eligible`。未来 Index Scope 新策略不应使 File Library 管理状态与 `global_entries` 的来源身份脱钩。

### 2.1 真实成本应按步骤分解，避免「排除 80% 文件 = 节省 80% 资源」的错误推理

可将单卷初始成本建模为：

`T_total = T_native_enum + T_stage + T_directory_resolve + T_metadata + T_sink_sqlite + T_fts + T_ai_admission + T_status + T_contention`

`N_native` = MFT/Spotlight 交付原生记录数；`N_staged` = 临时记录数；`N_resolved` = 可定位路径数；`N_included` = policy Included 数；`N_anchor` = 必须留用的父身份锚点数；`N_written` = 最终入库数；`N_ai_candidate` = 进入 AI 候选判定数。这些变量不可互换。

- **只在 AI 层排除**：通常保留上游初次索引和 SQLite/FTS 成本，仅降低模型和某些队列成本。
- **sink 前排除**：可期望减少 `global_entries`/FTS 写入、AI 候选和后续增量持久化，但未必减少 MFT staging 或 `symlink_metadata`。
- **native metadata 读取之前排除**：有机会减少额外 `stat`/对象构造/数据库操作；Windows MFT 原生卷级枚举和既有 staging 初版仍在。
- **OS 自身发现工作**：不在第一版可优化承诺内。Microsoft 官方明确 `FSCTL_ENUM_USN_DATA` 用于枚举 MFT 记录；Apple `NSMetadataQuery` 只提供结果范围/predicate 接口，不等于 Spotlight 自身索引成本降低。

### 2.2 现有基准不是索引成本实测

当前 `global_search_benchmark.rs` 有规模配置 **100k、500k、1m、2m、5m**，12 类查询、候选 4096、30 次 warm/reopened 抽样与 100 ms warm p95 合同；源码确认这些是现有能力，**不能推导所有规模已通过**。历史 #342 已 CLOSED，#348 已 CLOSED，近期 #368 记录 500k variance 的研究文档。这些结果主要描述**搜索查询**性能，不足以回答**索引发现、处理、FTS 写入、重建、watcher 资源消耗**。新成本研究应扩展同一 benchmark/harness 而不是建立第二套查询延迟权威。

## 3. P0 验证发现：当前名字黑名单的误排范围

已核对 `src-tauri/src/path_filter.rs`：`is_ignored_dir_name` 对目录文件名直接 `to_ascii_lowercase()` 并查 `SKIP_DIR_NAMES`；其中包含 `.git`、`.vscode`、`node_modules`、`.venv`、`target`、`build`、`vendor`、`appdata`、`windows`、`program files`、`programdata` 等。`scanner.rs`、`db/queries/mod.rs`、`watcher.rs` 使用它。因此**确认有按目录名剪枝的机制风险**，不代表已在真机逐一复现全部反例，也不代表 Global Index 已过滤。

| 路径/类别 | 应具备的产品语义 | 现有名称匹配的潜在风险 | 改进方向 |
|---|---|---|---|
| `D:/Projects/Windows/README.md` | 用户项目应可管理、搜索 | 名为 Windows 的项目根子树可能被剪掉 | 系统目录必须是平台识别且属于已确认系统卷/路径 |
| `D:/Design/build/logo.ai` | 用户资产应可管理 | 仅凭 build 名可能剪掉设计作品 | build 需要项目生成物/上下文证据，未知时保守 Include |
| `D:/Configs/AppData/notes.json` | 不能把任意同名 AppData 当成系统内部 | AppData 名称假阳性 | 从系统 known folder/稳定身份判别，仅细化缓存目录 |
| `.vscode/settings.json` | 不应断言无用户价值 | 编辑器配置/任务定义可能被剪掉 | 与用户文件政策、显式覆盖/保留原有 Managed truth 协调 |
| `node_modules`, Python `.venv`, Rust `target`（真实依赖目录） | 普通索引可默认精简、AI 默认排除 | 当前 Managed 名称剪枝虽大体合理，但可能碰撞自定义目录 | 结构证据 + 显式 include 优先级，且 Global Index 和 Managed 策略分离 |
| 用户手选根目录自身等于 `build` | 必须解释为何范围不可见 | 祖先/根自身是否被跳过有实现级边界 | 独立边界/根恢复测试 |

**Owner 决定**：此问题需要独立的 Managed Scan 正确性评审与 test-only 回归先行，不把整套新 Index Policy 黑名单立即替换当前 Managed 扫描器。也不把「低价值」认作可以永久隐藏的法律或用户价值事实。

## 4. P0 验证发现：SQLite 主键删除的级联损失

`src-tauri/src/db/schema.rs`（Schema 37）已定位如下外键：

| 引用表 / 列 | 指向 | 删除父 `global_entries` 的行为 | 决策 |
|---|---|---|---|
| `managed_scopes.global_entry_id` | `global_entries(id)` | `ON DELETE SET NULL` | 可能失去范围对特定文件身份的绑定；不能误作无影响 |
| `managed_entries.global_entry_id` | 同上 | `ON DELETE CASCADE` | 可能清掉具体 Managed membership 和用户控制状态 |
| `ai_analysis_state.global_entry_id` | 同上 | `ON DELETE CASCADE` | 可能直接清掉语义评估/校正状态 |
| `ai_jobs.global_entry_id` | 同上 | `ON DELETE CASCADE` | 可能清掉队列和任务历史 |
| `ai_job_items.global_entry_id` | 同上 | `ON DELETE CASCADE`（且其 `job_id` 指向 `ai_jobs`，同为 CASCADE） | 可能连带移除 job item 记录 |

**法律式约束**：任何通过 Index Scope 排除产生的旧行清退，都必须预先对 FK/触发器/非 FK identity 引用做动态 inventory，并区分正在运行、已完成、用户校正、不可再生历史；不存在完整的可恢复证据时只能保留这些行或暂停清退。仅不展示/不更新某条 Search 元数据与物理 DELETE 不是同一行为。SQLite 官方文档确认 CASCADE 和 SET NULL 具体语义；WAL 同时只容许一个 writer，分批清退仍有写锁和 checkpoint 成本。

**尚未完成**：逐字段的用户 Notes、其他以路径或 file ID 间接关联的数据源所有权以及生命周期动态值统计；这些不得凭上面五条 FK 猜测为完整枚举。

## 5. P0 验证发现：全排除的零结果陷阱

`windows/mft.rs::MftBaselineStats` 目前只有 `staged_records`、`resolved_entries`、`emitted_entries`、`checkpoint_usn`。`validate_initial_baseline()` 对 `emitted_entries == 0` 返回 `GlobalIndexError::RebuildRequired`。这在默认广覆盖下可用作异常信号，但如果用户明确选择「索引零文件」或整个受管理卷被合法排除，绝不能自动要求无限重建。

未来必须在发布任何新的 Policy 排除之前定义：

- 源完好：真实原生枚举结束且具有受支持的原生 coverage/identity 证据；
- 排除合法：`native_seen > 0`、`resolved > 0`、`policy_excluded = resolved`、`included = 0`，没有来源损坏/权限错误（例，具体跨平台资格还需设计）；
- 源异常：真实 native 0、staging 或目录解析失败、USN gap、权限不明/部分可用，不得伪装“全被过滤”；
- 状态完成：只对具体 `active_policy_revision + source_checkpoint + provider_generation` 的覆盖判 `ready/complete`。

这要求新状态协议与 replay/backfill 测试；尚不授权直接修改 MFT 的空结果断言。

## 6. 实验设计：不混淆实验与产品已实现

| 组 | 行为 | 可提供的证据 | 不足 |
|---|---|---|---|
| A / broad | 当前完整管线，无策略 | baseline 结果、耗时、行数、资源 | 无过滤收益 |
| B / dry-run | 只计算策略决策，照常写入 | 排除解释、误排、匹配开销 | 不代表减少真实 SQLite 写入 |
| C / sink-gated | 在**合成隔离库**的入库前拦截，仅保留 Included/Anchor | 新增行数/FTS/WAL/AI 候选减少，正确性 A/B 对照 | 不保证节约 MFT stat |
| D / pre-metadata | 在合成/受控原生夹具的属性读取前过滤 | 属性读取次数/实际耗时，对 C included 集合同一性 | native 提前过滤能力需要平台验证 |

合成数据固定种子，独立保存分布清单而不是「随机造一百万文件」：用户文档/照片视频、项目 `src`/README、非用户 package 依赖、缓存、真正构建产物、名字反例、隐藏项目配置、中文路径、Unicode 规范化变体、reparse/symlink/junction 测试标签、云占位。不能让真实用户数据进入公开 Actions artifact。

### 6.1 预算与口径

- **每平台/规模**：SHA、tree、Runner/OS、CPU/内存/文件系统、样本数、固定种子、首次/热启动、A/B/C/D 相同输入、持久 DB 格式与 schema；多次同 runner paired 对照。不同 GitHub hosted 机器之间的耗时只能说明跨-run variance，不能直接作因果归因。
- **阶段事件**：`native_seen`, `staged`, `resolved`, `excluded`, `included`, `anchor`, `stat_attempted`, `stat_success`, `db_row_upsert`, `fts_trigger`, `ai_admission_candidate`, `queue_insert`, `rebuild_trigger`, `source_error`; 强制守恒关系 `included + excluded + undetermined == policy_evaluated`（Anchor 有重叠时需单列优先级）。
- **成本**：stage bytes/time、path resolve CPU/time、metadata stat 次数、SQLite pages / WAL / db size、FTS entries/triggers、批次和总耗时、UI 搜索 p50/p95/p99、Idle RSS/CPU、维护风暴和掉电/中断 recovery。
- **正确性**：结果集 identity 稳定，包含所有用户内容反例，no false Ready/Complete、策略变更 backfill、USN rename、Service/Direct parity、WAL/BUSY、AI/Consent no unintended work。
- **门槛**：硬性 0 数据静默丢失、0 非授权 AI、0 不必要重复队列、0 误将受保护系统域视为可删除；收益阈值由实际 A/B/C/D paired 分布决定，不提前给百分比承诺。

### 6.2 Codex 唯一允许的工程配合（先报告 Owner，保持代码试验隔离）

1. 在独立 test/benchmark-only 分支实现受控数据分布/计数与 trace exporter；不可修改生产索引默认 behavior、Schema、Service IPC、真实用户 DB。
2. 使用现有 benchmark authority，先较小样本证实计数和正确性，确认资源预算后再推进 100k/500k/1m；2m/5m 需另外给出价值和 runner 预算，不能假称已跑。
3. 报告每种实验实际实现了哪一级（单纯模型 C/D != 真实原生提前 stat）；没有真机的 macOS 和真实 Windows MFT 成本标 `UNVERIFIED`。
4. 测试完整后提交 test-only Draft PR 供 Owner 进行人工 review；不启用 Codex Review，不修改 Windows 本机环境。

## 7. 路线决策与阻断

| Gate | 何时可继续 | 当前结论 |
|---|---|---|
| ISG-00 static research | 已完成源代码抽样与外部 API 核验，可定义测量目标 | **PASS：静态研究可供评审** |
| ISG-00 measurable baseline | 有同 head 固定种子、可复现数据，A/B/C/D 的真实计数/耗时 | **NOT MEASURED：Codex 仅承担试验代码和执行** |
| ISG-01 draft ADR | 产品覆盖与迁移语义、人类可理解的 UI truth，明确受保护资产 | **可由 Owner 完成；参见独立 ADR** |
| ISG-02 纯策略测试 | ADR Owner 签署、反例通过，不触发生产索引副作用 | **BLOCKED FOR APPROVAL** |
| ISG-03/04 生产索引缩减和清退 | FK 保存、零结果、USN/Source、Service/Direct parity，native 证据及收益 | **NOT AUTHORIZED** |

### 7.1 与仍在进行的故障拆分

- **#328**：历史 `database is locked` 已有单 writer 竞争复现实验，但历史真实持锁者未查明；最新 `Another global index operation is already running` 出自 `service_host.rs` 的 `index_service_busy`，与 SQLite BUSY 不可混同。D:/E:/F: 0 项错误需要 per-source 根因；不能直接通过新版排除策略解决。
- **#340**：Rust Windows Test Fixture 共享父目录并发删除造成的可能 `AccessDenied`，独立 test-only Codex 修复；不影响已通过的 #329 原生核心路径判断。
- **#329**：已 merged/closed，master post-merge CI SUCCESS。此新索引研究不回滚或重测 #329 已验证的限定范围产品功能。

## 8. 证据和可追溯性

### 源码（固定 SHA）
- [MFT Windows](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/global_index/windows/mft.rs)
- [USN Windows](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/global_index/windows/usn.rs)
- [Windows Service Host](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/global_index/windows/service_host.rs)
- [macOS Spotlight](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/global_index/macos/spotlight.rs)
- [Global Index Repository](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/global_index/repository.rs)
- [Database Schema](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/db/schema.rs)
- [Managed Scan Path Filter](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/path_filter.rs)
- [Global Search synthetic benchmark](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/global_index/tests/global_search_benchmark.rs)
- [ADR-0012](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/docs/project/DECISIONS/0012-files-authority-and-compatibility-retirement.md)
- [ADR-0013](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/docs/project/DECISIONS/0013-managed-ai-eligibility-and-queue-authority.md)

### 官方机制参考
- [Microsoft FSCTL_ENUM_USN_DATA](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_data)
- [Apple NSMetadataQuery](https://developer.apple.com/documentation/foundation/nsmetadataquery)
- [SQLite Foreign Keys](https://www.sqlite.org/foreignkeys.html)
- [SQLite WAL](https://www.sqlite.org/wal.html)

**最终标签：`ISG-00 STATIC RESEARCH COMPLETE — MEASUREMENT PENDING`.**