# #474 — 文件搜索产品契约与架构再选型（Research / Owner decision gate）

**状态：研究与产品决策草案，不是新搜索实现、生产优化或 Release GO。** 来源基线：master @ df940915084b810101a8a3eb78796cdf8730d55b。该文件不改代码、索引范围、Schema、排序、CI 门槛或实际 UI。

## 一、回顾：我们当初为什么采用 SQLite + FTS5

2026 年 7 月的 Global Index/Managed AI 边界修复与合并（PR #15），将文件身份、volume/source 有效性、扫描与恢复、禁用卷排除、索引完整/不完整状态，以及 Managed AI admission 绑定到可靠的 SQLite/事务状态。SQLite 不是单纯的搜索引擎：它同时是文件事实与业务持久化的权威；其 WAL read transaction 让候选行与 source facts 一致。FTS5 trigram 是在现有 SQLite 行上添加索引化文件名/路径/扩展名子串匹配的可行实现，触发器将 name/path/extension 变化同步到搜索索引。

**没有找到早期产品层面正式比较过 BM25 和 Everything/Oil Find 式文件名相关性排序并选择 BM25 的证据。** 目前精确 BM25(8,2,1)、mtime DESC、id ASC、五层优先级、4096 候选窗口的严格保持，来自既有正确性/回归合同，不能倒推成产品已证明 BM25 最优。既有保证也不能被本研究悄悄删除：所有生产语义调整必须显式提案、获 Owner 决策并独立验收。

2026-10-08 PR #341 Oil Find 审计、Issue #342 当时禁止未经真实基准就重写 SoA/SIMD/cache，是合理工程控制：**不能把旧方案的安全权威与它当时采用的检索算法混为一谈。** 现在有了 1M 与稀疏查询基准，允许重新审视产品排序，但仍不能以此认定需要一套第二持久索引。

## 二、现有证据与已排除方案

- 现有 Windows 1M Release 高命中 FTS substring：report p95 **331.420ms**、invoice **346.993ms**，超过冻结的 **100ms** 门槛。此前较快的前缀/扩展名层不等于 FTS 通过。
- #490 固定 512+1 FTS rank frontier 无法保留前 80 的正确性：80th 与 513th BM25 同分，必须观察全部边界 tie；既有 joined rank stream 亦曾回归。禁止将少量采样排名称作精确 Top-K。
- #491 Windows 100k Release（run 40623060605）：修改时间倒序扫描的高命中 report/invoice SQL p95 1.283/1.347ms，但零命中 70.758ms、单条最旧命中 72.402ms；同期现有 pooled Search 为 1.153/1.738ms，且 BM25 排序、macOS 大小写语义不等价。**否决默认 recency-first 扫描**；高命中 80/80 相同仅是合成数据巧合。
- #492 是从已经全量计算的 SQLite FTS5 分数构建分块的**收集器正确性模型**，并非真实高效倒排索引，也不证明任何 1M 性能改善；保留 Draft 证据，停止把它当作下一生产落地的必然前提。
- 目前所有 1M 和 100k 指标的来源/范围不能互换。尤其 SQL-only 与 end-to-end、Windows Hosted 与真实桌面、warm 与 cold 必须分栏。

## 三、先明确产品目标，而非让旧 BM25 决定架构

Zen Canvas 是 Windows/macOS 低占用文件管理器，并非通用文章搜索引擎。建议将基础搜索与高级搜索区分：

**基础“查找文件”**：通常按文件名、必要时路径及扩展名找到目标；输入即响应，支持中文/Unicode、大小写、标点，明确当前索引覆盖范围；无匹配必须很快；搜索可分页、可取消、可解释匹配原因。

**高级检索**：可保留 FTS5 trigram / 日后内容索引 / 语义检索，具有独立的查询语法、权限、成本和降级说明。不能因为高级搜索有 BM25，就要求整个文件名查找必须逐次评分所有高命中。

建议的**待 Owner 审定相关性规则**（非已生效规范）：
1. 文件名完整匹配；
2. 文件名前缀／词边界匹配；
3. 文件名内部子串匹配；
4. 路径组件／路径内部命中；
5. 各类内部优先修改时间 DESC，完全同分以稳定 ID ASC 打破。

**必须讨论的歧义**：当前扩展名 exact/prefix 处于 FTS 之前；未来这些匹配应独立分类、显式筛选，还是混入通用规则？文件夹与文件、应用与文档、低价值路径、最近使用的加权如何做？如果规则变化，历史搜索结果的 ID/排序、跨页稳定性和打开/定位行为都必须有显式迁移/验收，不可宣传“与 BM25 完全等价”。

## 四、三个候选实现，严格区分取舍

| 方案 | 文件事实主库 | 搜索热路径 | 优点 | 实际风险 |
| --- | --- | --- | --- | --- |
| A — SQLite/FTS5 轻改 | 现有 SQLite | FTS5 负责候选；轻量分级/排序 | 最低新增常驻内存、复用 triggers | 高命中仍需枚举大量 FTS 候选、排序与 JOIN；改 BM25 不保证 <100ms |
| B — 紧凑、可丢弃的**文件名只读内存视图** | **仍是** SQLite | 连续名字字节/offset 与小型元数据，批量过滤＋Top-K | 避免大批 SQL 行/评分；可对新增输入增量收窄 | 额外 RAM、加载/更新成本、缓存失效与同步复杂、跨平台 Unicode；不能单独覆盖路径/内容 |
| C — 独立持久倒排索引／Tantivy / 精确 BM25 block pruning | SQLite + 第二索引 | 新 query engine | 可能降低高命中打分开销 | 第二持久数据结构、replay/crash/upgrade、词法/排名不等价、低功耗成本高 |

**优先顺序：A 和 B 做可复现实验对照 → 按用户可理解的文件相关性规则选择 → C 仅当 A/B 证明失败且产品确实需要更复杂的全文相关性时再开。**

B 不是照搬 Oil Find 成为第二文件事实权威；应只允许从有版本号的 SQLite 快照**重建**，不可负责 file identity、delete/rename 权威或 AI admission。内存预算不允许以“搜索很快”自动豁免，且不能复制全路径/大对象而宣称 SoA。索引默认范围继续由用户授权与低价值排除**目标政策**决定，不得效仿 Everything 默认扫描所有固定 NTFS 卷。**当前状态不能冒充已实现：PR #489 仅处理 Windows 索引授权且仍为 Draft，未合并；它明确未实现 #371 Smart Lean 低价值文件过滤。此功能必须与新搜索加速分开验收。**

## 五、分阶段验证计划（以证据确定下一步）

**Stage 0 — 先冻结比较口径，纯研究。**
- 从当前正式 Search 提取一份 12+ 类回归事实：exact/prefix/extension/FTS/punctuation、零命中、旧文件唯一命中、中文、大小写/组合字符、禁用卷、stale、managed/source 状态、重复 ID、分页窗口与 UI 取消。
- 新 product relevance 排序用**独立 ID/分类 oracle**；精确匹配/可见性/身份一致必须保留，但 BM25 score equality 不应冒充新产品规则的验收标准。
- 先画出基础快速搜索和高级 FTS 之间的调用边界及清晰的降级行为：不要“先做时间扫描、空结果才 FTS”，这会重现 #491 的负面尾部。

**Stage 1 — SQLite A 轻量原型（测试专用）。**
- 在同一 snapshot 中测 FTS5 先获取 ID 和名称/路径分类信息，再于内存执行规则排序；严格计入 100k 的结果数和高命中数量，分页/过滤/补齐不偷算。查询计划需确认未借 BM25 全排序；即使 SQL-only 很快也必须测 full pooled/search snapshot。
- 如匹配结果过多，实验必须计入枚举、分级排序、hydrate、source facts 的完整耗时，以及 SQL temp B-tree。禁止固定截断后声称“无损”。

**Stage 2 — B 紧凑内存搜索小原型（测试专用）。**
- 从 SQLite 已授权、有效 source 的一致事务读取名字及最小元数据；使用连续 UTF-8 bytes＋offsets / stable IDs；仅先做文件名 exact/prefix/substring、稳定 Top-K。不要引入 SIMD，先测普通 Rust 扫描。
- 在 100k / 500k / 1M 同一合成库上记录 **加载时间、真实 RSS 增量、额外堆内存、查询 p50/p95/p99、CPU、索引/更新 I/O、每次重命名/删除的维护成本、长时间空闲常驻占用**，同时单独在低内存 Windows/macOS 目标平台测。样本须包含 0、1、几十、5000、50k 命中和热/冷。
- 结果只能视为“文件名匹配快速路径”；路径命中、文件内容与完整 FTS5 必须有明示的回退/合并策略，不能不返回文件名未命中的路径文件。
- 模拟并行 rename/delete、卷关闭、mtime/权限、源覆盖配置变化、重启/索引损坏/版本失配：必须**拒绝旧快照或准确回退 SQLite**；不能让一个输入过程跨 revision 产生陈旧结果。
- 连续输入缩小候选必须证明 query 单调、索引 revision 和选项完全一致；非单调编辑、中文 IME 和大小写规则变化自动重扫。

**Stage 3 — 先过产品判断，再改生产。**
- 必须先确认基础排序符合用户直觉；同时比较 A/B 的高命中、无命中与低端设备内存/CPU 写放大；不能只比较 report/invoice。
- 沿用现有 100ms FTS gate 监控旧行为；若正式产品变更了 ranking contract，应单独设立新 gate 和迁移验收，保留旧 gate 作为旧行为的记录，**不能偷换指标**。
- 没有 Owner 对产品排序、路径回退和内存预算的明确评审，不开始生产 UI/IPC/Schema 或双索引写入改造。

## 六、外部一手事实与适用边界

- SQLite FTS5 官方文档 https://www.sqlite.org/fts5.html ：trigram 匹配是可索引的子串匹配；默认大小写不敏感；全文 MATCH <3 Unicode 字符不返回结果；这是重要的 Unicode/短输入边界。不能把 Rust ASCII 子串当成 FTS5 等价实现。
- Everything 官方 https://www.voidtools.com/support/everything/indexes/ ：维护文件名和路径，按需加属性/fast sort 索引会增加内存；它默认包括固定 NTFS 卷，**不能复制这个授权默认值**。
- Oil Find 开源 https://github.com/oil-oil/oil-find/blob/main/docs/ARCHITECTURE.md 与 Searcher.swift、coilfind.c ：SoA 连续数据、SIMD、增量收窄、独立 custom integer relevance、chunk Top-K；其内存与时间数据属于作者环境，不能作为 Zen Windows/macOS 的达标证据。
- Zen 旧决策与门槛：PR #15、PR #341、Issue #342、Issue #360、Issue #474、Draft #490/#491/#492。

## 决策结论（本文件建议，不是已批准产品更改）

**保留 SQLite durable authority 与现有权限/索引覆盖机制；保留 FTS5 用于高级路径/子串匹配；优先检验是否值得建立轻量文件名查找的可重建派生热路径。** BM25 8:2:1 是当前必须防回归的实现合同，但并非已经证明“文件名搜索最优”的产品目标。暂不再投入精确 BM25 block-pruning 生产开发，#492 保持原有 Draft 证据而不合并；#491 保留失败路线证据。#474 的 GitHub Issue 当前状态为 **CLOSED / completed（2026-10-27）**；该状态不等于两个超时指标达标。本研究应以一个新的 Owner 决策/实施任务承接后续方案，而不是擅自重开 #474。按照现有证据，两项 1M FTS p95 仍超 100ms，不能宣称性能验收或 Release GO。


### 实际 Issue 治理状态与交接边界（补记）

#474 在 GitHub 于 2026-10-27 17:16Z 被关闭，state_reason=completed；其 issue 文本同时约束：**生产代码和测试实施由 Codex Cloud 负责，Owner/ChatGPT 负责架构决策和 PR 审查；不要反复用纯研究 PR 代替生产推进。** 因此此 Draft 是一次性的产品契约复评文档，不能形成无限研究循环；下一步应让 Owner 从 A/B 原型中选择可测实施包，交给开发代理交付真实代码，再由 Owner 独立审核。不能因为本报告在关闭的 Issue 中留言就把 Issue 称为 reopened。

当前关联但不混淆的另外两项工作：#489 Windows fixed NTFS 启动索引 opt-in 仍是 OPEN/Draft，#371 Smart Lean 默认排除低价值文件没有包含在 #489。初次扫描授权、索引内容选择、查询排序与 FTS5 性能应分别证明，不能相互代替验收。
