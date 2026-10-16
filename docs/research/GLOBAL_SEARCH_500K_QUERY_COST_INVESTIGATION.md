# Zen Canvas #352 — Global Search 500k 查询成本根因调查

> 本报告记录 #352 的证据优先调查。研究分支只增加隔离的 test-only 诊断、该诊断专用的 Hosted workflow 路由和本报告；生产搜索、Schema、IPC、排序/分页语义、100ms 门槛均未修改。500k 数据是合成 SQLite Global Index 行，不等于 500k 个真实文件。

## 1. 基线、范围与冲突检查

- 仓库：`ArdenZC/Zen-Canvas`。
- 开始时 fetch 的 `origin/master` 与最后复核值相同：`9ac78cf86ed99deed16caaddab4164bdd631c3be`。本分支以该 SHA 为 PR base。
- 分支：`research/issue-352-global-search-500k-performance`。
- #350、#354 均已合入；#352 在本报告提交时保持 OPEN。
- 起始时检查的其他 Open/Draft PR 是 #335（#329）和 #356（#353）；它们不修改 Global Search。#358 是本调查的独立 Draft PR：[PR #358](https://github.com/ArdenZC/Zen-Canvas/pull/358)。
- #354 的合入提交检查 run `38011498035` 有两个 performance shards 被取消，aggregate 为失败；该失败记录保留。包含这些提交的起始 master `9ac78cf` 随后的 `38011663236` 在 Source/Scope/Plan 与 Documentation-only validation 上成功；docs-only 分类下 Windows/macOS Quality 和 performance lanes 按计划跳过。#350 的报告合入该 master；其 docs-only validation 同样为成功。跳过不是成功的原生测试证据。
- 先前已阅读 #342、#346、#348 正式报告及当前 Global Search 实现；#347 的 FTS-first join-order 改动保持有效。此次不把延迟归因于 #347 回归。
- 限定范围：Windows Hosted synthetic SQLite 读取诊断、SQLite query plan/counter、局部 SQL 候选与语义 fixture。不实施生产优化；不重跑 #350 的完整 12 类矩阵；不运行 1m benchmark；不运行 Codex Review。

关键源码：
- 生产入口与 tier/limit 实现：`src-tauri/src/global_index/search.rs`。
- Tauri command：`src-tauri/src/global_index/commands.rs`。
- Repository snapshot 及其 source/status 查询：`src-tauri/src/global_index/repository.rs`。
- 既有合成 fixture 和基准：`src-tauri/src/global_index/tests/global_search_benchmark.rs`。
- 新 test-only 分阶段诊断：`src-tauri/src/global_index/tests/global_search_query_cost_diagnostic.rs`。

## 2. 已确认的 500k 性能问题

正式历史基线为 Windows Hosted run `37928710426`，source SHA `9fe67765a94998b1659edd0f41242867fa3c749b`，artifact ID `11616034036`，ZIP SHA-256 `5ae4cfe1bb22827e0945346691865cdecd810d0b4619ff643e8b08c14157f505`。正确性与分页断言 12/12 通过；既有 warm p95 ≤100ms 门槛 5/12 通过、7/12 未通过。

| #350 查询类别 | 匹配数 | warm p95 | 100ms |
|---|---:|---:|---|
| exact basename | 1 | 0.428ms | PASS |
| name prefix | 25,000 | 122.733ms | FAIL |
| common prefix | 24,999 | 124.964ms | FAIL |
| FTS report | 25,000 | 157.715ms | FAIL |
| FTS invoice | 25,000 | 162.472ms | FAIL |
| extension exact | 90,630 | 0.511ms | PASS |
| extension prefix | 40,629 | 88.581ms | PASS |
| duplicate basename | 25,000 | 0.351ms | PASS |
| no result | 0 | 0.457ms | PASS |
| Chinese prefix | 25,000 | 122.870ms | FAIL |
| punctuation prefix | 25,000 | 121.500ms | FAIL |
| accented Unicode prefix | 25,000 | 118.173ms | FAIL |

100k 对照 run `37921985290` 中 FTS report/invoice 各命中 5,000 行，warm p95 为 47.373/45.921ms；500k 正式 fixture 对应 25,000 行和 157.715/162.472ms。100k artifact ID `11612184267`，ZIP SHA-256 `ff74b107c5f2eef90c752d810c1f996115c02cf2625fa4b44030d1de3f4bfd38`。两次来自不同 Hosted run 和 source，不是同一物理 runner 配对实验；不能把 5 倍数据/命中数和约 3.4 倍延迟视为单独的因果估计。

## 3. 生产搜索执行路径

`search.rs` 将查询 trim、校验空输入，将 limit clamp 到 1–200，并把跨 tier 候选总量限制在 4,096。offset 达 4,096 时返回空。对普通查询，按 exact name → name prefix → exact extension → extension prefix → FTS / punctuation fallback 的优先级逐层补足当前 target；结果按稳定 entry ID 去重，最后应用 offset 和 limit。每个检索层排除 disabled volume 与 stale row，并投影 managed 标记。Unicode、标点路由使用现有归一化和 tier 规则。

前缀计划使用现有 active-name/active-extension 范围索引，再按 `modified_at_fs DESC, id ASC` 排序。高扇出结果的计划有 `USE TEMP B-TREE FOR ORDER BY`。FTS 计划仍为 `global_entries_fts MATCH → global_entries.rowid → global_volumes → managed EXISTS → ORDER BY rank, modified_at_fs, id`；FTS 虚表使用 FTS MATCH，rowid 回表和 volume lookup 仍由 SQLite 计划完成。这保留 #347 的 FTS-first 顺序，没有回到旧 join-order 问题。

完整 Tauri command `commands.rs:11` 调用 `search_global_entries_snapshot`。Repository 在同一个读事务内取得搜索结果、source-health/revision 与 index status（`repository.rs:342-360`），之后 command 还会取 coordinator/provider 状态并组装响应。此次完整时间样本到 repository snapshot 为止，不含后续 coordinator/provider、序列化、IPC 或 UI 端到端延迟。

## 4. 诊断方法与实际执行

诊断 source SHA：`da23bd2f501e39d843ccd99af6b832ef140028cb`；Hosted run `38020014723`，attempt 1，job `114118717901`，job 结论 SUCCESS。run log 中 test 结果为 1 passed / 0 failed，1,169 filtered，test elapsed `1619.43s`。workflow job 的 test step 与 artifact upload 均成功。

- Runner：Windows Server 2025，image `windows-2025-vs2026` / `20260925.250.1`，x86_64；SQLite `3.51.3`。
- PRAGMA：WAL、page size 4096、temp_store MEMORY、synchronous FULL、mmap_size 2GiB。
- fixture：既有确定性 500,000 synthetic-entry generator，使用生产 Schema/index/FTS triggers 和 512-row insert transactions；数据库 main 655,818,752 bytes，WAL 23,768,312 bytes，main + WAL 679,587,064 bytes，page count 160,112。
- fixture 生成 947.845ms，population 1,166,469.391ms，build 总 wall 1,167,825.399ms（约 19m28s）。这些是本次成功 attempt 的独立记录；#350 的 1,420,740.2ms 和失败 attempt1 的 1,531,278.5ms 是不同 run，不作平均或覆盖。
- 诊断计时：分阶段 SQL 3 warmups、10 warm samples；production entrypoints 与 A/B 5 warmups、30 warm samples。候选 A/B 在同一个 Windows job、同一个连接/fixture 上逐样本交替先后顺序。量化采用 artifact 记录的 linear interpolation。
- 分阶段 timer 在 statement prepare 之后开始；完整 production timing 包含其真实执行所需的 prepare/result mapping。Warm/reopened connection 不清空 Windows 文件 cache，故不是 cold disk 数据。
- artifact 有 146 条 JSONL 记录：1 environment/dataset、87 query stage、6 production tier plans、44 production entrypoint timings、4 paired variants、1 disposable index cost、1 hit-scale summary、1 semantic gate、1 diagnostic-complete record。
- artifact `global-search-query_cost_diagnostic-500000-rows-38020014723`，ID `11658812266`，ZIP SHA-256 `c52096acda9f9484ab453cb5f753b275988d66d913b7d0d607941553590eda6c`；解包 JSONL SHA-256 `d6bb00cee70ea6961a1ea2d93a5ed314eb9287029c8eb53e6ef09ae46e5bb8d1`。原始 artifact 可在 [Hosted run 38020014723](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38020014723) 的 artifact 11658812266 复核。

### #350 主基准未改写

warm p95 ≤100ms 历史门槛、12 个既有查询类型、fixture generator、生产搜索语义均未改动。诊断 profile 是额外 test-only job，不是正式 12 类 matrix，也不是新 gate。诊断收尾记录明确为：production search source unchanged、normal semantics unchanged、official full matrix not repeated、one-million benchmark not run、base rows rollback 后仍为 500,000。

## 5. 前缀查询的阶段成本

六类查询全部命中现有 name 或 extension 范围索引；EXPLAIN 没有全表扫描。下表是每阶段 10 个 warm sample 的 p95（ms），每个 stage 的 p50/p99、参数、输出行数、SQL、EXPLAIN、temp-B-tree 与 SQLite counters 均在 JSONL 中。

A 为范围读取；B 加 enabled-volume/stale 条件；C 对全命中集按生产排序；D 生产排序 + LIMIT 80；E 改为 LIMIT 4096；F 投影完整结果列但不计算 managed；G 为生产 tier 的完整 projection 与 managed EXISTS。

| 查询（全命中数） | A range | B join/filter | C sort all | D LIMIT 80 | E LIMIT 4096 | F wide projection | G full tier |
|---|---:|---:|---:|---:|---:|---:|---:|
| quarterly（25,000） | 4.443 | 19.676 | 23.131 | 21.894 | 37.272 | 67.513 | 68.961 |
| IMG_（24,999） | 4.057 | 18.763 | 24.154 | 20.141 | 33.817 | 57.677 | 67.093 |
| 数据库（25,000） | 7.241 | 20.486 | 22.809 | 21.196 | 34.361 | 61.186 | 67.002 |
| final-v2（25,000） | 3.987 | 21.275 | 27.594 | 21.091 | 40.719 | 61.508 | 86.277 |
| RÉSUMÉ（25,000） | 4.239 | 21.555 | 25.592 | 23.676 | 36.366 | 61.089 | 67.255 |
| jp（40,629） | 6.557 | 33.769 | 39.876 | 24.916 | 23.796 | 25.221 | 25.412 |

以上是分阶段 SQL，不是可以直接相加的成本分解。查询计划和受控对照表明：原始 prefix range scan 不是瓶颈（p95 4–7ms）；过滤与排序把高命中前缀行提升到约 19–40ms；full-width result projection 后在该 stage 组达到约 25–68ms，production tier stage 为约 25–86ms。普通前缀的 production pool 完整搜索仍可超过 100ms，说明 tier/完整入口成本不能由 range stage 独立代表。Extension prefix 40,629 matches 的前缀 SQL stages 约 24–40ms，历史正式完整基准 p95 88.581ms，仍在原门槛内。

前缀排序计划都包含临时 B-tree；A 范围阶段没有临时排序。SQLite sort counter 和 VM steps 是工作量指标，不是 CPU 时间、临时结构字节数或内存峰值。完整 SQL/plan 可从 artifact 的 query-stage 记录取得。

## 6. FTS 分阶段成本

报告将高扇出 report/invoice（25,000 matches）、单命中 `0499981` 和 no-result 分开。下面是每阶段 10 个 warm sample 的 p95（ms）：

A 只 MATCH rowid；B 加 BM25 rank 不排序；C 按 BM25 排序全命中集；D rowid join entries；E 加 enabled/stale 条件；F rank/order + LIMIT 80；G LIMIT 4096；H 完整列但不算 managed；I 完整 production projection/rank/managed。

| 查询（全命中数） | A MATCH | B BM25 | C 排序全命中 | D rowid join | E filters | F LIMIT 80 | G LIMIT 4096 | H wide | I production |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| report（25,000） | 8.585 | 38.654 | 42.643 | 22.452 | 26.918 | 63.964 | 78.295 | 94.500 | 101.233 |
| invoice（25,000） | 11.837 | 43.296 | 52.624 | 29.428 | 54.208 | 80.738 | 90.770 | 106.942 | 116.082 |
| low fan-out（1） | 0.380 | 1.213 | 1.199 | 0.402 | 0.387 | 0.757 | 0.920 | 0.846 | 0.799 |
| no result（0） | 0.023 | 0.025 | 0.023 | 0.025 | 0.025 | 0.042 | 0.043 | 0.060 | 0.071 |

计划仍以 `SCAN global_entries_fts VIRTUAL TABLE INDEX 0:M3` 开始，随后 entries 主键 rowid 回表、volume 主键 lookup、managed correlated subquery 与排序临时 B-tree。没有退回从 entries 驱动并反复扫 FTS 的坏计划。MATCH-only p95 约 9–12ms；BM25 的计算变体约 39–43ms；对全 hit 排序变体约 43–53ms；完整列与 managed 变体达到约 101–116ms。这证明 160ms 不能直接归因给 MATCH。各阶段 SQL 的过滤/排序形状不同，时间差只用作受控 stage 证据，不将 p95 差值当作严格可加的单项因果比例。

## 7. 实际 production entrypoints 与 snapshot 计时

每个 cell 为 30 次 warm 样本的 `p50 / p95 / p99 ms`。`tier` 是 search.rs 实际单 tier helper；`pool` 是 Database::search_global_entries（包括连接池 checkout 和所有 production tiers）；`snapshot` 是 repository 搜索 snapshot；persistent connection 不含 pool checkout；reopened connection 在计时外重开配置连接，但不清空 OS cache。未采集的 stage 标记为 n/a。

| 查询 | tier | pool entrypoint | repository snapshot | persistent connection | reopened connection |
|---|---|---|---|---|---|
| quarterly | 68.143 / 71.628 / 89.563 | 113.929 / 117.835 / 122.963 | 835.238 / 859.838 / 866.520 | 69.822 / 75.717 / 76.862 | 91.819 / 95.172 / 96.662 |
| IMG_ | 64.336 / 69.656 / 71.740 | 110.194 / 114.564 / 115.258 | 790.310 / 836.833 / 858.586 | 68.043 / 74.766 / 77.736 | 93.143 / 102.283 / 105.290 |
| 数据库 | 62.486 / 64.631 / 65.262 | 111.244 / 119.963 / 122.386 | 781.324 / 840.988 / 854.308 | 66.182 / 73.753 / 74.665 | n/a |
| final-v2 | 64.725 / 74.263 / 88.110 | 112.294 / 140.378 / 169.816 | 777.178 / 808.111 / 848.398 | 65.568 / 72.134 / 84.756 | n/a |
| RÉSUMÉ | 63.957 / 65.669 / 66.252 | 105.529 / 110.794 / 111.995 | 769.989 / 804.347 / 808.664 | 65.702 / 70.531 / 85.224 | n/a |
| jp | 22.345 / 24.157 / 28.721 | 73.113 / 76.318 / 76.384 | 740.583 / 787.975 / 797.403 | 24.235 / 34.707 / 36.078 | n/a |
| report | 101.862 / 106.719 / 108.225 | 141.910 / 158.783 / 168.706 | 825.055 / 890.520 / 959.720 | 109.446 / 112.448 / 117.144 | 134.583 / 140.800 / 142.930 |
| invoice | 112.888 / 119.677 / 153.858 | 156.152 / 162.964 / 194.701 | 851.620 / 897.621 / 907.843 | 114.400 / 117.675 / 119.702 | 140.422 / 148.621 / 183.579 |
| 0499981 | 0.839 / 0.927 / 0.945 | 1.143 / 1.181 / 1.186 | 749.291 / 795.868 / 808.455 | 1.175 / 1.279 / 1.346 | n/a |
| zzznomatchtoken | 0.105 / 0.178 / 0.200 | 0.425 / 0.463 / 0.469 | 754.531 / 864.608 / 878.201 | 0.407 / 0.462 / 0.483 | n/a |

### Repository snapshot metadata 成本

生产 `search_global_entries_snapshot` 完整 p95 横跨约 788–898ms，包含 search 结果、source health/revision 与 index status 查询；连 no-result query 的 snapshot p95 也有 864.608ms。它确实是 `commands.rs` 每次 search command 先调用的 production repository API，不能与较快的 `Database::search_global_entries` / SQL tier 混为一谈。但它不包括 command 后面的 coordinator/provider、序列化和 IPC。

test-only mirror 与 `repository.rs` 的 SQL 字面形状一致：

| snapshot SQL mirror | p50 / p95 / p99 | plan/counters |
|---|---:|---|
| source health：按 volume GROUP BY 的 COUNT(ge.id), MAX(last_seen_at) | 759.365 / 789.753 / 795.446ms | volume→entries index lookup；GROUP BY 与 ORDER BY 各一项 temp sort；18,500,060 VM steps |
| index status：enabled/stale entries 与各状态计数 | 18.344 / 18.646 / 18.680ms | covering volume/entry/status indexes；1,500,262 VM steps；无全表扫描 |

结论是 per-search source-health 分组汇总占 snapshot 大头；index-status 同时计数的成本约 19ms，不是主要来源。二者的时间不与 snapshot totals 相减作为严格归因。下一轮如处理此路径，必须保留当前同一读事务的 source/status 快照一致性合同。

## 8. 命中数扩展性与候选窗

成功 attempt 在同一份 500k fixture 的回滚事务中插入 40,201 条诊断 overlay，形成 0、5、100、1,000、4,096、10,000、25,000 个 prefix hits；query 只读取排序后的 LIMIT 80。下表为该固定数据库总量、相同生产 name-prefix index plan 的 10-sample p95：

| 命中数 | 80-row 查询 p95 | SQLite VM steps | 全表扫描 |
|---:|---:|---:|---|
| 0 | 0.015ms | 22 | 否 |
| 5 | 0.017ms | 112 | 否 |
| 100 | 0.078ms | 1,822 | 否 |
| 1,000 | 0.575ms | 18,022 | 否 |
| 4,096 | 2.578ms | 73,750 | 否 |
| 10,000 | 6.739ms | 180,022 | 否 |
| 25,000 | 16.775ms | 450,022 | 否 |

阶段延迟与 VM steps 随命中数上升，且 index range seek 起点很低；在这个控制变量里，主要放大因素是匹配行规模，而非读取 500k 主表的全表扫描。上表是 key/rowid query，不含完整 managed/result projection，所以不能拿 16.8ms 当成实际页面搜索延迟。scale overlay 整体写入与采样 query wall 为 6,187.080ms，事务已 rollback，最终 entry count 仍为 500,000。

首页请求 target 通常是 80；4096 是跨 tier/offset 的候选上限，只有偏移读到窗口后段时才扩张。name query 的 stage p95 从 LIMIT 80 的 21.894ms 到 LIMIT 4096 的 37.272ms；report FTS 从 63.964ms 到 78.295ms。差异可测，但不是 100ms 历史首页超限的唯一或主要解释。此次没有缩小窗口。

## 9. 候选 SQL 对照与语义证明

paired 实验每种 SQL 各 5 warmups / 30 samples，在同一 Windows job、同一 SQLite 连接/fixture 上逐样本交替执行顺序。下表为 `p50 / p95 / p99 ms`；所有候选 SQL 都没有进入生产代码。

| 候选 | 原生产 p50/p95/p99 | 候选 p50/p95/p99 | p50变化 | 语义证据 / 限制 |
|---|---:|---:|---:|---|
| key-only MATERIALIZED prefix CTE（quarterly） | 73.011 / 78.532 / 81.001 | 36.536 / 37.045 / 37.201 | -49.96% | 标准页 full fields 相同；prefix CTE 对 adversarial fixture 和候选窗至 4096 完整字段/顺序相等 |
| disposable time-first index（quarterly） | 72.747 / 85.716 / 87.903 | 0.617 / 1.242 / 1.801 | -99.15% | 标准 80-row page 逐字段相等；未对该索引候选单独运行完整 adversarial/低 fan-out/page 矩阵 |
| key-only MATERIALIZED FTS CTE（report） | 107.922 / 112.579 / 115.192 | 78.781 / 80.811 / 87.106 | -27.00% | full result equality；原 BM25 权重、过滤先于 LIMIT 和 rank/mtime/ID 顺序保留 |
| key-only MATERIALIZED FTS CTE（invoice） | 113.355 / 117.087 / 118.781 | 83.665 / 86.216 / 86.536 | -26.19% | 同 report；high-fanout 25k fixture |

按 paired p95 计算，prefix CTE 从 78.532ms 降到 37.045ms（2.12×）；FTS report 从 112.579ms 降到 80.811ms（1.39×），invoice 从 117.087ms 降到 86.216ms（1.36×）。time-first index 从 85.716ms 降到 1.242ms（69×），但该结果的低 fan-out/分页/写入覆盖不足。\n\nPrefix/FTS CTE 在实验里先按生产过滤、rank/sort、stable tie-break 与 limit 收集窄键，然后回表做宽 projection/managed 标记；不改变 LIMIT/offset/cap、rank 权重或输出字段。artifact semantic gate 证实 prefix/FTS fields 相等、prefix CTE 至 4096 相等，并覆盖多个 enabled volumes、disabled volume、stale rows、managed/unmanaged、重复 basename/跨 volume 路径、相同 mtime 的 ID tie-break、tier overlap 去重、limit clamp、offset 4,016 与 4,096 边界。

明确拒绝把 enabled/stale 过滤移到 LIMIT 之后：adversarial rows 下它返回 0 条而不是填满 80 条，造成 underfill。未将去掉 managed marker 的 SQL 当优化候选；它会改变结果字段。time-first index 虽在 high-fanout query 极快且首屏全字段一致，但没有低选择性、跨 tier、4096 offset 和 adverse cases 的完整等价/成本矩阵，因此仅列研究候选。

## 10. Disposable index 的额外成本

仅在可丢弃 diagnostic DB 建立 partial index `(modified_at_fs DESC, id ASC, name_normalized, volume_id) WHERE is_stale=0`，查询后删除：

- build：3,708.635ms。
- page count +8,410（4KiB/page），估算 index bytes +34,447,360。
- main + WAL bytes 增量：49,501,840。
- time-first query plan 顺序扫描新索引，不用 temp B-tree；原 production prefix plan 未被索引 hint 取代。FTS 计划前后相同。
- 5 次、每次 256-row rollback insert 的事务样本，p50 从无该索引的 21.935ms 变成 78.043ms（+255.79%）。这是小事务诊断，不是 bulk population/writer benchmark。
- 仅测季度名称的高命中首屏，没有测 0/低命中查询。为找到稀少命中，time-first scan 可能检查很长的时间序列；写入与磁盘增长也是真实代价。

因此不建议当前直接增加 Schema index。即使高命中首屏最快，也必须先测低 fan-out、不同 tier、写入吞吐和完整 end-to-end 路径。

## 11. 根因回答

### Q1 — 为什么 25k prefix 会落在约 120ms？

范围查找用现有索引，A stage p95 仅 4–7ms，没有全表扫描。B enabled/stale filtering 后为约 19–34ms，包含排序的 rowid/limit stages 为约 21–40ms，加入 full result projection 后到约 25–68ms，完整单 tier production stage 到约 25–86ms。完整 Database pool entrypoint 的 30-sample p95 为约 105–140ms，故慢点不只是前缀定位：高 fan-out 排序、组装宽结果/执行所有 tier 和入口开销都参与。另一个更大的 production command 热点是下述 repository source-health snapshot。

### Q2 — 为什么 FTS 从 100k 的约 45–47ms 升到 500k 的约 158–162ms？

100k 与 500k 的 hit count 同时由约 5k 增至约 25k，数据库也扩大 5 倍；两数据点并非 runner-paired。500k 专用诊断在 25k hits 上 MATCH rowid p95 约 9–12ms，BM25 变体约 39–43ms，rank-sort 变体约 43–53ms，完整 tier 约 101–116ms，pooled full search 约 159–163ms。证据不支持 “MATCH 本身慢” 或 #347 join-order 回归；BM25、排名排序、过滤/回表、managed projection 与其余 entrypoint 成本需要共同看待。

### Q3 — 检索、排序、BM25、投影谁是主因？

name/extension range lookup 不是主因；行数命中选择性对 SQLite VM work 有显著影响。prefix 与 FTS 都需要 temp B-tree 排序。FTS MATCH-only 不到 12ms，而完整 FTS tier 超过 100ms；BM25 与排序是可测部分。普通 prefix 完整 projection阶段显著高于 rowid-only；FTS 无 managed 与 production managed 变体在高命中时约相差 7–9ms。最重且直接覆盖实际 command 前置 repository API 的是 source-health COUNT/MAX grouped snapshot（mirror p95 约 790ms），远大于单独 query tier。

### Q4 — 4096 candidate window 是否为根因？

不是首页主要原因。stage 80 与 4096 有额外排序/取行工作（name p95 21.9→37.3ms；report FTS 64.0→78.3ms），但历史超限约 120–160ms；full command 的 source-health snapshot 还有约 800–900ms。不得缩 cap 来隐藏完整路径成本；高 offset 语义必须保留。

### Q5 — 能否用局部规划改善而不重构架构？

可以形成下一轮生产验证的局部 SQL CTE 候选。same-runner paired 数据显示 prefix p50 约减半、FTS p50 约降四分之一，且 CTE semantic gate 通过。无需引入新索引/内存索引/SIMD 才能验证这一方向。完整 repository snapshot 仍需另行处理。

### Q6 — 保留所有语义时，25k 查询能否落在 100ms？

候选 SQL 单 tier 在当前 diagnostic warm samples 达到 p95 37.045ms（prefix）、80.811ms（FTS report）和 86.216ms（FTS invoice），语义对照成立。可据此说该候选 query shape 能在这个 synthetic fixture 上达到 100ms query-tier p95；不能据此说完整 Global Search command 已达标。当前 pooled entrypoint 的 name/FTS p95 为 117.835/158.783/162.964ms；repository snapshot p95 为 859.838/890.520/897.621ms。没有对候选跑整个搜索 pipeline 与正式 benchmark matrix，历史 7/12 FAIL 状态仍保留。

## 12. Attempt 1 harness 失败与修复记录

| 项目 | Attempt 1 事实 |
|---|---|
| run / source | `38016944122` / `1b4303b2e37a5b62cabd61da2d09cb60abf38314` |
| 结果 | FAILURE；失败发生在 scale overlay harness assertion，不是生产搜索断言或门槛 |
| artifact | ID `11658055574`；ZIP SHA-256 `43c279be25a22bf01b86706264b9cd60a7b0e467c4ccbf0c88a9bfc8f688c934`；JSONL SHA-256 `d86c377ab2acd03887e9cb3ed82cce4e53e044f332c89663b594af4615ea19ab` |
| duration/cost | test 2320.59s；fixture generation/population 已执行；scale/adversarial/diagnostic-complete records 尚未运行 |
| exact error | expected overlay row count 40,201，查询 `id LIKE 'issue352-scale-%'` 实际得到 0 |

生产 generator 的 ID 格式是 `issue352-scale5-...`、`issue352-scale25000-...`；原 LIKE literal 中间多了连字符。该 assertion 在主要 stage 与 paired 候选已写入 artifact 后才触发。test harness 修正为 `issue352-scale%`，并在 source SHA `da23bd2f501e39d843ccd99af6b832ef140028cb` 上重新执行同一 500k fixture diagnostic。成功 run 完整产出 match-scale/adversarial/diagnostic-complete；attempt 1 原失败仍作为真实成本与历史保留，没有重解释为成功。

## 13. 已知范围与未验证

- 数据全是 synthetic SQLite rows，没有创建 500k NTFS/APFS 文件；真实文件发现、权限、网络卷、用户数据分布及 indexer 并发均 NOT VERIFIED。
- 专用诊断仅在 Windows Server 2025 Hosted 测试；macOS native SQLite/Spotlight/FSEvents 环境结果 NOT VERIFIED。
- Hosted CPU 型号/负载、CPU 使用率、RAM/working set/RSS、冷盘延迟、临时 B-tree 实际字节数没有采集。SQLite VM/sort counters 不能替代这些资源指标。
- pool checkout 没有独立计时；pool 与 persistent 的差值不可全部归因于连接池。reopened connection 未清 OS file cache。
- Tauri command 后续 coordinator/provider、serialization、IPC、搜索 UI input debounce / cancel / latest-query-wins 不在此测试范围。
- 100k/500k 之间不是 paired runner；分阶段查询变体也不是可以直接相减并相加的正交实验。
- time-first index 未覆盖低选择性/0-hit/高 offset/多查询分布；小型 insert 样本不能推算批量写入退化。
- 此次没有并发 scan/SQLite lock workload；不将 #328/#345/#349 的锁竞争假设成 #352 根因。
- 无 1m benchmark。没有更改 100ms gate，也没有用 benchmark 的弱化阈值判为 PASS。

## 14. 建议的最多三项后续任务（均需 Owner 单独授权）

| 等级/任务 | 改动位置 | 复杂度与风险 | 实测收益依据 | 评价 |
|---|---|---|---|---|
| P0：优化每次 search 的 source-health snapshot 代价，同时保留 snapshot 一致性 | repository.rs `342–360`、`532–544`；command contract 在 commands.rs `11–26` | 中；高正确性风险，必须证明单读事务里的 counts/revision/source state 一致且 failure behavior 不变；无须先改 Schema | source-health mirror p95 789.753ms；实际 repository snapshot p95 788–898ms，含 no-result | **首个代码任务建议**：先建立同事务正确性回归与替代查询/状态复用实验，再单独比较完整 command 路径 |
| P0：将高扇出 name-prefix/FTS 的宽 projection 延后到有序候选键确定后 | search.rs 当前 tier SQL及 global_search_query_cost_diagnostic.rs 对应安全 CTE | 中；SQL planner、跨 tier 去重、分页/offset、BM25/mtime/ID、managed 与 4096 cap 风险；不增加索引写放大 | paired prefix p50 73.011→36.536ms；FTS report 107.922→78.781ms、invoice 113.355→83.665ms；相关 CTE adversarial gate 通过 | 值得授权为独立验证 PR；仍需 100k/500k 正式门槛及完整 pool/snapshot endpoint 验证 |
| P1：评估 time-first partial order index（当前不建议直接落库） | 未来 migration 若获授权；本分支只在 diagnostic fixture 使用 | 高；额外约 34.4MB page data、49.5MB main+WAL，256-row rollback insert p50 +255.79%；低 fan-out scan 与所有写路径风险未测 | quarterly 25k-hit 首屏 p50 72.747→0.617ms，p95 85.716→1.242ms；标准 80-row full fields 相同，但无完整 adversarial/low-hit gate | 仅保留研究，不作为下一项生产改动 |
| P2 | 无 | 当前不提出索引重构、SoA、SIMD、内存索引或 FTS tokenizer 改造 | 本调查没有支持其必要性的证据，且它们不解决已量出的 repository snapshot cost | 不启动 |

性能排序：key-only CTE 是维护成本较低、已证明候选；time-first index 数字更快但写放大明显、适用性不明；最优先要处理的实际 command 延迟来自 repository source-health 聚合，应先研究保持快照合同的局部修复。生产优化均等待 Owner 决定。

## 15. CI、静态检查与交付状态

- 当前 PR source 的正常 Hosted CI run `38020014704`：SUCCESS；required source/scope/plan、Windows Global Index qualification、Windows/macOS quality、Performance/Search 与 Performance profile 均成功。Search lane 使用 `--profile=extended`，日志显示执行 FTS 100k 与 Global Search 100k；这不是 500k 正式 12-query gate，也未执行 1m。
- 专用 500k diagnostic workflow run `38020014723` / attempt 1：SUCCESS；同一 job 跑一次 fixture 和 query-cost test。没有重跑 500k 完整正式矩阵。
- 本地 Linux full Rust test binary 需要 GTK 3 / WebKitGTK 4.1 development libraries。当前 Debian 13 executor 是非特权 agent、没有 sudo；尝试 apt-get update 时因 /var/lib/apt/lists/partial 权限失败，所以未安装依赖或运行本地 Linux Rust test binary。cross-target Windows Rust check/Clippy 使用 no-link resource shim，只证明 Rust type/lint 层，不代表 Windows linker/native runtime；Hosted Windows diagnostic 提供运行时证据。
- 文档提交后的本地检查：`DOCS_DIFF_BASE=origin/master npm run test:docs`（1 changed Markdown file）通过；`npm run test:governance` 通过；`actionlint .github/workflows/global-search-benchmark.yml` 通过；`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 通过；`git diff --check origin/master...HEAD` 通过。
- 本 PR 的允许文件仅：global-search benchmark workflow、现有 benchmark test module 的 test-only wiring、新 query-cost test-only diagnostic、研究报告。生产代码：**NO**。
- PR `358` 保持 OPEN / Draft；Issue #352 保持 OPEN；不 merge、不启动生产改造、不触发 Codex Review。

## 16. 最终结论

1. **是否该优化现有 query SQL？** 是。局部 key-only CTE 有同一 runner paired 收益与语义证据，足以进入独立生产验证。
2. **是否应重做索引结构？** 否。当前 range/FTS plans 有效，命中规模与排序/projection 的成本有直接证据；time-first index 的写/磁盘代价高且分布测试不足。
3. **是否引入 SIMD / SoA / 全内存索引？** 否。既没有必要性证据，也不属于最小修复方向。
4. **100ms 是否已经恢复？** 否。历史 100ms gate 仍为 7/12 FAIL；本轮 query-only 候选达到部分 SQL-tier 的 <100ms，但完整 pooled search 与真实 command repository snapshot 未达标/未证明达标。
5. **最小风险顺序？** 先处理 repository source-health snapshot 的每次命令成本并保留一致性；随后单独验证 key-only prefix/FTS CTE 对完整生产入口和正式 100k/500k gate 的影响；只有低 fan-out与写路径数据支持后再讨论新 index。
6. **下一步是否自动授权代码？** 否。全部生产改动等待 Owner 选择任务并明确授权。

**Final status: #352 PERFORMANCE ROOT CAUSE CONFIRMED — READY FOR OWNER DECISION**

## 17. Final Hosted evidence fields

| 字段 | 值 |
|---|---|
| Starting master | `9ac78cf86ed99deed16caaddab4164bdd631c3be` |
| Diagnostic source SHA | `da23bd2f501e39d843ccd99af6b832ef140028cb` |
| Diagnostic run / attempt | `38020014723` / 1 |
| Artifact ID | `11658812266` |
| ZIP / JSONL SHA-256 | `c52096acda9f9484ab453cb5f753b275988d66d913b7d0d607941553590eda6c` / `d6bb00cee70ea6961a1ea2d93a5ed314eb9287029c8eb53e6ef09ae46e5bb8d1` |
| Normal CI | `38020014704` — SUCCESS |
| PR / final report HEAD | `358`；文档提交 SHA 见 PR 元数据，与 benchmark source SHA 分开记录 |
| Changed files | `.github/workflows/global-search-benchmark.yml`；`src-tauri/src/global_index/tests/global_search_benchmark.rs`；`src-tauri/src/global_index/tests/global_search_query_cost_diagnostic.rs`；本报告 |



## Appendix A. 所有 query-stage 的 p50 / p95 / p99

以下每个单元为同一 query-stage 的 p50 / p95 / p99（ms）。这些分位数来自每 stage 3 warmups、10 warm samples；完整 SQL、parameters、EXPLAIN、每阶段输出行数、sort/fullscan/VM counters 见 JSONL artifact。Section 5/6 保留简版 p95 以便阅读。

### Prefix stages

| query | A range | B join/filter | C sort all | D LIMIT 80 | E LIMIT 4096 | F wide projection | G full tier |
|---|---:|---:|---:|---:|---:|---:|---:|
| quarterly | 4.190 / 4.443 / 4.586 | 17.346 / 19.676 / 19.911 | 22.869 / 23.131 / 23.144 | 20.666 / 21.894 / 22.176 | 34.602 / 37.272 / 38.406 | 63.957 / 67.513 / 68.293 | 67.160 / 68.961 / 69.126 |
| IMG_ | 3.983 / 4.057 / 4.066 | 16.575 / 18.763 / 18.965 | 21.499 / 24.154 / 25.024 | 19.640 / 20.141 / 20.232 | 32.288 / 33.817 / 33.843 | 55.749 / 57.677 / 58.124 | 63.852 / 67.093 / 68.528 |
| 数据库 | 4.150 / 7.241 / 7.623 | 18.856 / 20.486 / 20.586 | 22.554 / 22.809 / 22.861 | 20.152 / 21.196 / 21.465 | 32.360 / 34.361 / 34.688 | 58.544 / 61.186 / 61.876 | 63.411 / 67.002 / 67.376 |
| final-v2 | 3.951 / 3.987 / 3.987 | 18.495 / 21.275 / 21.505 | 23.566 / 27.594 / 28.658 | 20.658 / 21.091 / 21.182 | 36.534 / 40.719 / 41.339 | 58.128 / 61.508 / 61.883 | 65.551 / 86.277 / 89.645 |
| RÉSUMÉ | 4.085 / 4.239 / 4.301 | 19.928 / 21.555 / 21.791 | 24.605 / 25.592 / 25.776 | 21.885 / 23.676 / 23.754 | 35.181 / 36.366 / 36.791 | 60.192 / 61.089 / 61.136 | 64.009 / 67.255 / 68.372 |
| jp | 6.481 / 6.557 / 6.568 | 31.575 / 33.769 / 34.192 | 38.597 / 39.876 / 40.127 | 22.485 / 24.916 / 25.644 | 23.469 / 23.796 / 23.811 | 23.125 / 25.221 / 26.302 | 24.226 / 25.412 / 25.582 |

### FTS stages

| query | A MATCH | B BM25 | C sort all | D rowid join | E filters | F LIMIT 80 | G LIMIT 4096 | H wide | I production |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| report | 8.393 / 8.585 / 8.618 | 37.056 / 38.654 / 39.131 | 40.170 / 42.643 / 43.491 | 21.108 / 22.452 / 22.487 | 25.174 / 26.918 / 27.231 | 61.556 / 63.964 / 64.029 | 74.781 / 78.295 / 79.102 | 92.561 / 94.500 / 94.888 | 98.652 / 101.233 / 101.279 |
| invoice | 11.436 / 11.837 / 11.896 | 42.452 / 43.296 / 43.600 | 48.184 / 52.624 / 55.224 | 28.187 / 29.428 / 30.044 | 37.476 / 54.208 / 58.448 | 76.431 / 80.738 / 83.096 | 88.873 / 90.770 / 90.783 | 105.290 / 106.942 / 106.982 | 112.115 / 116.082 / 116.477 |
| 0499981 | 0.360 / 0.380 / 0.381 | 0.781 / 1.213 / 1.222 | 1.182 / 1.199 / 1.205 | 0.374 / 0.402 / 0.403 | 0.360 / 0.387 / 0.388 | 0.743 / 0.757 / 0.758 | 0.746 / 0.920 / 1.014 | 0.764 / 0.846 / 0.877 | 0.773 / 0.799 / 0.800 |
| zzznomatchtoken | 0.022 / 0.023 / 0.024 | 0.023 / 0.025 / 0.025 | 0.023 / 0.023 / 0.023 | 0.024 / 0.025 / 0.025 | 0.023 / 0.025 / 0.025 | 0.037 / 0.042 / 0.042 | 0.039 / 0.043 / 0.044 | 0.056 / 0.060 / 0.062 | 0.067 / 0.071 / 0.072 |
