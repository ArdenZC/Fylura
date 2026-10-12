# S1R4-P3 R2 consolidated runtime fix

`S1R4-P3 R2_CONSOLIDATED_RUNTIME_FIX_READY_FOR_OWNER_REVIEW`

分支 `remediation/s1r4-p3-runtime-r2`，直接基于 R1 准备提交 `2f5a6136be17df9a1149b9a8facab98ebcec231f`。本轮没有 Hosted 执行；新 Runner 是独立候选，未获 Windows R2 资格。

修复：

- Case 02：Runner clean wrapper、Harness status、四份未激活 Workflow preflight 和两个研究 Validator 的可执行 status 参数均改为 `--untracked-files=all`。干净 gate 不放宽，不使用忽略规则隐藏脏文件。全脚本扫描仅剩历史 `.Trim()` 检测引用和既有激活 Workflow 中的旧写法；二者按历史保全要求保留。
- Case 12：显式保存已初始化的 stdout/stderr reader，分别判空后关闭；不对启动失败时未创建的流调用 Close。保留原始 launch_error；清理错误仍优先分类为 PROCESS_CLEANUP_FAILED，不能被 LAUNCH_FAILED 掩盖。超时、WaitForExit 和进程树 Kill 源码区域完全未变。
- Case 13：统一捕获字段及类型合同；原始 wrapper 记录合法地没有 Harness 时间字段。增强记录的四个时间/预算字段必须全有或全无；检查真实时间顺序、有限非负 elapsed 和完整预算，不补造时间戳。Case 11/13 共用该合同；仍严格检查退出码、双流完整性、错误、超时及 cleanup 状态。
- 身份：历史 freeze Head/Tree/Blob 仅作来源证明；新 Runner Blob/SHA 独立钉住 Harness 和未来草案，旧 freeze 身份不会用于通过新代码校验。

改动文件：Runner、Harness、新增 `scripts/testIssue380S1R4P3GitRuntimeR2.ps1`、RuntimePlan Validator、Preparation / PreflightNullSafety 两个研究 Validator、POWERSHELL-RUNTIME / PREFLIGHT-NULL-SAFETY / RUNNER-NULL-SAFETY / BOUNDED-GIT-CAPTURE 四份未激活 Workflow 草案，以及本报告和 [证据](evidence/issue-380-s1r4-p3-r2-runtime-fix/)。Artifact Helper、Child、Controller、原 71 项回归程序保持不变。

实际验证：

- Debian 13 Cloud / PowerShell **7.4.13** / 真实 Linux Git，`git.exe` 是临时名称适配。原 **71 项回归 PASS**；新增 **62 项 PowerShell 检查 PASS**；RuntimePlan **20 项静态合同 PASS**（原 19 项 + R2 范围/参数/身份/超时合同）。三份修改的 Python 文件编译、六段未来草案 PowerShell 语法、YAML、身份、无触发检查及 diff check PASS。
- 原始 Cases **01、02、03、03b、04、10、11、12、13 PASS**；**08 UNPROVEN**。Case 12 的最终 Windows CIM 探针使用明确标记的离线 double，Git 启动失败及错误字段为真实执行；未运行完整 Harness / Runner 主入口。
- 新增真实 Git 场景：干净、未跟踪、已修改、嵌套未跟踪、Git 执行失败。真实缺失工作目录得到 LAUNCH_FAILED、原始 launch_error、null PID，无无效流关闭；双类记录、JSON round-trip、不合法计时组和成功状态矛盾负例均通过。
- 部分初始化、读取异常、Close / Dispose 清理失败通过**候选精确 AST + 受控对象/真实 faulted Task**在 PowerShell 中验证；这是明确标记的分支测试，不能描述为 Windows / OS 故障注入。Case 08 不因此变成 PASS。
- 原 Artifact Helper 回归继续验证 7 payload + SHA256SUMS，无遗漏/额外/重复，排除 Git 数据库，所有权/Reparse/250 文件/20 MB 限制不变。[测试结果与身份](evidence/issue-380-s1r4-p3-r2-runtime-fix/candidate-identity-and-diff-audit.json) 绑定源码 Blob/SHA；最终提交后再次验证同一源码。

运行命令：

```sh
PATH="/workspace/scratch/git-runtime-bin:$PATH" /workspace/scratch/powershell/opt/microsoft/powershell/7/pwsh -NoProfile -File scripts/testIssue380S1R4P3GitRuntimeRemediation.ps1 -OutputRoot <fresh-output-root>
PATH="/workspace/scratch/git-runtime-bin:$PATH" /workspace/scratch/powershell/opt/microsoft/powershell/7/pwsh -NoProfile -File scripts/testIssue380S1R4P3GitRuntimeR2.ps1 -OutputRoot <another-fresh-output-root>
python scripts/validateIssue380S1R4P3GitRuntimePlan.py
git diff --check
```

Runner 修改仅限 `Invoke-GitCapture` 与 `Assert-GitWorktreeClean`；其他源码区域逐字一致，包括 NSIS、HKCU、FileWrite、TaskRoot。P1/P2/S0、现有 Workflow 和历史证据 diff 为空。R1 HEAD/Tree/Run/Job/Artifact 未修改、覆盖或重跑；无 master、生产代码、Schema、PR、Merge、Issue 关闭、Release 或 Codex Review 操作。

未来草案完整但未激活：仅 `qualification/s1r4-p3-windows-runtime-r2` 与单一新 Workflow 路径；新候选和辅助工具身份已钉住，Owner Parent sentinel 保留，checkout/upload-artifact 使用 R1 实际 v4 Commit SHA。当前修复分支不匹配任何 push Workflow。

**NOT RUN / 风险**：Windows 专属 Git aliases、继承句柄、进程树终止及 CIM；Hosted R2、实际 Windows R2 Artifact；完整 Runner/NSIS/安装器/Service/IPC/数据库。Case 08 的确定性终止失败仍 UNPROVEN；返回后 read task 终态和同步 Kill/Close/Dispose 全函数最坏耗时仍未证明。历史 NSIS 阶段 Validator 未运行为 R2 资格 gate，原始历史清单按当时字节核验。没有 RUNTIME_MATRIX_PASS 或 Windows PASS 声明。完成后停止，由 Owner 统一审核。
