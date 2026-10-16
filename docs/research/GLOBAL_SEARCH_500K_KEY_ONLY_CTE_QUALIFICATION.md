# Zen Canvas #360 — Global Search Key-only CTE Qualification

> 状态：Windows Hosted 100k / 500k 同 fixture 配对资格测量已完成；生产 SQL / Schema / IPC / 100ms Gate 未改动。最终结论以完整 pooled search 与 Repository Snapshot 数据为准。

## 1. 审计基线与边界

| 项目 | 记录 |
|---|---|
| Repository | `ArdenZC/Zen-Canvas` |
| Issue | [#360](https://github.com/ArdenZC/Zen-Canvas/issues/360)，保持 OPEN |
| Starting `origin/master` SHA | `16b9c791acf9e2e907174ce21863025d079e5351`（重新 fetch 后与本地基线一致） |
| 分支 | `research/issue-360-global-search-key-only-cte-qualification` |
| PR | [#362](https://github.com/ArdenZC/Zen-Canvas/pull/362)，OPEN / Draft；不合并、不关闭 Issue |
| 变更类别 | test-only SQL candidate、诊断、独立 Hosted workflow、研究报告 |
| 生产搜索与 source-health SQL | 未修改；Candidate B (#359) 保持当前 master 生产实现 |
| Schema / FTS index / IPC / query gate | 未修改；仍为 limit clamp 1–200、候选窗口 4096、warm p95 100ms |
| 1m benchmark / Codex Review | 未运行 / 不运行 |

开始前检查了开放 PR。#356 是 Windows process heap 研究，#335 是 onboarding scan-scope；均未修改本次 Global Search SQL、Schema 或诊断。PR #361 已合并至当前 master，Issue #359 已关闭。

### #359 合并后质量状态

master exact-head hosted run [38050482258](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38050482258)，head `16b9c791acf9e2e907174ce21863025d079e5351`，结论 **FAILURE**。Source checkout、scope/routing、validation plan、Windows Global Index service、Windows/macOS Rust quality、Windows/macOS release compile、Native macOS performance、Library & Content、Workspace Foundation、Search、Preview Platform、Scan & Schema 均成功。`Performance / Intelligence (push)` 在 “Run Intelligence performance suite from prepared binary” 失败；聚合 Performance profile 因该 shard 失败，Windows/macOS Quality gates 随后因依赖聚合失败而失败。该失败是本次研究的 CI blocker 记录，不在本分支修复或重跑。

## 2. 研究问题与候选

比较同一 fixture 上的：

1. **Original**：当前生产 `search.rs` 中的 prefix、extension-prefix 与 FTS tier SQL。
2. **Key-only CTE**：研究用的 `WITH candidates AS MATERIALIZED` 变体；先按生产谓词、排序键和 limit 收集 `rowid / id / modified_at_fs / rank`，再按 rowid 回表投影完整 `GlobalSearchResult` 并计算 managed 标记。

候选仅通过 `#[cfg(test)]` thread-local override 进入现有 tier、pooled search、Repository Snapshot 的真实调用路径。release build 不包含 override；生产 SQL 文本和默认执行路径不变。精确 SQL 与 query bind 参数由 JSONL 证据记录，源码生成器位于 `src-tauri/src/global_index/search.rs` 的 `key_only_*_sql` 函数。

Prefix 与 extension-prefix 在 candidates 中先过滤 enabled volume 和 `is_stale = 0`，然后按 `modified_at_fs DESC, id ASC` 排序再 limit。FTS 保留 `global_entries_fts CROSS JOIN global_entries CROSS JOIN global_volumes` 的 FTS-first 顺序、`bm25(global_entries_fts, 8.0, 2.0, 1.0)`、相同 rank/mtime/id 排序。最终查询根据候选 rowid 回表，并保留同一 managed EXISTS 语义。Exact basename 与 exact extension 不适用 CTE，继续走 Original 路径。


### 2.1 候选 SQL（完整结构）

下面是研究代码生成的 SQL。`{{FROM}}`、`{{WHERE}}`、`{{RANK}}`、`{{INNER_ORDER}}`、`{{OUTER_ORDER}}` 与 `{{LIMIT}}` 均按随后列出的 tier 定义替换；完整展开文本和 bind 参数也分别保存在每个 JSONL 的 `original_sql` / `candidate_sql` 字段。

```sql
WITH candidates AS MATERIALIZED (
    SELECT ge.rowid AS entry_rowid,
           ge.id AS entry_id,
           ge.modified_at_fs AS modified_at_fs,
           {{RANK}} AS candidate_rank
    FROM {{FROM}}
    WHERE {{WHERE}}
    ORDER BY {{INNER_ORDER}}
    LIMIT {{LIMIT}}
)
SELECT ge.id, ge.volume_id, ge.platform_file_id, ge.name, ge.path,
       ge.extension, ge.is_directory, ge.size, ge.created_at_fs,
       ge.modified_at_fs, ge.file_attributes, ge.is_hidden, ge.is_system,
       ge.source_provider,
       EXISTS (
           SELECT 1
           FROM managed_entries me
           JOIN managed_scopes ms ON ms.id = me.managed_scope_id
           WHERE me.global_entry_id = ge.id
             AND me.enabled = 1
             AND ms.enabled = 1
       ) AS managed,
       candidates.candidate_rank AS rank
FROM candidates
JOIN global_entries ge ON ge.rowid = candidates.entry_rowid
ORDER BY {{OUTER_ORDER}};
```

Tier substitutions (bind numbering matches `candidate_sql`):

| Tier | `FROM` | `WHERE` | Rank / inner order / outer order / limit |
|---|---|---|---|
| Name prefix | `global_entries ge INDEXED BY idx_global_entries_active_name_order CROSS JOIN global_volumes gv` | `gv.id = ge.volume_id AND gv.enabled = 1 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?2 AND ge.name_normalized <> lower(?1)` | `0.0`; `ge.modified_at_fs DESC, ge.id ASC`; `candidates.modified_at_fs DESC, candidates.entry_id ASC`; `?3` |
| Punctuation prefix | `global_entries ge INDEXED BY idx_global_entries_active_name_order CROSS JOIN global_volumes gv` | `gv.id = ge.volume_id AND gv.enabled = 1 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?1` | `0.0`; `ge.modified_at_fs DESC, ge.id ASC`; `candidates.modified_at_fs DESC, candidates.entry_id ASC`; `?2` |
| Extension prefix | `global_entries ge INDEXED BY idx_global_entries_active_extension_order CROSS JOIN global_volumes gv` | `gv.id = ge.volume_id AND gv.enabled = 1 AND ge.is_stale = 0 AND ge.extension GLOB ?2 AND ge.extension <> lower(?1)` | `1.0`; `ge.modified_at_fs DESC, ge.id ASC`; `candidates.modified_at_fs DESC, candidates.entry_id ASC`; `?3` |
| FTS | `global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv` | `global_entries_fts MATCH ?1 AND ge.rowid = global_entries_fts.rowid AND gv.id = ge.volume_id AND gv.enabled = 1 AND ge.is_stale = 0` | `bm25(global_entries_fts, 8.0, 2.0, 1.0)`; `candidate_rank ASC, ge.modified_at_fs DESC, ge.id ASC`; `candidates.candidate_rank ASC, candidates.modified_at_fs DESC, candidates.entry_id ASC`; `?2` |

The two exact-match tiers do not use this candidate. The CTE applies enabled-volume and stale filters before limit; the intentionally unsafe limit-before-filter prefix and FTS forms were also exercised and both underfilled an 80-row page.


## 3. 实验方法

- Windows Hosted 单 runner；分别建立 deterministic synthetic Global Index 100k 与 500k fixture。每种规模只 population 一次；Original 与候选复用同一个数据库、连接及查询参数，不运行 filesystem scan。
- fixture 使用现有 production schema/index/FTS trigger 与 512 行写批次。每种大小有 1 个 fixture，Original/CTE 的 A/B 样本顺序交替；每一候选每一 profile 5 次 warmup、30 次 measured samples。
- 各层分别测 SQL tier、`Database::search_global_entries` pooled search、`Database::search_global_entries_snapshot`；报告 p50/p95/p99。Snapshot 的 search results、source-health、source_revision、index_status 在同一读事务内比较，两边均使用 master 当前 #359 Candidate B。
- JSONL 记录 runner、SQLite 版本、PRAGMA、生成/填充时间、数据库大小、source SHA、EXPLAIN、VM steps、Fullscan steps、Sort operations、返回行数、全部结果对象及资源样本。
- 12 类正式矩阵覆盖 exact/prefix/high-fanout/FTS/extensions/duplicate/no-result/Chinese/punctuation/accented Unicode。另覆盖第一屏 80、limit 200、500 clamp、offset 4016/3896/4096、4096 窗口、0/1/5/100/1000/4096/10000/25000 hit overlays、两 enabled volumes、disabled/stale、managed/unmanaged、tie break、cross-tier dedup。
- Windows 进程 CPU / working set / private bytes 是每次调用前后 boundary samples 汇总，不是 OS peak，也不归因到单条 SQL。SQLite 临时 B-tree 字节 **NOT VERIFIED**；Snapshot entrypoint 本身没有 statement counters。

### 3.1 首轮完整 Hosted 执行环境

| Fixture | Runner / image | SQLite | Rows | Generation | Population | Main DB | Main + WAL |
|---:|---|---:|---:|---:|---:|---:|---:|
| 100k | Windows x86_64 / win25-vs2026 20260925.250.1 | 3.51.3 | 100,000 | 294.3 ms | 119549.4 ms | 125.2 MiB | 144.3 MiB |
| 500k | Windows x86_64 / win25-vs2026 20260925.250.1 | 3.51.3 | 500,000 | 1531.7 ms | 824483.4 ms | 625.4 MiB | 648.1 MiB |


## 4. 历史基线（非本次配对结果）

正式 500k 历史测试 #350：12/12 correctness 通过、5/12 warm p95 ≤100ms、7/12 未通过。历史 Windows 500k synthetic p95（不可与本次 A/B 混为同一 runner 配对）：

| 查询类别 | 历史 p95 (ms) | 100ms |
|---|---:|---|
| Exact basename | 0.428 | PASS |
| Name prefix | 122.733 | FAIL |
| Common/high-fanout prefix | 124.964 | FAIL |
| FTS report | 157.715 | FAIL |
| FTS invoice | 162.472 | FAIL |
| Exact extension | 0.511 | PASS |
| Extension prefix | 88.581 | PASS |
| Duplicate basename | 0.351 | PASS |
| No result | 0.457 | PASS |
| Chinese prefix | 122.870 | FAIL |
| Punctuation prefix | 121.500 | FAIL |
| Accented Unicode prefix | 118.173 | FAIL |

历史 #352 局部 SQL 配对只测 query tier：name prefix p95 `78.532 → 37.045ms`、FTS report `112.579 → 80.811ms`、FTS invoice `117.087 → 86.216ms`。这不是 pooled 或完整 snapshot 证据。#359 已合并后的当前 production Snapshot p95（原 Candidate B source-health，历史 500k run）：no-result `432.649ms`、prefix `658.827ms`、FTS `770.355ms`。本研究已完成两次同 fixture 配对验证；见第 6.7–6.8 节的最终代码头数据与跨 run 复现差异。


## 5. 正确性与 adversarial 证据
| 检查 | 100k | 500k |
|---|---|---|
| 12 类 count oracle + 完整 result object（含 rank/排序） | 通过 12/12 | 通过 12/12 |
| offset 4016 / 4096、最后一页 | 6 类分页记录全相等；offset 4096 空 | 6 类分页记录全相等；offset 4096 空 |
| Prefix / FTS 4096 candidate window | 全字段相等；窗口 4096 | 全字段相等；窗口 4096 |
| Hit overlay：0/1/5/100/1,000/4,096/10,000/25,000 | 全部 count、字段及顺序断言通过 | 全部 count、字段及顺序断言通过 |
| Enabled/disabled volumes、stale filter | 2 个 enabled volumes；stale/disabled 排除 | 2 个 enabled volumes；stale/disabled 排除 |
| 两种 unsafe limit-before-filter | Prefix / FTS 均 underfill 80 行页（returned 0） | Prefix / FTS 均 underfill 80 行页（returned 0） |
| Managed/unmanaged、duplicate basename/path、同 mtime/id tie、cross-tier dedup | 全部相等 | 全部相等 |
| Snapshot results/source-health/source_revision/index_status | 12/12 相等；相同 read transaction | 12/12 相等；相同 read transaction |
| rollback / schema / persistent index | overlay rollback；row count 100,000；signature 不变 | overlay rollback；row count 500,000；signature 不变 |
| 生产 search/source-health SQL、Schema、100ms gate | 未修改 | 未修改 |

FTS EXPLAIN 在 Original 与 CTE 均以 `SCAN global_entries_fts VIRTUAL TABLE INDEX 0:M3` 开始；CTE 没有从 `global_entries` 驱动 FTS，也保留了 `bm25(..., 8.0, 2.0, 1.0)` 和 stable tie-break。两个 unsafe 查询在 limit 后才应用 volume/stale 过滤，均返回 0 条，而安全原路径和 CTE 都保留 80 条候选所需的正确语义。



## 6. 配对性能结果（首轮 run 38053163599）

首轮 source SHA `75988266957fc8dcfff795130c067febd6369b89`，GitHub Actions run attempt 1。所有数值为 Windows Server image `win25-vs2026`、SQLite 3.51.3、合成 SQLite fixture。每个 profile 为 Original/CTE 同 runner、同 fixture、同 connection 的 5 warmup + 30 sample 交替配对。百分比正值表示 CTE p95 下降。

### 6.1 Raw SQL tier：p50 / p95 / p99（ms）

| 规模 | 查询 | 命中数 | Original p50/p95/p99 | Key-only CTE p50/p95/p99 | p95变化 | VM steps | Fullscan steps | Sort ops | 临时 B-tree |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---|
| 100k | `name_prefix` / `name_prefix` | 5000 | 22.379/22.830/22.856 | 10.368/10.652/10.700 | +53.3% | 231,224 → 127,673 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `common_prefix_high_fanout` / `name_prefix` | 4999 | 23.577/24.335/25.524 | 10.426/11.011/14.380 | +54.8% | 231,178 → 127,648 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `chinese_prefix` / `name_prefix` | 5000 | 22.848/25.381/28.434 | 10.278/10.461/10.577 | +58.8% | 231,223 → 127,672 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `punctuation_prefix` / `punctuation_prefix` | 5000 | 22.722/23.655/24.751 | 9.989/10.742/14.455 | +54.6% | 216,222 → 112,671 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `unicode_accented_prefix` / `name_prefix` | 5000 | 22.392/23.194/25.505 | 10.493/11.039/15.319 | +52.4% | 231,224 → 127,673 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `extension_prefix` / `extension_prefix` | 8129 | 8.413/8.587/8.681 | 10.217/10.463/10.491 | -21.8% | 141,737 → 181,751 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `fts_high_fanout_report` / `fts` | 5000 | 36.067/39.356/40.820 | 25.152/27.158/28.353 | +31.0% | 231,221 → 132,670 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `fts_high_fanout_invoice` / `fts` | 5000 | 37.970/39.777/39.980 | 27.061/29.009/31.652 | +27.1% | 231,221 → 132,670 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `fts_low_fanout` / `fts` | 0 | 0.563/0.630/0.744 | 0.648/0.718/0.739 | -14.0% | 21 → 30 | 0 → 0 | 1 → 1 | yes / yes |
| 100k | `fts_no_result` / `fts` | 0 | 0.259/0.320/0.330 | 0.345/0.410/0.465 | -28.1% | 21 → 30 | 0 → 0 | 1 → 1 | yes / yes |
| 100k | `hit_scale_0` / `name_prefix` | 0 | 0.273/0.373/0.412 | 0.395/0.471/0.512 | -26.3% | 22 → 31 | 0 → 0 | 1 → 1 | yes / yes |
| 100k | `hit_scale_1` / `name_prefix` | 1 | 0.203/0.247/0.271 | 0.279/0.333/0.338 | -34.8% | 85 → 91 | 0 → 0 | 1 → 1 | yes / yes |
| 100k | `hit_scale_5` / `name_prefix` | 5 | 0.252/0.393/0.430 | 0.323/0.577/0.581 | -46.8% | 329 → 323 | 0 → 4 | 1 → 1 | yes / yes |
| 100k | `hit_scale_100` / `name_prefix` | 100 | 0.743/0.784/0.836 | 0.811/0.911/1.040 | -16.2% | 5,824 → 5,173 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `hit_scale_1000` / `name_prefix` | 1000 | 4.456/7.498/7.531 | 2.240/4.221/4.338 | +43.7% | 47,224 → 27,673 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `hit_scale_4096` / `name_prefix` | 4096 | 15.192/19.294/20.655 | 6.967/8.393/9.162 | +56.5% | 189,640 → 105,073 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `hit_scale_10000` / `name_prefix` | 10000 | 45.150/61.127/63.329 | 21.856/25.293/31.068 | +58.6% | 461,224 → 252,673 | 0 → 79 | 1 → 1 | yes / yes |
| 100k | `hit_scale_25000` / `name_prefix` | 25000 | 114.446/130.675/133.652 | 56.222/73.097/79.431 | +44.1% | 1,151,224 → 627,673 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `name_prefix` / `name_prefix` | 25000 | 119.915/121.973/132.228 | 56.246/61.530/63.578 | +49.6% | 1,151,224 → 627,673 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `common_prefix_high_fanout` / `name_prefix` | 24999 | 127.501/133.493/137.235 | 57.685/61.097/64.526 | +54.2% | 1,151,178 → 627,648 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `chinese_prefix` / `name_prefix` | 25000 | 123.985/129.993/132.624 | 54.979/56.762/60.747 | +56.3% | 1,151,223 → 627,672 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `punctuation_prefix` / `punctuation_prefix` | 25000 | 122.178/139.245/145.283 | 56.810/79.535/90.492 | +42.9% | 1,076,222 → 552,671 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `unicode_accented_prefix` / `name_prefix` | 25000 | 122.215/129.047/131.084 | 58.636/65.184/70.420 | +49.5% | 1,151,224 → 627,673 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `extension_prefix` / `extension_prefix` | 40629 | 50.157/55.838/59.922 | 60.193/64.457/67.528 | -15.4% | 694,237 → 896,751 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `fts_high_fanout_report` / `fts` | 25000 | 192.876/200.263/201.976 | 134.367/143.553/144.676 | +28.3% | 1,151,221 → 652,670 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `fts_high_fanout_invoice` / `fts` | 25000 | 201.310/211.122/212.603 | 144.433/151.480/155.368 | +28.2% | 1,151,221 → 652,670 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `fts_low_fanout` / `fts` | 1 | 1.739/1.755/1.780 | 1.833/1.861/1.877 | -6.0% | 82 → 89 | 0 → 0 | 1 → 1 | yes / yes |
| 500k | `fts_no_result` / `fts` | 0 | 0.267/0.311/0.336 | 0.352/0.407/0.421 | -30.9% | 21 → 30 | 0 → 0 | 1 → 1 | yes / yes |
| 500k | `hit_scale_0` / `name_prefix` | 0 | 0.203/0.257/0.260 | 0.278/0.340/0.400 | -32.3% | 22 → 31 | 0 → 0 | 1 → 1 | yes / yes |
| 500k | `hit_scale_1` / `name_prefix` | 1 | 0.229/0.275/0.287 | 0.311/0.377/0.384 | -37.1% | 85 → 91 | 0 → 0 | 1 → 1 | yes / yes |
| 500k | `hit_scale_5` / `name_prefix` | 5 | 0.250/0.306/0.329 | 0.332/0.477/0.523 | -55.9% | 329 → 323 | 0 → 4 | 1 → 1 | yes / yes |
| 500k | `hit_scale_100` / `name_prefix` | 100 | 0.846/0.883/0.917 | 1.070/1.166/1.177 | -32.0% | 5,824 → 5,173 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `hit_scale_1000` / `name_prefix` | 1000 | 4.475/4.526/4.538 | 2.692/2.774/2.780 | +38.7% | 47,224 → 27,673 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `hit_scale_4096` / `name_prefix` | 4096 | 17.290/17.887/18.188 | 8.774/9.072/9.253 | +49.3% | 189,640 → 105,073 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `hit_scale_10000` / `name_prefix` | 10000 | 42.577/44.572/49.187 | 20.767/25.061/28.134 | +43.8% | 461,224 → 252,673 | 0 → 79 | 1 → 1 | yes / yes |
| 500k | `hit_scale_25000` / `name_prefix` | 25000 | 105.905/113.256/113.757 | 49.876/54.498/57.694 | +51.9% | 1,151,224 → 627,673 | 0 → 79 | 1 → 1 | yes / yes |

SQL tier 的 100ms 是对照参考值，不是 official full-search gate。500k name/prefix/Unicode tiers 从 122–139ms 降至 56–80ms；FTS 高扇出从 200–211ms 降至 144–151ms，仍高于 100ms。Extension-prefix CTE 慢 15.4%（55.838→64.457ms）。FTS 单命中/零命中及无结果 prefix overlay 的 CTE 绝对时间不到 2ms，但相对增幅较高。

### 6.2 100k pooled search + 完整 Repository Snapshot：p50 / p95 / p99（ms）

| 查询类别 | 命中数 | Pooled Original p50/p95/p99 | Pooled CTE p50/p95/p99 | Pooled p95变化 | Snapshot Original p50/p95/p99 | Snapshot CTE p50/p95/p99 | Snapshot p95变化 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `exact_basename` | 1 | 1.075/1.126/1.151 | 1.342/1.369/1.421 | -21.6% | 78.983/82.760/86.156 | 79.416/82.677/87.678 | +0.1% |
| `name_prefix` | 5000 | 40.662/41.900/45.733 | 26.732/27.953/31.633 | +33.3% | 121.430/129.439/144.575 | 107.031/116.712/119.591 | +9.8% |
| `common_prefix_high_fanout` | 4999 | 42.804/49.158/56.181 | 27.211/28.495/37.143 | +42.0% | 122.726/128.507/130.195 | 107.208/113.564/113.982 | +11.6% |
| `fts_substring_report` | 5000 | 60.498/66.990/68.941 | 46.908/49.669/51.584 | +25.9% | 137.166/143.959/147.641 | 123.887/131.044/134.446 | +9.0% |
| `fts_substring_invoice` | 5000 | 60.533/65.010/67.730 | 47.979/50.209/51.977 | +22.8% | 137.873/146.146/153.075 | 126.040/136.596/137.842 | +6.5% |
| `extension_exact` | 18130 | 1.063/1.123/1.137 | 1.179/1.251/1.419 | -11.4% | 78.887/81.234/82.592 | 78.866/82.338/85.349 | -1.4% |
| `extension_prefix` | 8129 | 25.332/26.173/26.421 | 27.724/31.592/32.352 | -20.7% | 100.102/104.954/106.880 | 103.047/113.334/116.085 | -8.0% |
| `duplicate_basename` | 5000 | 0.654/0.708/0.711 | 0.667/0.779/0.979 | -10.0% | 79.513/89.944/93.525 | 79.286/90.546/100.005 | -0.7% |
| `no_result` | 0 | 1.121/1.214/1.366 | 1.397/1.475/1.768 | -21.5% | 79.247/86.811/92.710 | 79.928/82.193/85.873 | +5.3% |
| `chinese_prefix` | 5000 | 42.098/43.862/47.590 | 26.992/27.450/27.774 | +37.4% | 120.107/128.580/132.808 | 105.047/114.489/121.790 | +11.0% |
| `punctuation_prefix` | 5000 | 41.448/42.299/42.533 | 26.657/30.864/32.353 | +27.0% | 121.080/142.858/149.415 | 106.477/117.756/120.012 | +17.6% |
| `unicode_accented_prefix` | 5000 | 41.892/47.612/48.855 | 27.474/32.904/35.067 | +30.9% | 137.337/153.412/156.179 | 119.589/147.055/151.440 | +4.1% |

### 6.3 500k pooled search + 完整 Repository Snapshot：p50 / p95 / p99（ms）

| 查询类别 | 命中数 | Pooled Original p50/p95/p99 | Pooled CTE p50/p95/p99 | Pooled p95变化 | Snapshot Original p50/p95/p99 | Snapshot CTE p50/p95/p99 | Snapshot p95变化 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `exact_basename` | 1 | 1.096/1.154/1.159 | 1.360/1.396/2.068 | -21.0% | 394.291/430.528/453.630 | 395.145/432.174/458.269 | -0.4% |
| `name_prefix` | 25000 | 203.460/212.698/214.107 | 136.529/146.222/147.162 | +31.3% | 597.381/608.007/608.747 | 533.085/543.506/544.051 | +10.6% |
| `common_prefix_high_fanout` | 24999 | 205.038/225.139/235.742 | 133.326/139.944/140.243 | +37.8% | 604.474/622.332/625.533 | 532.518/548.512/555.438 | +11.9% |
| `fts_substring_report` | 25000 | 297.010/310.199/316.327 | 228.762/252.519/259.571 | +18.6% | 688.437/701.702/703.933 | 626.458/646.315/704.298 | +7.9% |
| `fts_substring_invoice` | 25000 | 301.432/313.725/316.464 | 239.466/246.709/248.108 | +21.4% | 702.997/717.468/724.576 | 639.402/653.881/657.607 | +8.9% |
| `extension_exact` | 90630 | 1.107/1.297/1.318 | 1.282/1.446/1.477 | -11.5% | 411.633/421.365/422.935 | 405.948/418.311/421.204 | +0.7% |
| `extension_prefix` | 40629 | 135.077/142.225/149.793 | 146.036/156.034/158.165 | -9.7% | 543.996/675.704/689.186 | 551.182/683.668/690.975 | -1.2% |
| `duplicate_basename` | 25000 | 0.701/1.688/2.248 | 0.681/1.619/1.959 | +4.1% | 412.499/500.277/509.342 | 414.308/479.100/527.876 | +4.2% |
| `no_result` | 0 | 1.169/2.551/2.849 | 1.472/3.061/3.294 | -20.0% | 410.804/418.776/419.914 | 407.189/414.761/416.303 | +1.0% |
| `chinese_prefix` | 25000 | 215.081/244.561/254.926 | 141.523/159.750/166.101 | +34.7% | 648.184/684.383/714.419 | 570.256/599.607/601.078 | +12.4% |
| `punctuation_prefix` | 25000 | 217.767/239.515/245.798 | 147.580/162.823/172.047 | +32.0% | 648.701/699.802/778.404 | 581.188/696.816/765.542 | +0.4% |
| `unicode_accented_prefix` | 25000 | 217.320/236.085/238.333 | 149.765/170.500/179.669 | +27.8% | 658.710/690.350/699.927 | 586.239/612.034/652.066 | +11.3% |

### 6.4 100ms Gate 结果

- 100k 首轮 diagnostic：pooled Original 与 CTE 均 12/12 p95 ≤100ms；PR official baseline run `38053163483` 也 12/12 通过（其 warm p95 为 `1.100–64.507ms`）。
- 500k 首轮 paired diagnostic：Original 和 CTE 均为 4/12 pass、8/12 fail；**0 类别跨越 pass/fail**。两边都 pass：exact basename、exact extension、duplicate basename、no-result。两边都 fail：name prefix、common prefix、FTS report、FTS invoice、extension prefix、Chinese、punctuation、accented Unicode。
- #350 历史 standalone 500k baseline 为 5/12 pass，extension-prefix 当时 p95 `88.581ms`；本次同-run Original extension-prefix p95 `142.225ms`。这两个非配对 Hosted run 的差异属于环境/run-to-run 变化，不能归因于 CTE。本次 candidate 与 Original 在同 runner 同 fixture 下均 fail，CTE 未把任何查询从 FAIL 变 PASS。
- 完整 Repository Snapshot 没有被 100ms gate 覆盖；500k CTE Snapshot 高命中 p95 仍约 `543–697ms`。当前 #359 source-health Candidate B 是 Original/CTE 共用生产路径，未混入 source-health Candidate C。

### 6.5 执行计划与 SQLite work counters

| 500k tier/query | Original → CTE p95 | VM steps | Fullscan steps | Sort ops | EXPLAIN 特征 |
|---|---:|---:|---:|---:|---|
| Name prefix `quarterly`（25k hits，SQL tier limit 80） | 121.973 → 61.530ms | 1,151,224 → 627,673 | 0 → 79 | 1 → 1 | 两边均 name active index range seek + temp B-tree；CTE 额外 `MATERIALIZE candidates`，随后 80 row CTE scan + rowid lookups |
| FTS report（25k hits，SQL tier limit 80） | 200.263 → 143.553ms | 1,151,221 → 652,670 | 0 → 79 | 1 → 1 | 两边均 FTS `VIRTUAL TABLE INDEX 0:M3` first；CTE 在 rank/排序/limit 后回表 80 rows |
| Extension prefix `jp`（40,629 hits） | 55.838 → 64.457ms | 694,237 → 896,751 | 0 → 79 | 1 → 1 | 两边均 extension active index；CTE 引入更高 VM work，与实测回退一致 |
| FTS zero-hit | 0.311 → 0.407ms | 21 → 30 | 0 → 0 | 1 → 1 | FTS-first 顺序保留；materialization 有固定低命中成本 |

Fullscan=79 是扫描 80 行 materialized candidate rows 的 StatementStatus 计数；并非对 `global_entries` 的全表扫描。两边的 sorter 都显示 `USE TEMP B-TREE FOR ORDER BY`；SQLite statement counters 中 temp-B-tree 字节数 **NOT VERIFIED**。

### 6.6 数据库、CPU 与内存观测

| Fixture | Runner / image | SQLite | Rows | Generation | Population | Main DB | Main + WAL |
|---:|---|---:|---:|---:|---:|---:|---:|
| 100k | Windows x86_64 / win25-vs2026 20260925.250.1 | 3.51.3 | 100,000 | 294.3 ms | 119549.4 ms | 125.2 MiB | 144.3 MiB |
| 500k | Windows x86_64 / win25-vs2026 20260925.250.1 | 3.51.3 | 500,000 | 1531.7 ms | 824483.4 ms | 625.4 MiB | 648.1 MiB |

在 `temp_store=MEMORY` 下，SQLite EXPLAIN 表明两种查询均使用临时排序 B-tree。SQL-specific temporary bytes 无法读取。Windows 进程 boundary sample 的代表值：

| 规模 / profile | Original CPU / working set / private | CTE CPU / working set / private |
|---|---|---|
| 500k name-prefix SQL，30 calls 汇总 | 101.1% CPU; WS 265.8 MiB; private 19.2 MiB | 96.6% CPU; WS 265.8 MiB; private 19.2 MiB |
| 500k name-prefix pooled，30 calls 汇总 | 100.0% CPU; WS 267.0 MiB; private 19.8 MiB | 99.5% CPU; WS 267.0 MiB; private 19.8 MiB |
| 500k name-prefix Snapshot，30 calls 汇总 | 98.9% CPU; WS 262.7 MiB; private 19.8 MiB | 99.2% CPU; WS 262.7 MiB; private 19.8 MiB |
| 500k FTS report SQL，30 calls 汇总 | 99.7% CPU; WS 266.6 MiB; private 19.8 MiB | 99.6% CPU; WS 266.6 MiB; private 19.8 MiB |

这些 CPU 百分比是累计 30 次样本的进程 CPU 时间 / 样本 wall time；大致围绕单逻辑核饱和。进程内还包含 SQLite/cache/runtime 工作，无法归因成单 SQL CPU。500k 的全 SQL profile 最高 boundary-sampled working set 约 350 MiB、private bytes 约 24.5 MiB；pooled/Snapshot profile 的相应最大采样值约 267 MiB / 20.1 MiB。它们是 profile process 前后采样值，不是 OS peak，也不是 CTE 专属增量。


### 6.7 最终代码头复验：Windows Hosted run 38056013822 / attempt 1

该 run checkout source SHA `50c95b42227b479f6ed3f3746d2fce26ad8d3a17`。相较首轮 `75988266957fc8dcfff795130c067febd6369b89`，只有 test diagnostic helper 参数改为 `KeyOnlySqlPair`，候选 SQL、fixture、样本次数和查询语义未变。每档独立建立单一合成 fixture，Original/CTE 在该档同一 fixture/connection 上配对；100k/500k 串行，未运行 1m。

| Fixture | Runner / image | SQLite | Rows | Generation | Population | Main DB | Main + WAL |
|---:|---|---:|---:|---:|---:|---:|---:|
| 100k | win25-vs2026 20260925.250.1 | 3.51.3 | 100,000 | 305.4 ms | 254436.3 ms | 125.2 MiB | 144.3 MiB |
| 500k | win25-vs2026 20260925.250.1 | 3.51.3 | 500,000 | 1562.8 ms | 1818725.2 ms | 625.4 MiB | 648.1 MiB |

两个规模均为 correctness 12/12；所有 `GlobalSearchResult` 字段（含 BM25 rank / 顺序）和 Snapshot results/source-health/source_revision/index_status 相等。count oracle、分页、managed 与 unmanaged、enabled/disabled volumes、stale rows、equal-time tie、duplicate/cross-volume path、stable-ID dedup、4096 window 与 limit clamp 均通过；unsafe filter-after-limit prefix/FTS 都 underfill 80-row page。overlay rollback 后 base rows 保持不变，schema/index signature 不变。完整逐项断言位于 artifact JSONL。

#### 100k — raw SQL tier

| Query class / tier | Hits | Original p50/p95/p99 | CTE p50/p95/p99 | p95 change | VM steps O→C | Fullscan O→C | Sort O→C | Temp B-tree O/C |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| `name_prefix` / `name_prefix` | 5000 | 24.717/25.029/25.496 | 12.138/12.460/12.714 | +50.2% | 231,224 → 127,673 | 0 → 79 | 1 → 1 | yes / yes |
| `common_prefix_high_fanout` / `name_prefix` | 4999 | 26.319/27.444/29.435 | 12.315/12.588/15.910 | +54.1% | 231,178 → 127,648 | 0 → 79 | 1 → 1 | yes / yes |
| `chinese_prefix` / `name_prefix` | 5000 | 25.445/25.974/26.076 | 11.802/12.234/13.535 | +52.9% | 231,223 → 127,672 | 0 → 79 | 1 → 1 | yes / yes |
| `punctuation_prefix` / `punctuation_prefix` | 5000 | 25.071/25.453/26.444 | 11.846/13.959/15.507 | +45.2% | 216,222 → 112,671 | 0 → 79 | 1 → 1 | yes / yes |
| `unicode_accented_prefix` / `name_prefix` | 5000 | 24.668/25.326/25.954 | 12.447/12.682/13.288 | +49.9% | 231,224 → 127,673 | 0 → 79 | 1 → 1 | yes / yes |
| `extension_prefix` / `extension_prefix` | 8129 | 10.792/11.799/15.662 | 12.634/13.189/13.659 | -11.8% | 141,737 → 181,751 | 0 → 79 | 1 → 1 | yes / yes |
| `fts_high_fanout_report` / `fts` | 5000 | 39.160/40.800/44.284 | 27.614/27.962/28.088 | +31.5% | 231,221 → 132,670 | 0 → 79 | 1 → 1 | yes / yes |
| `fts_high_fanout_invoice` / `fts` | 5000 | 40.958/42.419/44.746 | 29.599/31.794/34.699 | +25.0% | 231,221 → 132,670 | 0 → 79 | 1 → 1 | yes / yes |
| `fts_low_fanout` / `fts` | 0 | 0.584/0.653/0.776 | 0.686/0.749/0.810 | -14.7% | 21 → 30 | 0 → 0 | 1 → 1 | yes / yes |
| `fts_no_result` / `fts` | 0 | 0.266/0.291/0.303 | 0.352/0.416/0.444 | -43.0% | 21 → 30 | 0 → 0 | 1 → 1 | yes / yes |
| `hit_scale_0` / `name_prefix` | 0 | 0.183/0.242/0.265 | 0.257/0.322/0.433 | -33.1% | 22 → 31 | 0 → 0 | 1 → 1 | yes / yes |
| `hit_scale_1` / `name_prefix` | 1 | 0.205/0.253/0.267 | 0.283/0.347/0.358 | -37.2% | 85 → 91 | 0 → 0 | 1 → 1 | yes / yes |
| `hit_scale_5` / `name_prefix` | 5 | 0.231/0.289/0.302 | 0.313/0.371/0.380 | -28.4% | 329 → 323 | 0 → 4 | 1 → 1 | yes / yes |
| `hit_scale_100` / `name_prefix` | 100 | 0.766/0.812/0.816 | 0.803/0.869/0.886 | -7.0% | 5,824 → 5,173 | 0 → 79 | 1 → 1 | yes / yes |
| `hit_scale_1000` / `name_prefix` | 1000 | 4.090/5.692/6.037 | 2.230/2.355/2.452 | +58.6% | 47,224 → 27,673 | 0 → 79 | 1 → 1 | yes / yes |
| `hit_scale_4096` / `name_prefix` | 4096 | 15.966/20.554/21.987 | 7.364/9.256/10.757 | +55.0% | 189,640 → 105,073 | 0 → 79 | 1 → 1 | yes / yes |
| `hit_scale_10000` / `name_prefix` | 10000 | 41.855/44.713/45.292 | 20.546/21.312/22.879 | +52.3% | 461,224 → 252,673 | 0 → 79 | 1 → 1 | yes / yes |
| `hit_scale_25000` / `name_prefix` | 25000 | 102.558/113.612/116.401 | 49.576/50.525/57.175 | +55.5% | 1,151,224 → 627,673 | 0 → 79 | 1 → 1 | yes / yes |

#### 500k — raw SQL tier

| Query class / tier | Hits | Original p50/p95/p99 | CTE p50/p95/p99 | p95 change | VM steps O→C | Fullscan O→C | Sort O→C | Temp B-tree O/C |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| `name_prefix` / `name_prefix` | 25000 | 120.904/126.052/128.887 | 56.869/58.598/59.988 | +53.5% | 1,151,224 → 627,673 | 0 → 79 | 1 → 1 | yes / yes |
| `common_prefix_high_fanout` / `name_prefix` | 24999 | 127.761/131.910/142.504 | 58.509/63.900/66.199 | +51.6% | 1,151,178 → 627,648 | 0 → 79 | 1 → 1 | yes / yes |
| `chinese_prefix` / `name_prefix` | 25000 | 121.943/129.039/129.929 | 55.014/58.400/60.704 | +54.7% | 1,151,223 → 627,672 | 0 → 79 | 1 → 1 | yes / yes |
| `punctuation_prefix` / `punctuation_prefix` | 25000 | 119.776/127.680/129.706 | 56.677/59.958/62.352 | +53.0% | 1,076,222 → 552,671 | 0 → 79 | 1 → 1 | yes / yes |
| `unicode_accented_prefix` / `name_prefix` | 25000 | 120.813/129.867/134.831 | 59.344/62.810/66.024 | +51.6% | 1,151,224 → 627,673 | 0 → 79 | 1 → 1 | yes / yes |
| `extension_prefix` / `extension_prefix` | 40629 | 51.149/51.691/51.703 | 59.862/62.514/63.989 | -20.9% | 694,237 → 896,751 | 0 → 79 | 1 → 1 | yes / yes |
| `fts_high_fanout_report` / `fts` | 25000 | 193.311/200.301/207.230 | 134.936/143.964/148.231 | +28.1% | 1,151,221 → 652,670 | 0 → 79 | 1 → 1 | yes / yes |
| `fts_high_fanout_invoice` / `fts` | 25000 | 203.265/210.569/211.454 | 146.428/150.721/151.853 | +28.4% | 1,151,221 → 652,670 | 0 → 79 | 1 → 1 | yes / yes |
| `fts_low_fanout` / `fts` | 1 | 1.741/1.763/1.783 | 1.843/1.866/1.969 | -5.8% | 82 → 89 | 0 → 0 | 1 → 1 | yes / yes |
| `fts_no_result` / `fts` | 0 | 0.265/0.388/0.469 | 0.355/0.416/0.421 | -7.2% | 21 → 30 | 0 → 0 | 1 → 1 | yes / yes |
| `hit_scale_0` / `name_prefix` | 0 | 0.184/0.192/0.194 | 0.268/0.335/0.340 | -74.5% | 22 → 31 | 0 → 0 | 1 → 1 | yes / yes |
| `hit_scale_1` / `name_prefix` | 1 | 0.207/0.272/0.364 | 0.287/0.358/0.373 | -31.6% | 85 → 91 | 0 → 0 | 1 → 1 | yes / yes |
| `hit_scale_5` / `name_prefix` | 5 | 0.231/0.283/0.292 | 0.316/0.379/0.380 | -33.9% | 329 → 323 | 0 → 4 | 1 → 1 | yes / yes |
| `hit_scale_100` / `name_prefix` | 100 | 0.775/0.812/0.820 | 0.844/0.900/0.912 | -10.8% | 5,824 → 5,173 | 0 → 79 | 1 → 1 | yes / yes |
| `hit_scale_1000` / `name_prefix` | 1000 | 4.103/4.287/5.576 | 2.254/2.411/2.484 | +43.8% | 47,224 → 27,673 | 0 → 79 | 1 → 1 | yes / yes |
| `hit_scale_4096` / `name_prefix` | 4096 | 17.371/20.561/21.873 | 8.664/8.935/12.236 | +56.5% | 189,640 → 105,073 | 0 → 79 | 1 → 1 | yes / yes |
| `hit_scale_10000` / `name_prefix` | 10000 | 42.692/46.818/49.816 | 20.595/21.244/25.835 | +54.6% | 461,224 → 252,673 | 0 → 79 | 1 → 1 | yes / yes |
| `hit_scale_25000` / `name_prefix` | 25000 | 105.163/106.819/107.306 | 49.900/58.586/61.103 | +45.2% | 1,151,224 → 627,673 | 0 → 79 | 1 → 1 | yes / yes |

#### 100k — pooled search + full Repository Snapshot

| Query class | Hits | Pooled Original p50/p95/p99 | Pooled CTE p50/p95/p99 | Pooled Δp95 | Snapshot Original p50/p95/p99 | Snapshot CTE p50/p95/p99 | Snapshot Δp95 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `exact_basename` | 1 | 1.092/1.139/1.223 | 1.362/1.412/1.529 | -24.0% | 83.837/89.345/91.027 | 84.168/90.762/93.611 | -1.6% |
| `name_prefix` | 5000 | 43.351/44.380/45.282 | 29.508/33.294/39.346 | +25.0% | 126.226/136.117/137.204 | 112.276/117.240/118.441 | +13.9% |
| `common_prefix_high_fanout` | 4999 | 44.188/45.261/49.298 | 29.244/31.043/34.038 | +31.4% | 127.631/138.163/145.760 | 111.684/120.779/122.014 | +12.6% |
| `fts_substring_report` | 5000 | 61.930/67.953/68.710 | 48.773/49.520/49.761 | +27.1% | 144.145/151.421/151.807 | 131.066/139.432/141.427 | +7.9% |
| `fts_substring_invoice` | 5000 | 63.883/67.358/69.302 | 51.077/53.170/53.319 | +21.1% | 145.312/159.144/159.374 | 132.820/142.722/145.888 | +10.3% |
| `extension_exact` | 18130 | 1.100/1.129/1.137 | 1.197/1.236/1.241 | -9.5% | 83.827/90.724/93.408 | 83.553/84.969/85.124 | +6.3% |
| `extension_prefix` | 8129 | 28.256/30.830/34.227 | 30.875/31.710/33.268 | -2.9% | 106.998/112.917/117.474 | 109.849/120.256/122.715 | -6.5% |
| `duplicate_basename` | 5000 | 0.673/0.721/0.725 | 0.670/0.794/1.141 | -10.1% | 83.428/91.363/91.708 | 83.460/88.290/95.281 | +3.4% |
| `no_result` | 0 | 1.102/1.160/1.166 | 1.410/1.437/1.446 | -23.9% | 83.825/88.861/90.625 | 84.027/93.934/100.976 | -5.7% |
| `chinese_prefix` | 5000 | 44.194/45.237/49.229 | 29.430/31.018/32.150 | +31.4% | 127.239/134.114/135.714 | 111.978/119.514/120.668 | +10.9% |
| `punctuation_prefix` | 5000 | 44.222/48.707/50.760 | 29.738/30.546/33.481 | +37.3% | 127.424/137.388/141.417 | 112.462/116.063/118.561 | +15.5% |
| `unicode_accented_prefix` | 5000 | 42.977/45.436/49.137 | 29.625/30.788/33.601 | +32.2% | 126.941/133.258/136.518 | 112.224/125.350/130.300 | +5.9% |

#### 500k — pooled search + full Repository Snapshot

| Query class | Hits | Pooled Original p50/p95/p99 | Pooled CTE p50/p95/p99 | Pooled Δp95 | Snapshot Original p50/p95/p99 | Snapshot CTE p50/p95/p99 | Snapshot Δp95 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `exact_basename` | 1 | 1.064/1.130/1.229 | 1.339/1.386/1.520 | -22.7% | 294.013/377.777/446.794 | 296.091/371.560/414.980 | +1.6% |
| `name_prefix` | 25000 | 118.975/125.194/130.782 | 55.295/60.088/64.996 | +52.0% | 421.451/594.458/599.212 | 354.378/363.741/365.375 | +38.8% |
| `common_prefix_high_fanout` | 24999 | 129.599/134.005/135.408 | 58.953/67.395/69.482 | +49.7% | 430.302/593.221/603.033 | 351.650/370.479/395.789 | +37.5% |
| `fts_substring_report` | 25000 | 197.344/208.486/209.871 | 138.068/144.497/145.771 | +30.7% | 497.812/591.862/650.651 | 430.897/469.254/480.694 | +20.7% |
| `fts_substring_invoice` | 25000 | 204.995/217.426/222.006 | 147.674/155.503/155.673 | +28.5% | 506.534/651.845/670.761 | 439.973/506.375/544.173 | +22.3% |
| `extension_exact` | 90630 | 1.091/1.116/1.121 | 1.196/1.227/1.238 | -9.9% | 298.256/314.996/318.852 | 297.263/333.350/354.053 | -5.8% |
| `extension_prefix` | 40629 | 52.554/53.235/54.473 | 61.480/70.550/74.122 | -32.5% | 350.272/430.358/502.586 | 354.488/377.859/455.196 | +12.2% |
| `duplicate_basename` | 25000 | 0.676/0.778/0.799 | 0.681/0.732/0.766 | +5.9% | 295.051/380.816/450.194 | 295.574/340.814/395.166 | +10.5% |
| `no_result` | 0 | 1.133/1.165/1.205 | 1.402/1.439/1.458 | -23.5% | 299.708/420.266/446.036 | 295.979/341.779/355.957 | +18.7% |
| `chinese_prefix` | 25000 | 124.748/131.984/136.678 | 56.598/65.204/66.915 | +50.6% | 420.874/508.880/576.166 | 354.674/445.774/470.639 | +12.4% |
| `punctuation_prefix` | 25000 | 123.192/129.054/130.467 | 59.800/64.162/64.407 | +50.3% | 422.085/439.419/465.509 | 355.578/433.224/475.713 | +1.4% |
| `unicode_accented_prefix` | 25000 | 122.982/136.302/140.127 | 59.599/60.399/61.290 | +55.7% | 422.467/446.804/546.247 | 354.731/371.228/376.806 | +16.9% |

#### 本轮 100ms pooled gate

- 100k: Original 12/12 pass，CTE 12/12 pass。
- 500k: Original 5/12 pass，CTE 10/12 pass；发生 5 个 FAIL→PASS crossing：`name_prefix`, `common_prefix_high_fanout`, `chinese_prefix`, `punctuation_prefix`, `unicode_accented_prefix`. FTS report/invoice 两类仍 FAIL；两边 exact / extension-exact / extension-prefix / duplicate / no-result 通过。
- 500k CTE 的高扇出 name/Chinese/punctuation/accented pooled p95 为 `60.088–67.395ms`；FTS report/invoice `144.497 / 155.503ms`。Extension-prefix CTE `70.550ms` 对 Original `53.235ms`，虽然都小于 100ms，CTE 自身慢 32.5%。

#### SQLite plan / work counters（最终 500k）

| Query | Original→CTE p95 (ms) | VM steps | Fullscan steps | Sort ops | Plan evidence |
|---|---:|---:|---:|---:|---|
| Name prefix, 25,000 hits | 126.052→58.598 | 1,151,224→627,673 | 0→79 | 1→1 | existing active index retained |
| FTS report, 25,000 hits | 200.301→143.964 | 1,151,221→652,670 | 0→79 | 1→1 | FTS virtual table remains first |
| Extension prefix, 40,629 hits | 51.691→62.514 | 694,237→896,751 | 0→79 | 1→1 | existing active index retained |
| FTS zero-hit | 0.388→0.416 | 21→30 | 0→0 | 1→1 | FTS virtual table remains first |

FTS 两边仍以 `SCAN global_entries_fts VIRTUAL TABLE INDEX 0:M3` 驱动；CTE 在 FTS rank/order/limit 后只对 80 候选回表。CTE query plan 可见 `MATERIALIZE candidates`、`SCAN candidates` 和 rowid lookup。candidate 的 fullscan=79 表示对 80 个已 materialize candidate rows 的遍历，不是扫描 500k 全表。两边仍有 `USE TEMP B-TREE FOR ORDER BY`；`temp_store=MEMORY`，实际 temp B-tree bytes **NOT VERIFIED**。

#### Process-wide resources（最终 run）

| Scale / profile | Original CPU / WS / private | Candidate CPU / WS / private |
|---|---|---|
| 100k name-prefix SQL | 92.6% CPU / 69.6 MiB WS / 16.5 MiB private | 106.8% CPU / 69.6 MiB WS / 16.5 MiB private |
| 100k name-prefix pooled | 100.9% CPU / 69.1 MiB WS / 17.2 MiB private | 96.5% CPU / 69.1 MiB WS / 17.2 MiB private |
| 100k name-prefix Snapshot | 100.0% CPU / 68.8 MiB WS / 17.2 MiB private | 98.5% CPU / 68.8 MiB WS / 17.2 MiB private |
| 100k FTS report SQL | 97.7% CPU / 68.9 MiB WS / 17.1 MiB private | 99.9% CPU / 68.9 MiB WS / 17.1 MiB private |
| 500k name-prefix SQL | 98.9% CPU / 266.3 MiB WS / 20.2 MiB private | 100.4% CPU / 266.3 MiB WS / 20.2 MiB private |
| 500k name-prefix pooled | 99.6% CPU / 470.7 MiB WS / 18.2 MiB private | 99.8% CPU / 470.7 MiB WS / 18.2 MiB private |
| 500k name-prefix Snapshot | 99.6% CPU / 470.7 MiB WS / 18.2 MiB private | 98.8% CPU / 470.7 MiB WS / 18.2 MiB private |
| 500k FTS report SQL | 99.3% CPU / 263.3 MiB WS / 17.2 MiB private | 98.7% CPU / 263.3 MiB WS / 17.2 MiB private |

以上为每次调用边界处采样后汇总的进程数据，不是 OS peak，CPU 与 working set 不可归因到单 SQL。最终 500k SQL profile 最大边界采样 WS/private 为约 548.7 / 23.0 MiB；pooled/Snapshot 最大 WS/private 约 471.1 / 19.5 MiB。临时 B-tree bytes 不可测。

### 6.8 两次 500k paired run 的复现差异

首轮 run 38053163599 / attempt 1 使用 source SHA `75988266957fc8dcfff795130c067febd6369b89`；最终 run 38056013822 / attempt 1 使用 `50c95b4…`。该 SHA 差异仅来自诊断 helper 参数打包和报告更新，生产 SQL、候选 SQL 与 fixture/query profile 未改变。两次都使用 Windows `win25-vs2026` image `20260925.250.1`、SQLite 3.51.3、同样的 100k/500k synthetic dataset definition、单 fixture per size、5 warmups + 30 paired samples。数据库大小一致，但数据填充时间 100k `119.5s→254.4s`、500k `824.5s→1818.7s`（约 2.1–2.2 倍）；运行环境负载存在明显差异。

#### 500k pooled / Snapshot p95 跨 run 对照（ms）

| Query class | Pooled Original p95 R1→R2 | Pooled CTE p95 R1→R2 | Pooled Δp95 R1→R2 | Snapshot Original p95 R1→R2 | Snapshot CTE p95 R1→R2 | Snapshot Δp95 R1→R2 |
|---|---:|---:|---:|---:|---:|---:|
| `exact_basename` | 1.154→1.130 | 1.396→1.386 | -21.0%→-22.7% | 430.528→377.777 | 432.174→371.560 | -0.4%→+1.6% |
| `name_prefix` | 212.698→125.194 | 146.222→60.088 | +31.3%→+52.0% | 608.007→594.458 | 543.506→363.741 | +10.6%→+38.8% |
| `common_prefix_high_fanout` | 225.139→134.005 | 139.944→67.395 | +37.8%→+49.7% | 622.332→593.221 | 548.512→370.479 | +11.9%→+37.5% |
| `fts_substring_report` | 310.199→208.486 | 252.519→144.497 | +18.6%→+30.7% | 701.702→591.862 | 646.315→469.254 | +7.9%→+20.7% |
| `fts_substring_invoice` | 313.725→217.426 | 246.709→155.503 | +21.4%→+28.5% | 717.468→651.845 | 653.881→506.375 | +8.9%→+22.3% |
| `extension_exact` | 1.297→1.116 | 1.446→1.227 | -11.5%→-9.9% | 421.365→314.996 | 418.311→333.350 | +0.7%→-5.8% |
| `extension_prefix` | 142.225→53.235 | 156.034→70.550 | -9.7%→-32.5% | 675.704→430.358 | 683.668→377.859 | -1.2%→+12.2% |
| `duplicate_basename` | 1.688→0.778 | 1.619→0.732 | +4.1%→+5.9% | 500.277→380.816 | 479.100→340.814 | +4.2%→+10.5% |
| `no_result` | 2.551→1.165 | 3.061→1.439 | -20.0%→-23.5% | 418.776→420.266 | 414.761→341.779 | +1.0%→+18.7% |
| `chinese_prefix` | 244.561→131.984 | 159.750→65.204 | +34.7%→+50.6% | 684.383→508.880 | 599.607→445.774 | +12.4%→+12.4% |
| `punctuation_prefix` | 239.515→129.054 | 162.823→64.162 | +32.0%→+50.3% | 699.802→439.419 | 696.816→433.224 | +0.4%→+1.4% |
| `unicode_accented_prefix` | 236.085→136.302 | 170.500→60.399 | +27.8%→+55.7% | 690.350→446.804 | 612.034→371.228 | +11.3%→+16.9% |

两次 run 中，prefix 与高命中 FTS 的**同 run CTE 相对 Original 方向均改善**；SQL-tier p95 与 VM step 也近似稳定。绝对 pooled latency、Snapshot p95 改善幅度和 gate crossing 则受运行差异影响：首轮 500k Original/CTE 分别 4/12、4/12 pass，0 crossing；最终 run 分别 5/12、10/12 pass，5 个 prefix 类 crossing。run2 的五个 crossing 不应被解释为已稳定保证的 100ms gate 达标，尤其同一批次 population wall time 约翻倍。需要 Owner 在生产改造的下一阶段决定是否追加独立重复确认。

首轮/最终 run 资源采样对照也表明进程 WS 差异：500k pooled/Snapshot 名称 prefix 约 `267MiB→471MiB`，而 private bytes 约 `19.8MiB→18.2MiB`；memory-mapped DB/page cache 及 runner 状态会影响观测。因此只以同一 run 内 Original-vs-CTE 配对估计候选差异，不把跨 run 绝对时间当作因果对照。

## 7. Artifact、CI 与复现限制

### 7.1 最终代码头受控 CTE run 与 JSONL artifact

Windows Hosted run [38056013822](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013822)，attempt 1，exact benchmark source SHA `50c95b42227b479f6ed3f3746d2fce26ad8d3a17`，SUCCESS；单 runner 串行完成 100k + 500k。Windows job 从 13:31:05Z 到 14:20:07Z，共 49m 02s，其中 100k step 8m 24s、500k step 39m 45s。每档 Original/CTE 同 fixture、同 SQLite connection，5 warmups + 30 paired samples。

| Fixture | Artifact ID / download | JSONL SHA-256 (GHA + downloaded file) | Uploaded ZIP SHA-256 |
|---:|---|---|---|
| 100k | [11672311826](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013822/artifacts/11672311826) | `143e1980577a9c5a1468a48548ef2f96bd6271fbbb8a72a5b027a689064c380f` | `a5b13d8be2ca3ebf63d06e3ca14b7fcbf778e2874ef0af155742ab6a5ed30d70` |
| 500k | [11672217005](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013822/artifacts/11672217005) | `080b553a556cc536cf11e7a06b2d7ff15673f823220eb6ea51de8c2e4abdef0c` | `e8e5d703e2d1d4f2e9abc54dcdcf8ac6fe2a6b35d9639128e3d2e9d50f488955` |

GHA PowerShell 的 `Get-FileHash SHA256` 与 Cloud 下载 ZIP 后解压再计算的 JSONL hash 完全一致；artifact retention 30 天。100k/500k fixture completion 记录均为 12/12 correctness PASS、schema/index signature unchanged、生产 SQL/source-health/schema/gate unchanged、overlay rollback，`one_million_row_benchmark_run=false`。

### 7.2 首轮受控 CTE run 与 JSONL artifact

Windows Hosted run [38053163599](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38053163599)，attempt 1，source SHA `75988266957fc8dcfff795130c067febd6369b89`，run SUCCESS。

| Fixture | Artifact ID / 下载 | JSONL SHA-256 | Uploaded ZIP SHA-256 |
|---:|---|---|---|
| 100k | [11671590338](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38053163599/artifacts/11671590338) | `933055241ff97818fe5a129aeb63f6c5801fe4a832680150a9547e6a97961766` | `02bce72ec4acab268b173d21abd3679b49bdfbabedd69ca34a78341ffed0dc18` |
| 500k | [11671170715](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38053163599/artifacts/11671170715) | `f05ce02cbb542bcbeff768f9d704b34460bfbbf3987fd136780d8fa9f7c3578b` | `91db14c2ccb5944fd6071401ccfe92c624853ba07b2999ab83baa7ad00295ae7` |

JSONL SHA 在 Hosted Windows job 中以 `Get-FileHash SHA256` 生成；随后通过 GitHub artifact connector 下载 ZIP 并在 Cloud workspace 重新计算 extracted JSONL SHA，两个 fixture 均匹配。artifact 保留 30 天。

### 7.3 首轮 official 100k baseline artifact

PR baseline run [38053163483](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38053163483) SUCCESS；12/12 correctness、12/12 original warm p95 ≤100ms。Artifact `11670886787`，Uploaded ZIP SHA-256 `d8e197f20d5b3c4904c7b56dceed302edfa53e4b5d4a61e574736e2a53e3fb9d`。

| Query class | Original warm p95 ms | 100ms |
|---|---:|---|
| `exact_basename` | 1.151 | PASS |
| `name_prefix` | 42.125 | PASS |
| `common_prefix_high_fanout` | 45.535 | PASS |
| `fts_substring_report` | 60.205 | PASS |
| `fts_substring_invoice` | 64.507 | PASS |
| `extension_exact` | 1.152 | PASS |
| `extension_prefix` | 26.568 | PASS |
| `duplicate_basename` | 1.100 | PASS |
| `no_result` | 1.814 | PASS |
| `chinese_prefix` | 48.040 | PASS |
| `punctuation_prefix` | 45.196 | PASS |
| `unicode_accented_prefix` | 44.299 | PASS |

### 7.4 最终代码头 official 100k baseline 与 PR CI

- Global Search synthetic benchmark [38056013794](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013794)，attempt 1，source SHA `50c95b42227b479f6ed3f3746d2fce26ad8d3a17`，SUCCESS；12/12 correctness 和 12/12 warm p95 ≤100ms，regressions 为空。Artifact `11671871510`，JSONL SHA-256 `2680e8d3b065222e71bc5f5caa75f5624bccff920a2cfa2d0d56d8dd762d1e4d`，uploaded ZIP SHA-256 `896416f0902983959cb98f63dc2834de3e7ac5a730b5556a311aef58f313f1a5`。本 benchmark 是独立官方 100k baseline；Original 与 CTE 的因果比较来自同一 diagnostic fixture 的配对 artifacts。

| 100k query class | Warm p95 ms | 100ms |
|---|---:|---|
| `exact_basename` | 1.363 | PASS |
| `name_prefix` | 45.718 | PASS |
| `common_prefix_high_fanout` | 42.572 | PASS |
| `fts_substring_report` | 64.861 | PASS |
| `fts_substring_invoice` | 65.323 | PASS |
| `extension_exact` | 1.129 | PASS |
| `extension_prefix` | 27.884 | PASS |
| `duplicate_basename` | 0.705 | PASS |
| `no_result` | 1.178 | PASS |
| `chinese_prefix` | 43.737 | PASS |
| `punctuation_prefix` | 48.462 | PASS |
| `unicode_accented_prefix` | 43.565 | PASS |

- Final exact-head PR CI [38056013797](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013797)，source SHA `50c95b42227b479f6ed3f3746d2fce26ad8d3a17`，SUCCESS。Windows/macOS Rust quality、Windows Global Index service qualification、Windows/macOS release compile、Native macOS performance、Performance profile 与全部 required performance shards 均通过；test-only Clippy fix 经两平台验证。

### 7.5 PR exact-head CI on first PR head

Run [38053163496](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38053163496) on head `75988266957fc8dcfff795130c067febd6369b89` completed **FAILURE**. Checkout/evidence/scope/validation plan、Windows Global Index service、Performance Prepare、全部 Performance shards（Intelligence/Search/Library & Content/Scan & Schema/Workspace Foundation/Preview Platform）、Performance profile、Windows/macOS Release compile、Native macOS performance 全通过。Windows/macOS Rust quality 与 Quality aggregate 失败于 Rust clippy `too_many_arguments` on new test-only `profile_key_only_sql_pair` function. Failure is preserved. Local fix now wraps profile inputs in `KeyOnlySqlPair`; 在当前 Cloud 环境复跑该 Windows GNU Clippy 时，依赖构建因缺少 `x86_64-w64-mingw32-gcc` 停在 Clippy 之前（exit 101）；该工具缺失不代表 lint 失败。后续 PR exact-head run [38056013797](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013797) 已在 `50c95b4` 通过 Windows/macOS Rust quality，Clippy 参数问题修复已获 Hosted CI 验证。

### 7.6 其他运行与限制

- `cargo fmt -- --check`、`actionlint`、`git diff --check`、`DOCS_DIFF_BASE=origin/master npm run test:docs`（1 个 changed Markdown file）、`npm run test:governance` 通过。Windows GNU target `cargo check --features desktop-runtime --lib --tests` 通过（交叉编译，不是 Windows execution）。
- Local Linux targeted test 在进入 test 前被 pre-existing `src/ai/settings.rs` 对 Windows/macOS-only `keyring` target dependency 的 8 个 unresolved errors 阻断（exit 101）；不得称为搜索测试失败或 PASS。
- CTE 正式 SQL A/B 只在 Windows Hosted synthetic SQLite 上测量。Windows 本机 filesystem discovery、macOS CTE performance、NTFS/APFS/FSEvents、真实 Tauri IPC/UI latency 均 **NOT VERIFIED**。macOS Hosted quality/native performance CI 不是本 CTE SQL bench。
- 历史 #350 500k standalone 结果与本次不同 Hosted run 无法交叉解释；同一 fixture 的配对 Original-vs-CTE 是性能因果证据。


## 8. 风险与 Owner 建议

### 正确性 / 性能结论

- **A — Correctness: PASS（两次 Windows synthetic paired run，最终 run 12/12）**。全量 `GlobalSearchResult`、FTS rank / order、分页、Snapshot results/source-health/revision/index_status 相等；adversarial fixtures 覆盖 stale/disabled filter-before-limit、managed 标记、tie-break、跨 tier dedup。所有 overlays rollback，schema/index signature 不变。
- **B — SQL-tier：prefix / FTS 高扇出收益方向可复现。** 两个 run 的 500k name-prefix p95 约减 50%，高扇出 FTS 约减 28%；VM steps 在 name prefix `1.15M→0.63M`、FTS report `1.15M→0.65M`。Extension-prefix SQL 慢约 15–21%；零/单命中 FTS 增加约 0.1ms，绝对仍低于 2ms。最终 run 高命中 FTS SQL p95 仍 `144–151ms`。
- **C — Full pooled search：相对改善稳定，绝对 gate 结果不稳定。** 首轮 500k 两边均 4/12 pass、0 crossing；最终 run Original 5/12、CTE 10/12 pass，有 5 个高扇出 prefix 类 crossing。最终 run name/common/Chinese/punctuation/accented prefix p95 为 `60–67ms`；FTS report/invoice 仍 `144 / 156ms` FAIL。两 run 中 extension-prefix CTE 均比 Original 慢（首轮 +9.7%，最终 +32.5%），即使最终 run 两边都低于 100ms。
- **D — Full Repository Snapshot：高扇出 prefix / FTS 均更快，未达到交互目标。** 首轮主要高扇出类别 p95 改善约 0–12%，最终 run 约 1–39%；最终 CTE prefix/FTS p95 约 `364–506ms`，绝对仍高。两次之间填充时间和 process working set 明显变化，因此不把跨 run 的绝对 CTE p95 差异视为单独的 SQL 因果效应。

### 是否授权生产 SQL 改造

建议 Owner **不要批准 blanket production replacement**。可考虑另开一个窄范围 production review，只覆盖高扇出 name-prefix（含中文、标点、重音 normalization）与高命中 FTS；明确排除 extension-prefix 和 exact tiers。两次 run 的 raw SQL、pooled search、Snapshot 同 run 内都显示高扇出 prefix / FTS 改善，但 pooled 100ms crossing 只在其中一次复现，Snapshot 仍达数百毫秒。应先把两 run 的绝对时间 / population 差异和 500k gate 不一致交由 Owner 评估，再决定是否需要额外重复基准或生产实现。

本研究 PR 不把 CTE 移入 production、不改 100ms gate、不改变 #359 Candidate B。低命中 FTS / no-result 的固定成本和 extension-prefix 回退均交 Owner 决定；若 Owner 授权后续生产任务，应分 tier 实施、保留过滤先于 limit 的语义、同 run correctness + performance profile，并完成 exact-head Windows/macOS CI。

### 残余风险 / 未测量项

- CTE 与 sorter 在 `temp_store=MEMORY` 下仍使用临时 B-tree，具体 bytes 未验证；窗口改成 4096 时其临时内存与 CPU 可能高于 SQL tier limit 80 实验。
- 100k/500k 均为合成 SQLite 行，不是 10 万/50 万真实文件、真实多磁盘或 watcher 增量场景。
- Candidate CTE 在本轮未进行 Windows/macOS native filesystem / actual UI measurement。


## 9. 当前交付状态

首轮与最终代码头两次 Windows paired bench、100k/500k correctness、CI artifacts 和最终 exact-head PR CI 均已完成。最终 run 的 `38056013822` JSONL SHA 与 artifact SHA 已双向核验；见第 7 节。生产 SQL 不变。报告最终 docs-only commit 将再触发 PR checks；PR #362 维持 OPEN / Draft，Issue #360 OPEN，不合并。

当前 verdict：**OIL FIND / KEY-ONLY CTE QUALIFICATION COMPLETE — READY FOR OWNER REVIEW**。不建议将 CTE blanket 移入生产；prefix 与高命中 FTS 值得 Owner 考虑独立窄范围生产 review，但 100ms crossing 在重复 run 中不稳定，extension-prefix 存在稳定的 SQL / pooled regression。PR #362 保持 Draft、不合并。
