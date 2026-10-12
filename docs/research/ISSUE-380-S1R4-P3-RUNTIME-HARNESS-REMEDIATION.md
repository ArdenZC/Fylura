# S1R4-P3 Runtime Harness consolidated remediation

Status: `S1R4-P3 RUNTIME_HARNESS_CONSOLIDATED_REMEDIATION_READY_FOR_OWNER_REVIEW`

Base: `18b8e5d4826b246c3741bf37d772e47d98340d16`.
Branch: `remediation/s1r4-p3-runtime-harness-consolidated`.

## 修复及触发条件

- Case 返回 Hashtable / PSCustomObject 缺少 `unproven`：统一结果合同，必需 expected/actual 非 null、状态布尔类型检查、互斥 PASS/UNPROVEN、拒绝缺少状态及矛盾 status；`passed=false` 登记 FAIL。异常保存原始错误、错误位置和该 Case Git 调用；不再因 Case FAIL 中断后续调度；最终 FAIL 优先于 UNPROVEN，完整 PASS 必须有 14 条记录。
- Case 结果为零/单元素数组：避免 `if` 表达式的管道展开，直接赋值保留数组及 JSON 形状。
- Child 记录缺少可选 child_pid：按字段存在性读取；读取时验证 mode、nonce 和对应 PID 类型，错误/外来记录失败关闭，所有权未放宽。
- 空 TaskRoot：错误信息不再通过空数组的 `.Name` 属性触发第二次 StrictMode 异常。
- 最终清理部分完成后抛错：仍写出已收集 cleanup 记录。JSON round-trip 失败追加错误，保留原始 Case 错误；summary / Controller JSON 无法写入时禁止成功状态/退出码。
- 草案 cleanup 检查抛错：先写 cleanup.json，再令步骤失败，保留错误证据。
- SHA256SUMS 覆盖临时 `.git` 而上传排除隐藏文件：独立归档 helper，仅选择 evidence 下非隐藏的顶层证据文件，以及明确列举的 task ownership / child JSON 诊断文件。临时仓库、Git 数据库、源码副本及配置不入归档。上传仅指向该独立 archive 目录，保持 `include-hidden-files: false`。逐文件核对复制前后 SHA-256，再验证清单哈希、缺失、额外、重复；SHA256SUMS 自身是唯一自哈希例外。原所有权、Reparse Point、源文件 250 / 20 MB 限制保留，并对归档（含清单）追加同等限制；遍历先拒绝链接再进入目录。

## 实际改动

`scripts/issue380S1R4P3GitRuntimeQualification.ps1`、`scripts/issue380S1R4P3GitRuntimeController.ps1`、新增 `scripts/issue380S1R4P3GitRuntimeArtifact.ps1`、新增 `scripts/testIssue380S1R4P3GitRuntimeRemediation.ps1`、`scripts/validateIssue380S1R4P3GitRuntimePlan.py`、未来未激活 Workflow 草案 `ISSUE-380-S1R4-P3-POWERSHELL-RUNTIME-WORKFLOW.md`，以及本报告和独立 [整改证据目录](evidence/issue-380-s1r4-p3-runtime-remediation/)。Child 经检查及解析未修改。没有生产代码、Schema、既有 Workflow 或历史证据改动。

## 已执行验证

- Debian 13 Codex Cloud，微软官方 Debian 包 PowerShell **7.4.13** 在临时目录解压运行（未部署 Windows 环境）。**71 项真实 PowerShell 检查 PASS**：AST 解析、原始 Case 01/03/03b 与真实 Git、原始 Case 08 UNPROVEN、合同正负例、零/单元素数组、异常与下一 Case、Child 记录、空 TaskRoot、实际 finalizer JSON、实际 Controller 输出及写失败处理、草案 cleanup 错误保留、实际 Artifact helper 及负例。`git.exe` 是 Cloud 临时目录指向 Linux Git 的名称适配；冻结四函数经 AST 原样提取，无 Runner 主入口执行。
- Windows cleanup 用离线 double，仅验证 finalizer 行为；Controller 写失败执行实际 catch；并未声称 Windows 进程清理或完整 Controller 入口通过。输出中 sentinel 异常是主动注入且被断言的负例。
- **19 项 Python 静态合同 PASS**（保留原 17 项断言目的，增加整改合同与 push 触发审计）；历史清单对研究基线字节验证，不将历史清单重写为新工具哈希。草案工具 pins 更新，Owner 激活 sentinel 保留。
- 实际 PowerShell 归档 helper 生成 7 个 payload + SHA256SUMS；Python 仅负责将同一集合模拟为 ZIP 并独立逐项验证，**8 文件集合完全一致、7 个 SHA-256 全部匹配、遗漏/额外/重复为零**。见 `artifact-fixture.zip`、`archive-proof.json`、`artifact-zip-proof.json`。这是离线文件集合证明，未执行 GitHub upload-artifact。
- `git diff --check` PASS。`identity-and-preservation.json` 记录所测源码 Blob / SHA-256；最终提交后再次对该 HEAD 验证。

运行命令：

```sh
PATH="/workspace/scratch/git-runtime-bin:$PATH" /workspace/scratch/powershell/opt/microsoft/powershell/7/pwsh -NoProfile -File scripts/testIssue380S1R4P3GitRuntimeRemediation.ps1 -OutputRoot /workspace/scratch/runtime-remediation-final-v2
python scripts/validateIssue380S1R4P3GitRuntimePlan.py
git diff --check
```

## 保全及剩余风险

冻结 Runner Blob `c2cef7e5a6c8f98fe47b3136e1e4017270dc6bfa`、SHA-256 `92e99edccc65749a1394d0a40b19ce303fc6632a2eebc6301c7fc9aa27264ece` 与历史 Hosted HEAD `ccb682a64f88edaf8271a0f34273b3cc43107513` 一致；P1/P2/S0、现有 Workflow、历史证据目录对基线 diff 为空。历史 Run `38297813809` / Job `114973061712` / Attempt 1 未修改、覆盖或重跑。

**NOT RUN**：Windows Git aliases / 管道继承 / 进程树超时（Cases 05–07/09）、Windows CIM 所有权清理、完整 Windows Harness/Controller、GitHub Hosted 执行和实际 upload-artifact、完整 P3 Runner、NSIS、安装器、Service、IPC、数据库。Case 08 的确定性 kill/cleanup 故障注入仍 UNPROVEN；没有完整 Runtime Matrix PASS 声明。现有同步 Kill/Close/Dispose 的总耗时上界仍未证明。未来激活需要 Owner 审核；本轮仅推送已审计无 push 触发的整改分支，不创建或修改 PR。
