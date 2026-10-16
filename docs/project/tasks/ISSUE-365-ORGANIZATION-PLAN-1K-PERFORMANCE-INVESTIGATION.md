# Issue #365 — Organization Plan 1k 性能调查与修复

状态：修复代码 exact-head Hosted CI 已成功；报告已提交，文档检查通过。

## 1. 版本与范围

- 仓库：`ArdenZC/Zen-Canvas`
- 起始 master：`58062c5c356969f332f19c7458028bf2e097595e`
- 起始 tree：`32ce51b988cefb962efd2202da84da8b7215c338`
- 历史失败运行代码：`16b9c791acf9e2e907174ce21863025d079e5351`
- 诊断基线提交：`42c3c53f787965a050db680c9685038c613aef1e`
- 修复代码提交：`8f80a280c5faf9920b968c91561cb6200801aa1d`
- 最终修复代码 HEAD（exact-head CI）：`8f80a280c5faf9920b968c91561cb6200801aa1d`。
- 分支：`fix/issue-365-organization-plan-performance`
- Draft PR：[#369](https://github.com/ArdenZC/Zen-Canvas/pull/369)，关联 Issue #365；Issue 保持 OPEN，PR 保持 OPEN / Draft。

历史失败提交是起始 master 的祖先。对失败提交到起始 master 的 `src-tauri/src/db/queries/organization/mod.rs` 与 `src-tauri/src/db/queries/library/mod.rs` 比较无差异，因此下述执行路径适用于失败代码。

## 2. Owner 合同与历史证据

Issue #365 及其评论要求保留失败证据，不删除测试、不降低预算、不跳过性能门槛；限定最多两次定向 Windows Hosted 性能验证；不把并行 Issue #366 的 macOS managed-scan 问题归因到 #365。Owner 授权在证实问题后于本任务范围内直接做最小修复。未改 CI 路由、性能预算或共享 Schema。

| 运行 | 证据 |
|---|---|
| [38050482258](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38050482258)，Windows Job `114211210304` | `performance_task06_plan_100_1k_10k_repository` 在 `1k execution preparation Some(1193.0789)ms` 失败，预算为 1000ms。日志留下 100 项记录及 1k 断言失败，但没有 1k 分段数据。失败向 Performance Profile 与 Windows/macOS Quality 聚合传播；独立 Rust quality jobs 成功。 |
| [38064775222](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38064775222)，Windows Intelligence Job `114251531810` | 1k execution preparation `948.8979ms`，dry-run `462.4446ms`，测试及 Windows Intelligence、Performance Profile、Windows Quality 通过。整体 CI 的独立失败是 #366 的 macOS managed-scan。 |
| [38068879734](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38068879734)，Windows Intelligence Job `114264865993` | 1k execution preparation `876.8509ms`，dry-run `414.5294ms`，测试及该运行的质量门槛通过。 |

上面三次运行使用同一 Windows Server 2025 镜像版本，但运行区域不同。它们证明结果跨运行有较大差异，但仅凭历史日志不能量化每次运行的阶段成本或把全部差异归于调度器。

## 3. 原执行准备路径与计时边界

性能用例构造 100、1000、10000 个计划条目，并创建索引文件、Managed AI 当前状态、已完成任务和权威 Operation Preview。100 和 1000 项运行 dry-run；仅 1000 项调用执行准备；10000 项保持 `kept` 并测量 ledger 查询和 refresh，不运行 dry-run 或执行准备。

`execution_ms` 使用 `Instant` 包围 `Database::begin_organization_plan_execution`。原始路径为：

1. 校验确认标志。
2. 在独立连接/事务中完整执行 `get_organization_plan_dry_run`，计算并比较请求指纹。
3. 打开另一个连接和事务，检查计划 revision/status。
4. 再次完整构建 live dry-run 并比较请求指纹。
5. 选择可执行项；对每项读取快照并构造 `OperationPreviewRequest`。
6. 在同一事务内逐项将 item 标为 `executing`，更新 plan，再提交。

计时包含两个完整 dry-run、连接/事务操作、指纹比较、逐项 snapshot SELECT 和 claim UPDATE、plan UPDATE 与 commit；不包含 fixture 创建、独立 dry-run、文件操作实际执行、执行失败后的恢复操作。它测量的是 wall time；分段 profile 额外取进程累计 CPU 时间和内存，不依赖超时宽限。

每次 dry-run 包含计划状态查询、source query 加载、selected item 查询、当前文件分块加载、管理范围成员资格验证、当前语义 proposal、目标与父目录文件系统检查，以及指纹和结果构造。原实现对每个计划项调用 `file_matches_authoritative_query_scope`，每次重复 canonicalization 和同一 root/filter 权限解析。

## 4. 分段性能证据

修复前的诊断基线是 PR #369 首次 Hosted Windows 运行：[38074170047](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38074170047)，Windows Performance / Intelligence Job `114279539929`，测试通过。以下为该轮 1k profile；阶段百分位来自每次阶段调用的样本，完整 dry-run / execution 顶层只有 1 或 2 个样本，不能解释为跨运行 p95/p99。

| 阶段 | 样本数 | p50 ms | p95 ms | p99 ms |
|---|---:|---:|---:|---:|
| standalone `dry_run.current_proposal_query` | 1000 | 0.132 | 0.167 | 0.191 |
| standalone `dry_run.scope_membership_query` | 1000 | 0.039 | 0.050 | 0.078 |
| standalone `dry_run.current_files_batch_load` | 1 | 3.255 | 3.255 | 3.255 |
| standalone `dry_run.selected_items_query` | 1 | 2.645 | 2.645 | 2.645 |
| standalone target collision check | 1000 | 0.006 | 0.007 | 0.009 |
| standalone parent directory check | 1000 | 0.008 | 0.009 | 0.013 |
| standalone fingerprint/result build | 1000 | 0.004 | 0.005 | 0.008 |
| standalone dry-run wall / process CPU | 1 | 207.369 / 203.125 | — | — |
| execution preflight dry-run | 1 | 204.571 | 204.571 | 204.571 |
| execution live dry-run | 1 | 208.859 | 208.859 | 208.859 |
| execution item snapshot SELECT | 1000 | 0.003 | 0.006 | 0.009 |
| execution item claim UPDATE | 1000 | 0.008 | 0.010 | 0.015 |
| execution total wall / process CPU | 1 | 428.711 / 421.875 | — | — |

Execution profile 中 dry-run 阶段合并记录 2 次构建：范围查询 2000 个样本，p50/p95/p99 为 `0.039/0.052/0.071ms`；语义 proposal 2000 个样本为 `0.132/0.166/0.198ms`。item snapshot SELECT 与 claim UPDATE 合计约 11ms 中位数；计划 guard、ID 收集、selection build 和 plan claim 远小于 dry-run 成本。`execution.total` 顶层为 428.694ms，外层 wall 为 428.711ms。

完整阶段明细如下。单次/双次请求阶段的百分位来自 n=1/n=2，仅用于定位成本；按条目调用的阶段为对应调用延迟分布。

| 上下文阶段 | n | p50 ms | p95 ms | p99 ms |
|---|---:|---:|---:|---:|
| standalone plan ID validation | 1 | 0.001 | 0.001 | 0.001 |
| standalone revision/status query | 1 | 0.054 | 0.054 | 0.054 |
| standalone source query load | 1 | 0.008 | 0.008 | 0.008 |
| standalone selected items query | 1 | 2.645 | 2.645 | 2.645 |
| standalone current files batch load | 1 | 3.255 | 3.255 | 3.255 |
| standalone scope membership query | 1000 | 0.039 | 0.050 | 0.078 |
| standalone current proposal query | 1000 | 0.132 | 0.167 | 0.191 |
| standalone target collision filesystem check | 1000 | 0.006 | 0.007 | 0.009 |
| standalone parent directory filesystem check | 1000 | 0.008 | 0.009 | 0.013 |
| standalone fingerprint/result build | 1000 | 0.004 | 0.005 | 0.008 |
| execution dry-run plan ID validation (2 builds) | 2 | 0.001 | 0.001 | 0.001 |
| execution dry-run revision/status query (2 builds) | 2 | 0.011 | 0.017 | 0.018 |
| execution dry-run source query load (2 builds) | 2 | 0.005 | 0.005 | 0.005 |
| execution dry-run selected items query (2 builds) | 2 | 2.197 | 2.260 | 2.265 |
| execution dry-run current files batch load (2 builds) | 2 | 2.986 | 3.021 | 3.025 |
| execution dry-run scope membership query (2 builds) | 2000 | 0.039 | 0.052 | 0.071 |
| execution dry-run current proposal query (2 builds) | 2000 | 0.132 | 0.166 | 0.198 |
| execution dry-run target collision filesystem check (2 builds) | 2000 | 0.006 | 0.006 | 0.010 |
| execution dry-run parent directory filesystem check (2 builds) | 2000 | 0.008 | 0.010 | 0.014 |
| execution dry-run fingerprint/result build (2 builds) | 2000 | 0.004 | 0.005 | 0.009 |
| execution connection checkout | 1 | 0.005 | 0.005 | 0.005 |
| execution transaction begin | 1 | 0.002 | 0.002 | 0.002 |
| execution revision guard | 1 | 0.053 | 0.053 | 0.053 |
| execution select executable items | 1 | 0.008 | 0.008 | 0.008 |
| execution collect item IDs | 1 | 0.130 | 0.130 | 0.130 |
| execution item snapshot SELECT | 1000 | 0.003 | 0.006 | 0.009 |
| execution selection construction | 1000 | 0.001 | 0.001 | 0.001 |
| execution item claim UPDATE | 1000 | 0.008 | 0.010 | 0.015 |
| execution plan claim UPDATE | 1 | 0.031 | 0.031 | 0.031 |
| execution transaction commit | 1 | 1.220 | 1.220 | 1.220 |

100 项 standalone dry-run 的对照：scope membership（n=100）`0.020/0.040/0.049ms`，proposal 查询（n=100）`0.067/0.102/0.146ms`，target 检查 `0.004/0.005/0.007ms`，parent 检查 `0.003/0.005/0.005ms`，fingerprint/result `0.003/0.004/0.005ms`；整次 dry-run wall 为 11.400ms。10000 项没有 dry-run/execution 阶段样本，因为该 fixture 分支只测 ledger 和 refresh。

该轮进程 CPU `421.875ms / 428.711ms wall = 98.4% 单核比例`，说明这次测量主要受测试进程 CPU 工作影响。此比例不是宿主机 CPU 使用率，也不排除其他运行的调度影响。

## 5. 100 / 1k / 10k 对比

同一修复前 Windows Hosted 运行的测试输出：

| 条目数 | create ms | first page ms | deep keyset ms | decision ms | dry-run ms | execution prep ms | refresh ms |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 100 | 4.553 | 12.385 | 0.000 | 9.925 | 11.400 | 不运行 | 16.161 |
| 1,000 | 42.063 | 12.670 | 105.746 | 101.354 | 207.369 | 428.711 | 不运行 |
| 10,000 | 128.734 | 7.264 | 471.109 | 206.214 | 不运行 | 不运行 | 407.660 |

1000 项 dry-run 为 100 项约 18.2 倍，工作量增加 10 倍；其中 proposal 和逐项 scope membership 为线性成本。原 execution prep 为同轮单次 dry-run 的约 2.07 倍，与源码中的双重完整重建吻合。10000 项用例只测 ledger/query/refresh，因此不能据此声称 10k 执行准备通过。

## 6. SQL、索引与复杂度

该 benchmark fixture 的 SQL 次数按实际源码路径和语义 fixture 作 source-derived 估算，不是 SQLite 全局 statement counter：

- 原单次 dry-run：3 个 plan/source/selected 查询 + 2 个当前文件批次 + 1000 × 3 个 scope 查询 + 1000 × 6 个语义 proposal 查询 = 9005 条 SQL。
- 原 execution prep：2 次 dry-run + revision guard + 1000 个 item snapshot SELECT + 1000 个 item claim UPDATE + plan claim UPDATE = 20012 条 SQL。
- 原范围验证中每项重复查 enabled roots、root path、file membership；其余语义 proposal 仍为每项 6 条 SQL。
- dry-run 每次还有每项 target/parent 各一次文件系统 `exists` 检查；原 execution prep 因完整构建两次而做约 4000 次检查。

修复后，一次 execution dry-run 使用 2 个最多 500 ID 的 scope membership 批次：单次 dry-run 估算 6011 条 SQL，execution prep 估算 8013 条 SQL，较原估算减少约 60%。逐项语义 proposal 查询和 item claim 仍保留，因为它们提供当前状态校验与事务内 claim。

修复前 EXPLAIN QUERY PLAN：

- selected plan items：`idx/autoindex organization_plan_items_3 (plan_id=?)`。
- file scope membership：`sqlite_autoindex_files_1 (id=?)`。
- global identity：`idx_global_entries_path_normalized (path_normalized=?)`，volume primary key；同时出现 `USE TEMP B-TREE FOR ORDER BY`。
- item snapshot SELECT 与 item claim UPDATE：Organization Plan item primary-key autoindex。

因此这次失败并非单个计划项 SQL 全表扫描。scope SQL 有可索引访问，但按条目反复解析 root authority 是不必要的查询放大；semantic proposal 仍按条目执行，是修复后剩余的线性主成本。

## 7. 环境与测量局限

修复前诊断轮：Windows Server 2025 Hosted runner，runner `2.337.0`，image release `win25-vs2026/20260925.250`，Azure region `centralus`。进程 working set 从 18.1 MiB 到 19.9 MiB，private bytes 从 8.8 MiB 到 10.4 MiB。SQLite `3.51.3`，WAL，synchronous=1，temp_store=2，mmap_size=2147418112，page_size=4096，page_count=1967；数据库主文件 7,991,296 bytes，WAL 5,751,552 bytes，SHM 32,768 bytes。

GitHub Actions 日志未提供 runner CPU 型号/频率/核数、宿主机 RAM、磁盘 I/O 延迟、宿主机 load 或 background workload。历史运行的 region 为 northcentralus/eastus/westcentralus，诊断轮是 centralus；同一镜像不代表相同 CPU 性能或宿主机负载。现有数据可以证明代码执行了 CPU 密集的重复工作，但无法将历史单次 1193ms 的超预算部分按宿主机调度、SQLite cache/I/O 或 CPU 型号精确拆分。

修复后 exact-head 运行同为 Windows Server 2025、runner `2.337.0` 与 `win25-vs2026/20260925.250` image，Azure region 为 `westus`。进程 working set 从 18.1 MiB 到 20.1 MiB，private bytes 从 8.8 MiB 到 10.7 MiB。SQLite `3.51.3`，WAL，synchronous=1，temp_store=2，mmap_size=2147418112，page_size=4096，page_count=1964；主文件 7,974,912 bytes，WAL 5,735,072 bytes，SHM 32,768 bytes。进程 CPU 为 390.625ms / 400.303ms wall（97.6% 单核比例）；Actions 日志仍没有宿主机 CPU/内存/磁盘负载数据。

## 8. 根因结论

已验证的产品代码问题是 execution prep 重复执行相同完整 dry-run：修复前诊断轮中两次分别耗时 204.571ms 和 208.859ms，合计约占总执行准备的 96%。该检查之外还有一处重复解析：每个条目重新验证相同的管理范围；测试路径下 1000 个 membership 调用 p50 为 39µs，累计约 39ms/次 dry-run。它们均为确定的额外工作，与后续通过运行中 execution prep 约为 standalone dry-run 两倍相符。

修复前诊断轮执行 prep 为 428.711ms，但历史同版本代码的不同运行从 876.851ms、948.898ms 到失败的 1193.079ms。后续成功运行不能否定该失败；单次 428.711ms 也不能证明旧失败全部由 CI 噪声造成。具体 runner CPU/负载与历史缓存状态未被 Actions 日志记录，仍无法判定 1193ms 超预算中精确的硬件/调度占比。可信的代码改进依据已经成立，所以在此处直接修复，不以扩大阈值或只重跑取绿代替修复。

## 9. 最小修复与语义约束

只修改 `src-tauri/src/db/queries/organization/mod.rs`：

1. 删除事务外 preflight dry-run。仍在执行 claim 事务内重建 live dry-run，并将其 fingerprint 与用户提交的 fingerprint 比较；任何不匹配仍返回 `organization_dry_run_expired`。
2. dry-run 将 scope membership 校验改为每 500 个 ID 调用已有的 `files_matching_authoritative_query_scope`。每批单独记录成功/错误，再映射回各 file ID，保持 `managed_scope_membership_changed` / `managed_scope_unavailable` 的原有结果分类。
3. 新增错误 fingerprint 回归断言，证明事务内重验证仍拒绝过期 dry-run；现有执行测试继续验证有效请求构建相同目标并成功 claim。
4. profile 中执行阶段 dry-run 次数、scope 批次数、SQL估算和 500-ID EXPLAIN 均按新路径更新。性能预算与 CI 路由未改。

结果顺序不变：live dry-run 的 selected items 遍历和 fingerprint 顺序未变；HashMap 只查 scope membership，不用于输出排序。执行 claim 仍在一个事务内完成，失败不会部分提交。计划 revision/status、item validity/decision、Operation Preview 和 Managed AI authority 校验均保留。

## 10. 修复前后性能

| 指标 | 修复前诊断运行 | 修复后最终 Windows Hosted 运行 |
|---|---:|---:|
| 1k standalone dry-run | 207.369ms | 364.290ms |
| 1k execution prep | 428.711ms | 400.303ms |
| source-derived execution SQL 数 | 20012 | 8013 |
| 1k test | PASS | PASS |

修复后分段（standalone dry-run 与执行内部 live dry-run 均为单次构建）：

| 修复后阶段 | n | p50 ms | p95 ms | p99 ms |
|---|---:|---:|---:|---:|
| dry-run plan ID validation | 1 | 0.001 | 0.001 | 0.001 |
| revision/status query | 1 | 0.043 | 0.043 | 0.043 |
| source query load | 1 | 0.012 | 0.012 | 0.012 |
| selected items query | 1 | 3.489 | 3.489 | 3.489 |
| current files batch load | 1 | 4.496 | 4.496 | 4.496 |
| scope membership, 500 IDs/batch | 2 | 1.209 | 1.282 | 1.288 |
| current proposal query | 1000 | 0.292 | 0.373 | 0.448 |
| target collision filesystem check | 1000 | 0.013 | 0.018 | 0.031 |
| parent directory filesystem check | 1000 | 0.019 | 0.027 | 0.044 |
| fingerprint/result build | 1000 | 0.006 | 0.009 | 0.013 |
| execution connection checkout | 1 | 0.010 | 0.010 | 0.010 |
| execution transaction begin | 1 | 0.002 | 0.002 | 0.002 |
| execution revision guard | 1 | 0.026 | 0.026 | 0.026 |
| execution live dry-run | 1 | 363.992 | 363.992 | 363.992 |
| select executable items | 1 | 0.014 | 0.014 | 0.014 |
| collect item IDs | 1 | 0.127 | 0.127 | 0.127 |
| item snapshot SELECT | 1000 | 0.008 | 0.019 | 0.035 |
| selection construction | 1000 | 0.001 | 0.001 | 0.004 |
| item claim UPDATE | 1000 | 0.019 | 0.033 | 0.049 |
| plan claim UPDATE | 1 | 0.041 | 0.041 | 0.041 |
| transaction commit | 1 | 1.618 | 1.618 | 1.618 |
| execution total wall / process CPU | 1 | 400.303 / 390.625 | — | — |

修复前不同运行观察到的范围为 876.851–1193.079ms；该历史失败保留在报告中，不用修复后通过结果覆盖。

修复后 100 项 dry-run 为 23.760ms；10k refresh 为 857.764ms，10k 仍不运行 dry-run/execution。修复后 1k profile 的 live dry-run 为 363.992ms，item snapshot SELECT p50/p95/p99 为 `0.008/0.019/0.035ms`，claim UPDATE 为 `0.019/0.033/0.049ms`。dry-run 的 per-item proposal 为 `0.295/0.367/0.416ms`，parent filesystem check 为 `0.019/0.026/0.046ms`。本次 runner 的这些逐项阶段约为修复前诊断轮的两倍，因此 428.711→400.303ms 不是可控同机 A/B；该轮 400.303ms 是一次明确的修复后 Hosted 观察，不能据此声称精确的百分比加速。

修复前 execution prep 中，仅 preflight 与 live 两个 dry-run 已占 413.430ms；修复后仅保留一个 363.992ms live dry-run，运行 wall 为 400.303ms。结合 runner 单项耗时变化和两次独立历史通过数据，证据确认删除了实际高成本重复工作，且新的 execution prep 在另一相对较慢的 Hosted worker 上仍低于预算约 600ms。单次修复后运行不足以估计跨 Hosted worker 的新 p95/p99；稳定性结论来自代码成本移除、两轮受控 profile、历史运行对比和预算余量，不是多个修复后重复抽样的统计承诺。

修复后每次 dry-run 的 scope membership 仅有 2 个批次样本，p50/p95/p99 为 `1.209/1.282/1.288ms`；另一次执行内部 dry-run 为 `1.239/1.287/1.292ms`。修复后 EXPLAIN 对 500 ID scope query 选择 `idx_files_is_stale (is_stale=?)`，而单 ID 查询选择 file primary-key autoindex。该批处理路径已被 Organization Plan projection 使用，fixture 上每个批次约 1.2ms，但真实全局文件表更大时该索引选择可能有不同成本；没有运行被禁止的 500k/1m benchmark。保留该观察供 Owner 决定是否另开大库计划专项，当前 1k 修复未改共享 Library SQL 或索引。

## 11. 精确 HEAD CI 与独立失败

- 诊断基线 Windows Intelligence / Performance Profile：运行 [38074170047](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38074170047)，目标 Organization Plan 100/1k/10k 测试、Performance Profile、macOS Rust Quality 与两平台 Release compile 通过。Windows Rust Quality 后续被 PR 新提交取消，aggregate Windows Quality job 因该 cancelled job 失败；它不是测试失败，修复 HEAD 的第二次 PR CI 会重新验证。
- 修复代码 HEAD：`8f80a280c5faf9920b968c91561cb6200801aa1d`。
- 修复代码 exact-head PR CI：[38075396451](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38075396451)，commit `8f80a280c5faf9920b968c91561cb6200801aa1d`，整体 **SUCCESS**。Windows Performance / Intelligence Job `114282628581`、Performance Profile Job `114282930077`、Windows Quality Job `114283907505`、macOS Quality Job `114285461749`、两平台 Rust Quality、两平台 Release compile、source/evidence 与 routing/governance gates 均成功。Organization Plan 100/1k/10k 通过，1k execution prep `400.3032ms`。本轮没有其他独立 CI 失败，也没有 #366 失败。
- 诊断基线 Windows Intelligence、Performance Profile、macOS Rust Quality 和两平台 Release compile 通过；Windows Rust Quality 因后续推送而 cancelled，使基线 run 的聚合 Windows Quality 失败。此取消不是测试失败，修复 exact-head 上 Windows Quality 已通过。
- 报告提交只改变文档；性能证据对应上面的 exact source HEAD。报告 HEAD `d8a505fa669a67318c4f9315bad46f79f35b3487` 的 PR 运行 [38076945618](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38076945618) 被取消；该 PR-wide workflow 将重新排入性能 lane，但在 Performance / Intelligence 测试启动前取消。此次没有第三次定向 Windows 性能测试；报告 Markdown 的本地 documentation/governance validation 通过。PR 的报告提交与修复代码 HEAD 分开记录，避免把 report-only 后继误称为性能测试 SHA。

## 12. 本地验证与 Owner 建议

- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过。
- `git diff --check`：通过。
- `DOCS_DIFF_BASE=origin/master DOCS_DIFF_HEAD=$(git write-tree) npm run test:docs`：通过，覆盖 1 个报告 Markdown 和项目治理检查。
- 本地 targeted cargo test 曾在项目编译前被系统缺少 `glib-2.0` development package 阻断（Tauri GTK native dependency）；此项是 Cloud workspace 环境依赖缺失，不是测试断言失败。Windows Hosted PR CI 负责验证该 Rust 改动。
- Owner 建议：审阅本报告记录的 execution prep wall time 与 source-derived SQL 数变化；确认同一事务内 live fingerprint 校验满足执行准备合同。PR 保持 Draft，Issue 保持 OPEN。
