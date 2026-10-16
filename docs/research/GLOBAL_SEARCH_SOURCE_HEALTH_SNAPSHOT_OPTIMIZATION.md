# #359 Global Search Source-health Snapshot 优化审计

> 状态：Owner 补充验收已在 Windows Hosted 上以同一个 500k fixture 完成四种 stale / volume 拓扑；run `38046309971`、attempt 1、source SHA `ddaa782307ac87daf46d8f81f634bbca18649909` SUCCESS。B 在四种拓扑都快于 Original，但 test-only Candidate C 在两种 90% stale 拓扑显著快于 B（SQL p95 快 27.9% / 39.1%，10 卷 no-result / high-hit Snapshot p95 快 36.2% / 20.4%）。因此旧的“建议直接接受 B”结论由第 10 节取代：保持生产 SQL 不变，暂不建议合并，等待 Owner 决定是否接受 B 的拓扑折衷或另行授权 Candidate C 实现评审。PR #361 保持 OPEN / Draft，Issue #359 保持 OPEN。结果仅代表 Windows Hosted 合成 SQLite fixture，不代表 Windows/macOS 实机文件系统或完整 Tauri IPC/UI 延迟。

## 1. 基线与审计范围

| 项目 | 证据 |
|---|---|
| Repository | `ArdenZC/Zen-Canvas` |
| 起始 `origin/master` | `d3f6bb9da163a3fcf236ecba61827d99f084af61`，开始前 fetch 后核验 |
| 分支 | `fix/issue-359-global-search-source-health-snapshot` |
| Draft PR | [#361](https://github.com/ArdenZC/Zen-Canvas/pull/361)，OPEN / Draft |
| Issue | [#359](https://github.com/ArdenZC/Zen-Canvas/issues/359)，OPEN |
| #358 post-merge CI | run `38024962557`，SUCCESS |
| #352 500k 根因诊断 | run `38020014723`，artifact `11658812266`；source SHA `da23bd2f501e39d843ccd99af6b832ef140028cb`；ZIP SHA-256 `c52096acda9f9484ab453cb5f753b275988d66d913b7d0d607941553590eda6c` |
| 变更边界 | repository source-health SQL、隔离测试/诊断、benchmark workflow 触发路径与报告；不改 Schema、IPC、prefix/FTS 搜索语义、性能门槛、写路径或 AI 逻辑 |

其他同时打开的 PR 已检查：#335、#356，与此次 source-health 聚合无关。审计未合并 PR、未更改 Issue 状态、未运行 Codex Review、未运行 1m/更大 benchmark，也未启动 #360。

原 #352 报告 `docs/research/GLOBAL_SEARCH_500K_QUERY_COST_INVESTIGATION.md` 记录了 500k Hosted Windows source-health SQL p95 `789.753ms`，完整 no-result `search_global_entries_snapshot()` p95 `864.608ms`，FTS invoice snapshot p95 `897.621ms`。该 fixture 是合成 SQLite 行，不是 500k 实际文件。旧诊断 SQL 执行约 `18,500,060` VM steps，并有 GROUP BY 与 ORDER BY 临时 B-tree；index-status p95 `18.646ms`，不是主要热点。

## 2. Zen Canvas 当前生产架构与合同

### 搜索调用路径

`src-tauri/src/global_index/commands.rs::search_global_entries` 校验请求后调用 `Database::search_global_entries_snapshot()`，然后在事务提交后读取 coordinator/provider 状态，计算 coordinator conflict、`collection_complete` 与 `result_state`，最终构造 `GlobalSearchResponse`。本次只审计 Repository snapshot 内 source-health 聚合。

`src-tauri/src/global_index/repository.rs::Database::search_global_entries_snapshot` 的执行顺序是：

1. 从连接池借连接并 `BEGIN` SQLite 读事务；
2. 在该事务内读取搜索结果；
3. 读取 source-health 与 `source_revision`；
4. 在同一事务内读取 `GlobalIndexStatus`；
5. 提交事务，再返回完整 snapshot。

因此搜索结果、每个 volume 的健康状态、revision facts 与 index status 共享同一数据库快照。本次候选没有把这些读取拆出事务。

### 原始 source-health SQL

```sql
SELECT gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at,
       COUNT(ge.id), MAX(ge.last_seen_at)
FROM global_volumes gv
LEFT JOIN global_entries ge
  ON ge.volume_id = gv.id AND ge.is_stale = 0
GROUP BY gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at
ORDER BY gv.id ASC
```

`repository.rs::load_global_search_sources_from_connection` 将每行映射为完整 `GlobalSearchSourceHealth` 和有序 `GlobalSearchRevisionFact`；后者以 serde JSON 字节序列化，再经 BLAKE3 输出十六进制 `source_revision`。source-health 覆盖 disabled volume。查询必须实际计数并求最大 `last_seen_at`；`global_volumes.entry_count` 是维护字段，不能替代真实 active row count。

### 原热点

#352 的 Windows Server 2025 synthetic 500k 诊断将热点定位到这条 source-health 聚合：原 SQL 的 p50/p95/p99、完整计划及 counters 会在本次 artifact 的 baseline record 中复核。旧报告的计划显示按 volume 索引访问 entries，再对宽 volume 字段 GROUP BY，并为 GROUP BY、ORDER BY 各创建临时 B-tree。它每次 Global Search snapshot 都运行，即使搜索返回 0 条结果也照样运行。

## 3. 候选查询与预期机制

候选在本次受控诊断之前仅以 `#[cfg(test)]` 暴露，不改变发布版 SQL。每个候选通过 production `search_global_entries_snapshot()` 的 test-only 查询覆盖点运行，因此搜索、连接池、读事务、index-status、结果排序/分页与错误传播均走完整入口。

### Candidate A：逐 volume 相关子查询

```sql
SELECT gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at,
       (SELECT COUNT(ge.id)
        FROM global_entries ge
        WHERE ge.volume_id = gv.id AND ge.is_stale = 0),
       (SELECT MAX(ge.last_seen_at)
        FROM global_entries ge
        WHERE ge.volume_id = gv.id AND ge.is_stale = 0)
FROM global_volumes gv
ORDER BY gv.id ASC
```

预期可避免宽 GROUP BY，但 COUNT 与 MAX 是两次相关扫描，且按 volume 数量重复。该候选不是默认更快；以本次同 fixture 配对测量及执行计划决定是否拒绝。

### Candidate B：按窄 volume key 预聚合后连接

```sql
WITH active_volume_facts AS (
    SELECT volume_id,
           COUNT(id) AS active_entry_count,
           MAX(last_seen_at) AS max_last_seen_at
    FROM global_entries
    WHERE is_stale = 0
    GROUP BY volume_id
)
SELECT gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at,
       COALESCE(active_volume_facts.active_entry_count, 0),
       active_volume_facts.max_last_seen_at
FROM global_volumes gv
LEFT JOIN active_volume_facts ON active_volume_facts.volume_id = gv.id
ORDER BY gv.id ASC
```

该形状只按 `volume_id` 聚合 entries，再将一行聚合事实左连接给每个 volume。外层 `LEFT JOIN` 保留空卷/全部 stale 卷以及 disabled volume；外层 `ORDER BY gv.id ASC` 维持稳定次序。这里使用 `COUNT(id)` 而非 `COUNT(*)`，保持原 `COUNT(ge.id)` 在 nullable row-id fixture 上的精确语义。没有使用方便计数列、缓存、Schema、索引或持久化数据。

### Candidate C：按唯一 volume ID 分组的原式 JOIN

复核 Candidate B 的执行计划后，另做一次非正式 Python SQLite 小规模预检，检查能否只对唯一的 `gv.id` 分组，同时选择由该 volume 行决定的字段：

```sql
SELECT gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at,
       COUNT(ge.id), MAX(ge.last_seen_at)
FROM global_volumes gv
LEFT JOIN global_entries ge
  ON ge.volume_id = gv.id AND ge.is_stale = 0
GROUP BY gv.id
ORDER BY gv.id ASC
```

50k / 10-volume Python SQLite `3.53.1` 预检的 plan 仍按 `(volume_id,is_stale)` 对每个 volume seek，没有宽字段 GROUP BY / ORDER BY temp B-tree。在全 active 数据上 p95 `14.270ms`（原始 `35.967ms`，B `11.064ms`）；90% stale 数据上 p95 `1.303ms`（原始 `3.642ms`，B `2.717ms`）。四个 SQL 的有序 source facts 在这个简化 fixture 中相同。该预检仅是探索结果；第 10 节的 Windows Hosted 500k 配对测量现已正式覆盖 Candidate C 的 SQL 与完整 Repository Snapshot。Candidate C 仍只存在于测试代码，本次没有替换生产 SQL。

## 4. 语义与同一快照验证

新增隔离测试位于 `src-tauri/src/global_index/tests/global_search_source_health.rs`，两候选均与原始查询逐行比较：

- 0 volume、空 volume、多个 enabled/disabled volume、volume ID 排序；
- 有 active/stale 混合、全部 stale、NULL `last_seen_at`、相同最大时间；
- 一个临时 shadow table 注入 `id IS NULL` 行，验证 `COUNT(id)` 与 `COUNT(ge.id)` 一样排除 NULL，而 `MAX` 保留 NULL 语义；
- 故意使 `global_volumes.entry_count` 与真实行数不一致，确认查询采用 active entries 实数；
- 修改非最大/并列最大/新最大时间，检查 revision 对应变化；插入、删除、stale、恢复 entry；
- 切换 volume enabled、provider、index status、last_error、updated_at；
- 比较完整有序 revision fact JSON bytes 与 BLAKE3 `source_revision`，而不只比较公开 source-health；
- 比较完整 snapshot 中 search results、source-health、revision、index status；
- 32 轮 barrier 控制 reader/writer 交错：先由 reader 建立事务快照，再让 writer 完成 volume 状态与 stale/restore 更新，然后在 reader 原事务中依次比较原 SQL 与候选。这样比较的是同一逻辑 SQLite snapshot，不把事务间正常变化误判成查询不等价。

Hosted 500k 诊断也在每轮 SQL/function/snapshot 配对中检查 row/facts/revision 与完整返回值相等。具体测试数、Hosted correctness record 与结果以最终 artifact 记录为准。

## 5. 500k Hosted 配对诊断设计

### 小数据预检（仅方向性）

在执行受控 Hosted run 前，以 Python SQLite `3.53.1` 做过两次 50k / 10-volume 短样本预检。全 active 的 5-sample 查询预检：原 SQL p50/p95 `37.077/37.184ms`，Candidate A `36.404/37.329ms`，Candidate B `22.887/26.093ms`。随后追加 Candidate C 与 90% stale 场景，5 warmups / 30 samples，数值见上节。它们都不是 Rust production path 或 Windows Hosted `3.51.3`，仅作方向性/风险证据；stale-heavy 与多-volume结论以第 10 节同 fixture 500k artifact 为准。

| 项目 | 设置 |
|---|---|
| Workflow | `Global Search synthetic benchmark`，固定 `source_health_diagnostic`、500,000 行 |
| 初次受控 run | `38026682828`，attempt 1，Windows Hosted；源码 SHA `b7ff5bda49bf45a13f526bb6e241502adbad1091` |
| fixture | 单个 synthetic SQLite fixture；原 SQL 与 A/B 交替复用，不为候选重复生成 DB |
| 样本 | 每候选 5 次 warmup、30 次交替 paired warm samples；报告 p50/p95/p99 |
| 测量范围 | SQL-only；source-health 完整函数（行映射、JSON revision facts 序列化、BLAKE3）；完整 repository snapshot；actual `Database::search_global_entries_snapshot()` 对 test-only candidate snapshot |
| snapshot query class | no-result `zzznomatchtoken`、prefix `quarterly`、high-fanout FTS `report`；样本包含 search/source/revision/index-status 同事务工作 |
| Query counters | SQLite VM steps、SORT、Fullscan steps；完整 EXPLAIN QUERY PLAN、SQL、参数、行数和数据库文件大小写入 JSONL |
| 正确性 | 官方既有 12 类 query correctness 复核；source-health candidate SQL 与 full snapshots equality |
| 写入/Schema | 不加持久索引或 migration；临时诊断数据使用 rollback；保留原正式 100ms gate |

单个 workflow 运行耗时较长，是因为 Windows Hosted 需要准备/生成 500k 合成数据并运行现有查询阶段诊断。此 run 不重跑官方 500k 性能门槛矩阵，不运行 1m。两次 diagnostic 都是 attempt 1；首轮用于 A/B 与实际入口预验证，第二轮在 production SQL 源 SHA 上验证最终实现。两次均记录 artifact、哈希、SQLite 与 runner/OS 版本。

### Hosted 配对测量结果

下面各 cell 为同 runner、同 fixture 配对样本。`SQL / fn` 分别是 SQL-only / 完整 source-health 函数，单位 ms。`Snapshot` 为整个 repository snapshot，单位 ms。无结果、prefix、FTS 分列，不将不同入口阶段的延迟相减作为严格因果归因。

| 测量 | 原 p50 / p95 / p99 | Candidate B p50 / p95 / p99 | 样本 |
|---|---:|---:|---:|
| Source-health SQL | 1153.417 / 1171.716 / 1227.177 | 267.036 / 287.758 / 298.672 | 每组 30，5 warmup |
| 完整 source-health 函数 | 1250.215 / 1269.309 / 1277.997 | 363.257 / 384.596 / 388.372 | 每组 30，5 warmup |
| Repository snapshot：no result | 1296.740 / 1311.050 / 1316.009 | 409.981 / 419.680 / 422.463 | 每组 30，5 warmup；0 results |
| Repository snapshot：prefix `quarterly` | 1515.496 / 1552.570 / 1562.026 | 630.061 / 643.555 / 647.589 | 每组 30，5 warmup；80 results |
| Repository snapshot：FTS `report` | 1663.265 / 1735.612 / 1744.893 | 748.901 / 874.966 / 935.985 | 每组 30，5 warmup；80 results |

Candidate A 的相同配对 p95：SQL `1187.856 → 384.049ms`，source-health 函数 `1280.653 → 591.125ms`，完整 snapshots no-result `1373.824 → 632.109ms`、prefix `1523.914 → 841.669ms`、FTS report `1746.382 → 1003.868ms`。A 比原 SQL 快，但 B 的每项 p95 都更低；全 active 的 50k/10-volume 预检中 A p95 `37.329ms`，接近原 SQL `37.184ms`，也慢于 B 的 `26.093ms`。因此 A 不作为默认生产形状。此前 Candidate C 只有 stale-heavy 小样本；第 10 节现已有正式 Hosted 500k / Rust Repository Snapshot 证据，结论是 C 在 stale-heavy 下明显胜 B、在全 active 下慢于 B，故当前生产 B 保持不动并等待 Owner 决策。

在独立的 actual-entrypoint validation record 中，真实 `Database::search_global_entries_snapshot()` 的旧生产路径与 test-only Candidate B snapshot helper 同 runner 交替比较：

| Query | 原 production entrypoint p50 / p95 / p99 | Candidate B snapshot helper p50 / p95 / p99 | source-health / revision / index / results |
|---|---:|---:|---|
| no result | 1319.028 / 1612.042 / 1649.837 | 412.139 / 438.133 / 440.900 | 全部相等 |
| prefix `quarterly` | 1541.306 / 1816.608 / 1845.158 | 642.931 / 732.066 / 761.885 | 全部相等 |
| FTS `report` | 1668.188 / 1787.972 / 1881.240 | 742.567 / 796.836 / 800.304 | 全部相等 |

该 record 在真实 Repository snapshot method 与候选之间只通过 `#[cfg(test)]` thread-local 覆盖 source-health SQL；候选仍在同一个 method、同一个读事务中运行，search、index-status、pool checkout、事务边界和完整返回结构没有改写。`repository_snapshot_paired` 表与 actual-entrypoint validation 的样本顺序/计时环节略有不同，应分别引用，不能彼此替换。

### 最终 production-source Hosted 验证

生产改动提交 `cd02f47ac4994413106d0ddfeabc6cfa9caf1530` 的 500k run `38030877088` / attempt 1 / job `114151377167` 于 Windows Hosted SUCCESS（44m52s）。Artifact ID `11662358132`，ZIP SHA-256 `393ad4571eb5eb3fd3570d91873c4c8065aa28955ad5173c1b0e144af9b7eb6d`，JSONL SHA-256 `baba395fc424378234805f62fbd51453012489fd01286c7b25587b37f47f421f`。文件缓存于 `/workspace/.cache/zen-canvas/issue359/evidence/38030877088/`。原始记录确认 source SHA、run ID、attempt、Windows x86_64、`win25-vs2026` / `20260925.250.1`、SQLite `3.51.3`。500k fixture population `1,272,175.311ms`；主库 `655,818,752 bytes`、WAL `23,768,312 bytes`、合计 `679,587,064 bytes`；`temp_store=MEMORY`。无进程 CPU/RSS 样本。

最终 run 同一 fixture、同一 Windows runner 上每组 5 warmups / 30 交替 warm samples：

| 测量 | 原 SQL p50 / p95 / p99 | Candidate B p50 / p95 / p99 | 样本 |
|---|---:|---:|---:|
| Source-health SQL | 1152.043 / 1191.642 / 1196.385 ms | 260.159 / 269.662 / 283.143 ms | 每组 30，5 warmup |
| 完整 source-health 函数 | 1223.958 / 1257.650 / 1295.087 ms | 344.817 / 351.322 / 354.601 ms | 每组 30，5 warmup |
| Repository snapshot：no result | 1281.237 / 1328.452 / 1333.700 ms | 396.232 / 417.145 / 429.331 ms | 每组 30；0 results |
| Repository snapshot：prefix `quarterly` | 1524.593 / 1709.584 / 1822.783 ms | 640.892 / 753.081 / 766.486 ms | 每组 30；80 results |
| Repository snapshot：FTS `report` | 1668.645 / 1764.120 / 1789.730 ms | 745.455 / 825.244 / 839.858 ms | 每组 30；80 results |

这里的 `repository_snapshot_paired` 两侧均通过同一个 repository snapshot method 与同一读事务运行，原 SQL 通过 test-only override 注入；三类 query 的 search results、source-health、source_revision、index status 全部相等。实际未覆盖 override 的 production `Database::search_global_entries_snapshot()` 也在该 run 上测得：no-result `410.832 / 432.649 / 454.832ms`、prefix `625.274 / 658.827 / 663.086ms`、FTS `733.098 / 770.355 / 784.847ms`（p50/p95/p99，n=30），并逐项与 Candidate B helper 返回值相等。旧 production entrypoint 的配对基线来自首轮 run `38026682828`；不要把跨 run 原始 p95 当成配对因果值，本节采用同 run 的 `repository_snapshot_paired` 作为 before/after。

Candidate A 最终 run p95 仍慢于 B：SQL `1195.447 → 380.071ms`、完整 source-health function `1301.811 → 598.934ms`、snapshot no-result `1316.512 → 608.040ms`、prefix `1572.107 → 863.834ms`、FTS `1718.325 → 1001.857ms`（每组 n=30）。最终 Candidate B 事实/完整 snapshot equality 均为 true；`revision_facts_bytes_and_blake3_equal=true`，12/12 official query correctness PASS，`diagnostic_complete` PASS，rollback 后保留 500,000 base rows；没有 Schema/index 变化、正式 performance gate 改动、官方完整 gate matrix 重跑或 1m run。

Candidate A 最终 run 的完整分位数也保留在 artifact；以下原 SQL/A 数值各自来自同一个交替配对（n=30，5 warmups），单位 ms：

| 测量 | 原 SQL p50 / p95 / p99 | Candidate A p50 / p95 / p99 |
|---|---:|---:|
| Source-health SQL | 1156.934 / 1195.447 / 1241.779 | 372.618 / 380.071 / 387.124 |
| 完整 source-health 函数 | 1241.456 / 1301.811 / 1329.336 | 546.852 / 598.934 / 650.454 |
| Snapshot：no result | 1277.130 / 1316.512 / 1341.586 | 592.966 / 608.040 / 615.931 |
| Snapshot：prefix `quarterly` | 1522.773 / 1572.107 / 1578.766 | 840.961 / 863.834 / 870.220 |
| Snapshot：FTS `report` | 1621.430 / 1718.325 / 1737.837 | 948.301 / 1001.857 / 1021.488 |

最终 run 的 SQL plan 和 StatementStatus 与首轮一致：原 SQL 18,500,060 VM / 2 SORT / 0 Fullscan，A 5,500,045 / 0 / 0，B 7,500,077 / 0 / 499,999。最终 source-health SQL p95 为 `1191.642 → 269.662ms`（降低 77.4%）；同 run no-result/prefix/FTS snapshot p95 分别为 `1328.452 → 417.145ms`（68.6%）、`1709.584 → 753.081ms`（55.9%）、`1764.120 → 825.244ms`（53.2%）。第 10 节现已补测 stale-heavy / 多卷 500k，并采集进程级 CPU / working set / private commit。B 仍会全扫 entries index；SQLite 临时内存字节不能由现有采样隔离，标记为 NOT VERIFIED。

### 执行计划与 SQLite counters

```text
Original:
SCAN gv USING INDEX sqlite_autoindex_global_volumes_1
SEARCH ge USING INDEX idx_global_entries_volume (volume_id=? AND is_stale=?) LEFT-JOIN
USE TEMP B-TREE FOR GROUP BY
USE TEMP B-TREE FOR ORDER BY

Candidate A:
SCAN gv USING INDEX sqlite_autoindex_global_volumes_1
CORRELATED SCALAR SUBQUERY 1
SEARCH ge USING INDEX idx_global_entries_volume (volume_id=? AND is_stale=?)
CORRELATED SCALAR SUBQUERY 2
SEARCH ge USING INDEX idx_global_entries_volume (volume_id=? AND is_stale=?)

Candidate B:
MATERIALIZE active_volume_facts
SCAN global_entries USING INDEX idx_global_entries_volume
SCAN gv USING INDEX sqlite_autoindex_global_volumes_1
SEARCH active_volume_facts USING AUTOMATIC COVERING INDEX (volume_id=?) LEFT-JOIN
```

每次 SQL 查询产生 1 个 source-health 行，SQL 无绑定参数；schema/index changes=false。SQLite StatementStatus 为最后一次查询的 counters：

| SQL | VM steps | SORT operations | Fullscan steps |
|---|---:|---:|---:|
| Original | 18,500,060 | 2 | 0 |
| Candidate A | 5,500,045 | 0 | 0 |
| Candidate B | 7,500,077 | 0 | 499,999 |

B 消除了两项临时排序，VM steps 比原 SQL 少 59.5%，SQL p95 少 75.4%；代价是扫描 `idx_global_entries_volume` 上 500k rows（SQLite Fullscan counter 499,999），并 materialize 聚合 CTE / 创建自动 covering index。它不是“避免扫描”：它以一次全索引聚合扫描替代 volume 查询及宽 GROUP BY/ORDER BY。A 无 fullscan/sort，p95 仍慢于 B。VM steps、SORT 与 Fullscan 是工作量计数，不代表 CPU 时间、临时存储字节或内存峰值。

首轮 run artifact：run `38026682828` / attempt 1 / job `114138897338`，结论 SUCCESS；source SHA `b7ff5bda49bf45a13f526bb6e241502adbad1091`。artifact ID `11661506696`，ZIP SHA-256 `27dff119839559c825b59a05d20d6a295d9894dfc1883f369b8fc16f7a217091`，JSONL SHA-256 `ff2e3626b6ee16d281f3b48165f00c046745bc4a0641aa549882abbd6136d4a5`。最终 production-source run 的 run/source/artifact/digests 见上一节。Hosted environment 两次均为 Windows x86_64 image `win25-vs2026` / `20260925.250.1`、SQLite `3.51.3`；单个合成 DB、同一 fixture 重复配对，没有为候选分别建库。

该历史 production-source run 的 process CPU 与 working set/RSS **NOT CAPTURED**；这是该 run 的采样范围，不适用于第 10 节新的拓扑 run，后者已加入 Windows process sampler。候选不新增持久表、持久索引、migration 或 writer 工作；source-health SQL 本身只读，临时物化发生在 SQLite memory temp-store。历史 fixture 只含一个 source row；第 10 节另外补测 10-volume 500k 拓扑。

上面的 plan 块列出每个 SQL 的完整 operator 输出，不是摘要。生产替换门槛是：事实/哈希/事务等价、候选有稳定且显著的 SQL 和完整 snapshot 收益、没有新增持久写入/索引或额外磁盘数据；SQLite 临时内存的峰值、进程 CPU 与 RSS 没有从首轮 artifact 采集，仍作为残余风险报告，不能据此宣称资源成本为零。

## 6. 100k / 500k 与现有正式门槛

历史 #350 500k 正式 fixture 的既有 correctness/page assertions 是 12/12 PASS，而原 100ms warm p95 只通过 5/12（7 项失败，主要是高命中 prefix/FTS）；本 issue 不修改这些正式门槛，也不宣称全局搜索已经达标。历史 #352 100k 对照仅为独立 run，FTS report/invoice 命中 5,000 条，warm p95 为 47.373/45.921ms；它与 500k 不是同源 runner 配对数据。

PR CI run `38026682784`（attempt 1，SHA `b7ff5bda49bf45a13f526bb6e241502adbad1091`）已完成，overall FAILURE。Source checkout/change-scope/validation plan、Windows Global Index service qualification、Windows/macOS Rust quality、Windows/macOS release compile、Windows quality、六个 performance shards（包括 Search、Workspace Foundation）和 Performance profile aggregate 均 SUCCESS。macOS Quality aggregate FAILURE 是 native performance 依赖失败传递，不是 Rust quality 编译/测试失败。

唯一失败的 native performance job `114138992324`（`Native macOS performance (arm64) (merge_integration)`）在 Workspace Foundation 的 `managed_scan_admission_fails_closed_after_busy_timeout_without_partial_authority` 用例中失败：

```text
admission did not fail boundedly at the busy timeout: timed out waiting on channel
test ...managed_scan_admission_fails_closed_after_busy_timeout_without_partial_authority ... FAILED
Prepared performance binary failed with exit code 101.
```

失败日志定位到 `src/db/queries/scan.rs:3626:27`，是 #345 managed-scan SQLite busy-timeout 用例，不属于本 PR 修改范围。run `38026682784` 保留，未手工 rerun 或改写其失败结果，也没有把 Performance profile aggregate PASS 混称为 native job 通过。

production source SHA `cd02f47ac4994413106d0ddfeabc6cfa9caf1530` 的 exact-code CI run `38030877064` 已总体 SUCCESS。Source checkout / evidence contract、change-scope / routing contract、Windows Global Index service qualification、validation plan、Windows/macOS Rust quality、Windows/macOS Release compile、Native macOS performance、六个 100k performance shards（Search、Workspace Foundation、Preview Platform、Library & Content、Scan & Schema、Intelligence）及 Performance profile 均 SUCCESS。这个新 CI run 包含正常 PR 验证，不是对旧 run 的手工重跑；#345 忙锁用例没有再次失败。另有 final report-only commit 的 PR-head CI 在提交后单独记录于最终回复；生产代码 SHA 与本节 production-source 500k run 一致。

本 issue 的 500k source-health diagnostic 与历史官方 12 类性能 gate 分开。最终 source-health diagnostic 的 12 类既有 query correctness 检查为 12/12 PASS；没有重跑官方 gate performance matrix，没有改 100ms 门槛，也没有跑 1m/更大数据。Hosted 合成 SQLite 结果只证实对应数据库场景，不解释为 NTFS/APFS、真实文件系统枚举、实机资源占用或 macOS 体验。

**平台边界：**source-health SQL 性能诊断仅在 Windows GitHub Hosted runner 上执行。Windows/macOS Hosted Release compile、Rust quality 和 native performance suite 通过，但这些结果不是 macOS source-health 500k SQLite 性能测量。macOS 原生 Global Search source-health SQL 性能为 NOT VERIFIED；Owner 的 Windows/macOS 实机 Tauri/UI/IPC/文件系统 latency 也为 NOT VERIFIED。

## 7. 性能、资源与正确性风险

候选 B 把原先按 volume/stale 复合索引 seek + 宽字段 GROUP BY/ORDER BY 改成 active entries 索引全扫、窄键聚合 CTE 及临时 materialization。首轮 one-volume 结果的拓扑局限已由第 10 节补测：四个 500k 拓扑中 B 的完整 Snapshot p95 均快于 Original；在 90% stale 下 Candidate C 进一步减少扫描工作量并取得更低 p95。Hosted 已测进程 CPU、working set 与 private commit；SQLite 临时分配字节不能从 SQLite connection/cache 中单独分离，仍标为 NOT VERIFIED。该只读查询没有变更持久 DB schema/index，也没有新增持久写放大或磁盘数据。

语义风险集中在 LEFT JOIN 对空卷的保留、stale 过滤位置、NULL 与 `COUNT(id)`、volume 稳定排序、revision fact 字段顺序/序列化和事务快照边界。新增 fixtures 对这些点均设有显式比较。500k artifact 标记 12/12 既有 search correctness classes 通过、`diagnostic_complete` 成功、rollback 后仍为 500,000 rows、无 schema/index change、未改正式 100ms gate。连接池、search tier、pagination/window、FTS/prefix SQL、source_revision 算法及错误传播没有计划改动。

## 8. 本地验证记录

在 production SQL 补丁 `cd02f47ac4994413106d0ddfeabc6cfa9caf1530` 上完成：

| 检查 | 结果 |
|---|---|
| `cargo test --manifest-path src-tauri/Cargo.toml --lib global_index::tests:: -- --nocapture --test-threads=1` | 22 passed、0 failed、3 个现有 100k/500k benchmark ignored；包含 4 个 source-health semantic cases、并发 writer/read snapshot、source revision consistency |
| Linux Cargo 环境 | 因仓库仅在 Windows/macOS target 引入 keyring，为本地 test build 临时追加 `keyring` `linux-native` dev dependency；退出 trap 恢复 `Cargo.toml`/`Cargo.lock`，两者没有 diff。构建只有既有 `atomic_move.rs` unused-variable warnings |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | PASS |
| `actionlint .github/workflows/global-search-benchmark.yml` | PASS |
| `npm run typecheck` | PASS |
| `npm run test:performance:architecture` | 3 files / 30 tests PASS |
| `npm run test:governance` | PASS |
| `npm test -- --reporter=dot` | 重跑 PASS：175 files / 1,860 tests；182.77s。首次同样 1,860 个测试通过，但 Vitest 在结束时收到 `fileLibraryW205Interaction.test.ts` teardown 后的异步 `window is not defined`，所以首次命令 exit 1；单独重跑该文件 11/11 PASS。重跑结果用于判定整套测试通过，首次异步异常作为一次时序波动保留记录 |
| `DOCS_DIFF_BASE=origin/master DOCS_DIFF_HEAD=HEAD npm run test:docs` | PASS：最终报告内容在 diff 中由 checker 检查（1 个 Markdown 文件）；governance PASS |

除报告外，本地 test 所需的临时 Cargo 依赖不在提交中。上述测试未执行 ignored 100k/500k benchmark；正式 500k Hosted 证据由本报告单列的 diagnostic runs 提供。

## 9. 结论与实施建议

性能根因由真实 SQL/plan 和同 runner、同 fixture 的配对测试确认：source-health 每次 snapshot 都计数全部 active entries 并求 `MAX(last_seen_at)`，原宽 GROUP BY/ORDER BY 在 500k 上产生 2 个临时排序。最终 production-source 500k run 上 SQL p95 `1191.642 → 269.662ms`（下降 77.4%），完整 source-health function `1257.650 → 351.322ms`（下降 72.1%）；完整 Repository snapshot 的 no-result/prefix/FTS p95 分别 `1328.452 → 417.145ms`（下降 68.6%）、`1709.584 → 753.081ms`（下降 55.9%）、`1764.120 → 825.244ms`（下降 53.2%）。实际 production `search_global_entries_snapshot()` 当前 p95 是 no-result `432.649ms`、prefix `658.827ms`、FTS `770.355ms`；结果与 Candidate B helper 完整相等。仍都高于 100ms；本任务没有宣称 Global Search 全面达标，也没有改原始 100ms 门槛。

Candidate B 在生产 SQL 源 SHA `cd02f47ac4994413106d0ddfeabc6cfa9caf1530` 的最终 Hosted run `38030877088` 上通过：12/12 correctness、完整 source-health facts、revision JSON/BLAKE3 与三类完整 snapshot facts/results 相等；SQLite 计划/counters 方向与首轮一致。Local source-health/adversarial/concurrency tests 为 22 passed、3 个既有 benchmark ignored；生产源 Exact-HEAD CI `38030877064` SUCCESS。生产改动只替换 `repository.rs` 的 source-health SQL 查询字符串，保留 `GlobalSearchRevisionFact` 字段/序列化、BLAKE3、同一个 SQLite read transaction、row mapping、错误处理和所有搜索语义。

第 10 节的 Owner 补充验收 supersedes 本节基于单卷全 active 500k 的建议：B 在四种测得拓扑中都比 Original 快，语义与完整 snapshot 均一致；但 Candidate C 在两种 stale-heavy 拓扑的 SQL p95 快 27.9% / 39.1%，10 卷 stale-heavy 的完整 Snapshot no-result / high-hit p95 快 36.2% / 20.4%。在全 active 情形 C 又比 B 慢。因此这是拓扑折衷，不是无条件优胜者；本次结论为**暂不建议合并，等待 Owner 决定**，不自动把 C 换入生产。进程 CPU/RSS 已采样，但临时 SQLite 内存仍 NOT VERIFIED；macOS SQL 性能和真实 Windows/macOS Tauri/IPC/文件系统体验也仍 NOT VERIFIED。

本次没有实现索引 schema 重构、缓存或事务外快照。Owner 决定前不合并、不关闭 Issue、不启动 #360。

### 建议任务拆分

1. Owner 复核 production-source run `38030877088` 与拓扑 run `38046309971` 的 JSONL artifacts、Original/B/C 计划与完整分位数；
2. Owner 审阅 `repository.rs` 中的最小查询替换以及 adversarial/concurrency tests；
3. Owner 在真实 Windows/macOS 设备上做 Global Search 端到端 smoke/延迟验收；Hosted 结果不能代替；
4. 只有另行授权后再讨论 prefix/FTS 或其他瓶颈，避免在 #359 里扩展范围。

## 10. Owner 补充验收：500k stale-heavy / 多卷拓扑

本节按 PR #361 Owner 评论 `6096506704` 执行，补足旧实验未覆盖 stale-heavy 与多卷 500k 的风险。它不改生产 SQL，也不改变前述任何历史 artifact 或失败记录。测试在 source SHA `ddaa782307ac87daf46d8f81f634bbca18649909` 上运行；测试代码含 test-only Candidate C，生产仍使用 Candidate B。

### Run、artifact 与 fixture

| 项目 | 结果 |
|---|---|
| Hosted run / attempt / job | [38046309971](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38046309971) / 1 / `114196449981`，SUCCESS |
| Source SHA | `ddaa782307ac87daf46d8f81f634bbca18649909` |
| Runner | Windows x86_64，`win25-vs2026`，image `20260925.250.1` |
| SQLite | `3.51.3`；`journal_mode=WAL`、`synchronous=1`、`foreign_keys=1`、`temp_store=MEMORY`、`mmap_size=2147418112`、`page_size=4096` |
| Fixture | 一次生成一个 production-schema 合成 SQLite DB，共 500,000 rows；四态均在同一文件上事务更新并恢复，无 per-candidate / per-topology 重建 |
| Fixture 建立 | `988,867.255ms`；人口写入 `987,433.868ms`；DB `655,818,752 B` + WAL `23,768,312 B` = `679,587,064 B` |
| Artifact | [ID `11668666649`](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38046309971)；ZIP SHA-256 `5d076132a68771e578e14fea85c5973e3ade3c0b7750f0d526238faff264a117`；JSONL SHA-256 `577fcb2f339b61a89f0e8e18ab30f7ac6fe93f0b3383b58562c8646eef10dbfd` |
| Scope guards | 固定 500k；未重跑完整 12 类 performance matrix；未跑 1m；production SQL/schema/index/performance gate 均未修改 |

四态的 volume 全部 enabled，disabled=0；每卷缓存 `entry_count` 与实际 active rows 一致：

| 拓扑 | Volume 数 / rows 每卷 | Active rows | Stale rows | enabled / disabled |
|---|---:|---:|---:|---:|
| `one_volume_zero_stale` | 1 / 500,000 | 500,000 | 0 | 1 / 0 |
| `one_volume_ninety_percent_stale` | 1 / 500,000 | 50,000 | 450,000 | 1 / 0 |
| `ten_volumes_zero_stale` | 10 / 50,000 | 500,000（50,000/卷） | 0 | 10 / 0 |
| `ten_volumes_ninety_percent_stale` | 10 / 50,000 | 50,000（5,000/卷） | 450,000（45,000/卷） | 10 / 0 |

每态先记录完整行计数、卷状态与 cached count，再比较候选并 rollback 回单卷、全 active 基线后才应用下一态。四态均完成；最终仍有 500,000 rows，四次 rollback 校验通过，schema/index signature 前后相同 `44c7419a0ad16e8c7af554d794be256ceee94c4ad08280f6d0448393d248e671`。状态切换 p50/p95 `62,491.045/145,964.392ms`，恢复 p50/p95 `49,324.619/115,375.342ms`；此为 fixture 准备开销，不计入 candidate latency。

### Source-health SQL：配对延迟与工作量

每一候选 5 warmups + 30 samples，同 runner、同 SQLite fixture，候选次序轮转。单位 ms；每个 cell 为 p50 / p95 / p99。`Fullscan` 是 SQLite StatementStatus 计数；不是 CPU 时间。

| 拓扑 | Candidate | SQL p50 / p95 / p99 | VM steps | SORT | Fullscan |
|---|---|---:|---:|---:|---:|
| 1 卷 / 0% stale | Original | 782.768 / 839.750 / 855.465 | 18,500,060 | 2 | 0 |
| 1 卷 / 0% stale | B（当前 production） | 217.548 / 227.923 / 233.725 | 7,500,077 | 0 | 499,999 |
| 1 卷 / 0% stale | C（test-only） | 376.007 / 399.666 / 423.258 | 10,000,043 | 0 | 0 |
| 1 卷 / 90% stale | Original | 77.432 / 86.520 / 93.786 | 1,850,061 | 2 | 0 |
| 1 卷 / 90% stale | B（当前 production） | 49.715 / 62.742 / 75.160 | 2,550,077 | 0 | 499,999 |
| 1 卷 / 90% stale | C（test-only） | 43.104 / 45.237 / 46.093 | 1,000,044 | 0 | 0 |
| 10 卷 / 0% stale | Original | 713.182 / 743.973 / 761.018 | 18,500,375 | 2 | 9 |
| 10 卷 / 0% stale | B（当前 production） | 232.621 / 240.687 / 242.552 | 7,500,500 | 0 | 500,008 |
| 10 卷 / 0% stale | C（test-only） | 295.210 / 302.136 / 320.544 | 10,000,241 | 0 | 9 |
| 10 卷 / 90% stale | Original | 66.149 / 72.305 / 73.161 | 1,850,376 | 2 | 9 |
| 10 卷 / 90% stale | B（当前 production） | 51.722 / 57.152 / 57.898 | 2,550,500 | 0 | 500,008 |
| 10 卷 / 90% stale | C（test-only） | 32.636 / 34.794 / 35.558 | 1,000,242 | 0 | 9 |

代表性 `EXPLAIN QUERY PLAN`（各拓扑每个候选的 plan 形状一致；卷数改变对应 `Fullscan` 小计）：

```text
Original:
SCAN gv USING INDEX sqlite_autoindex_global_volumes_1
SEARCH ge USING INDEX idx_global_entries_volume (volume_id=? AND is_stale=?) LEFT-JOIN
USE TEMP B-TREE FOR GROUP BY
USE TEMP B-TREE FOR ORDER BY

Candidate B:
MATERIALIZE active_volume_facts
SCAN global_entries USING INDEX idx_global_entries_volume
SCAN gv USING INDEX sqlite_autoindex_global_volumes_1
SEARCH active_volume_facts USING AUTOMATIC COVERING INDEX (volume_id=?) LEFT-JOIN

Candidate C:
SCAN gv USING INDEX sqlite_autoindex_global_volumes_1
SEARCH ge USING INDEX idx_global_entries_volume (volume_id=? AND is_stale=?) LEFT-JOIN
```

B 在全 active 时将 sort/VM 工作换成遍历约 500k index rows，仍明显快于 Original；但 90% stale 时 Original/C 只 seek 并遍历约 50k active rows，B 仍遍历约 500k。C 按唯一 `gv.id` 分组，保留逐卷 active/stale 索引 seek，去掉宽分组和两项排序；这与 C 在 stale-heavy 拓扑更快相符。B 在这四种拓扑均未慢于 Original；C 相对 B 的 SQL p95：90% stale 单卷快 `27.9%`、90% stale 十卷快 `39.1%`；全 active 单卷慢 `75.4%`、全 active 十卷慢 `25.5%`。

### 完整 Repository Snapshot 配对测量

以下每候选每查询 5 warmups + 30 samples，单位 ms，p50 / p95 / p99。请求在同一 `Database::search_global_entries_snapshot()` 方法和一个 read transaction 内执行搜索、source facts/revision 与 index status；B 是当前未 override 的生产 SQL，Original/C 只用已有 test-only SQL override。`no_result` 使用 `zzznomatchtoken`，high-hit FTS 使用 `report`，结果数分别为 0 和 80。

| 拓扑 / 查询 | Original | Candidate B（生产） | Candidate C（test-only） | snapshot equality |
|---|---:|---:|---:|---|
| 1 卷 / 0% / no-result | 750.408 / 803.456 / 826.322 | 237.326 / 293.004 / 299.978 | 258.041 / 278.940 / 317.154 | 3 项候选一致 |
| 1 卷 / 0% / high-hit FTS | 882.629 / 970.512 / 989.512 | 389.892 / 401.693 / 419.803 | 407.460 / 426.508 / 454.790 | 80 results；一致 |
| 1 卷 / 90% / no-result | 80.808 / 92.920 / 107.005 | 52.280 / 59.085 / 76.288 | 46.583 / 56.570 / 64.527 | 3 项候选一致 |
| 1 卷 / 90% / high-hit FTS | 128.761 / 137.177 / 154.819 | 102.282 / 106.608 / 108.185 | 96.050 / 100.451 / 124.677 | 80 results；一致 |
| 10 卷 / 0% / no-result | 731.837 / 771.930 / 786.338 | 251.580 / 255.904 / 277.307 | 312.509 / 339.030 / 347.464 | 3 项候选一致 |
| 10 卷 / 0% / high-hit FTS | 848.307 / 888.906 / 894.781 | 365.017 / 400.823 / 406.862 | 426.081 / 435.166 / 459.078 | 80 results；一致 |
| 10 卷 / 90% / no-result | 63.147 / 68.545 / 70.179 | 49.082 / 55.308 / 58.920 | 32.438 / 35.302 / 36.189 | 3 项候选一致 |
| 10 卷 / 90% / high-hit FTS | 113.213 / 123.716 / 127.244 | 99.146 / 110.909 / 116.605 | 82.202 / 88.286 / 104.370 | 80 results；一致 |

Candidate C 的完整 Snapshot p95 对 B：1 卷 90% stale no-result / FTS 分别快 `4.3% / 5.8%`；10 卷 90% stale 分别快 `36.2% / 20.4%`。全 active 拓扑中 C 则落后 B：10 卷 no-result / FTS 分别慢 `32.5% / 8.6%`。这些数据说明 Candidate C 不是跨拓扑无条件更快，且 1 卷 high-hit 的 C p99 `124.677ms` 高于 B `108.185ms`；报告其 p95 改善时保留这一尾延迟观察。

### 语义、资源与限制

每个 topology revision-fact equality 记录均确认 Original/B/C 的有序 source facts 相等、revision facts JSON 字节相等、source-revision BLAKE3 原始字节相等；4/4 topology 成功：

| 拓扑 | revision facts JSON bytes | BLAKE3 `source_revision` | Ordered facts / JSON bytes / hash bytes equal |
|---|---:|---|---|
| 1 卷 / 0% stale | 191 | `cad19ef2c872c87fa3ff032a5afd191d234aa1dc6c81d735631447686404ee64` | true / true / true |
| 1 卷 / 90% stale | 190 | `7d5f527b9e9397bc9202ccd04b2e494b9f42638462eb25d309e848356dd67292` | true / true / true |
| 10 卷 / 0% stale | 1,954 | `8c1e10c77270fb69694bcdefb9db12e4355ddbe89fe36f7021814a1920a03bfb` | true / true / true |
| 10 卷 / 90% stale | 1,944 | `dcec2a7f006cadea0132d66d2271b766d278f2dbb6e4464546ba7e146d818925` | true / true / true |

24/24 Snapshot records 均确认 search results、source-health、revision、index_status 全部相等。Snapshot 候选仍在同一个单读事务中运行。数据库 rows 及 schema/index signature 在结束后恢复；Candidate C 只在 `#[cfg(test)]` 测试路径中。

每个 topology/candidate 的下表 resource 值取同 topology 下该 candidate 的 SQL、no-result Snapshot 与 high-hit Snapshot resource records 的最大采样工作集/private commit；CPU 是这些 records 平均进程 CPU 的范围与最大 observed query 值。内存是整个 Windows 测试进程采样，不是 SQLite 单条语句的分配归因，也不是设备级常驻内存预算。

| 拓扑 | Candidate | 平均 CPU % of one core（记录范围）/峰值采样 | Peak working set MiB（相对 topology 起点增量） | Peak private commit MiB（相对起点增量） |
|---|---|---:|---:|---:|
| 1 卷 / 0% | Original | 99.6–99.8 / 103.1 | 283.98 (+48.24) | 70.17 (+52.79) |
| 1 卷 / 0% | B | 98.5–99.9 / 106.6 | 280.79 (+45.05) | 66.86 (+49.48) |
| 1 卷 / 0% | C | 100.0–100.4 / 105.8 | 280.62 (+44.88) | 66.38 (+48.99) |
| 1 卷 / 90% | Original | 99.6–100.6 / 118.2 | 398.44 (+134.43) | 60.00 (+1.52) |
| 1 卷 / 90% | B | 100.2–101.7 / 128.7 | 398.44 (+134.43) | 60.00 (+1.52) |
| 1 卷 / 90% | C | 97.1–99.9 / 115.8 | 398.44 (+134.43) | 60.00 (+1.52) |
| 10 卷 / 0% | Original | 99.5–99.8 / 103.2 | 547.82 (+123.43) | 77.93 (+9.98) |
| 10 卷 / 0% | B | 99.8–100.8 / 105.8 | 547.82 (+123.43) | 77.93 (+9.98) |
| 10 卷 / 0% | C | 99.0–100.0 / 106.3 | 547.82 (+123.43) | 77.93 (+9.98) |
| 10 卷 / 90% | Original | 96.4–100.6 / 124.8 | 398.97 (+87.52) | 69.63 (+0.12) |
| 10 卷 / 90% | B | 97.1–102.2 / 127.5 | 398.97 (+87.52) | 69.59 (+0.09) |
| 10 卷 / 90% | C | 100.0–100.5 / 148.0 | 398.97 (+87.52) | 69.59 (+0.09) |

`temp_store=MEMORY` 已记录。**SQLite temp allocation bytes = NOT VERIFIED**：当前 sampler 无法把临时 B-tree / materialization 的真实 allocation 与 SQLite connection/cache 及 memory-mapped 页分开，因此不能声称临时内存为零。进程级 CPU 约一个逻辑 core；peak sampling 中短窗口百分数超过 100%，按 observed peak 原样保留，不能当作持续多核 CPU。最高采样工作集约 `547.82 MiB`，最高 private commit 约 `77.93 MiB`；峰值主要因 topology/process 窗口和 SQLite memory map/caches，候选间差值不能解释为单条语句新增内存。

### Owner 决策结论与本轮状态

- B 相对 Original：四种拓扑所有 SQL 与完整 Snapshot p95 都更快，没有测到 B 相对 Original 的 stale-heavy / multi-volume 回退；但 90% stale 的 B 仍为全 entries index scan，而 Original/C 通过复合索引只遍历 active rows。
- C 相对 B：在 stale-heavy SQL 上 27.9% / 39.1% p95 收益，在 10 卷 stale-heavy Snapshot 上 no-result / high-hit 取得 36.2% / 20.4% p95 收益；全 active 时 C 比 B 慢。达到 Owner 评论要求的“若 C materially wins 则回报 Owner 并等待决策”条件。
- **不建议当前直接合并**：不因这个诊断把生产 SQL 从 B 换成 C，也不修改生产 query；将 B vs C 的拓扑取舍留给 Owner。Candidate C 实现若获批准应另作受控生产改造，并保留本节 evidence 做回归基线。
- PR exact-head CI：run `38046309973`，SHA `ddaa782307ac87daf46d8f81f634bbca18649909`，19 jobs SUCCESS / 0 failure / 8 scoped skips。Owner benchmark job `114196449981` SUCCESS，artifact upload SUCCESS。
- PR #361 仍 OPEN / Draft，Issue #359 仍 OPEN。没有 merge、#360 工作、1m benchmark 或 Codex Review；历史 run/failure 原样保留。

**最终状态：`#359 OWNER TOPOLOGY EVIDENCE COMPLETE — MERGE ON HOLD — WAITING FOR OWNER DECISION`**
