# Fylura: B 方案第一批实施记录与验收范围

**批准：** Owner，2026-11-01。**源基线：** `master@c2e26672923526e6a29da3ea3b5fd317301e125a`。此文是实施说明，不取代 `DEVELOPMENT_WORKFLOW.md`。

## 本批覆盖（R01 / 部分 R02 / R08）
- CI 变更分类器保留 workflow 变更标识、high-risk 分类和四项 required Quality 名称；拆分 **共享 CI/性能编排** 与 **独立工作流配置**。只有前者、DB core、缺失基线或明确 Full 才触发六领域 100k 性能资格。独立工作流改动不再无条件启动六域性能；仍在现有 `Change scope / routing contract` 里强制运行三项路由/工作流回归测试。
- Windows Global Index service 资格由 **原生服务、持久来源写入/授权、Indexer 所有者和关联诊断** 触发。纯 `search.rs`、`filename_search.rs` 与 Search 测试继续 Search 正确性/性能与 Rust 相关检查，但不因代码放在 `global_index/` 就启动特权服务恢复脚本。混合 producer/search 的 PR 仍触发服务资格。
- 扩展真实文件路径的正反测试，并更新 STATUS 顶部历史状态区分。这一批 **没有** 修改 Playwright/SQLite/Tauri 产品代码、Windows Service Pause 行为、Writer +10% 或内存五轮严格增长的老门槛。
- 将合并 / 候选正式启用 / 两平台公开发布三个结论写入已有开发规范，不新建审批流程。Protect master 的四项 required checks、分支线性历史及 PR 保护都保持不变。

## 已知事实基线（不是长期平均数）
在 2026-11-01 的研究窗口，最近 100 次 Actions 里主 CI 71 次，成功 18、失败 25、取消 27、运行中 1；主 CI 完成样本墙钟近似 p50 13.5 分钟、p90 23.1 分钟。历史 PR #525 的 CI 路由变更遭 Windows PrivateUsage 五轮增长阻断；#518 的 Windows Pause 与 macOS RSS 约 +80 KiB 造成 Quality FAIL，但原生 Windows 搜索权限闭环已独立验证。上述案例是**分类器与测量法研究的动机**，不能充当新 HEAD 的 PASS。

## 必须保持阻断的正例
- `global_index/repository.rs`、`coordinator.rs`、`index_scope/**`、`windows/**` → 原生 Service 资格。
- `db/schema.rs`、`db/connection.rs`、`ci.yml`、`ci-full.yml`、分类器/性能编排脚本 → 六域性能测试。
- `file_ops/**`、`fs_safety/**` → 既有真实身份/日志/恢复与相关原生测试；此批不改其路由。
- 任何 Full、缺失 diff base → 所有领域资格；Windows/macOS Quality 名称和安全门槛不变。

## 第二批（R03–R06）严格前置条件
- **内存:** 使用同一源码、相同负载及 runner 配置的多次独立进程/稳定态采样确定噪声包络、资源绝对预算及长期趋势。不能凭 80 KiB 一次样本设免检阈值；先出诊断结果，另行决定硬门槛调整。
- **Windows Service Pause:** 记录 Pause 操作 ID、来源修订、请求/ACK、最终写入不变量与有界收敛时间，定位竞态还是生产持久化缺陷。真实 durable Pause 丢失必须继续失败。
- **Smart Lean macOS:** 将托管 Spotlight sourceStatus 未索引记录为 `ENVIRONMENT_UNVERIFIED`；不改变发布缺口、不声称 macOS 原生 PASS。
- **Writer:** 保留 p95 +10% 产品预算，记录 checkpoint 分层与置信区间宽度；未有新 Owner 数据量批准不重跑矩阵。

## 第三/四批（R07–R12）
在 #512/#487 主线之下对 34 个历史开放 PR 分类（33 个 Draft）；先检查独特安全代码再决定归档，不自动关闭。优先整理 STATUS 单一当前入口，巨大文件只在实功能修改时小范围拆解。继续沿用 `dev:preflight`、`dev:ci-failure`、`dev:ci-metrics`，两个迭代周期对照一次通过率、无意义重复 CI、PR 首推到合并时长和产品可用功能交付数。缓存重用必须绑定可信编译身份。

**注意：** 任何跨越这些范围的性能阈值、权限放宽、required status 改名及 Windows/macOS 正式发布判定，都需要在相应实现 PR 中再次审查。这里没有批准跳过现有红灯或放松用户文件安全。
