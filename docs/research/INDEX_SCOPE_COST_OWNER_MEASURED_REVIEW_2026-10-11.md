# ISG-00 Owner 实测复核：合成索引成本与生产决策门槛

> 日期：2026-10-11
> Owner 研究判断：**合成基线有效，生产收益与默认策略尚未证明**
> 代码证据：[#379](https://github.com/ArdenZC/Zen-Canvas/pull/379) test-only Benchmark，实验 HEAD `d6eb49d3b16f72fb07e953b6996645c85433c558`，Tree `d0fbff2d07309a42f935e7e0f011a3a8a90e81d8`
> 该文档由 ChatGPT/Owner 对 **两份原始 JSONL** 和 PR 源码独立复核；代码开发职责仍归 Codex Cloud。

## 一、证据及审查范围

- Windows raw：[Actions artifact 11694073240](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38116630000/artifacts/11694073240)；macOS raw：[Actions artifact 11694605755](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38116630000/artifacts/11694605755)。
- [Benchmark run 38116630000](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38116630000) 和 [full CI 38116630002](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38116630002) 均为实验 HEAD 的 SUCCESS。首轮失败与中间取消不当作最终失败，也不删历史。
- 每个平台各有 1k smoke（四组）及 100k 全量（每组三次，共 12 个 ingestion_sample），每个 100k 样本以独立临时 SQLite 数据库运行当前 schema / `upsert_global_entries_batch` 与搜索查询；三次重复采用轮换组顺序。两个平台均有全部采样记录、分组摘要、配对比较和 limitation 记录。
- 这是**固定种子 / 固定权重的合成工作负载**，无 Windows NTFS MFT/USN、Spotlight/FSEvents、真实 stat、存量数据库或用户真实路径读取。样本工作目录的 Windows 风格路径属于测试数据，包括在 macOS Hosted 上也是如此。

## 二、固定 100k 合成分布

| 类别 | 数量 | 合成判断 |
| --- | ---: | --- |
| 个人文档/影音 | 30,000 | Include |
| 项目 README/docs/src/manifests | 15,000 | Include |
| Windows/build/AppData/Program Files2 等高价值同名用户路径 | 10,000 | Include |
| 依赖子树 | 15,000 | Exclude（Dependency） |
| 生成构建目录 | 10,000 | Exclude（部分 target 路径归入 Dependency） |
| 短期缓存 | 10,000 | Exclude |
| 系统内部身份 | 5,000 | Exclude |
| 未确定 Unicode 路径 | 3,000 | Undetermined，暂准入 |
| 项目最小祖先 | 2,000 | Minimal Ancestor，准入 |

分类结果：Include 55,000 + Minimal Ancestor 2,000 + Undetermined 3,000 + Exclude 40,000 = 100,000。排除原因分别为 Dependency 18,333、Cache 10,000、Generated Build 6,667、Protected System 5,000。

**40% 被排除是合成输入本身设定的类别/证据分布，不代表真实 Zen 用户磁盘低价值比例，也不等于零误排的外部证据。** 代码中的期望分类 oracle 来自预设 `CorpusClass`，并非人工标注的真实路径集合；测试验证了实验分类器与 fixture 的内部一致性。

## 三、原始数据重新计算

| 组 | 入库行 | 合成 metadata accessor 次数 | 主 DB＋WAL 字节 | FTS shadow 页字节 |
| --- | ---: | ---: | ---: | ---: |
| A：广覆盖 | 100,000 | 100,000 | 201,270,912 | 51,064,832 |
| B：模拟分类但全部入库 | 100,000 | 100,000 | 201,270,912 | 51,064,832 |
| C：模拟 sink 前过滤 | 60,000 | 100,000 | 117,908,200 | 28,819,456 |
| D：模拟 metadata 前过滤 | 60,000 | 60,000 | 117,908,200 | 28,819,456 |

对固定合成样本，C/D 相比 A 减少 **40% 数据记录**、**41.42% SQLite 主文件＋WAL 体积**、**43.56% FTS shadow 页占用**。FTS shadow 页**已经包含在 SQLite 主文件中，不能和主 DB＋WAL 的下降比例重复相加**。WAL 为采样时点体积，不代表长期稳定落盘大小。

| 平台（Hosted） | A 平均±样本标准差 | B | C | D |
| --- | ---: | ---: | ---: | ---: |
| macOS ARM64，秒 | 38.867 ± 3.159 | 50.859 ± 7.948 | 21.338 ± 2.009 | 21.324 ± 0.730 |
| Windows x64，秒 | 267.012 ± 97.546 | 217.058 ± 25.261 | 92.977 ± 5.037 | 113.293 ± 29.220 |

样本数每平台每组 **n=3**。Windows A 的 CV 为 **36.53%**，D 为 **25.79%**；强波动与 Hosted runner 噪声使倍速外推不可信。macOS C/D 时间近似相同，Windows D 均值反而比 C 慢约 **21.85%**，不能据此宣称前置过滤比 sink 过滤快。D 的 40,000 次省略只是**内存字段克隆调用次数**，不是 OS stat、MFT staging 或 Spotlight gather 减少。

两平台 12/12 样本的 exact identity set 与 11 个代表性搜索用例 PASS；FTS fixture sentinel 与入库数一致，AI jobs/items 为 0。11 个用例不等价于全业务查询正确性，尤其不能证明真实路径误排率为 0、跨代源状态正确、FK 历史保存或迁移回滚安全。现有 12-query Global Search P95 门槛未被调整。

## 四、Owner 结论和 Go/No-Go

**GO：**保留现有 schema、真实 repository 写入链路上的 **test-only** benchmark 作为可重复的实验平台；它证实“减少待写入行数可减少这些固定合成负载的 SQLite/FTS 存储开销”，也证明分类判断本身而不阻止写入（B）没有相同的存储收益。

**MERGE BLOCKER（仅 CI 路由成本）：** PR #379 的 `global-search-benchmark.yml` 在匹配 `pull_request.paths` 的**每个相关 PR**上自动加跑 Windows 和 macOS **100k×四组×三次**实验，且 `cancel-in-progress:false`，会导致后续索引代码 PR 重复承担高成本，违背零负担开发与性能 CI 预算目标。要求 Codex Cloud 在同一 PR 将日常触发限制为 smoke/有界小规模检验；完整 100k 双平台和三轮须保留为**显式 workflow_dispatch/专项基准验证**，确保来源 SHA/tree、历史 P95 门槛和产物格式不变。改动需有 actionlint、exact-HEAD 工作流验证与结果记录，再由 Owner 复核。

**NO-GO：**基于上述合成数据直接将 Smart Lean 设为新/老用户默认、删除旧 `global_entries`、批量 VACUUM、忽略用户选择的开发目录、声称 MFT/Spotlight CPU/RSS/电量收益，或启动 AI 自动分析及迁移。ADR-0014 当前仍是 **PROPOSED**，非正式授权。

**下一门槛：**在 #371 Owner ADR 语义获批后，先进行纯策略正确性与例外覆盖测试；如需真实环境计数，再由 Codex 实现受控、隐私安全、显式开启的原生阶段计数并在 Owner Windows/macOS 验收环境取证；Windows 原生机器不得承担开发任务。所有文件排除都只影响可检索覆盖和资源使用，绝不代表文件可删除或已经获得内容/操作权限。

## 五、最终定位

`ISG-00 SYNTHETIC MEASUREMENTS VERIFIED — OWNER CONDITIONAL CODE REVIEW PASS — DEFAULT FILTERING NOT APPROVED`。

此文是 Owner 的研究/数据解释成果，#379 的代码与工作流修正由 Codex Cloud 负责。#370 保留开放直至验收与合并完成；#374 可在 test-only harness 合并并经 post-merge CI 后收口。