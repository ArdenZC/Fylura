# B 方案：Workspace Foundation 原生资源噪声校准

**状态：测量工具；不改变当前 CI 资源门槛、Zen Zero 预算或发布资格。**

## 为什么做

旧五轮资源门禁只要每轮比上轮大就算持续增长，例如 38,027,264 → 38,043,648 → 38,060,032 → 38,076,416 → 38,109,184 字节。这证明采样值单调增长，不直接证明真正泄漏。Windows 使用 PrivateUsage/句柄；macOS 使用 RSS/FD。内部资源零持有者与四个 protected status 仍是硬要求。

历史 macOS CI：#525 +32 KiB PASS、#521 +16 KiB PASS、#518 +80 KiB BLOCKED、#529 0 PASS。**这些是不同代码/环境的对照资料，不能用来声称同一构建的统计噪声已经测定。**

## 不增加自动 CI 的一次性人工校准

在相应平台先通过现有 Performance / Prepare 生成 **同一个 release + performance-test-tauri lib 测试可执行文件**，使用官方原生性能准备产物；不要重新编译三次，也不要从不同 SHA 的程序混合结果。原生测试必须列出以下精确被忽略的测试名：

file_workspace::integration::performance::steady_state::resource_and_registry_steady_state_after_browse_preview_switches

执行示例（换成实际测试可执行文件的绝对路径）：

    node scripts/calibrateWorkspaceResourceTrend.mjs --binary="/absolute/path/to/fylura_tauri-HASH" --runs=3 --source-sha=EXACT_HEAD --output=".tmp-resource-calibration/workspace-resource.json"

该命令用同一二进制启动三个**独立的测试进程**，每进程执行原有五个 epoch ×20 次 Browse/Preview/Thumbnail/切换工作量，记录原始 RSS、PrivateUsage、句柄、FD、registry；对 [zc-perf] 场景执行必需字段、平台、数量校验，不缺项补 PASS；记录二进制 SHA-256、操作系统版本、退出码和每次 native HARD PASS/BLOCKED。

- 三次全 PASS 时工具退出码 0；这**只代表收集的原生校准样本全部 PASS**，不是新的发布或合并资格，也不证明不存在泄漏。
- 发生原有 BLOCKED 或子进程失败，工具仍输出 JSON，退出码为 2；不能把非零视作无效证据，也不能把 JSON 报告当成合格证明。
- 缺少必需指标、未知版本、错误测试二进制或参数时，工具退出码为 1；标记证据不完整。
- 运行前确认同一 OS/架构、工具链、测试输入和原生测试选项。不同构建或云主机不能混合作为独立重复。
- 证据仅保存在指定输出路径；不自动上传用户目录、测试文件或系统盘内容。

## 后续设计决策（本 PR 不执行）

比对三次独立进程的 within-run 与 between-run 尾部增长、峰值及稳定期，结合文件句柄/FD、内部注册表及真实引用持有者判断。Windows 已有 PrivateUsage 故意持有 4 MiB ×4 epoch 正控测试。macOS 应另验证故意持有内存的正控，以及较长负载下增长是否会趋于平稳。

必须能捕捉真实持续泄漏、显著内存回归及超出 Zen Zero 预算，才考虑提出替代判定。**没有形成经过验证的判定方法前，不改 resource_epoch_trend 的 HARD PASS/BLOCKED 和受保护 Quality 汇总。**

关联：[#353](https://github.com/ArdenZC/Fylura/issues/353)、[ADR-0017](../project/DECISIONS/0017-risk-proportionate-ci-routing.md)。
