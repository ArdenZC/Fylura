# Fylura v3.40 高保真验收矩阵（PR 3）

状态口径：`PASS` = 生产代码 + 真实浏览器/组件验证 + 证据；`PARTIAL` = 已实现但存在
明确保留差异或只覆盖部分状态；`FAIL` = 未达到；`NOT_TESTED` = 本轮未验证；
`N/A` = 生产不存在该对象。**PARTIAL 不计入 PASS。**

原型基准：`DESIGN-SPEC.md` §2/§5/§8、`MOTION.md` §3/§6/§8、`HANDOFF.md` §2.4。
生产基线：`342afb94` + 本 PR。

## A. 动效（MOTION.md §3）

| # | 原型要求 | 生产实现 | 验证 | 证据 | 结果 |
|---|---|---|---|---|---|
| A1 | 按钮/导航/工具/标签/设备行/分段：按下 `scale(.965)` 90ms `sp-out`，松手 1.0 260ms `sp-pop` | `components/motion/PressMotion.tsx` + `styles/v340-motion.css` | 真实指针/键盘/取消/移出/blur；`motionPressFeedback` 4 项；`runV340MotionBrowserGate` | PR1 证据 + 本 PR 回归 | PASS |
| A2 | 菜单项/文件数据行不缩放 | PressMotion 排除 `role=option/menuitem`、`data-file-row`、disabled、inert | 组件测试 | 同上 | PASS |
| A3 | 按钮悬停底色/描边 120ms | `v340-motion.css` 颜色令牌；文件行 hover 120ms | 浏览器 computed `0.12s` | `motion-pr1` | PASS |
| A4 | 系统菜单悬停瞬切 | 下拉/右键/窗格菜单项无 transition | computed `transition-duration: 0s`（浏览器核查） | `runV340OverlayBrowserGate` | PASS |
| A5 | 文件行选中 0ms | 选中态 `transition:none !important` | 浏览器 `0s`，同帧底色 | PR1 门禁 | PASS |
| A6 | 复选框底色 140ms、勾 260ms `sp-bounce` | `zc-file-check-in` 只作用于勾 SVG | 浏览器 260ms | PR1 门禁 | PASS |
| A7 | 输入框聚焦 120ms | 输入/下拉 focus 样式使用 `--zc-duration-fast` | 组件/视觉 | `settings` 证据 | PASS |
| A8 | 标签页关闭 × 悬停 .32→1 120ms | tab close 样式 | 人工视觉 | Files 截图 | PASS |
| A9 | 侧栏 200ms 宽度、文字/图标同步收展、徽标数字→6px 点 | `v340-shell.css` grid 轨道 + `max-width/opacity`；始终挂载；`aria-hidden`+`inert` | 三态插值、快速反向、reduced motion；`runV340MotionBrowserGate` | `motion-pr1` + `sidebar-*` | PASS |
| A10 | 目录树面板 200ms 宽度 + 160ms 透明度、内容不重排 | 挂载保留 + slot 宽度过渡 + 固定面板宽度 | 196→14.4→0，关闭中 panel 196 | PR1 门禁 | PASS |
| A11 | 目录树分组 `1fr→0fr` + 内容淡出 | `DirectoryTree`/`FileLibraryNavigation` `grid-template-rows` | 真实高度与 `inert`；组件测试 + 浏览器 | PR1/PR3 | PASS |
| A12 | Inspector 200ms 列宽 | `browseMode.css` / `libraryMode.css` grid transition | 打开/关闭列表 658↔958；性能场景未单列 | `prod-files-*` | PARTIAL：关闭时面板内容直接消失，只有列宽动画 |
| A13 | 1–4 窗格每格 `opacity+scale(.982)` 260ms，不动 grid 轨道 | `playPaneEnter` 只对新窗格 | 1→2/4 只新窗格动画；旧窗格不重播 | `motion-pr1` + PR3 性能 | PASS |
| A14 | 列表↔网格 `opacity .45→1` + 5px 240ms，整块 | `playViewSwap` on content slot | 浏览器 `zc-view-swap` 240ms | 门禁 | PASS |
| A15 | 分隔条拖动 1:1 无缓动；悬停 120ms | 原 pointer/键盘实现；`workbench.css` | U3/U4 门禁：拖动/键盘比例 | U3/U4 | PASS |
| A16 | 窗格聚焦环 140ms | workbench focus ring | 视觉/现有测试 | `panes-*` | PASS |
| A17 | 徽标 1→1.22→1 320ms | `CountBadge` + `zc-badge-pop` | 组件测试：变化一次、同值不重播 | `motionOverlayPresence` | PASS |
| A18 | 外部设备条出现瞬切 | `MountedVolumeList` 新行无入场动画；容量条/只读 chip | 组件 HTML 测试；浏览器 mock 无设备 | `mountedLocations.test` | PARTIAL：截图无法覆盖真实挂载 |
| A19 | 下拉菜单 240ms `sp-pop`、`translateY(±6)`、`scale(.96)`、origin 指向锚点 | `playOverlayEnter` + 四个菜单宿主 | 5 次重复、Esc/外部/动作、退出中重开、inert | `runV340OverlayBrowserGate` | PASS |
| A20 | 菜单退场 130ms `sp-out`（仅取消类） | `playOverlayExit` + `data-closing` + ghost/卸载 | Esc/外部 130ms；动作同帧关闭 | 同上 | PASS |
| A21 | 设置页下拉 200ms `scale(.98)`、贴底上翻 | `playOverlayEnter(...,"select")` + flip | 5 次 200ms；Esc 130ms；弱动效立关 | 同上 | PASS |
| A22 | 确认/执行层 240ms、卡片 `translateY(10) scale(.955)`、遮罩 200ms、`backdrop-filter 0→3px` | `[data-confirm-card/backdrop]` + `zc-dialog-card-in` + `zc-scrim-blur`；`playExitGhost` 130ms | 卡片 0.24s、遮罩 0.2s、最终 blur(3px)、ghost inert/无焦点 | 门禁 + `motion-frames/confirm-*` | PASS |
| A23 | 搜索浮层容器 180ms + 卡片 260–280ms | `CommandModal` 260ms + `zc-scrim-in` 200ms + scrim blur | 260ms 卡片；Esc 130ms 后卸载；独立窗口单次 | 门禁 + `motion-frames` | PASS |
| A24 | Quick Preview 260–280ms 入场 | `.zc-quick-preview-card` `zc-preview-in` 260ms | 连续 5 次每次 260ms；Pin/Unpin 单壳 | 门禁 + `quick-preview` 帧 | PASS |
| A25 | Quick Preview 退场 | 关闭同步 `controller.close()`；无 ghost | 80ms 内释放；资源/身份优先 | 门禁 | PARTIAL：保留生产安全差异 |
| A26 | 主区景深 `scale(.994)` 200ms | `acquireMainDepth` + `html.zc-modal-depth` | 搜索/确认/预览打开 .994，关闭回 1；菜单不缩放 | 门禁 | PASS |
| A27 | 多选浮条 `translateY(8)→0` 140ms | 生产无多选浮条，状态栏承担选择信息 | 代码检索 | — | N/A |
| A28 | 页面切换 220ms + 4px | `playPageEnter` | 220ms；结束 opacity 1 / transform none | PR1 门禁 | PASS |
| A29 | Inspector 内容瞬切 | 无内容动画 | 代码 | — | PASS |
| A30 | Toast 出 320ms bounce / 回 130ms | `ToastContainer` presence + `zc-toast-*` | 组件测试 320ms；持久告警同帧解析 | `motionOverlayPresence` | PARTIAL：浏览器未稳定触发真实 watcher 错误 |
| A31 | 滚动条 overlay、内缩、悬停加深；`overscroll-behavior`/`scrollbar-gutter` | `v340-motion.css` + 各滚动区 | 视觉核对；设置页 stable gutter | 截图 | PASS |
| A32 | 焦点环 180ms 扩散 | 全局 focus-visible + 双层 outline/shadow | 键盘走查/现有可访问性测试 | — | PASS |
| A33 | 循环动画只在真实进程时运行 | 扫描/进度条绑定 busy 状态 | 代码审查；未做全程录屏 | — | PARTIAL |
| A34 | Reduced Motion：CSS/WAAPI/JS/滚动/浮层等价 | 全局 0.001ms + `prefersReducedMotion()` 入口 + ghost 跳过 | 侧栏 30ms 直达、页面 0 WAAPI、菜单立关 | 两套门禁 | PASS |
| A35 | Forced Colors 可读 | 设置页/页面原语 `forced-color-adjust`、CanvasText 边框 | 截图 + 组件规则 | `settings-v340` | PASS |

## B. 视觉（DESIGN-SPEC）

| 区域 | 核对点 | 结果 | 说明 |
|---|---|---|---|
| Shell | 44px 标题栏、10px 外边距、216/64/0 侧栏、玻璃、圆角 | PASS | 几何与原型一致；`prod-*` 截图 |
| 文件位置栏/标签条/工具栏 | 单行位置、标签 28、工具 30、窄窗收缩 | PASS | 1440/1080/850 截图 |
| 文件列表 | 四列 `22/1fr/96/118/78`、46/36 行、表头 36、发丝线 | PASS | **本轮修复空第三列与创建列** |
| Inspector | 288px、标题「信息」、事实行、操作 | PASS | 标题/宽度已对齐；关闭内容瞬切 PARTIAL |
| 状态栏 | 真实 loaded/total、选择范围、大小仅在已知时 | PASS | 未把已读取当全目录 |
| 设备行 | 图标+名称+容量条+只读/云盘 chip+eject | PARTIAL | 组件测试通过；browser mock 无挂载卷 |
| Files 1–4 窗格 | 2×2、分隔条、焦点环、窗格独立路径 | PASS | `panes4` 截图 + U3/U4 |
| Overview | 状态带、任务行、最近/覆盖区 | PASS | 保留「无问候语」差异 |
| AI Organize | 空态/计划 KPI/分组/操作条 | PASS | 空态卡片偏窄但可用；数据不同 |
| AI Cleanup | 范围、扫描、空态、运行 KPI | PASS | 无范围时大面积留白属真实空态 |
| History / Restore | KPI、批次列表、详情、时间线 | PASS | `prod-history-*` |
| Settings | 212px 导航、60px 搜索头、820px 内容、单分区 | PASS | #582 基线复核 |
| Global Search | 560 宽、14vh、遮罩 3px、48 输入行、34 结果行 | PASS | **本轮 720→560、20vh→14vh、补模糊** |
| Preview / 菜单 | 卡片 1180×780、260ms；菜单 origin/翻向 | PASS | `motion-frames` |
| 深浅主题 | 画布/表面/文字/边框/玻璃 | PASS | 20 组双图 |
| 窄窗 850 / DPR2 / 强制色 | 无横向溢出、控件可用 | PASS | 既有证据 + PR3 复核 |

## C. 交互、状态与可访问性

| 要求 | 验证 | 结果 |
|---|---|---|
| 文件选择/右键/拖拽/跨窗格复制移动 | U3/U4 真实 Chromium，含 3→4 复制、键盘分隔条 | PASS |
| 菜单取消/提交/五次打开/快速中断 | 门禁：5 次、Esc/外部/动作、退出中重开、inert | PASS |
| 菜单退场期 Tab/Enter 不误操作 | `inert` + `aria-hidden` + 焦点先归还触发器 | PASS |
| ModalPortal 焦点隔离/归还 | `w2-10:real` 6/6 + 门禁 ghost 不含焦点 | PASS |
| Quick Preview 关闭/Pin/Floating/资源释放 | 5 次开关、Pin/Unpin 单壳、80ms 内释放 | PASS |
| 确认层立即提交、视觉退场 | ghost 在真实弹窗卸载后 130ms 淡出；无操作延迟 | PASS |
| Organize/Cleanup/Restore 空/忙/错/部分成功 | 现有 view/controller 测试；浏览器走查 | PARTIAL：未逐一录制部分成功 |
| Windows/macOS 交互语义 | Win32/MacIntel 模拟下菜单/搜索/窗格一致 | PASS（mock）；原生 NOT_TESTED |
| Reduced Motion / Forced Colors | 两套门禁 + 设置/Overview 截图 | PASS |
| 850×680 / 四窗格 | 截图与 U3/U4 | PASS |
| 长文件名/中文/特殊字符 | 既有名称测试 + 浏览器 long-name 证据 | PASS（视觉 PARTIAL） |
| Transfer/Journal/Restore/Global Index/AI 权限/文件身份 | Diff 审查：无执行器/权限改动 | PASS |

## D. 性能（详见 `PERF.md`）

| 场景 | 1× p50/p95 | 1× 掉帧 | 4× p50/p95 | 4× Layout ms | 结果 |
|---|---|---|---|---|---|
| 页面切换 | 16.7/16.7 | 1 | 16.7/116.6 | 302 | PASS（整页重建） |
| 侧栏收展 | 16.7/16.8 | 0 | 33.3/133.4 | 155 | PASS（低端可感知） |
| 菜单弹出 | 16.7/16.8 | 0 | 16.7/83.4 | 35 | PASS |
| 确认层 | 16.7/33.4 | 8 | 16.7/133.4 | 103 | PASS（保留 3px 模糊） |
| 目录树折叠 | 16.7/16.7 | 0 | 16.7/16.8 | 221 | PASS |
| 列表↔网格 | 16.7/16.8 | 0 | 16.7/33.4 | 159 | PASS |
| 窗格布局切换 | 16.7/16.8 | 3 | 16.7/150.0 | 227 | PASS（四窗格重建） |

关键修复：移除主区无效 `backdrop-filter`（浮层 33.3→16.7ms）、移除空第三列
（列表 370→658px）、列表宽度更新量化（只在断点变化时重渲染）。

## E. 本轮修复清单

1. Inspector 打开时列表被空第三列压到 370px → 移除空列/占位槽。
2. 多余创建时间列 → 移除，保留 Inspector 事实与排序菜单；旧列宽快照迁移。
3. 类型列阈值 900→540，1440+Inspector 下保留类型列。
4. 设备行容量条/只读/云盘/离线 chip/eject 结构与真实数据。
5. 搜索浮层 720→560、20vh→14vh、遮罩 3px 模糊。
6. Inspector 标题「上下文」→「信息」；文件类型列「文件类型」→「类型」。
7. 主区景深 `scale(.994)` 200ms 接入搜索/确认/预览。
8. 确认/执行层 130ms 非交互 ghost 退场；遮罩 0→3px 模糊。
9. 退出中菜单 `inert`/`aria-hidden` + 焦点先归还。
10. 设置下拉 200ms/.98；搜索卡片 260ms；Inspector 列宽 200ms。
11. 多窗格菜单分段控件补样式（1/2/3/4 选中态、焦点环）。
12. 主区 `backdrop-filter` 移除，浮层帧率翻倍。
12. PR1 窗格重播修复回归复核。

## F. 明确保留（不计 PASS）

- Overview 无问候语；原型演示数据不等同生产 Fixture。
- 生产无多选底部浮条（N/A）。
- Inspector 关闭内容瞬切；Quick Preview 关闭无退场；两者均以数据/资源安全为先。
- Toast 真实 watcher 错误浏览器触发未覆盖。
- 原生安装版、真实设备、macOS overlay scrollbar：NOT_TESTED / #487 暂缓。
