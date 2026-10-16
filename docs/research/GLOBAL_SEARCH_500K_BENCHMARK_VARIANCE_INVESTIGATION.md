# #363 — Global Search 500k Benchmark Variance Investigation

> **#363 BENCHMARK VARIANCE INVESTIGATION COMPLETE — ROOT CAUSE NOT VERIFIED**
>
> 三轮历史证据显示 Candidate C 的 name-prefix / FTS raw SQL 收益方向一致，但 pooled 绝对时延和 100ms 分类不稳定。最终 PR HEAD 的复验为 Candidate C 4/12；现有证据不支持开始 #364 生产实现。本轮没有启动新 500k benchmark，没有修改生产 SQL、Candidate B、Schema、缓存、candidate window 或 100ms Gate。

## 1. 基线、source SHA 与交付边界

| 项目 | 值 |
|---|---|
| 实际 fetch 的起始 origin/master | `58062c5c356969f332f19c7458028bf2e097595e`（与任务给定值一致） |
| 第一轮 benchmark source | `75988266957fc8dcfff795130c067febd6369b89` |
| 第二轮 benchmark source | `50c95b42227b479f6ed3f3746d2fce26ad8d3a17` |
| 第三轮 source / #362 最终 PR HEAD | `bbd53469fd2b0cbbbabcd82dffde0dbfdb220b3e` |
| #362 | 已合并；本调查审计其 test-only CTE、诊断与报告 |
| #363 | 独立 Draft PR；仅报告与保留证据 |
| 新 benchmark / #364 | 未运行 / 未获授权 |
| #365 / #366 | 未修改，也未归因于 Candidate C；保持独立问题 |

三个 source SHA 间的 diff：首轮至第二轮只将 SQL helper 参数封装为 `KeyOnlySqlPair`；fixture、SQL、测量顺序、样本数、统计方法未变。第二轮至第三轮只有 #360 报告变更。`search.rs`、`repository.rs`、`db/connection.rs`、fixture generator 未变。第三轮是任务要求优先采用的最终 PR HEAD 复验。

## 2. GitHub artifacts、logs 与 SHA-256

三个 workflow run 均为 attempt 1、job/run conclusion `success`；单 job 中 100k 后串行 500k。诊断 workflow 成功代表执行和 correctness 完成，不代表 12 类性能 Gate 全部通过。JSONL 的 12 类 correctness / 完整结果对象 / Snapshot 等价检查均 12/12。

| Run | Source SHA | Job / region | Artifact | ZIP SHA-256（与 API digest、下载字节一致） | 解压 JSONL SHA-256 |
|---|---|---|---|---|---|
| [38053163599](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38053163599) | `75988266957fc8dcfff795130c067febd6369b89` | 114216231675 / eastus | [11671170715](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38053163599/artifacts/11671170715) | `91db14c2ccb5944fd6071401ccfe92c624853ba07b2999ab83baa7ad00295ae7` | `f05ce02cbb542bcbeff768f9d704b34460bfbbf3987fd136780d8fa9f7c3578b` |
| [38056013822](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013822) | `50c95b42227b479f6ed3f3746d2fce26ad8d3a17` | 114224596861 / northcentralus | [11672217005](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38056013822/artifacts/11672217005) | `e8e5d703e2d1d4f2e9abc54dcdcf8ac6fe2a6b35d9639128e3d2e9d50f488955` | `080b553a556cc536cf11e7a06b2d7ff15673f823220eb6ea51de8c2e4abdef0c` |
| [38059974375](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38059974375) | `bbd53469fd2b0cbbbabcd82dffde0dbfdb220b3e` | 114236064932 / centralus | [11673403938](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38059974375/artifacts/11673403938) | `d09c33fed288b9b3c79cc2d290ef20b84a7e5552260a3af7758be1fed36dc449` | `2654f417c74a26291b4303cff1e648f08d90fa329f2aa2257113e20a30fc042e` |

原始 JSONL 逐字节保存在 [evidence/issue-363](evidence/issue-363/)；[artifact-manifest.csv](evidence/issue-363/artifact-manifest.csv) 和 [SHA256SUMS](evidence/issue-363/SHA256SUMS) 保存 provenance 与校验和。126 条 SQL / pooled / Snapshot profile 的 p50/p95/p99、resources、SQL counters 和 Gate 字段均在 [cross-run-metrics.csv](evidence/issue-363/cross-run-metrics.csv)。

## 3. Hosted runner、fixture 与 SQLite 对照

日志三轮均为 Windows Server 2025 10.0.26100、x86_64、image `windows-2025-vs2026` version `20260925.250.1`；runner `2.337.0`、Hosted Compute Agent `20260901.588`。worker 和 region 不同。

| Run | Worker ID / region | SQLite / PRAGMA | Fixture generation / population / build (ms) | Population transaction p95 (ms) | Main DB / WAL / 合计 / pages |
|---|---|---|---:|---:|---|
| 38053163599 | 746ddb31-e554-4a81-b216-639b142617e3 / eastus | 3.51.3；WAL, sync=1, FK=1, temp_store=2, mmap=2,147,418,112, page=4096 | 1,531.7 / 824,483.4 / 826,441.2 | 1,741.2 | 655,818,752 / 23,768,312 / 679,587,064 bytes / 160,112 |
| 38056013822 | 0073361d-e2ed-493b-87c8-b5592cb03cbe / northcentralus | 同上 | 1,562.8 / 1,818,725.2 / 1,820,771.0 | 3,987.4 | 同上 |
| 38059974375 | 7bb4b222-9faa-413a-9927-ff9b134e13db / centralus | 同上 | 1,557.3 / 1,496,945.8 / 1,498,978.9 | 3,319.1 | 同上 |

三轮皆为 500,000 行 deterministic synthetic Global Index fixture、production schema/index/FTS triggers、512 行一批插入；不做 filesystem scan。DB/WAL/SHM 尺寸、page_count/page_size 和 SQLite/PRAGMA 完全一致；query hit counts 和结果也一致。population 最慢/最快相差 2.21 倍，每批事务 p95 相差约 2.29 倍，entry generation 本身约 1.53–1.56 秒。

**完整性限制：**相同尺寸及确定性生成器不等于已证明 DB 字节、页布局、SQLite stats 或 cache 相同。每轮代码会计算 schema/index signature 并断言本轮前后未变，但 JSONL 只记录 `schema_index_signature_unchanged=true`，没有 signature 值；也没有 DB hash、row fingerprint 或 `sqlite_stat1` 内容。三轮 query plans 和 SQL counters 相同，是计划稳定的旁证，不是文件字节证明。

## 4. 12 类 pooled 100ms Gate

每格为 Original / Candidate C 的 pooled p95 (ms)，P 表示 ≤100ms，F 表示 >100ms。三轮记录的 Gate 数分别为 Original **4/12、5/12、4/12** 与 Candidate C **4/12、10/12、4/12**。第三轮是最终 #362 PR HEAD，应优先判断最终稳定性：Candidate C 仍为 4/12。

| 查询类 | 38053163599 O / C | 38056013822 O / C | 38059974375 O / C |
|---|---:|---:|---:|
| exact_basename | 1.154 P / 1.396 P | 1.130 P / 1.386 P | 1.584 P / 2.006 P |
| name_prefix | 212.698 F / 146.222 F | 125.194 F / 60.088 P | 214.566 F / 140.654 F |
| common_prefix_high_fanout | 225.139 F / 139.944 F | 134.005 F / 67.395 P | 223.417 F / 148.071 F |
| fts_substring_report | 310.199 F / 252.519 F | 208.486 F / 144.497 F | 313.249 F / 257.652 F |
| fts_substring_invoice | 313.725 F / 246.709 F | 217.426 F / 155.503 F | 324.589 F / 255.826 F |
| extension_exact | 1.297 P / 1.446 P | 1.116 P / 1.227 P | 1.554 P / 1.591 P |
| extension_prefix | 142.225 F / 156.034 F | 53.235 P / 70.550 P | 146.454 F / 156.152 F |
| duplicate_basename | 1.688 P / 1.619 P | 0.778 P / 0.732 P | 0.716 P / 0.720 P |
| no_result | 2.551 P / 3.061 P | 1.165 P / 1.439 P | 1.317 P / 1.656 P |
| chinese_prefix | 244.561 F / 159.750 F | 131.984 F / 65.204 P | 225.134 F / 150.918 F |
| punctuation_prefix | 239.515 F / 162.823 F | 129.054 F / 64.162 P | 249.729 F / 169.058 F |
| unicode_accented_prefix | 236.085 F / 170.500 F | 136.302 F / 60.399 P | 222.874 F / 152.361 F |

Run 2 的 10/12 不能作为稳定 PASS：五类 prefix 只在该轮过线，在 run 1 与最终 run 3 又失败。两类高命中 FTS 三轮都失败。extension-prefix 仅 run 2 过线，但该轮 CTE 比 Original 慢，因此不是 CTE Gate 收益。

## 5. SQL / pooled / Snapshot 时延拆分

### name-prefix 的 p50 / p95 / p99（ms）

| 层 / Run | Original | Candidate C |
|---|---:|---:|
| SQL / 38053163599 | 119.915 / 121.973 / 132.228 | 56.246 / 61.530 / 63.578 |
| SQL / 38056013822 | 120.904 / 126.052 / 128.887 | 56.869 / 58.598 / 59.988 |
| SQL / 38059974375 | 119.154 / 129.568 / 132.450 | 54.979 / 57.504 / 59.349 |
| pooled / 38053163599 | 203.460 / 212.698 / 214.107 | 136.529 / 146.222 / 147.162 |
| pooled / 38056013822 | 118.975 / 125.194 / 130.782 | 55.295 / 60.088 / 64.996 |
| pooled / 38059974375 | 206.361 / 214.566 / 215.866 | 136.867 / 140.654 / 142.161 |
| Snapshot / 38053163599 | 597.381 / 608.007 / 608.747 | 533.085 / 543.506 / 544.051 |
| Snapshot / 38056013822 | 421.451 / 594.458 / 599.212 | 354.378 / 363.741 / 365.375 |
| Snapshot / 38059974375 | 618.659 / 640.681 / 667.051 | 546.398 / 556.857 / 557.413 |

Raw SQL p95 的跨轮范围小：Original 122–130ms，CTE 56–62ms。pooled p95 范围大：Original 125–215ms，CTE 60–146ms；p50 也整体移动，所以不是单个尾部样本所致。

pooled 与 SQL tier 的 name-prefix p95 描述性差值为：Original +90.7 / -0.9 / +85.0ms；CTE +84.7 / +1.5 / +83.2ms。这两个 profile 在不同测量窗口，**差值不能当成 pool wait 时间**。Run 1/3 的两种查询路径都有约 80–90ms 的共同偏移；run 2 pooled 接近 SQL tier。具体偏移来源尚未分解。

### Raw SQL CTE 收益与 query plans

| SQL tier | Original p95 范围 | CTE p95 范围 | 同轮变化 |
|---|---:|---:|---:|
| name_prefix | 122.0–129.6ms | 56.3–61.5ms | 快 49.6–55.6% |
| common_prefix_high_fanout | 131.9–133.5ms | 61.1–63.9ms | 快 51.6–54.7% |
| chinese_prefix | 126.8–130.0ms | 56.3–58.4ms | 快 54.7–56.3% |
| punctuation_prefix | 127.7–139.2ms | 60.0–79.5ms | 快 42.9–53.0% |
| unicode_accented_prefix | 129.0–134.2ms | 62.8–65.2ms | 快 49.5–51.6% |
| extension_prefix | 51.7–56.0ms | 62.5–68.9ms | 慢 15.4–23.1% |
| fts_high_fanout_report | 200.3–207.4ms | 142.9–144.0ms | 快 28.1–31.1% |
| fts_high_fanout_invoice | 210.6–216.4ms | 150.7–153.7ms | 快 28.2–29.0% |

三轮 18 个 raw SQL profile 的 Original / CTE plan 与 counters 相同。name-prefix VM steps 为 1,151,224 → 627,673；FTS report 1,151,221 → 652,670；extension-prefix 694,237 → 896,751。各自 Fullscan steps 为 0→79、sort ops 1→1；CTE 的 79 是扫描已 materialize 的 80 个候选行，不是全扫 500k entries。原始 SQL 与 CTE 均使用 temp B-tree 排序；temp B-tree 字节未测。counter 是最后一次 statement 状态，不是 30 次平均值。全量每 query / 每 run 的 p50/p95/p99、plans counter 派生列见 CSV。

## 6. Runner 与进程资源

日志中 OS/image/SQLite 相同，worker 与 region 不同。Hosted 日志没有 CPU 型号/core 数/频率、主机 RAM、磁盘类型/IO latency、host-wide CPU 或其他后台工作记录。不能从 runner image 名推定相同硬件或负载。

以下为 500k name-prefix profile 进程取样。CPU 是 30 个请求期间 process CPU time / wall time，按一个逻辑核归一；WS/private 是请求前后取样值最大值，不是系统 peak，也不按单条 SQL 归因。

| Run | SQL CPU O/C；WS/private MiB O/C | pooled CPU O/C；WS/private MiB O/C | Snapshot CPU O/C；WS/private MiB O/C |
|---|---|---|---|
| 38053163599 | 101.1/96.6%；265.8/19.2 与 265.8/19.2 | 100.0/99.5%；267.0/19.8 与 267.0/19.8 | 98.9/99.2%；262.7/19.8 与 262.7/19.8 |
| 38056013822 | 98.9/100.4%；266.3/20.2 与 266.3/20.2 | 99.6/99.8%；470.7/18.2 与 470.7/18.2 | 99.6/98.8%；470.7/18.2 与 470.7/18.2 |
| 38059974375 | 99.4/97.6%；264.4/20.7 与 264.4/20.7 | 99.4/100.0%；265.8/21.4 与 265.8/21.4 | 99.0/99.1%；259.2/21.4 与 259.2/21.4 |

run 2 pooled/Snapshot WS 约 471MiB，run 1/3 约 259–267MiB；private bytes 约 18–21MiB，而 SQL WS 三轮约 264–266MiB。run 2 同时取得 10/12 pooled Gate 分类。这个变化与更多 mapped/file-backed page residency 相符（mmap 约 2GiB，DB 约 625MiB），但现有计数不能证明具体是 SQLite mmap 页或其因果关系。三轮 hot query CPU 都约一个逻辑核饱和；没有 host telemetry 判断 CPU 争用、调度、频率或磁盘差异。

## 7. Benchmark harness 审计与十问结论

审计源码：query cost diagnostic、search path、repository snapshot、DB pool、fixture generator（路径分别为 `src-tauri/src/global_index/tests/global_search_query_cost_diagnostic.rs`、`src-tauri/src/global_index/search.rs`、`src-tauri/src/global_index/repository.rs`、`src-tauri/src/db/connection.rs`、`src-tauri/src/global_index/tests/global_search_benchmark.rs`）。

| 问题 | 结论 / 证据边界 |
|---|---|
| SQL 收益稳但 pooled 波动？ | 观测到 pooled 两 variant 在 run 1/3 有共同 +80–90ms 偏移，run 2 没有。变化位于 SQL-only 计时之外的完整路径或 runtime state；具体 pool / scheduler / cache 来源未证实。 |
| 同规模是否同 fixture / stats / cache？ | 行数、生成器、SQLite、PRAGMA、页数、DB/WAL size 一致；无 DB hash、schema hash 值、sqlite_stat1、WAL frame 或 cache 指纹，逐字节及统计/缓存状态未验证。 |
| Hosted CPU / memory / disk / background 差异？ | worker/region 与 population throughput 有差异；run 2 pooled/Snapshot WS 明显较大。硬件与 host load、磁盘指标未记录，不能确认根因。 |
| pool / prepare / thread / transaction？ | pooled 含 pool checkout 和全部搜索；Snapshot 另含 read transaction、source health、revision/hash、index status。SQL 每次 prepare。无 checkout/prepare/transaction 分段计时。 |
| 5 warmups / 30 samples 是否稳？ | quantile 线性插值；30 点 p95 依赖末尾约 2–3 点，p99 接近 max。代码用全量样本、不丢异常值；JSONL 只保留 quantiles/min/max，不保留 raw vector。p50 同时移动，尾部量不足不是唯一解释。 |
| order / cache / measurement window？ | 每个 30-sample pair Original-first / Candidate-first 交替；不是随机。固定 query 顺序，5 warmups 每轮 Original 在前。全为 warm-cache，无 OS/SQLite cache reset 或 cache hit/miss。 |
| Candidate B 是否与 pooled SQL 混算？ | 分层正确：pooled 不含 source health Candidate B；Snapshot 两侧都含同一版 #359 Candidate B。Snapshot 没有单独的 B 时间，不能把 total p95 additive 分解。 |
| build / overlay / rollback / checkpoint？ | 500k build 在正式采样前；hit overlay/adversarial rollback 在正式 12 query profile 后。代码无显式 checkpoint/ANALYZE/VACUUM；SQLite 默认 autocheckpoint 状态及 WAL frame/checkpoint history 未采集。 |
| Gate 如何保持可信？ | Gate 维持 per-query pooled warm p95 ≤100ms。预先固定 source / fixture / run 有效条件，保留每轮原值；有效 run 之间出现任何 PASS/FAIL 分歧即 STOP，不平均、不挑最快、不丢弃慢 run。 |
| 现证据能否支持 Prefix Tier 生产化？ | name/punctuation/Chinese/Unicode prefix raw SQL 收益稳定，值得后续验证；最终 run CTE 仅 4/12，五类 prefix 从 run 2 PASS 回落到 FAIL。不能开始 #364。extension-prefix 每轮 raw SQL 回退。 |

时间边界：SQL tier 包含 prepare、完整结果 drain/map，不含 pool/其它 tier/Snapshot。pooled search 包含 pool checkout、所有 production tiers、去重与 page extraction，不含 Snapshot。Snapshot 包含 checkout、read transaction setup/commit、完整搜索、Candidate B source-health、revision/hash 和 index status。Candidate B 没有在三轮中单独计时，Snapshot p95 不能简单相减归因。

Pool 配置为 max_size 8、min_idle 1；测试单线程顺序执行，但没有记录每次借到的连接 identity 或 pool wait。两个 variant 共享 SQLite/OS cache；读写阶段不隔离。SQL 每次新 prepare，没有复用 statement cache。候选顺序每对轮换可减轻 order bias，但固定 query matrix 和 warmup 顺序仍可能改变后续 cache state。

## 8. 已验证 / 未验证与后续计划

### 已验证

- 三份 raw artifact 和 decoded workflow logs 已下载检查；ZIP digest 与 API 和本地 SHA 匹配。
- 代码、fixture generator、pool、search / pooled / Snapshot 边界、PRAGMA 和 workflow 已静态审计。
- 三轮 JSONL 中 12 类 correctness 均通过，Original/CTE plan 与 statement counters 一致；Gate 矩阵和完整 126 条 profile 指标由 JSONL 派生。
- 新 500k/1m benchmark、Codex Review 和生产修改均未执行。

### NOT VERIFIED

- Hosted CPU 型号、可用 core/clock、物理 RAM、host 争用、并发后台负载和 disk IOPS/latency。
- run 2 WS 中 SQLite mmap 页占比、逐连接 page-cache hit/miss、OS file-cache residency 与 page faults。
- DB 文件字节 hash、跨轮 schema/index signature 值、sqlite_stat1、WAL frame/checkpoint state、checkpoint 后大小。
- pool checkout、prepare、transaction 子耗时；Snapshot 中 Candidate B 单独耗时；temp B-tree 字节。
- 逐样本 latency 向量和执行顺序、真实用户语料/Owner native Windows/macOS 的 500k 性能、受控条件下稳定 Gate PASS。

### Owner 批准后的最小验证计划（本轮未执行）

建议 3 个串行 Windows Hosted 500k-only job，固定同一 diagnostic source SHA；每个 job 只生成一份 500k fixture，记录 logical fixture / schema / sqlite_stat1 / DB / WAL fingerprint。不先跑 100k，减少本 job 的额外数据库工作。已有混合规模 job 约 37–49 分钟，因此先按最多约 2.5 小时串行容量预留。若 fixture fingerprint 不同，先停止解释差异；若相同 fixture 仍跨 job 波动，再评估复用同一个 DB 文件来隔离布局因素。

| 假设 | 最小可证伪测量 |
|---|---|
| fixture / stats 不同 | 记录 generator SHA、sqlite_master、sqlite_stat1、逻辑 fingerprint、page/freelist、DB/WAL、plan；相同 fingerprint/counters 会削弱此假设。 |
| mmap/page residency 或磁盘造成共同偏移 | 在 query window 同步记录 process WS/private、hard faults、cache hit/miss、系统可用内存、disk latency/queue；状态相同但 pooled 仍波动会削弱此假设。 |
| runner scheduling / host load | 记录 worker/region、CPU 型号/core count、process CPU/wall、system CPU 和 processor queue；三 job 串行。 |
| pool/prepare/transaction 成本 | 逐次记录 checkout、各 tier prepare/execute/map、transaction open/commit；继续保留 pooled 与 Snapshot 总指标。 |
| 30 点尾部噪声 | 原 5 warmup + 30 sample 是唯一 Gate；另外保存所有 raw sample，并可做 100-sample 诊断分布，但不能替代 Gate 或合并成 PASS。 |

冷缓存无法用当前 Hosted harness 可靠清空；资格结果仍为 warm-cache。若 Owner 要 cold-start，应另用新进程 / 新 runner 标记，不能与 100ms warm Gate 混算。

**预先停止条件：**3 个有效 job 中任一 query 的 PASS/FAIL 不一致，判定重复性未建立并 STOP；任一 run 未达原 100ms p95 或 correctness 失败，Candidate C 不具备生产资格。缺 artifact、fixture fingerprint/image 不一致或 telemetry 不足的 run 仍保留、标记未资格化；追加 job 需 Owner 再批准。不得重跑到绿、挑最快或丢弃慢 run。

## 9. Owner Go / No-Go

本轮没有改诊断代码；报告建议在未来获批验证中增加 raw sample、fixture/schema/stats/WAL fingerprints、host telemetry 和 checkout/prepare/transaction 分段计时，Gate 与阈值保持不变。

**建议：NO-GO for #364 implementation。** 第三轮最终 #362 HEAD Candidate C 为 4/12，尚无稳定 100ms PASS。先由 Owner 审阅报告与保留证据；仅在 3 个有效独立 run 对必需查询类别都满足现有 p95 ≤100ms，且 fixture/runner 信息足以审计后，再讨论仅应用到有证据支持的 name-prefix tiers。extension-prefix 当前为回退。

> **#363 BENCHMARK VARIANCE INVESTIGATION COMPLETE — ROOT CAUSE NOT VERIFIED**
