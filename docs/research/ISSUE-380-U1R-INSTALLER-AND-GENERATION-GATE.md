# #380-U1R-FIX Gate 0 / Gate 1 增补

当前状态：**Gate 0 PASS；Gate 1 等待唯一一次 Hosted Windows 运行**。本节是 U1R-FIX 的当前权威记录；下方 U1R 报告保留历史事实，包括 Run 38201379732 失败，不覆盖、不删除原始历史证据。

本阶段只修正研究 Harness，未改正式安装器、IPC、Coordinator、Schema、数据库迁移或 PR #386。U1R-FIX 的实现仍不是 #380 生产缺陷修复。

## Gate 0 基线与执行环境

| 对象 | SHA / Tree | 用途 |
|---|---|---|
| 起始研究分支 / U1R HEAD | `dcd1b0ea5e7a2d055bba2a041f022587bcd37316` / `d3616615c27682539fbf2a81e14066856d8d400e` | 允许继续的既有分支基线 |
| Frozen old master | `3275ac7c72df8ae4139a30f16c7f649971324e97` / `009576241cd4e8e2ba12cc644a368314f599faa6` | 旧版 NSIS 生命周期源码 |
| Frozen D1-A | `d260131f9d6ec354f2361e6c7460c79e62e38391` / `4765285668095fb3d7c0dcefd8a8ba5f2bd4fb69` | D1-A NSIS 生命周期及模板生成器 |
| 本地隔离容器 | Ubuntu 22.04 / PowerShell 7 / NSIS `v3.11` / Node `v24.19.0` | 与失败 Hosted run 的 NSIS 版本一致；不是 Windows 产品构建 |

Gate 0 使用 Docker 镜像 `issue380-u1r-gate0-test`（image ID `sha256:7adc66e030cf1a54a0d75ec4fb718fc49f57ebd63f6fc3418ec5cb3d69284e62`）。NSIS source tag commit 为 `7359413009afd4f0fff472d841fc2f2cc0e0a5f8`；Tauri NSIS data archive SHA-256 为 `c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1`；Tauri utility plugin SHA-256 为 `5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709`。旧版生成器在相同 makensis 3.11 环境下以 `label "u1r_file_trace_11_had_error:" already declared in function` 确定失败；这是对已知 Run 38201379732 错误的独立离线复现。

## Gate 0 结果

`scripts/testIssue380U1RNsisGate0.ps1` 从两个固定 SHA 的归档源码建立临时副本，执行候选 anchor 插桩、模板生成器检查、完整 `installer-lifecycle-wrapper.nsh` 的离线 makensis 编译及安全谓词 byte-preservation 检查。状态与 hashes 见 [`u1r-gate0-evidence.json`](evidence/issue-380-u1r-fix/gate0/u1r-gate0-evidence.json)。

- **旧生成器负向回归：PASS。** 它以重复的 `u1r_file_trace_11_had_error` 标签失败，完整原始日志为 [`legacy-generator-expected-failure.log`](evidence/issue-380-u1r-fix/gate0/legacy-generator-expected-failure.log)。
- **新 Harness 编译：PASS。** NSIS 3.11 编译旧版完整生命周期包装器，exit code 0；编译 D1-A 完整生命周期包装器与真实生成器输出的模板 DetailPrint fixture，exit code 0。日志和编译输入分别为 `old-new-generator-nsis-3.11.log` / `old-generated-macro-expansion-fixture.nsi` 与 `d1a-new-generator-nsis-3.11.log` / `d1a-generated-macro-expansion-fixture.nsi`。
- **生成器策略：** 宏内 per-line 数字标签会冲突；测试 Harness 现在保留观察行作为 NSIS `DetailPrint`，依靠 `/LOG` 采集。共享宏可能在 installer 与 uninstaller 上下文展开，不安全的 `Call`/文件写函数可能进入错误的 namespace；detail-only 插桩不增添 label、function、stack 操作、FileOpen 或错误标记更改。旧卸载器的 `/LOG` 参数仅在 isolated generated-template 副本增加。
- **安全谓词保持：PASS。** 移除测试 DetailPrint 后，old 与 D1-A 的 `installer-hooks.nsh` 各自与冻结源码逐字节相同，哈希均为 `ee3139d253efa4833e7f93105003a8813a5d434f9176efa0522cca40a0ea55cf`。Registry typed query/ExpectedSize、Preview Handler、Service ImagePath/ownership 检查保持原文；没有删除、绕过或弱化正式安全条件。
- **U1R_FOCUS 双候选注入：PASS。** 测试验证 old 和 D1-A 均应用各自真实代码锚点；旧客户端启动 gate 在 Coordinator 构造前，目标 Rebuild 在 gate 释放后、Coordinator 启动后。
- **Harness 数据前置条件：PASS。** JSON `-AsHashtable` 保留空注册表值名；NSIS 与同路径场景要求 canonical 安装目录已不存在并 fail-closed，不再删除未知目录；旧客户端运行中不读取它可能锁住的 SQLite 文件，测试先做 pre-snapshot，在进程退出后做 post-snapshot。
- Gate 0 没运行 Cargo/Rust、UI、安装包构建或完整 CI。实际 D1-A `buildWindowsPackage.mjs` 的注入代码通过 Node `--check` 与 converter contract smoke，Node 版本和输出源 hash 在 evidence JSON 中。

Gate 0 没有执行 Hosted Windows 安装或同路径跨代请求，不能据此判断 NSIS exit 2 根因，也不能声称旧进程已被隔离。Gate 1 只允许由本分支触发一次；运行后在本增补中记录 exact run/job/artifact/hash、根因谓词和状态变化。若这次 run 因 Harness 无法完成，则不再重跑，提交 `#380-U1R HARNESS_BLOCKED`。

---

# #380-U1R 安装器根因与代际隔离研究报告

状态：**BLOCKED — NSIS 首个生产失败条件与旧进程代际隔离均未证明**。最终窄范围 Hosted run 的 Harness 在 NSIS 编译阶段失败，目标场景没有运行。

本报告是隔离研究与测试 Harness 证据，不代表 #380 生产缺陷已修复。没有修改 #386 已验收生产实现、正式 Schema、IPC、NSIS 或产品版本。

授权来源：[#380 Owner U1 终审](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6115136508)。

#380 中 `Coordinator::rebuild()` 在可信 Service Admission 前修改源状态、stale 旧数据的持久化问题仍未由 D1-A/U1R 解决；本阶段只定义混合代际安装与旧进程写入边界。

## 1. 基线与证据边界

| 对象 | HEAD | Tree | 用途 |
|---|---|---|---|
| U1R Harness 起始提交 | `09b41695d9ff86825beff8a4b8e7519c032775c0` | `9d4543af73cee569300b7dafcf2963307563ee92` | U1 历史研究分支基线 |
| 冻结旧版 master | `3275ac7c72df8ae4139a30f16c7f649971324e97` | `009576241cd4e8e2ba12cc644a368314f599faa6` | 旧客户端、Service、旧 NSIS 安装器 |
| 冻结 D1-A | `d260131f9d6ec354f2361e6c7460c79e62e38391` | `4765285668095fb3d7c0dcefd8a8ba5f2bd4fb69` | 新客户端、Service、NSIS 升级目标 |
| U1R 初版 Hosted Windows 测试提交 | `061fe79c086e9fb91dd71576821dc843ae8d7384` | `3351ff8e3dea03f03479f7492e449c831a3993a9` | Run 38191176125；NSIS 诊断 Harness 在编译时失败 |
| U1R trace 修复测试提交 | `00f6d3bd2118c7b3485b47ca6cdc7444f9c4276b` | `a4db0578fa900cc8c0e717486816893aa79a1a31` | Run 38193395291；PowerShell Harness 插值失败 |
| U1R 首次可执行测试提交 | `3e66debd56952572c6289efcb989329bc4b744b1` | `f507518a6ad2f0876123fd05720a4c20fafd3084` | Run 38193683826；Harness 未记录内部 NSIS predicate，旧客户端未发出 Rebuild |
| U1R trace/request Harness 修正提交 | `75e0490bccaaa0e05b8c727b941baf889aa44e02` | `30e527fbe6eaa85b24d1229330f255d251d0a6a7` | Run 38197011540；旧 QA 候选编译失败，场景未运行 |
| U1R 复测提交 | `86526e675eaf471ca072a88e19f6b201ac267ba2` | `680a25b3c980ad2c81ef6fd5b8f6ab6550e2c51d` | Run 38197506250；在旧 QA 构建期间取消，未运行场景 |
| U1R 定向 Harness 提交 | `5998dc7a9c88333e3518f3a1845ca49b948acd4c` | `0d57101c918c46e099fcd210b256b9ce54f9fc9c` | Run 38198147350；旧/D1-A QA candidates 与安装包构建成功，但 Harness 在加载 Direct fallback JSON 时 fatal；两个目标场景均未启动 |
| U1R JSON Harness 修正提交 | `ebce7130e7e0eedc3bc17106a1ba8462889613ad` | `da1c6ebceea2dcc192dc36ecd293f553c274c069` | Run 38199772185；JSON 解析已修正，但 Focus 模式漏掉候选插桩并在 NSIS 前留有 canonical 目录；无跨代结论 |
| U1R Focus 插桩/清理修正提交 | `8387a96e624e6b4b3de16848b6089929d19d0a50` | `5c74d097c673a09a9f299062d2405360f6cac989` | Run 38201379732；旧候选 NSIS 测试构建被重复标签 Harness 错误阻断，未运行目标场景 |
| 最终报告提交 | 由最终交付回执记录 | 同提交 Tree | 研究文档与原始证据，不包含产品改动 |

U1R 分支从 U1 研究分支延续，不包含 D1-A 生产代码；D1-A 只作为测试时建立的独立 frozen candidate worktree。候选构建从各冻结 SHA 建立干净 detached worktree，并逐一检查 SHA/Tree。测试专用插桩仅位于候选 worktree：NSIS `DetailPrint`/日志参数、QA 进程代际标记和可控启动闸门。正式检查逻辑保持启用，候选 worktree 在采集 instrumentation patch 后恢复并验证干净。安装器在一次性 Hosted Windows Runner 上使用 canonical Program Files 路径；SQLite profiles、卷和 managed/AI 行均在任务临时目录中创建，不访问真实用户数据。U1 旧证据保留在 `docs/research/evidence/issue-380-u1/`，本轮原始工件放在 `docs/research/evidence/issue-380-u1r/`。

## 2. Hosted Windows 运行与候选身份

- 首次可执行工作流：Run [38193683826](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38193683826)，exact test commit `3e66debd56952572c6289efcb989329bc4b744b1`，Windows step 按场景失败退出并上传完整 artifact。复核发现其 `EXIT_2_ROOT_CAUSE_LOCATED` 是 Harness 误标：trace 只有 Detect 函数的总结果；同路径测试成功替换 EXE，但在旧客户端发出请求前试图读取被进程打开的 SQLite 文件，导致 Harness 中止。
- Run [38197011540](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38197011540) 使用 Harness 修正提交 `75e0490bccaaa0e05b8c727b941baf889aa44e02`、Tree `30e527fbe6eaa85b24d1229330f255d251d0a6a7`，但旧 QA 候选编译失败：测试插桩在 binary crate 错把 library 模块路径写成 `crate::global_index`。没有构建 D1-A，也没有运行 Windows 场景；artifact 保留。Run [38197506250](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38197506250) 随后在旧 QA 候选构建期间取消，避免其进入旧版 Direct fallback；未运行任何场景。Run [38198147350](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38198147350) 使用提交 `5998dc7a9c88333e3518f3a1845ca49b948acd4c`、Tree `0d57101c918c46e099fcd210b256b9ce54f9fc9c` 成功构建并提取两代候选安装包，但 Harness 在 `ConvertFrom-Json` 读取含空属性名的已归档注册表快照时 fatal；NSIS 与同路径探针均未开始。artifact 中错误为 PowerShell 要求 `-AsHashtable`。Run 38199772185 使用修正提交 `ebce7130e7e0eedc3bc17106a1ba8462889613ad`、Tree `da1c6ebceea2dcc192dc36ecd293f553c274c069` 可解析 baseline JSON，但因 `Build-Candidate` 没有对 `U1R_FOCUS` 注入 U1R instrumentation，旧 QA 客户端没有停在 pre-coordinator gate；它和旧 Service 在 file replacement 前运行了同代自动恢复。该恢复改了合成 DB 的 source/Managed/AI 状态，但无新 Service/替换身份绑定，因此不是跨代或 Direct fallback 证据。早退路径也没有删除 canonical install 目录，NSIS case 在安全 precondition 处拒绝运行。Run [38201379732](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38201379732) 使用 Focus 修正提交 `8387a96e624e6b4b3de16848b6089929d19d0a50`、Tree `5c74d097c673a09a9f299062d2405360f6cac989`，重建冻结旧版候选时在 `makensis` 阶段失败：测试插桩在可重复展开的 `ZC_REQUIRE_PRODUCT_STRING` 宏内生成重复的 `u1r_file_trace_6_had_error` 标签。旧版生产 EXE 已编译，但旧 NSIS 包未生成，D1-A 尚未构建，NSIS 安装与同路径探针均未运行。该 Harness 失败 artifact 已归档；按本阶段范围不再启动 Hosted 重试。
- 前置 Harness 尝试保留：Run [38191176125](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38191176125) 在 D1-A NSIS 编译时遇到测试注入的 `LogSet on` 与当前 makensis `NSIS_CONFIG_LOG` 不兼容；Run [38193395291](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38193395291) 在旧候选构建前，PowerShell 将 trace 生成器里的 `$R8` 错当变量。两次均未执行安装或同路径/Direct 场景，不能当作产品行为证据，原始 artifacts 保留。
- Hosted Windows 版本、架构、PowerShell：Run 38201379732 为 Windows Server 2025 Datacenter `10.0.26100`、64-bit、PowerShell `7.6.6`；Run 38193395291 的 runner 元数据一致。
- Run 38193683826 旧候选：源码 `3275ac7c72df8ae4139a30f16c7f649971324e97` / Tree `009576241cd4e8e2ba12cc644a368314f599faa6`；release EXE SHA-256 `c1c74cfc24c320d68165029f76a616abe1a9600db98f2fe978af04c24fd07bbe`；提取后 NSIS payload/实际旧安装 EXE SHA-256 `a190f909d54e219274d8928e5e7e5644751dd3f547e69c44144ce9dc25dc5bef`；QA EXE SHA-256 `97cc082ad501e6055f0a8fee11e21d3ddcc4312f159768179eeb4701c04ef3b1`；旧 NSIS installer SHA-256 `3be76b6c8df60855e46e5de3130d09812a939a5267bf17eee8ec6c309ee924ca`。
- Run 38193683826 D1-A 候选：源码 `d260131f9d6ec354f2361e6c7460c79e62e38391` / Tree `4765285668095fb3d7c0dcefd8a8ba5f2bd4fb69`；release EXE SHA-256 `5c8c263e78146fed9d539e4523012de366d523bcbe72271344873b36ae378eb3`；提取后 NSIS payload SHA-256 `6c8929fc197ae76d953f054cc52efb8f6f705ac4215782e3578b6fa02c75a11a`；QA EXE SHA-256 `b233c181b38fa129f8f6f4ac9a76ff1673989e6c942f76a26ea5886714dbe580`；D1-A diagnostic installer SHA-256 `22c78122959cecb97da8997197f6edefd5949c54e1f3f1e4307e731bfc4edfb6`。NSIS 提取 payload SHA 与 release EXE SHA 不同，报告将它们按产物阶段分别记录。
- Run 38198147350 虽没有启动场景，但两代候选构建与 NSIS payload extraction 成功，额外固定了这次 Harness-fatal 前的 binary 身份：旧 QA `zen-canvas-qa.exe` SHA-256 `3e79d5113772b8e3969ca6361401eb6063fb3d0b13fefbdd8c2979b23e724c78`、旧 QA probe `2586c5f815f8599232a61c35d29251c4630bdc16ee61fc4a09099213246e79ec`、旧 production build `ce55baf1b2ebd6e2cb9beb9a74b9e7a51ef0efa24b2de7907503a527f7996ca7`、旧 NSIS 提取 payload `3f87fee6dfcdc9916f67f32502c5992fe47c6edb974f45704e7700468663d817`、旧测试安装器 `2afccf98dfc782c9eef009779e6910ab6a1ea89bc9b18d54f5a6d98f4280df5c`；D1-A QA `zen-canvas-qa.exe` `9df26b6c021df6368964e6620b1969045dd70674925a91e19d51796492d7d120`、D1-A QA probe `5f377a15ce04f80139abe956781dc8f3ed3268ef4d6738215a903750bf84ccdf`、D1-A production build `1f845797d8e78088c1441123ea4a71f7cf2c32748118a86085685df7995eca98`、D1-A NSIS 提取 payload `4befedf6864559f7833111a092996a3a15947a26c2597cba51fe4f49a0f54ed4`、D1-A 测试安装器 `d432404c54023dc0d5d90f92b8f9f8c46e67677c888b401c3519b757f63aef7e`。这些是候选构建证据而非成功兼容测试；两代文件与 Cargo/Tauri/package 均报告 `0.1.40`、identifier `com.startlan.zencanvas`。
- Run 38199772185 候选身份来自 Harness JSON：旧代 QA / release / extracted NSIS payload / installer SHA-256 为 `1fa7b28d1e4971c323b96f507842f87e5903dbe27991e98719754dfd2d99472a` / `8eeb3b640072f230d5a411fb601593ce6025e913536c8f1ca6616f55c1eb7bec` / `9500c83d88eb0719646f255e36bffdf4a55eea0577df9a8127c82c33ab2334b7` / `79d1ae9582894ba897f2ecff6922d5b4afa9e099cc960291e3610c53ec375f4d`；D1-A 对应 SHA-256 为 `f2a2ae0712cb81a3d1d255bf62702139c145d4716ee09f07d2bba13a0edbe710` / `5b127faeb85ca5263f22da4ca54d8b0546483a9645d9a1d393f98e97c23c7cc4` / `54e5b0b353db2dc937a9d26f47fe98d958de619426960990f8a53a601e40b0bf` / `40a7fa5c27097577cbcd877829906ee071b1fe40a1e3bd6368b9edb63c25c410`。这些是已构建候选身份，不是成功场景证明。Run 38201379732 的 JSON 在 NSIS 编译前 fatal，`candidates={}`；只记录固定的旧 master SHA/Tree，D1-A `sourceStatus=UNBUILT`，没有该次 release/installer 二进制 SHA。
- 两候选包/Cargo/Tauri 版本均为 `0.1.40`，应用标识均为 `com.startlan.zencanvas`；版本字符串不能区分二进制代际。
- 针对性步骤：`.github/workflows/issue-380-u1r-hosted-windows.yml::Run focused U1R Hosted Windows probes only`，调用 `scripts/issue380U1HostedWindows.ps1 -Mode U1R_FOCUS`；目标为 `Invoke-U1RSamePathAndDirectTests -IncludeDirectFallback:$false` 与 `Invoke-U1RInstallerDiagnostics`。Direct fallback 仅载入 Run 38193683826 的原始证据，不重跑。Run 38201379732 CI 为 failure，失败发生在 `old-nsis-package` 构建：`makensis` 报 `label "u1r_file_trace_6_had_error:" already declared in function`，`candidates={}`，目标场景未开始。

U1R 没有重跑 U1 的同代或不同安装路径矩阵，也没有启动 100k/500k Benchmark 或完整 CI。

本阶段已有 Hosted Windows 原始 ZIP 均保留在 evidence tree，下载后 SHA-256 如下：

| Run | Artifact ZIP | SHA-256 | 结果 |
|---|---|---|---|
| 38191176125 | `issue-380-u1r-hosted-windows-38191176125-1.zip` | `b227eed29136f3b939ae6e4ded04c4be23c9d433bb7a892903320bed4df91da6` | NSIS instrumentation compile error；no scenarios |
| 38193395291 | `issue-380-u1r-hosted-windows-38193395291-1.zip` | `0d159ee1c803ddcedccbc9b8e6ba5dd138d6ad2e00170a8950c1adfdcf7c9a7c` | PowerShell trace interpolation error；no scenarios |
| 38193683826 | `issue-380-u1r-hosted-windows-38193683826-1.zip` | `b73456624dfa431d43ad0bed87d3a280c64217386aa1a1019ca48b6c2cc15754` | First executable U1R run; key evidence and Harness blind spots retained |
| 38197011540 | `issue-380-u1r-hosted-windows-38197011540-1.zip` | `dbf15b7ea357c7e40960551c25c84d10c73bccefee70da3d03aa973655fc3c2b` | old QA candidate compile failure; no scenarios |
| 38197506250 | `issue-380-u1r-hosted-windows-38197506250-1.zip` | `50fad30f1a04c8cad0ab0e46d8c0231291ea4b73cd868f242a67cb652f4949d1` | Canceled at candidate build; no scenarios |
| 38198147350 | `issue-380-u1r-hosted-windows-38198147350-1.zip` | `60cd7770ee353c2e17e13136ab1481b9d974e3b95b4a0ab11326f04dc703c27d` | Candidate builds succeeded; `ConvertFrom-Json` empty-property error occurred before scenarios |
| 38199772185 | `issue-380-u1r-hosted-windows-38199772185-1.zip` | `39b52041c66505694a643b7857798e63a46dbd78949839845db77f04e52922f2` | JSON 修正生效；Focus 漏掉插桩且 NSIS precondition 拒绝运行，非产品证据 |
| 38201379732 | `issue-380-u1r-hosted-windows-38201379732-1.zip` | `c24aad0df7747d3472ddeb172a7269cc7616b98656d98e202c01bde2413ed10e` | old candidate `makensis` failure：重复 Harness 标签；没有安装或同路径场景 |

已接受的 U1 运行 [`38178838324`](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38178838324) 的矩阵作为历史上下文：旧+旧、D1-A+D1-A 为 PASS；旧客户端+新 Service（不同路径）因 path check 被拒绝；新客户端+旧 Service（不同路径）fail-closed；同路径混合代际当时是 `UNVERIFIED`，由本轮专门补测。旧客户端在 Service absent 时的 Direct fallback 已观察到 Global Entries/`entry_count` 从 4 增至 5、FTS 新标记由 0 变 1、journal cursor 从 712 推进至 1288 并更新同步时间。原始记录保存在旧 U1 证据目录；本轮只对它增加 Managed/AI 关联数据与非空 checkpoint 覆盖。

| 组合 | 已接受 U1 证据 | U1R 状态 |
|---|---|---|
| 旧客户端 + 旧 Service | 同路径 PASS | 不重跑；仅作为 rich fixture 的数据准备基线，不再做兼容性资格测试 |
| D1-A 客户端 + D1-A Service | 同路径 PASS | 未重跑 |
| 旧客户端 + D1-A Service | 不同路径拒绝；同路径专测 | Run 38193683826 证明文件可在旧进程存活时原子替换，但请求未触发；Run 38197011540 编译失败；Run 38197506250 取消；Run 38198147350 JSON fatal；Run 38199772185 为未插桩的 old+old 自动恢复；Run 38201379732 在旧候选 NSIS Harness 编译阶段失败。U1R 同路径跨代 request 仍为 `UNVERIFIED` |
| D1-A 客户端 + 旧 Service | 不同路径 fail-closed；同路径未覆盖 | UNVERIFIED（不重复 U1 矩阵） |
| 旧客户端 + Service 暂不可达 | 旧版 Direct fallback 已观察到活动索引与 checkpoint 改变 | U1R rich fixture 再次证明 source health/timestamps 与 Managed/AI 关联行会改变；entry/checkpoint 保持 |

## 3. NSIS 原位升级首个失败条件

### 代码路径

生产检查证据来自冻结候选：

旧 master 与 D1-A 的精确 Git diff 只有 `docs/research/issue-380-d1a-mixed-version-contract.md`、`src-tauri/src/global_index/coordinator.rs`、`src-tauri/src/global_index/coordinator_recovery_tests.rs`、`src-tauri/src/global_index/windows/mod.rs`、`src-tauri/src/global_index/windows/recovery_qa.rs`、`src-tauri/src/global_index/windows/service.rs`、`src-tauri/src/global_index/windows/service_errors.rs`、`src-tauri/src/global_index/windows/service_host.rs`；没有 installer hooks/template、schema 或产品版本文件。安装器 root cause 所涉 NSIS 检查逻辑在两个候选中是同一份生产代码。

- `src-tauri/windows/tauri-2.11.2-installer.upstream.nsi:304-371::PageLeaveReinstall` 在普通原位更新路径读取旧 `UninstallString`，`ExecWait` 旧卸载器，并在非零 exit code 或旧主 EXE 仍存在时中止；`/UPDATE` 会跳过旧卸载器分支。
- `src-tauri/windows/installer-hooks.nsh:403-478::DetectZenCanvasPreexistingProduct` 校验 64-bit HKLM 的 `SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Zen Canvas`、产品字段、`EstimatedSize`、`SOFTWARE\startlan\Zen Canvas` 和 `uninstall.exe`。`EstimatedSize` 期望值按当前 `$INSTDIR` 重算：目录 size 排除 `uninstall.exe`，再加 NSIS `${ESTIMATEDSIZE}`。
- 同文件 `:252-259::ValidateZenCanvasPreviewCore` 校验 Preview CLSID、AppID、ThreadingModel、PreviewHandlers 列表与 InprocServer32；`:523-537::ValidateZenCanvasIndexServiceOwnership` 检查 `SYSTEM\CurrentControlSet\Services\ZenCanvasGlobalIndex\ImagePath` 必须是精确 `REG_EXPAND_SZ` 值，路径为当前 canonical 安装 EXE 加 `--index-service`。
- 旧卸载器 `installer-hooks.nsh:2223-2269::NSIS_HOOK_PREUNINSTALL` 先运行桌面进程门，再检查 Preview、Service 所有权和删除前产品证据，然后执行 Preview quiesce 与 Service stop。
- D1-A `src-tauri/windows/installer-lifecycle-final.nsh:106-132::ZCPrepareInstallLifecycleFinal` 在文件变更前停止捕获的 Service、关闭桌面进程、撤销 Preview Handler；失败时返回错误并执行可逆恢复。它只会在旧卸载流程返回且主 EXE 已删除后运行。

### 实际结果

- Run 38193683826：旧版生产安装器全新安装 exit 0；随后固定的 D1-A diagnostic NSIS package 对旧安装执行普通原位安装，外层安装器 exit 2。旧生产 EXE SHA-256 `a190f909…c25dc5bef` 与安装前一致，旧 Service 仍为 Running（PID 3044），旧桌面进程仍在（PID 1324）；安装阶段没有观察到旧 Service Stop 或文件替换。
- 源码与唯一可读取 trace 将首个可证明的失败定位到 `NSIS_HOOK_PREINSTALL → ValidateZenCanvasPreexistingProduct → DetectZenCanvasPreexistingProduct`：调用返回 `product=2 valid=0 uninstaller=1` 后执行 `Abort`。这是写文件/旧 Service 停止前的产品完整性 gate。trace SHA-256 `1220ee7d73f6c9b24b24356e4d31f56df78f690fad78f1905337486b293dd8df`，仅含上述总结果；首个内部 Registry/EstimatedSize predicate **尚未证明**。原始 artifact 的 `firstFailingValidationTrace=null`。本 run 的 Analyzer 把总结果误标成 `ROOT_CAUSE_LOCATED`，后续 Harness 已修正为必须有首个内部 mismatch trace 才能置位。
- 普通 `/S` 运行没有 old-uninstaller log，transition samples 中没有 nested uninstaller PID，且 old EXE/Service/注册表未变化；证据符合“在 `NSIS_HOOK_PREINSTALL` 产品检查拒绝”的路径。不能把外层 exit 2 描述成旧卸载器 failure。
- 安装前 64-bit HKLM 快照在触发条件上没有表面异常：ARP 的 `DisplayName=Zen Canvas`、`MainBinaryName=zen-canvas.exe`、`DisplayIcon`/`InstallLocation`/`UninstallString` 均指向规范路径，`DisplayVersion=0.1.40`、`Publisher=startlan`、`NoModify=1`、`NoRepair=1`、`EstimatedSize` 为 DWORD `35150`；`SOFTWARE\startlan\Zen Canvas` 默认字符串值为 `C:\Program Files\Zen Canvas`；Preview CLSID、AppID、Inproc DLL、`ThreadingModel=Apartment` 和 PreviewHandlers 项存在且匹配；Service `ImagePath` 为 `REG_EXPAND_SZ`，值为 `"C:\Program Files\Zen Canvas\zen-canvas.exe" --index-service`。但当前 trace 没记录内部类型/内容比较结果及 `GetSize + ${ESTIMATEDSIZE}` 的期望值，故不能由快照反推出首个失败项。
- `/S /UPDATE` 也 exit 2，trace 同样只到 `product=2 valid=0 uninstaller=1`。该路径绕过 `PageLeaveReinstall` 的旧卸载分支，仍被 preinstall product gate 拒绝；它不构成修复或安全绕行方案。
- 旧用户隔离 Schema 37 DB 的 `databaseBeforeInstaller` 与 `databaseAfterNoUpdateAttempt` 原始 JSON **逐字段完全相同**。数据包括 source health/checkpoint/timestamps/entry_count、4 个活动且 0 个 stale Global Entries、FTS hit 1、一个 Managed Scope/Entry、AI Analysis、AI Job/Job Item。进程退出后的快照也与安装前一致。原 Harness 的 `databaseStatePreserved.preserved=false` 与所有逐项比较布尔值 `true` 矛盾；Python 独立对比两个原始 JSON 得到完全相同。Run 38199772185 未运行 NSIS，故没有新的失败安装快照验证已修正的布尔汇总。本次失败安装实测仍只证明该次旧 U1 数据库快照在 Run 38193683826 中保持不变。

Run 38197506250 在旧 QA 构建期间取消，因此没有生成 ARP predicate trace。Run 38198147350 在任何 NSIS 场景前因 Harness JSON 解析失败退出；Run 38199772185 被残留的 canonical install directory precondition 拒绝；Run 38201379732 则在构建旧候选安装器时，由 Harness 生成重复 NSIS label 而失败。三者都没有运行目标 NSIS 安装或生成产品检查 trace。Run 38193683826 的外层 exit code 2 仍只证明 preinstall 产品完整性 gate 返回 Abort；首个 Registry/EstimatedSize/Preview/Service predicate 未定位，不能从 exit code 推断根因。

## 4. 同路径进程代际与 Direct fallback

### 同路径新旧进程

Harness 在一次性 Hosted Windows 上先由旧代 QA EXE 从 canonical Program Files 路径启动进程并打开隔离 Schema 37 数据库，再通过实际 SCM 停止并注销旧 Service，尝试原子替换同一路径的 EXE，之后从此路径通过 SCM 启动 D1-A QA Service；之后验证真实 v3 named-pipe request/event 路径，不复制 D0 状态机。旧版 `Database::open` 创建 `min_idle=1` 的 SQLite connection pool；QA gate 位于 DB open/coordinator 初始化后，因此进程等待时至少持有已打开的空闲池连接，不持有测试代码建立的写事务。替换若被 Windows 拒绝，Harness 不终止旧进程、不强制替换，并记录 `UNVERIFIED`。

Run 38193683826 原始证据：旧 QA 客户端 PID 9024，创建时间 `2026-10-12T02:16:10.712862Z`，启动时磁盘 QA EXE SHA-256 `97cc082a…04ef3b1`；stdout/stderr 的 `u1r_process_generation=old-master-3275ac7-test-instrumented` 确认是旧代代码。旧 Service PID 7916，ImagePath 指向相同 canonical EXE，进程创建时间 `2026-10-12T02:16:10.467341Z`；Stop 并注销后，PowerShell `[IO.File]::Replace` 在旧客户端 PID 9024 仍活着时成功：备份 SHA 仍为旧版，canonical 路径变成 D1-A QA EXE SHA `b233c181…14dbe580`。SCM 随后从同一路径启动 D1-A Service PID 9232，创建时间 `2026-10-12T02:16:13.203744Z`，磁盘 SHA 与 D1-A QA EXE 一致。旧客户端在替换后仍为同一 PID/创建时间。

但该 run 没有触发旧客户端请求：Service identity accept、Rebuild request、claim、source validation、request-ID correlation 和 route-complete 均为 false；旧客户端 trace 只有 `coordinator_cycle`，Service 没有可归档请求事件。Harness 在旧客户端仍持有 SQLite 连接时试图 `Get-FileHash` 主 DB，抛 `IOException: The process cannot access the file ... because it is being used by another process`。进程退出后用隔离 profile snapshot 读取，DB 仍为 `rebuild_required`、cursor `664`、活动条目 4、stale 0、FTS 1，Managed/AI 行与启动前一致。这不是身份校验通过或拒绝的证明，只证明 Windows 允许替换同一路径的 EXE，同时旧进程仍运行并持有 SQLite 连接。

Run 38197506250 在旧 QA 构建期间取消，未执行请求。Run 38198147350 在读取此前直接 fallback evidence 时停止，未启动旧客户端或新 Service。Run 38199772185 的 Focus binary 没有插入 gate/forced-Rebuild instrumentation，所以只观察到 file replacement 前同代旧客户端到旧 Service 的 automatic recovery。逻辑快照显示 source `rebuild_required → ready`，索引时间 `1791776461 → 1791776474`，Managed Entry `enabled=1 → 0`，AI Analysis `completed → stale`、Job/Item `pending → stale`；entries=4、active=4、stale=0、entry_count=4、FTS hit=1、journal ID/cursor=`134362500611528070`/`664` 不变。主 DB SHA-256 前后均为 `bac6e934…5b7a7a`，WAL/SHM 快照在关闭客户端后归档。由于没有 `fileReplace`/新 Service/PID/request identity 证据，这次同代自动恢复数据变化不能当作新 Service 接受旧客户端或旧版 Direct fallback 的证明。Run 38201379732 在旧候选 NSIS Harness 编译阶段失败，未启动旧客户端、新 Service 或 SQLite same-path 场景。故目前只有 Run 38193683826 证明 Windows 允许 canonical EXE 原子替换且旧 PID/创建时间仍存活，但没有证据说明该旧客户端是否被新 Service 接受；代际隔离仍为 `UNVERIFIED`。

D1-A Service 的 `PipeSink` 将 Provider 结果经 named pipe 送回调用端，Client `windows/mod.rs::apply_validated_index_event` 再调用本地 `GlobalIndexSink`/Repository 写数据库。因此这组证据必须用旧 Client PID/启动时间与其隔离 DB 快照建立关联；新 Service PID 自身不是该用户 SQLite 写者。

该 QA 场景的旧 Client 和 D1-A Service 来自精确冻结源码的 `native-qa` 可执行文件，含仅测试启用的代际/操作 trace；不是安装包里的生产 EXE。原位安装诊断运行冻结源码构建的 NSIS 安装包，只有测试候选的 NSIS 日志与 DetailPrint 插桩；检查条件没有删改。报告分别列出 package EXE SHA 与 QA EXE SHA，不将 QA 二进制伪称为 release payload。

### 旧版 Direct fallback

Run 38193683826 的旧 QA 客户端 PID 10176、源码代际与同路径相同；旧 Service 停止且确认 pipe 不存在后，真实旧 provider trace 证明先遇到 `u1r_service_transport_error` 再进入 `u1r_direct_fallback_started`，MFT 返回。用旧版实际扫描建立的 checkpoint `journal_id=134362449615844939`、cursor `664`、全量/增量索引时间 `1791771362` 作为基线；关联 Managed/AI 行为测试专用 sentinel。

快照前后可复核到：Global Entries=4、active=4、stale=0、`entry_count=4`、marker FTS hit=1、checkpoint ID/cursor 不变；source `rebuild_required → ready`，`last_full_index_at` 与 `last_incremental_sync_at` 从 `1791771362` 推进到 `1791772218`，`updated_at` 同步推进；Managed Entry `enabled=1 → 0`；AI Analysis `completed → stale`、AI Job 与 Item `pending → stale`，错误为 `global_entry_stale`，job 完成时间推进。Managed Scope 与行数不变。故这次 rich fixture 未新增 Global Entry 或推进 checkpoint，但 Direct fallback 仍改写索引健康/时间及用户关联状态。主 DB 文件 SHA-256 前后相同不能证明未写入：SQLite 允许变更留在 WAL；本次归档了完成后的 `zen-canvas.sqlite3-wal` 与 `-shm` 哈希，判定以事务快照的字段变化为准。旧 U1 简单 fixture 的真实旧 fallback 另有记录：entries `4→5`、FTS hit `0→1`、cursor `712→1288`，见原 U1 artifact。

结果需要逐项覆盖 source health、`entry_count`、活动/过期 Global Entries、FTS、`journal_id`/cursor、完整及增量索引时间、Managed 与 AI 关联行、数据库 SHA-256。失败或未完成的观察不得标成 PASS。

## 5. A/B/C U2 升级安全设计比较

A、B、C 是互补防线，不互斥。仅实施其中一项都无法覆盖旧进程仍可运行这一事实。

| 方向 | 主要保护 | 单独使用时的缺口 | 代价/约束 |
|---|---|---|---|
| A. IPC v4 代际握手 + 新客户端无 mutating Direct fallback | 新 Service 拒绝无代际凭据的旧请求；新客户端对缺失/旧 Service fail-closed | 旧 EXE 不讲 v4，可在 v3 pipe 不存在时走旧 Direct fallback；运行中的旧进程也可能仍在同一路径 | 协议和混合版本升级合同；同一 v3 Service 要停止服务兼容或明确双协议安全策略 |
| B. 安装维护 + Stop/Wait + repair-forward | 正常升级时阻止文件替换期间的新旧 Service/桌面并发写入 | 旧 EXE 不读维护标记；ACL/Mutex 不会约束不参与协议的旧进程；卡住的进程需安全拒绝升级 | 要覆盖所有登录 Session、Service PID、UI 进程、SQLite 连接和安装中断恢复 |
| C. 数据库 generation/path 隔离 | 即使旧进程残留，它也只能触及旧代文件，不能写新代活动 DB | 旧客户端在旧 DB 上继续写会造成数据分叉；需迁移/发布状态机，不会自动合并并发变化 | SQLite backup staging、额外磁盘空间、#381 Schema/迁移协调、首写之后只 repair-forward |

推荐 U2 将 A+B+C 作为单一 Owner 批准的升级合同：B 提供正常升级顺序，A 拒绝新代对旧协议的未授权请求并取消新端 Direct 重试，C 在旧进程漏过 B 时守住新活动数据库。

### A. IPC v4 代际握手与无 mutating Direct fallback

D1-A 精确源码的 `src-tauri/src/global_index/windows/service.rs` 仍使用 `IPC_PROTOCOL_VERSION = 3` 和 `ZenCanvas.GlobalIndex.v3`；旧 master 也用同一代协议及 pipe。其 `named_pipe::validate_pipe_client` 校验 `GetNamedPipeClientProcessId` 得到的交互 session、client image path 与 Service `current_exe()` 规范化后完全相同，但不校验进程映像内容哈希、签名、构建版本或安装代际。因而在同一安装路径替换磁盘文件后，旧进程仍能满足路径相等检查。

D1-A 校验 request/response protocol、request ID、Source 身份、Admission 与事件顺序，但没有二进制构建代际证明。`service_host.rs::handle_index_request` 先取得内存 busy claim，再重新发现并验证启用来源身份，发送 `Sources` Admission，之后才设置 indexing 并运行 Provider。`service_errors.rs` 把 ServiceBusy、SQLite DatabaseBusy、Provider/Permission 和未知结果分成不同类型；明确 busy 不会走 Direct fallback。客户端 `run_with_fallback_using` 对 `NotSent` 仍先写 source unavailable 再调用 Direct provider；未知执行结果则不重复执行 Direct。D1-A 与旧版应用标识 `com.startlan.zencanvas`、产品/Tauri/Cargo 版本 `0.1.40` 以及 Schema 37 相同，无法依赖版本字符串区分。

U2-A 至少需要仅供新协议使用的 `\\.\pipe\ZenCanvas.GlobalIndex.v4`，并在 v4 request/response/frame 上携带 `protocol_version=4`、`request_id`、不可混淆的代际字段。建议字段及方向为：Client request 携带 `client_build_id`、`client_generation_id`、`expected_service_generation_id`、`database_generation_id`；Service Admission/response 回送 `service_build_id`、`service_generation_id`、`database_generation_id`、所接受的 capability 和同一 `request_id`；每个后续 event frame 绑定同一 `request_id` 与实际 `service_generation_id`/`database_generation_id`；Admission 同时绑定 `source_id`/stable volume identity。generation 值不得只由产品版本字符串推导（本次新旧版同为 0.1.40），且字段与身份凭据生成方式由 Owner 在协议设计中冻结。

服务端在 claim、Admission 或任何事件/持久化状态之前验证握手；新客户端的 Start/Resume/Rebuild 在 NotSent、未知结果或不兼容响应时均 fail-closed，不调用 mutating Direct fallback，也不先写 unavailable。新 v4 客户端不自动降级发 v3 mutating request。该协议不能阻止旧客户端在新 Service 缺席时走旧 v3 Direct fallback，因此必须与 B/C 组合。

### B. 安装维护状态与 Stop-the-world

新版本可识别维护标记，但旧 EXE 不认识该标记。Named Mutex 只协调实际 acquire/check 同一个对象的客户端，旧版不会因此停下。默认 ACL 也不会序列化同一用户的进程；安装器用提升权限为安装路径设置明确的 deny-execute ACL，能挡住新的文件打开/启动，但不能撤销已映射的旧进程、已打开的 SQLite handle，或阻止用户从另一可执行副本启动旧代码，所以 ACL 只能作为启动窗口的闸门。Service 的 SCM Stop 返回后还必须等待实际 PID 退出；不能只依赖 `SERVICE_STOPPED` 瞬时状态。

U2-B 应先持久化维护阶段、阻止通过当前安装路径启动新客户端，并枚举所有交互 Session 中该安装路径的进程；逐个请求旧桌面客户端退出，Stop/Wait 旧 Service 与桌面进程，确认无该代写进程后才替换文件/迁移数据库。可用短期受限 DACL 阻止维护窗口中新打开旧 image path，同时把旧 EXE 移出自动启动的 canonical 路径；应记录该闸门的 ACL/rename state，以便安装器崩溃后由 repair 工具恢复。旧进程持有 SQLite 连接时，先发协作关闭请求并等待其完成事务、关闭连接和退出；旧版本不理解关闭协议或超时时取消安装、保留旧安装/数据库，不靠数据库锁检测代替进程退出。未经明确约定的强制终止可能造成用户状态丢失。

建议由 Owner 批准一个机器级、原子更新的安装事务记录（位置可在 `HKLM\SOFTWARE\startlan\Zen Canvas\SetupState`；最终注册表视图/ACL 仍需审查），至少包含 `transaction_id`、`phase`、`from_generation`、`target_generation`、每个用户 `database_generation`、旧/新 image 与 DB staging 路径、maintenance gate/ACL 状态、首个新代 DB 写入是否提交。它存放在 ProgramData/HKLM 安装元数据，不分配 Schema v38 字段。Service restart 前读取该记录；repair 程序根据 phase 继续新代初始化/Service 注册，完成验证后才撤掉 gate。旧 EXE 仍不会读取该记录，故维护记录不能替代 B 的进程退出，也不能替代 C 的 DB 路径隔离。

原位文件替换或安装步骤中断时采用 repair-forward：旧程序不能被自动重新启动来“回滚”到新 Service。只有在尚未激活新版本且新数据库从未接受写入时，才可切回旧代；其余情况重启同一新代安装修复，保持维护闸门直到 Service、桌面和数据库代际一致。

建议冻结的安装状态机：

| 阶段 | 允许的写入者 | 失败处理 |
|---|---|---|
| `OLD_ACTIVE` | 旧代 | 可正常工作 |
| `MAINTENANCE_REQUESTED` / `NEW_STARTS_BLOCKED` | 无新增客户端 | 旧进程不退出或启动门无法确认则取消，旧代仍可运行 |
| `OLD_SERVICE_STOPPED` / `ALL_OLD_CLIENTS_EXITED` | 无 | Stop timeout、其他 Session 仍有 PID 或连接未关闭时取消，禁止文件/DB 变更 |
| `OLD_DB_BACKED_UP` / `NEW_DB_STAGED` | 仅迁移进程操作 staging 副本 | 备份、迁移、完整性/关联校验失败时丢弃 staging，保留旧 DB |
| `NEW_FILES_STAGED` / `NEW_GENERATION_ACTIVATED` | 仍禁止普通客户端，直到路径/Service/DB 代际一致 | 激活前可以恢复旧文件与旧 DB；激活后不自动回滚 |
| `NEW_SERVICE_VERIFIED` / `NEW_ACTIVE` | 新代 | 首次新代写入后所有故障均 repair-forward；旧代程序不得自动启动 |

`MAINTENANCE_REQUESTED` 不构成旧 EXE 的阻断器；只有安装器进程枚举/等待和 C 的数据库路径隔离能覆盖不识别标记的旧进程。

### C. 数据库版本/代际隔离

冻结旧版与 D1-A 的 `database_path()` 都指向每个交互用户 AppData 下 `zen-canvas.sqlite3`；Service 通过 `CreateServiceW` 的默认账户注册为 LocalSystem，索引数据库写者仍是对应的桌面客户端 Sink。安装器因此要针对机器上每个真实 profile 迁移/保留数据库，不能把 Service 的 SYSTEM profile 当成用户库。`CURRENT_SCHEMA_VERSION` 是 37；未来版本拒绝 `user_version > 37` 的数据库只能阻止旧代码打开已经升到更高 schema 的数据库，不能隔离同路径上仍为 Schema 37 的文件，也不能区分相同 schema 下的二进制代际。

U2-C 建议固定 generation-specific 路径，例如每用户 AppData 下保留 legacy `zen-canvas.sqlite3`，新代使用 `zen-canvas.g2.sqlite3`，并由新二进制中的代际常量/同源 release manifest 选择，旧 EXE 仍只能写 legacy 文件。完整应用数据库迁移必须在写者退出后使用 SQLite backup API，从 WAL 一致性快照复制到 `zen-canvas.g2.staging.sqlite3`，执行迁移与完整性/关联校验后关闭所有句柄并原子发布；不得手动拷贝主 DB/WAL/SHM 文件。发布前失败删除 staging 并保留旧文件；发布后新数据库有写入则 repair-forward，不切回会漏掉新写入的旧数据库。该策略需要至少一个完整数据库副本的额外空间；若另留 rollback snapshot，峰值可能更多。U1R 没有测量真实用户数据库拷贝、迁移时间或峰值空间，因此 U2 必须先测量并在空间不足时于激活前中止。

因此 generation staging 的崩溃发布安全与用户库峰值空间在本 Gate **尚未实证**，不能作为已批准实施方案。上线前要用分布式 v37 profile 大小、WAL 规模、SQLite backup、迁移期间崩溃、原子发布和每 profile 可用空间做 exact-binary Windows 测量。若任一 profile 无法预留经实测的 staging/rollback 空间，安全替代行为是安装器不切换文件、不迁移、不启动新代，保留旧版本及 v37 DB 并要求运维/用户先清理或将备份迁至足够空间的卷，再由 repair-forward 恢复原事务；不得降级为无备份原地迁移或以“节省空间”为由激活半迁移 DB。若产品要求低空间机器也必须在线升级，需另行设计并验收可恢复的分块迁移与写入冻结协议，本报告不把该路线视为安全等价方案。

Schema v38 所属设计需要与 #381 协调。本阶段不新增或保留 v38 编号。`user_version` 不足以成为 generation identity；要么使用不依赖 v38 的路径隔离，要么由 #381 Owner 决定统一迁移元数据、备份、升级和回滚合同。

## 6. 建议的 U2 最小生产边界

本 Gate 的最低协议升级判断：新代 client/Service 应切到 IPC v4 且不为 mutating operation 自动降级 v3；只改 D1-A 的 IPC v3 字段不足以区分映像已替换后仍存活的旧进程。U2 的代际 DB 隔离本身可以通过 generation-specific 路径与安装器事务元数据实现，不必单独占用 Schema v38；但 #380 的持久 operation/revision 与完整 Rebuild 恢复若需写入 DB Schema，必须与 #381 协商并由 Owner 冻结共同版本/迁移合同。本 Gate 没有授权或证明 v38 结构。

1. `src-tauri/src/global_index/windows/service.rs`：IPC v4 pipe、握手与响应字段、客户端构建代际/能力验证、拒绝旧 v3 client 的合同。
2. `src-tauri/src/global_index/windows/service_host.rs`：在请求 claim、Admission 事件和任何 sink 事件前完成版本/代际握手；记录服务端代际；拒绝不能识别的 v3 请求。
3. `src-tauri/src/global_index/windows/mod.rs`：mutating `NotSent`、未知结果、不匹配代际均不走 Direct fallback、不先变更 Source State；继续保留显式 ServiceBusy 类型语义。
4. `src-tauri/src/lib.rs::database_path`、`src-tauri/src/db/connection.rs` 与 `src-tauri/src/db/schema.rs`：新代数据库路径、只读源校验、SQLite backup 到 staging、发布/失败恢复状态机。Schema 修改由 #381 批准并统一安排，不自行占用 v38。
5. `src-tauri/windows/installer-hooks.nsh`、`src-tauri/windows/installer-lifecycle-final.nsh`、`src-tauri/windows/tauri-2.11.2-installer.upstream.nsi` 及生成模板：维护状态、Service PID Stop/Wait、所有用户 Session 的桌面进程退出 gate、无写者确认、文件替换与 repair-forward。生成来源也要同步修改，不能只改生成产物。
6. `scripts/buildWindowsPackage.mjs`、`scripts/prepareWindowsNsisLifecycleTemplate.mjs`、`scripts/finalizeWindowsNsisLifecycleOrchestration.mjs`：将维护/代际协议随正式安装包生成，并保证中断恢复入口可执行。
7. `src-tauri/src/main.rs` 与 startup owner：维护期间在 Tauri/Service 启动入口 fail-closed，防止新建交互客户端或 Service；`src-tauri/tauri.conf.json` 与版本元数据的代际字段与握手、installer package、Service 二进制同源；仅产品版本字符串不能作为授权凭据。

## 7. U2 验收合同

- 旧客户端 + 旧 Service、新 + 新、旧 + 新、新 + 旧四种组合；旧 v3 请求在新 Service 下无 claim、无事件、无持久化副作用；新客户端遇旧 Service/pipe 缺失时无 mutating Direct fallback。
- 普通同 SID ACL/Mutex 反例；不同 Session 旧 UI 进程、旧后台进程、Service PID、已打开 SQLite 连接的 Stop/Wait 与超时取消。
- 在完整旧用户数据库上验证 v37→新代 staging backup、WAL、完整性与关联表校验、磁盘空间不足、迁移中崩溃、发布前后进程中断和 repair-forward；不使用真实用户数据库。
- 旧客户端继续运行/旧 Service 未退出时，证明它无法写新代活动 DB；Managed/AI/索引/checkpoint 数据在拒绝和安装失败后都可核验。
- 普通更新、`/UPDATE`、安装中断、替换失败、Service 启动失败、repair-forward 和仅在新代无写入前允许的回滚。所有边界绑定 exact HEAD、Tree、安装包和 EXE SHA-256。

## 8. 尚未证实的风险与 Owner 决策

- Run 38198147350、38199772185、38201379732 均未完成 U1R 目标场景：先后因 JSON 解析、clean-path 条件和 Harness 生成重复 NSIS label 失败。旧进程隔离和 NSIS 首个内部 predicate 均为 `UNVERIFIED`；为避免重复昂贵 Hosted 构建，本 Gate 不再重试。
- U1 的 interruption/repair 状态为 `UNVERIFIED`：普通原位安装先以 exit 2 失败，未能建立“替换后强制中断”场景。本阶段不重复整套 interruption 矩阵；U2 必须用新维护/repair-forward 状态机的真实安装包补测。
- U1R 的同路径场景会单独 Stop/Delete/Start SCM Service，不等同于 Windows 冷重启或跨重启维护标记恢复；U2 必须用正式安装包验证重启后 startup gate、Service ImagePath 与数据库 generation 一致。
- 多交互用户 Session、快速用户切换、Service 更新时并发登录和 per-user AppData 库迁移是否有安全方案，需在 U2 Host 上覆盖。
- Owner 需批准只用 v4 的代际/Capability 字段合同、旧 v3 请求拒绝策略以及新端 NotSent 时完全禁用 mutating Direct fallback。
- Owner 需批准 generation DB 的路径/指针策略、使用 SQLite backup staging 的迁移合同、对旧 DB 留存/保留期限，以及“新代首写后 repair-forward”的规则。
- Owner 需与 #381 协调 Schema v38 的归属、迁移元数据与升级顺序；本报告不占用 Schema 编号。
- Owner 需决定 Stop/Wait 超时之后是否只取消，何时可以要求用户关闭/强制结束进程，以及回滚/修复对未保存 UI 状态的产品行为。
- 未得到 U2 授权前，不改 #386 生产代码、Schema、IPC 或 NSIS。

最终结论：旧版 Direct fallback 已由真实旧客户端证明会更改 source health/索引时间、Managed Entry 和 AI 关联状态；Windows 也已由真实进程测试证明允许同路径替换 EXE 时旧 PID 仍存活。NSIS exit 2 的内部首个失败条件与旧客户端对新 Service 的同路径身份结果未能验证。U2 应按本报告 A+B+C 组合方案推进设计审议，但生产实施仍需 Owner 单独授权，且必须先修复并验证 Hosted 测试 Harness、定位 NSIS 首个 predicate、证明旧进程写入隔离。

最终状态：`#380-U1R INSTALLER AND GENERATION GATE READY_FOR_OWNER_REVIEW` — **Gate: BLOCKED**。
