# ADR-0014（提案）— Index Scope、低价值子树排除与覆盖真实性

> **Status：PROPOSED BY OWNER / 未批准生产实施。**
> **日期：2026-10-11** · **基线：** `master@df37531f785193a948409e2f8cb36369561c6a6f`（合并后 CI 38111501260 SUCCESS）
> **跟踪：** [#371](https://github.com/ArdenZC/Zen-Canvas/issues/371)；成本研究 [#370](https://github.com/ArdenZC/Zen-Canvas/issues/370)
> **前置：** ADR-0012 Files Authority；ADR-0013 Managed AI Eligibility / Queue Authority；#330 Owner Gate
> **命名核验：** 截至此 SHA `docs/project/DECISIONS/` 已使用 0001–0013；0014 为建议编号，落库前仍需重新核验是否冲突。
> **产品决议状态：** 已提出明确决策草案；**默认过滤策略未获 Owner 最终激活**、性能收益未实测、不得以本草案实施生产变更。

## 1. Context

Zen 的 Windows 原生 MFT/USN、macOS Spotlight/FSEvents 与 SQLite Global Search 提供广覆盖元数据发现。这种覆盖对极大文件库有价值，但系统/应用安装组件、依赖目录、缓存和构建产物可能占用大量读属性、SQLite/FTS 索引、后续增量维护与 AI 准入成本。Oil Find 的默认依赖目录/包内容排除思路可参考，但 Zen 必须遵循自己现有数据库外键、源权威和跨平台权限边界。

一个简单的 `is_ignored_dir_name()` 不能代表系统目录身份，也不能知道 `D:/Design/build/logo.ai` 是设计作品还是生成物。普通索引过滤还会改变用户 Search 返回范围，不能将 ADR-0013 的低价值**AI admission** 措施直接转为普通文件隐藏。

### 1.1 Five distinct authorities

1. **Index Scope**：Zen 是否维护该来源的 Global Search 元数据（本 ADR 唯一拟新增的产品维度）。
2. **Managed Scope**：哪些文件进入 File Library/managed 管理和扫描的现有权威。
3. **AI Eligibility**：既有 ADR-0013 的模型工作准入与预算。
4. **Content Consent**：内容读取、云端共享等按功能独立授权。
5. **Mutation Authority**：Operation Preview、Confirm、Journal、Safe Trash、Restore。索引排除绝不授予删除、改名或移动权限。

**禁止** `ExcludedFromIndex => SafeToDelete`、`Indexed => AIEligible`、`Visible => Managed`、`SourceReady => EntireMachineIndexed` 等推断。

## 2. Decision proposal（须经 Owner 最终审批后方生效）

### 2.1 三种产品模式

| Mode | 覆盖行为 | 启用条件 | 对现有用户 |
|---|---|---|---|
| **Broad Coverage / 兼容** | 尽可能维持现有 Global Index 广覆盖（仍受 OS 权限、源可用性限制） | 默认保持 | **不自动改变**现有搜索结果 |
| **Smart Lean / 智能精简** | 纳入用户资产、项目根/README/docs/src，保守排除有来源/结构证据的系统内部、依赖、生成物/缓存子树 | 性能、误排、恢复、Windows/macOS 实测 Gate 后启用新用户推荐 | 必须用户主动 opt-in、显示影响与恢复路径 |
| **Custom / 自定义** | 已授权合法根路径显式 Include / Exclude + 个别预设 | 覆盖、迁移及 UI 恢复能力通过 Gate 后可用 | 仅明确操作后改变覆盖，不静默隐藏 |

**暂不支持**把只索引 Desktop/Documents/Downloads 作为所有人的默认规则；D 盘资料、外接卷和独立工作区必须有表达能力。

### 2.2 策略枚举与结果

| Decision | 语义 | Search / 数据库行为 | AI / File operation |
|---|---|---|---|
| `Include` | 身份和策略确认应被收录 | 可创建/保留正常 `global_entries`，与 FTS 一致 | 仍需独立 Managed/Eligibility/Consent 判断 |
| `MinimalAncestor` | 为合法后代的父链、移动、身份恢复必须保留的最小锚点 | 可以保留既有父身份辅助状态，**不得暗中每个排除文件都创建「轻量行」**；是否展示目录明确在 UI 合同中定义 | 不自动授予 AI 工作 |
| `Exclude(reason)` | 确定在当前策略下不创建新的普通逐文件搜索行 | 不允许把旧行直接 DELETE，旧数据生命周期另议 | 默认不会进入新增 AI 工作；不授予文件删除权 |
| `Undetermined` | 路径/身份/权限不足以可靠判定 | 搜索覆盖**保守 Include 或显式 Partial**（按来源可信度）；不能默默隐藏 | AI 使用更严格 fail-closed 门禁 |

### 2.3 优先级和覆盖安全

建议候选判定顺序：

1. OS 访问控制、provider 来源健康、内容授权、安全不可覆盖项；
2. 平台/卷稳定身份识别的受保护系统内部（当前应用不应主动深索引的区域）；
3. 用户显式的、已经验证权限与源身份的 Include/Exclude；
4. 以项目根 + manifest + 子目录相对位置 + 典型结构识别的 dependency/build/cache；
5. 其他用户可见文件与项目入口保守 Include；
6. 不足以判定：`Undetermined`，不将主观「低价值」变为权限或删除事实。

**注意**：用户可以合法选择索引某些开发依赖，但绝不能绕过 OS 权限/系统保护、下载云占位内容或启用内容传输。用户显式 Include 的优先级必须由实现审计限定到**可覆盖的性能排除**，而非所有安全禁令。

### 2.4 排除原因的可解释性

推荐稳定 reason 类型：`protected_system`、`application_internal`、`dependency_tree`、`generated_build`、`ephemeral_cache`、`explicit_user_excluded`、`source_unavailable`（后者**不是正常策略排除**）。UI 至少显示来源、策略版本、排除说明及恢复动作，并能区别 Source unavailable 与 filtered-empty。

样例文案：
- “C: 本轮已按智能精简范围建立索引，系统与依赖目录已排除；不是全盘所有文件都已存入。”
- “该文件未在当前搜索结果中：可能位于被排除的目录。可查看覆盖范围或申请纳入。”
- “D: 来源不可用/权限受限；不是‘没有文件’，也不是‘全部策略排除’。”

## 3. 策略输入和身份合同

- Policy 是**版本化 canonical snapshot**：`policy_version`、hash/fingerprint、源的稳定 volume ID、经允许的规则集合、明确包含/排除记录、创建方/同意语义与 generation。
- 身份优先级：Windows NTFS volume+FRN/parent FRN、macOS file/volume/provider 语义与可确认的路径；绝不只凭字符串 `C:\\Windows` / `/System` 相似名称判系统身份。
- 目录组件边界：严格路径分段，`Program Files2` 不能匹配 `Program Files`；Unicode/大小写按平台真实行为，不能只用 ASCII lowercase 代替 Windows/macOS 文件系统语义。
- junction/reparse/symlink/hardlink：不得通过别名绕过系统排除、权限或无限循环；多硬链和路径变化的索引视图语义需验证。
- 祖先决策缓存键至少包含 `(source_identity, directory_identity, policy_revision)`；目录重命名、卷挂载身份变化、权限/来源恢复和策略切换都须明确失效；缓存不成为独立持久索引。
- 规则不基于未经校验的任意用户目录字符串传给 LocalSystem 服务执行；IPC v3 保持原安全边界，版本扩展需另立安全 ADR。

## 4. Source/Policy Generation 与覆盖真实性

索引状态要区分两维：

- **Source health**：原生来源是否可用、权限是否足够、journal/Spotlight/FSEvents 是否完整；
- **Policy coverage**：在**哪个已生效策略版本**下哪些路径被 Include/Exclude/留锚点，以及 backfill/reconcile 是否完整。

`ready` 仅能表示 `(source_identity, checkpoint, source_generation, policy_revision)` 已满足其定义范围；用户面板单独展示 `discovered/resolved/included/excluded/anchor/error` 计数。即使 `included=0`，只要原生范围真实可判定、且全合法排除，仍可视作**policy-complete empty**；反之 `native_seen=0`/读源失败/权限缺失仍然不完整，不能靠 “0 rows” 决定二者。

建议状态机（仅方案，不是新数据库 enum）：

`Stable(old)` → `Validated(new)` → `ImpactPreview` → `Reconfiguring` → `Reconcile/Backfill` → `CrossDomainValidation` → `Stable(new)`；出错进入 `Partial/RecoveryRequired`，在全验证之前不得发布 Stable。

每个写入/删除/增量动作必须携带或重检当前 policy generation；旧 generation 的 in-flight work 不得悄悄写回刚排除的目录。Source checkpoint + USN/FSEvents 缺口管理仍由现有权威完成。

## 5. Windows 原生实施边界

1. 第一阶段只在共同受控 sink 入库前做判定及模拟，不碰 Service/Direct 独立权限和 IPC；完整证明同一生产来源与策略的收录结果相同。
2. 跟踪 `native_seen`、`staged`、`resolved`、`policy_excluded`、`included`、`emitted`、`real_error`。由于 `FSCTL_ENUM_USN_DATA` 是 MFT 级原生枚举，初版**不宣称减少该操作获得的原生记录数或 MFT staging**。
3. 只在目录/身份可靠后，才把策略判定前移到 `stream_staged_entries` 中非目录后代的 `symlink_metadata` 之前。
4. USN 更新：`rename/move` 导致「排除→纳入」时，不能仅靠移动动作写一个目录就认为所有子文件完成；需要 bounded backfill / source reconcile。反向移动需受控 stale/exclude，保留旧用户语义状态。
5. 缺口、无权限、Service busy、0 emitted、暂停、重启、源禁用都是不同状态；不能把 `index_service_busy` 解释成文件不存在，也不能将 #328 问题按索引范围优化悄悄掩盖。
6. MinimalAncestor 需要最小父链身份，可选择复用现有已持久权威或非持久重建派生结构；不能一边把祖先删掉一边要求 `find_entry_by_identity` 仍可解析。

## 6. macOS 原生实施边界

1. 使用现有 `NSMetadataQuery` 结果，**在 Zen metadata 转换之前**按可靠 path/identity 做轻量资格判断。
2. 是否把策略放入 query `predicate`/`searchScopes` 须单独对照实测 gather 改善，不能默认 Apple Spotlight 自身少索引。Apple Query 仍有初始 gather 与 live update 阶段。
3. `~/Library`、iCloud Mobile Documents、File Provider placeholders、外接卷、`.app` 包对象及其内部是不同语义，不能全部按名硬排；云占位不得因 Index Policy 隐式下载。
4. 文件移动、bundle 更新、FSEvents dropped、权限撤销/恢复和 NSMetadataQuery partial 时必须回到 Source health 和 Policy coverage 的双维真实状态。
5. 无 Apple Silicon 真机则只能记录 Hosted macOS compile/test；不能写 Native PASS。

## 7. 历史数据、引用和回滚的无损迁移合同

**禁止立即 DELETE 已有 `global_entries`；必须先完成真实库引用审计。** Schema 37 的初步直接 FK 链：

- `managed_scopes.global_entry_id` → `global_entries`：`ON DELETE SET NULL`；
- `managed_entries.global_entry_id` → `global_entries`：`ON DELETE CASCADE`；
- `ai_analysis_state.global_entry_id` → `global_entries`：`ON DELETE CASCADE`；
- `ai_jobs.global_entry_id` → `global_entries`：`ON DELETE CASCADE`；
- `ai_job_items.global_entry_id` → `global_entries`：`ON DELETE CASCADE`；
- `ai_job_items.job_id` → `ai_jobs`：`ON DELETE CASCADE`（二级级联）。

不能仅审计这些直接 FK；还必须检查用户校正、notes、semantic artifacts、外部选择、已运行任务和依赖路径身份的非 FK 引用。

**受控切换协议（待专项 Code Gate 设计）**：

1. 仅做影响预览：候选目录数、行数、可能丢的结果与人工覆盖项、AI/job 数据风险；不能为了预览无预算扫描整个系统。
2. 显式 consent + fingerprint 冻结 + 有界 resource reservation。
3. 冻结新 generation 的 admission；禁止旧 generation in-flight 分析继续按旧策略发布。
4. 对新 Include 做 backfill；对新 Exclude 在缺少保全合同前只能保留 metadata/锚点或隐藏于 projection，不执行物理删除。
5. 获批分批清退时，按真实 FK 关联保护/迁移/保留、限速事务、FTS/WAL 观察、recoverable checkpoint；不中断 UI 不产生假 ready。
6. 验证所有 source IDs、USN/FSEvents checkpoints、Search/FTS identity、user corrections、Managed membership、AI currentness 与历史任务后再发布 `Stable(new)`。
7. 失败/崩溃恢复旧真实数据或标 `Partial/RecoveryRequired`；回滚仅切 flag 不能恢复已删行，需真实的再枚举/backfill 与引用恢复。

**Existing user gate**：已有用户继续 Broad Coverage，不允许新版本安装完成后默默消失若干搜索结果。新用户 Smart Lean 推荐默认也需在 native/覆盖实验达标后另行批准。

## 8. 跨权威兼容

- **ADR-0012**：新 Files/Query V2 使用 canonical owner，Legacy File Library Store 不得在这个变更中增生新 durable authority。
- **ADR-0013**：Index Scope 过滤不能绕过 Managed AI Job 的一个 durable queue、existing Worker/Scheduler/Consent，以及 eligibility 在入队/claim 时的再校验；无权新增每目录、每文件 cloud AI 分析。
- **#328**：保留原有 Global Index source 健康与 Service/Direct 语义；不能在调试旧故障时同时上线全新过滤，让两个问题无法归因。
- **#340**：测试夹具并发清理修复应独立；不能通过 skip/降低测试门槛使新 Index Scope 通过。

## 9. 验收矩阵（Owner 定义，交付前必须填真实结果）

| 用例 | 必须的行为 | 当前证据状态 |
|---|---|---|
| 真实系统 C:/Windows / macOS /System 内部 | 默认精简剔除普通非必要文件，权限边界不变 | 未实现 |
| 用户项目 `D:/Projects/Windows/README.md` | Include | 未实测：现 Managed 名称规则存在误排风险 |
| 用户设计 `D:/Design/build/logo.ai` | Include 或显式安全例外 | 未实测 |
| React `src`, README；依赖 `node_modules` | 源码与入口可搜；依赖可按新策略 Exclude | 未实现 |
| Python `.venv`、Rust `target` | 在结构确认后有界排除普通子树；祖先足够 | 未实现 |
| 名称相似 `Program Files2` | 不被系统目录策略命中 | 未实现 |
| 只有合法排除项，0 included | Policy Complete Empty，不触发损坏重建 | 未实现；现 MFT 0 emitted => RebuildRequired |
| 源 native 失败、USN gap、Spotlight partial | 绝不假 ready | 现有状态能力需集成回归 |
| 从 Exclude 移入 Include（反向亦然） | Backfill/stale/identity 正确 | 未实现 |
| 已被用户校正、已存在 AI job 的条目被排除 | 用户真值/任务不静默消失 | 未实现；FK 风险已证明 |
| Service/Direct 同一 source/policy | 相同结果且不提升权限 | 未验证 |
| macOS iCloud placeholder、外接卷、`.app` | 不隐式下载，准确报告来源状态 | Native UNVERIFIED |
| 不同 Windows UAC/SmartScreen 设置 | 安全与功能资格分别记录 | 完整安全配置 Native UNVERIFIED |

## 10. 严格实施排序（获批前无生产代码）

| Stage | Owner 负责的研究/审核 | Codex Cloud 负责的实施验证 | Go/No-Go |
|---|---|---|---|
| ISG-00 | 当前报告、调用链/成本模型、基准合同和解读 | benchmark-only 固定种子合成测试和执行 | 未实测收益则不进入生产 |
| ISG-01 | 本 ADR 的产品决策和修订 | 不负责编写 ADR；只有明确需要的文档校验技术支持 | Owner 显式批准 |
| ISG-02 | 评审策略行为/反例 | pure Rust policy test-only 实现、测试矩阵 | 无误排且不越权 |
| ISG-03A | 评审 coverage 和数据所有权 | 小范围共享 sink gate 代码与真实测试 | FK、source、AI gate 全绿 |
| ISG-03B/C | 评审性能及跨平台不变量 | Windows MFT 前移、macOS path-first 的各自独立 PR | 对应 Native evidence |
| ISG-04/05 | 用户迁移/UX/API 决策 | 获批的 transaction/rebuild/UI 实现 | 无历史数据丢失/假覆盖 |
| ISG-06 | Embedding 与 Token Cost 架构/预算决策 | 仅独立授权的 implementation | 不自动全库分析 |

## 11. Rejected alternatives

1. **立即默认排除全部隐藏、系统、build、AppData、开发目录**：会误排真实用户文档、配置、源码，且破坏 Search 覆盖。
2. **只过滤 AI admission**：有价值但不降低 Global Index/FTS 的主要元数据存储和维护成本。
3. **直接 DELETE global_entries 并 VACUUM**：当前级联风险高，真实恢复不可验证，容易触发 SQLITE_BUSY 与前台卡顿。
4. **跳过 Windows FSCTL_ENUM_USN_DATA / 用递归 walk 替代**：失去现有 MFT/USN/Service 权威且不符合现有优化插入点。
5. **新增第二索引/项目索引/后台轮询**：重复权威、增大常驻成本，和既有架构相冲突。
6. **默认修改 Spotlight predicate 就宣称更快**：OS 层实际 gather 优化未证实。

## 12. Final Decision Status

- **Owner 草案：已完成，可进入 ADR 决策评审。**
- **实施授权：NO。** 旧用户数据迁移、Schema/IPC、生产过滤、默认覆盖及大规模 Embedding 均未授权。
- **需先拿到的证据**：ISG-00 同 head paired 资源成本；用户反例误排零；Windows 原生源身份/合法零结果/USN 移入移出；FK 用户状态保全；macOS 原生仍须 UNVERIFIED 或补充有条件设备证据。

**标记：`ISG-01 OWNER ADR DRAFT COMPLETE — APPROVAL/MEASURED GATE PENDING`。**

### 源码与规范

- 固定基线：[Zen Canvas master](https://github.com/ArdenZC/Zen-Canvas/tree/df37531f785193a948409e2f8cb36369561c6a6f)
- [Schema 37](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/db/schema.rs)，[MFT](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/global_index/windows/mft.rs)，[path filter](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/src-tauri/src/path_filter.rs)
- [ADR-0012](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/docs/project/DECISIONS/0012-files-authority-and-compatibility-retirement.md)，[ADR-0013](https://github.com/ArdenZC/Zen-Canvas/blob/df37531f785193a948409e2f8cb36369561c6a6f/docs/project/DECISIONS/0013-managed-ai-eligibility-and-queue-authority.md)
- [Microsoft FSCTL_ENUM_USN_DATA](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_data)
- [Apple NSMetadataQuery](https://developer.apple.com/documentation/foundation/nsmetadataquery)
- [SQLite Foreign Keys](https://www.sqlite.org/foreignkeys.html)、[SQLite WAL](https://www.sqlite.org/wal.html)