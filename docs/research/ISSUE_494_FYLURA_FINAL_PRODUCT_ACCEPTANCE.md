# Fylura Global Search 最终产品验收报告

记录日期：2026-10-31（Asia/Shanghai）
验收范围：Issue [#494](https://github.com/ArdenZC/Fylura/issues/494)，现有 [Draft PR #518](https://github.com/ArdenZC/Fylura/pull/518)

> 本报告区分候选代码的原生 Rust harness、普通 CI 与真实桌面安装版。任一 harness 或编译通过都不代表安装版 GUI 已验收。

## 结论

**最终状态：`POINT_PASS_INCONCLUSIVE`。** Windows 512 行的生产 WAL total p95 点估计回退 +2.222%（低于 10%），但配对 bootstrap 95% 区间上界 +14.752% 超过门槛；自动 checkpoint 已在两 arm 实际观测到。按 Owner 规则，这一格不能作为产品通过证据，本轮不再追加性能运行。真实安装版 GUI、文件生命周期、撤权/重授权及完整进程树资源验收也仍 `ENVIRONMENT_UNVERIFIED`，因此不满足 `READY_FOR_OWNER_FINAL_REVIEW`。PR #518 保持 Draft/Open，未合并、未发布，候选搜索默认关闭。

## 1. 代码身份与变更范围

- 远端 `master`：`83607e67d63248e75e42ccba3e52918da101c123`。
- PR 分支已对齐该 master 基线；`git range-diff` 确认原 PR 的 7 个提交均保留。
- 本次 writer-only 测量代码 HEAD：`3a84806ce0040f884e1c1b9f32df2b7af40dfd52`。
- 测量代码 Tree：`03df0e3a24aedfce4b51913fabd241fda28883ba`。
- 报告以后的 docs-only 提交会更新 PR 分支 HEAD；性能、编译与测试证据仍指向上述精确代码 HEAD/Tree。最终 PR HEAD/Tree 会在交付回执中给出。
- PR 状态：OPEN、Draft；写入验收完成时 GitHub 返回 merge state `UNSTABLE`，Owner 合并审查仍未完成。

本轮正式验收代码只新增/调整三处：

1. `src-tauri/src/db/connection.rs`：抽出共同连接池初始化路径。生产入口显式传入无测试覆写，生产 Hook 决策保持现有 `should_install_mutation_hooks` 逻辑；新增 `#[cfg(test)]` writer qualification 入口，使基线池关闭候选 Hook、候选池打开 Hook，确保两 arm 经过真实生产 `r2d2_sqlite` pool 与 `configure_connection`，且避免环境变量污染基线。
2. `src-tauri/src/global_index/tests/issue494_writer_qualification.rs`：完善 200k synthetic fixture 的双臂配对 writer qualification，分别测量生产 WAL 自动 checkpoint 与独立 FULL stress。
3. `.github/workflows/issue-494-rowid-boundary-qualification.yml`：writer-only 专用提交跳过另外的 100k/200k Search 矩阵；1M job 未触发。

本轮没有修改 FTS 候选检索、排序、索引或门槛。`filename_search.rs` 与既有 1M 候选版本 `0251f3c1cbaa2f82703fffa3470153381f25c46f` 的文件内容相同；候选没有设为默认启用。最终 PR 中候选只在对应编译 feature 与显式 `ZC_GLOBAL_SEARCH_FILENAME_RELEVANCE_CANDIDATE=1` 同时满足时启用；writer 测试的候选 Hook 仅作用于隔离的 synthetic 数据库副本。

## 2. 生产 WAL 写入资格

唯一一次本轮双平台 writer-only Release run： [GitHub Actions 41210977037](https://github.com/ArdenZC/Fylura/actions/runs/41210977037)。这次执行固定了 200,000 行 synthetic fixture、5 次预热、每 arm/批次 100 对样本，512 与 5,000 行四组；基线和候选使用等价数据库副本及相同 row ID，交替 writer 顺序，不删离群样本。使用配对样本索引 bootstrap percentile，2,000 次重采样，xorshift64* 固定 seed：512 行为 `1229539345`，5,000 行为 `1229543833`（两平台、production 与 FULL 组按批次共用）；百分位采用线性插值。

连接合同在两端均核对：生产 `Database` pool（`r2d2_sqlite`，max size 8、min idle 1）、`journal_mode=WAL`、`synchronous=NORMAL`、`wal_autocheckpoint=1000`、外键启用及生产连接初始化。自动 checkpoint 未从 commit/total 耗时中移出。每一对原始样本都含 UPDATE、COMMIT、total、writer 顺序、row ID 起点，以及两端事务前后的 WAL frame 状态。

### 2.1 生产自动 checkpoint 门槛结果

硬门槛：每平台、每批次的 total p95 候选/基线回退不超过 10%，且配对 bootstrap 95% 区间上界不超过 10%；两 arm 都须观测到自动 checkpoint frame work。

| 平台 / 批次 | 基线 total p50 / p95 / p99 (ms) | 候选 total p50 / p95 / p99 (ms) | p95 回退 | 配对 bootstrap 95% 区间 | 自动 checkpoint 观测 | 判定 |
|---|---:|---:|---:|---:|---:|---|
| Windows x86_64 / 512 | 4.774 / 1,404.473 / 1,785.855 | 4.792 / 1,435.686 / 1,561.954 | +2.222% | [-18.969%, +14.752%] | 基线 26/100；候选 26/100 | `POINT_PASS_INCONCLUSIVE` |
| Windows x86_64 / 5,000 | 821.982 / 2,438.951 / 2,632.744 | 827.751 / 2,355.432 / 2,530.792 | -3.424% | [-7.432%, +4.904%] | 基线 100/100；候选 100/100 | `PRODUCTION_WRITER_PASS_WITH_MARGIN` |
| Apple Silicon / 512 | 5.590 / 28.191 / 41.993 | 5.583 / 27.277 / 30.709 | -3.242% | [-10.162%, +5.077%] | 基线 26/100；候选 26/100 | `PRODUCTION_WRITER_PASS_WITH_MARGIN` |
| Apple Silicon / 5,000 | 69.382 / 105.054 / 112.877 | 67.910 / 95.355 / 109.309 | -9.232% | [-14.229%, +1.643%] | 基线 100/100；候选 100/100 | `PRODUCTION_WRITER_PASS_WITH_MARGIN` |

双平台 UPDATE / COMMIT 明细（p50 / p95 / p99，ms）：

| 批次 / 阶段 | 基线 | 候选 | p95 回退 |
|---|---:|---:|---:|
| Windows 512 UPDATE | 4.189 / 4.598 / 4.749 | 4.217 / 4.688 / 4.716 | +1.957% |
| Windows 512 COMMIT | 0.560 / 1,400.302 / 1,781.456 | 0.568 / 1,430.988 / 1,557.421 | +2.191% |
| Windows 5,000 UPDATE | 42.896 / 47.495 / 51.549 | 44.032 / 49.629 / 56.347 | +4.493% |
| Windows 5,000 COMMIT | 770.851 / 2,398.983 / 2,590.266 | 779.044 / 2,314.310 / 2,488.175 | -3.530% |
| 512 UPDATE | 4.936 / 6.110 / 7.045 | 4.917 / 6.732 / 9.481 | +10.180% |
| 512 COMMIT | 0.630 / 23.283 / 35.372 | 0.629 / 22.387 / 25.704 | -3.848% |
| 5,000 UPDATE | 50.000 / 61.164 / 65.192 | 50.221 / 61.336 / 64.773 | +0.281% |
| 5,000 COMMIT | 12.901 / 53.819 / 58.249 | 12.974 / 43.000 / 47.504 | -20.103% |

UPDATE/COMMIT 分位数用于解释总耗时，不单独判定 writer 门槛。512 UPDATE 的 p95 回退超过 10%，但完整事务门槛以 total 为准；SQLite 自动 checkpoint 工作留在 COMMIT/total 内。

四个 production cell 中三个为 `PRODUCTION_WRITER_PASS_WITH_MARGIN`，Windows 512 为 `POINT_PASS_INCONCLUSIVE`；没有 point estimate 超过 10% 的 `WRITER_NO_GO` cell。由于 inconclusive 条件不能当成通过，生产 WAL 总体资格仍不充分。

### 2.2 强制 FULL checkpoint 压力诊断

每笔写事务后显式执行并计时 `wal_checkpoint(FULL)`，生产自动 checkpoint 在该独立压力组关闭。下表的 total 是 UPDATE+COMMIT+FULL；它不参与生产 writer 硬门槛。FULL checkpoint 单独列出 p50/p95/p99 和 p95 回退：

| 平台 / 批次 | total 基线 p50/p95/p99 | total 候选 p50/p95/p99 | total p95 回退 / bootstrap 95% | FULL 基线 p50/p95/p99 | FULL 候选 p50/p95/p99 | FULL p95 回退 / bootstrap 95% |
|---|---:|---:|---:|---:|---:|---:|
| Windows x86_64 / 512 | 233.491 / 290.697 / 444.342 | 235.222 / 277.353 / 312.154 | -4.590% / [-25.013%, +7.039%] | 225.269 / 282.675 / 354.871 | 226.545 / 269.122 / 304.052 | -4.795% / [-21.332%, +7.348%] |
| Windows x86_64 / 5,000 | 1,097.595 / 1,394.244 / 1,545.721 | 1,112.026 / 1,394.966 / 1,480.189 | +0.052% / [-5.696%, +7.588%] | 1,048.938 / 1,344.677 / 1,498.757 | 1,065.952 / 1,348.071 / 1,434.106 | +0.252% / [-5.879%, +7.798%] |
| Apple Silicon / 512 | 11.542 / 14.801 / 15.422 | 11.484 / 14.562 / 15.703 | -1.615% / [-6.174%, +5.421%] | 5.826 / 8.045 / 8.640 | 5.589 / 8.108 / 8.588 | +0.783% / [-7.002%, +6.652%] |
| Apple Silicon / 5,000 | 67.340 / 77.847 / 87.173 | 67.442 / 75.376 / 87.192 | -3.174% / [-5.051%, +1.401%] | 16.255 / 25.198 / 28.534 | 16.114 / 24.086 / 25.626 | -4.413% / [-15.147%, +1.010%] |

本轮 Apple Silicon 两个 FULL 组均未超过 10%。历史 macOS 1M / 5,000 行强制 FULL 写入仍为 **+12.638%**，且旧 workflow 的 macOS job 因此失败；这条原合同失败保留，不被本轮结果追溯改写。Windows 历史 1M job 成功。

FULL stress 的配对差值（候选减基线，p50/p95/p99，ms）：Windows 512 total `+3.477/+37.063/+80.993`、checkpoint `+3.070/+36.914/+79.950`；Windows 5,000 total `-10.773/+216.330/+310.424`、checkpoint `-10.028/+217.757/+310.630`；macOS 512 total `-0.061/+1.184/+2.040`、checkpoint `-0.039/+0.745/+1.168`；macOS 5,000 total `+0.008/+4.773/+7.164`、checkpoint `-0.201/+2.161/+4.432`。这是逐对差值分布，不等于两组各自 p95 之差；四个 FULL 组的 p95 overhead 都在 10% 内，bootstrap 上界也低于 10%，但全部仅作诊断。

Windows 512 生产组呈现明显 checkpoint 重尾：100 个样本中两 arm 各 26 次观察到 checkpoint frame work，各 13 个 total 样本达到 1,000 ms，且这 13 个均带 checkpoint-work 标记；COMMIT p50 约 0.56 ms、p95 约 1.40/1.43 秒。所有这些真实长耗时仍留在 p95。点估计只回退 2.222%，但样本重尾使配对 bootstrap 上界达到 14.752%，所以结果保留为 inconclusive；未删除异常样本，也未复跑。Windows 5,000 行每 arm 100/100 均观察到 frame work。强制 FULL 组的高绝对耗时来自被要求执行的同步 FULL checkpoint；相对差异均未超门槛，且它不是生产 gate。

### 2.3 原始写入证据

writer run artifact 内有平台 source record、SHA-256 manifest 与 JSONL（每组含完整 100 个原始配对样本，包含 WAL before/after frame 数和强制 FULL 返回值）。

| 平台 | JSONL artifact 文件 | SHA-256 | source record SHA-256 | 精确运行环境 |
|---|---|---|---|---|
| Windows x86_64 | `issue494-writer-windows-200000.jsonl` | `efaa55e0db6a54c3207b30f583bf197408b75d1113473f41b5ae13aa092ed502` | `21a5db60b322fb5596cfc38524852319220b0207ffa8a10c769fd94423a64dd6` | `windows-2025`, x86_64, rustc 1.97.1, SQLite 3.51.3 |
| Apple Silicon macOS | `issue494-writer-macos-200000.jsonl` | `528eb47f0bd090cdb2d28bb0d4f82891a8b34fa942ac0bb861830d4f3b764cf4` | `40caf9b2349cd7124f9cd89184fee959e407c6117c22a2ff01bb19fe523184e1` | `macos-26`, arm64, rustc 1.97.1, SQLite 3.51.3 |

本机已对下载的两个平台 JSONL 与 source record 独立重算 SHA-256，并与各自 workflow manifest 一致。artifact 按 workflow 配置保留 90 天；原始结果没有被编辑或删样本。

## 3. 既有 1M 搜索证据（未重跑）

本轮没有重新运行 1M Search qualification。引用历史 [run 41181789652](https://github.com/ArdenZC/Fylura/actions/runs/41181789652)，候选搜索代码 SHA `0251f3c1cbaa2f82703fffa3470153381f25c46f`、Tree `1f5b5934d1fec4e9211b3845f2646e4c34a8093a`。其 1,000,000 文件 / 约 50,000 高命中 / 完整 4,096 结果窗口使用独立文件名优先排序 oracle；搜索与 Snapshot 的页首和 offset 4016 p95 如下：

| 测试 | Windows x86_64 p95 (ms) | Apple Silicon p95 (ms) |
|---|---:|---:|
| Search 首页 | 28.623 | 22.624 |
| Search offset 4016 | 33.341 | 26.551 |
| Repository Snapshot 首页 | 27.499 | 25.755 |
| Repository Snapshot offset 4016 | 31.737 | 24.255 |

历史 artifact 原始 JSONL SHA-256：Windows `dbd436de8472dfd32b9e39dca6efaf04a608a3210a289b79c182ab847b2b14e8`；macOS `c63d644770a342bedd743aa23e11016d0bd3c8e42f109fd0b9f1f077713356bc`。对应候选源码记录 SHA-256：Windows `f2e9a29712dd559b1bf4c5c5b9c6a6bee848002de5bd411714e76f201e3d0646`；macOS `964e5c38c3c02d12fedd9dbb3b726dde7f9cf86e6c652559cca79fd00420b75b`。独立 validator 对 Windows artifact 通过；macOS 搜索、排序、回退与资源门此前检查通过，整份 validator 最后因 FULL stress 写入门失败而返回 failure。

此前 harness 已覆盖 source facts、修订变化、撤权后的候选回退，并确认排序 oracle 与完整窗口。它是 native Rust Database/Repository harness，不是已安装 Tauri 应用，不证明 GUI、真实文件 watcher 或完整安装版授权生命周期。

## 4. Windows/macOS 安装版 GUI 与文件生命周期

**真实安装版验收未完成。** 当前常规 CI 的 `Package NSIS`、`Package unsigned DMG`、Windows native Preview Handler 均 skipped；Release compile 生成的是编译检查结果，不是可供本机安装验收的安装包。当前 Windows 11 Pro x64 工作站（build 26300）没有可用 Fylura 安装包，桌面应用 inventory 未发现 Fylura 已安装；当前环境也没有 Apple Silicon macOS GUI 设备。普通 macOS hosted runner 的原生 harness/CI 不能替代用户桌面安装版验收。

| 要求 | Windows 安装版 | Apple Silicon macOS 安装版 | 现有证据 / 限制 |
|---|---|---|---|
| 启动、关闭、Global Search GUI、精确/前缀/token/子串、高命中与分页 | 未测 | 未测 | 无安装包/原生 GUI |
| 打开/定位结果、快速输入取消与过期响应 | 未测 | 未测 | 同上 |
| 无结果/单结果/稀疏结果、Unsupported query 回退 | 未测 | 未测 | 同上；旧 harness 只覆盖其实现范围 |
| 文件变化时查询正确性与高命中 GUI 响应 | 未测 | 未测 | 无真实 GUI |
| 创建、重命名、修改、移动/跨目录、删除、批量变化 | 未测 | 未测 | 无真实安装实例 |
| 外部程序修改、Watcher 处理、退出重启、索引失效/重建 | 未测 | 未测 | 同上 |
| 撤权/重授权、SQLite 外部写入、Hook 丢失、来源代次变化 | 未测 | 未测 | 旧 1M harness 有限的撤权 fallback 证据；应用全生命周期未测 |
| DB Busy、访问权限异常与撤权来源零泄露 | 未测 | 未测 | 无安装版端到端结果 |

### 4.1 默认来源与低价值目录安全（静态代码复核）

- Windows 卷发现把新来源置为 `enabled=false`、`not_authorized`；当前 `source_enabled_by_default` 对所有卷类型返回 false，协调器跳过禁用或未授权来源。静态路径未发现首次启动自动扫描 C 盘的授权。
- macOS 发现的卷默认 `enabled=false`；TCC/Full Disk Access 仍是独立权限。本轮未触发授权提示或实际扫描。
- `CoveragePolicy::default()` 为 Smart Lean。代码按目录身份/标记排除带项目标记的 `node_modules` 及已识别的 Python venv、Rust `target`、CMake build 输出和限定范围的已知缓存路径；不会因通用目录名自动排除所有内容。它不等于对每个系统、开发或缓存目录的完整排除保证。
- 无权限时是否在真实应用中避免持续重试、候选索引与 SQLite fallback 是否在 UI/重启/撤权竞态中始终零泄露，仍须安装版端到端验证。

以上是源码行为审查，不是实际桌面运行验收。

## 5. 资源、mmap 与 CPU

目前最完整的 1M 资源数据来自既有原生 Rust harness，不是实际安装版：

| 指标 | Windows x86_64 | Apple Silicon macOS | 解释 |
|---|---:|---:|---|
| 索引保留/构建堆峰值 | 约 101.0 MiB | 约 101.0 MiB | harness 测值 |
| 额外私有构建峰值 | 126.68 MiB | 88.30 MiB | 硬上限 128 MiB；Windows 仅余 1.32 MiB |
| 30 分钟空闲归一化 CPU | 0.0% | 0.0% | harness 样本，不是 Tauri/WebView 进程树 |
| SQLite mmap 配置 | 约 2,047.94 MiB | 约 2,047.94 MiB | mmap resident bytes 未能按 SQLite mapping 归因 |
| 30 分钟时 private committed/footprint | Windows private committed 125.54 MiB | macOS private footprint 80.05 MiB | 原生 harness 进程口径 |
| 30 分钟时 working set / RSS | Windows working set 16.28 MiB | 全进程采样 resident 约 1,609.89 MiB | macOS 数值是整个测试进程采样 RSS，不等同 Rust heap；不能据此归因 mmap |

本次 macOS 200k writer harness 在构建索引前后观测到 private footprint 增量 20,545,664 bytes、resident 增量 20,758,528 bytes；这不是 1M/已安装应用资源资格。

Windows 的旧 1M harness 峰值低于 128 MiB 硬门槛，但仅有 1.32 MiB 余量，不足以证明稳健余量。当前缺少真实安装版冷启动、索引峰值、稳定态、首次/连续搜索、深分页、高频批量变更/重建、WebView 与 Rust 分进程内存、SQLite page cache 与 mmap 驻留归因、完整进程树、30 分钟 idle CPU/内存变化及退出清理数据。因此无法据此判定安装版符合 Zen Zero 低占用目标；现有数据也不足以作 `RESOURCE_NO_GO` 结论。

## 6. 本轮 CI 与本机验证

- 专用 writer workflow： [run 41210977037](https://github.com/ArdenZC/Fylura/actions/runs/41210977037)，整体及 Windows/macOS 两个 job 均 SUCCESS。这里的 workflow SUCCESS 代表原生测量、artifact 和 SHA 步骤成功完成；Windows 512 生产 writer 的资格分类仍是 `POINT_PASS_INCONCLUSIVE`。100k/200k Search matrix 与 1M matrix 均 skipped。
- 普通 PR CI： [run 41210977122](https://github.com/ArdenZC/Fylura/actions/runs/41210977122)，整体 SUCCESS。双平台 Release compile、Rust quality、Quality、Windows Global Index service qualification、Native macOS performance、dependency audit 与常规性能 shards 均通过。`Package NSIS`、`Package unsigned DMG`、Package metadata smoke、Windows Preview Handler、Frontend and format quality、Documentation-only validation 均 skipped。常规 performance/search shard 不等同于 Issue #494 的 1M 资格。
- Global Search synthetic benchmark： [run 41210977064](https://github.com/ArdenZC/Fylura/actions/runs/41210977064) 两平台 ISG-00E 1k ingestion smoke 通过；A/B/C/D paired 搜索与 1M exact SQL 诊断 skipped。
- Smart Lean product qualification： [run 41210977002](https://github.com/ArdenZC/Fylura/actions/runs/41210977002) 的 macOS job 在 1 秒后 cancelled（PR checks 将其显示为 fail）；Windows job 在本报告收尾时仍 pending。它是相关 Index Scope 产品资格的未完成结果，不是安装版 Global Search GUI 证据。
- 本机通过：`npm ci`（139 packages，0 vulnerabilities）、`npm run dev:preflight -- --plan`、`npm run typecheck`、`npm run build:frontend`、`npm run test:governance`、两个 CI contract 测试（30 passed）、`cargo fmt --all -- --check`、`cargo metadata --locked`、`git diff --check`。Frontend build 有既存 CSS `text-[var(...)]` 生成警告。
- 本机 `npm test`：2,031 tests 通过；以下四个 research suite 在收集前报 `SyntaxError: Invalid or unexpected token`：
  - `research/zen-decision-bench/tests/preference-signal-b1.test.mjs`
  - `research/zen-decision-bench/tests/preference-signal-b4.test.mjs`
  - `research/zen-decision-bench/tests/preference-signal-adjudication.test.mjs`
  - `research/zen-decision-bench/tests/preference-frozen-tree.test.mjs`

  四个文件各自 `node --check` 均通过；复跑 B1 focused suite 仍出现 Vitest 错误。失败来自当前 master 已有、非本 PR 搜索变更文件。
- 本机 focused Rust gate 命令 `cargo test --locked --manifest-path src-tauri/Cargo.toml --features 'desktop-runtime global-search-filename-relevance-candidate' --lib writer_gate_classification -- --nocapture` 未能进入项目代码编译：Windows 开发环境没有 MSVC `link.exe`；GNU linker 也不适用于当前 MSVC target。Hosted native Release writer job 是该测试的原生执行证据。

## 7. 最终状态与 Owner 决策边界

最终分类为 **`POINT_PASS_INCONCLUSIVE`**，由 Windows 512 行 production WAL 的 bootstrap 上界超出 10% 所致。另有独立的安装版验收子状态 **`ENVIRONMENT_UNVERIFIED`**：真实 Windows/macOS 安装、GUI、文件生命周期、撤权端到端和完整资源矩阵缺少客观环境/安装包证据。因此不满足 `READY_FOR_OWNER_FINAL_REVIEW`。候选维持默认关闭。

PR #518 保持 OPEN / Draft；未合并、未发布。是否补齐原生安装版验收及何时进入最终合并审查由 Owner 决定。没有修改已批准的性能/资源/授权门槛。
