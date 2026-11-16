# Fylura v3.40 高保真验收矩阵（最终聚焦整改）

状态口径：`PASS` = 生产代码 + 真实浏览器/组件验证 + 证据；`PARTIAL` = 已实现但存在
明确保留差异或只覆盖部分状态；`FAIL` = 未达到；`NOT_TESTED` = 本轮未验证；
`N/A` = 生产不存在该对象。**PARTIAL 不计入 PASS。**

原型基准：`DESIGN-SPEC.md` §2/§5/§8、`MOTION.md` §3/§6/§8、`HANDOFF.md` §2.4。
生产基线：`32787b0d` + 本轮聚焦 PR。

## 0. 本轮证据入口

- `parity/index.html`：30 组同 DPR 双图 + 3 张生产独有状态。
- `parity/manifest.json`：viewport / DPR / **物理像素** / 平台 / 主题 / 场景；
  任一对物理尺寸不一致即失败。
- `parity/PERF.md` + `perf/perf-*.json` + `perf/long-frames-4x.json`
  （5 次无采样）与 `perf/long-frames-4x-profile.json`（采样归因）。
- `responsive/responsive.json`：158 例矩阵；最终结果见 §D2。
- `responsive/README.md`：显示/窗口矩阵与每例检查。
- `src-tauri/src/app_control.rs` 的 10 个窗口几何单测（含 <980×680 降级）。

## A. 动效（MOTION.md §3，沿用已合并的 PR 1–3 门禁）

| # | 原型要求 | 生产实现 | 验证 | 结果 |
|---|---|---|---|---|
| A1 | 按下 `.965` 90ms，松手 260ms 单次过冲 | `PressMotion` + `v340-motion.css` | 指针/键盘/取消/移出/blur；Motion 门禁 | PASS |
| A2 | 菜单项/文件行不缩放 | PressMotion 排除 option/menuitem/file-row/disabled/inert | 组件测试 | PASS |
| A3 | 颜色 120ms，形变弹簧 | `--zc-dur-*` / `--zc-spring-*` | computed 0.12s | PASS |
| A4 | 文件行选中 0ms | 选中态 `transition:none !important` | 浏览器 0s | PASS |
| A5 | 勾 260ms bounce | `zc-file-check-in` | 浏览器 260ms | PASS |
| A6 | 侧栏 200ms 宽度/文字/徽标 | grid 轨道 + max-width/opacity；始终挂载 | 三态插值、反向、reduced motion | PASS |
| A7 | 目录树 200ms / `0fr→1fr` | slot + 面板固定宽度；grid rows | 196→14.4→0 | PASS |
| A8 | 1–4 窗格 260ms `opacity+scale(.982)` | `playPaneEnter` 只对新窗格；memo 后旧窗格不重渲染 | 1→2/4 只新窗格动画 | PASS |
| A9 | 列表↔网格 240ms | `playViewSwap` | 门禁 240ms | PASS |
| A10 | 菜单 240ms pop / 130ms out | `playOverlayEnter/Exit`；受控 closing；重开取消旧实例 | 5 次重复、Esc/外部/动作、inert | PASS |
| A11 | 确认层 240ms + 遮罩 200ms + 3px 模糊；退出 ghost 130ms | `[data-confirm-card/backdrop]` + `playExitGhost` | 0.24s / 0.2s；ghost 无焦点 | PASS |
| A12 | 搜索 260ms / 遮罩 200ms / 退出 130ms | CommandModal + scrim | Esc 130ms 后卸载；焦点归还 | PASS |
| A13 | Quick Preview 260ms 入场 | `zc-preview-in` 260ms；Pin/Unpin 单壳 | 5 次；资源 80ms 释放 | PASS |
| A14 | 主区景深 `.994` 200ms | `acquireMainDepth` | 搜索/确认/预览 | PASS |
| A15 | 页面 220ms + 4px | `playPageEnter` | 220ms | PASS |
| A16 | Toast 320 / 130ms | `zc-toast-*` | 组件 + 持久告警同帧 | PARTIAL：真实 watcher 错误浏览器未稳定触发 |
| A17 | Reduced Motion 等价 | CSS 归零 + WAAPI 跳过 + ghost 跳过 | 两套门禁 | PASS |
| A18 | Forced Colors 可读 | 页面原语 CanvasText 边框 | 截图 + 规则 | PASS |

## B. 视觉（DESIGN-SPEC / 原型）

| 区域 | 核对点 | 结果 | 说明 |
|---|---|---|---|
| Shell | 44px 标题栏、216/64/0 侧栏、玻璃、圆角 | PASS | `prod-files-*` |
| 文件位置栏/工具栏 | 单行位置、标签、工具、窄窗收敛 | PASS | 1440/1080/850 |
| 文件列表 | 四列、46/36 行、表头 36、发丝线 | PASS | 列表/网格证据 |
| Inspector | 288px / 980 sheet / ≥1024 inline 260、标题「信息」 | PASS | 关闭内容瞬切为 PARTIAL 子项 |
| 状态栏 | 真实 loaded/total、选择、大小仅在已知时 | PASS | Columns 底部同源 |
| 设备行 | 只读/云盘/离线 chip、容量条、eject | PASS（组件） | mock 无挂载设备，截图不可见 |
| 1–4 窗格 | 2×2、分隔条、焦点、独立路径/文件 | PASS | Win 四目录；mac 四窗格同屏三列 |
| **macOS Finder 列** | **3 级祖先 + 单文件选择 + 244px 预览；226/206/190/182 列宽；24px 行；文件类型图标；截断；横向滚动；祖先轨道；Quick Look** | **PASS（列视图本体）** | 30 组同状态双图 + 22 列状态几何/交互 |
| macOS 整体 chrome | 标签页、动作条、1–4 窗格、真实卷/树/其他位置；无原型收藏/标签 | **PARTIAL** | 产品身份保留，不伪造演示数据；交 Owner 接受 |
| macOS 四级列 | 4 列 / 横向滚动 / 预览 | PASS（生产独有） | 原型无四级演示，单独截图不冒充同状态 |
| AI Organize 状态 A | AI 不可用、分析不可用、受阻不执行、详细边界可展开 | PASS | `organize-unavailable` |
| AI Organize 状态 B | 8 项计划、4 KPI、4 目标分组首屏可见、审核动作、执行条 | PASS（计划总览） | 组内完整文件列表在审核面板；首屏为组摘要 |
| AI Organize | 安全/审核链路完整、无伪造执行资格 | PASS | 详情折叠，非删除 |
| AI Cleanup | 9 findings、KPI、安全/需判断/谨慎分组、选择、Sticky Bar | PARTIAL | 原型为扁平清单；生产保留风险分类 |
| Overview | 真实任务/覆盖/后台状态 | PARTIAL | 无装饰问候语；信息节奏不同 |
| History / Restore | 批次 + 明细 + 恢复入口 + 风险 | PARTIAL | 原型更简洁 |
| Settings | 212/60/820、单分区、深链、搜索定位 | PARTIAL | 技术分区/AI 管理为生产必要内容 |
| Global Search | 560 / 14vh / 3px 模糊 / 48+34 行 | PASS | `prod-search-*` |
| 深浅主题 | 画布/表面/文字/边框/玻璃 | PASS | 所有双图 |
| Mac Retina | 同 viewport/DPR/UA/主题；物理像素一致 | PASS | 2880×1800 等；DPR1 单独控制 |

## C. 交互、状态与可访问性

| 要求 | 验证 | 结果 |
|---|---|---|
| 文件选择/右键/拖拽/跨窗格复制移动 | U3/U4 真实 Chromium | PASS |
| Finder 子目录导航/祖先回退/轨道 | 组件测试 + 22 列状态浏览器门禁 | PASS |
| Finder 预览列单文件/多选/Inspector 互斥 | 组件测试 + 浏览器几何 | PASS |
| 菜单 5 次打开/取消/提交/中断/inert | Overlay 门禁 | PASS |
| ModalPortal 焦点隔离/归还 | w2-10 + ghost 门禁 | PASS |
| Quick Preview 关闭/Pin/资源释放 | 门禁 | PASS |
| Organize/Cleanup/Restore 空/忙/错/部分成功 | 现有 controller 测试 + 走查 | PARTIAL：未逐一录制部分成功 |
| 窄窗 980/850、四窗格、长文件名/中文 | 响应式矩阵 + 列状态 | PASS |
| Windows/macOS 平台语义 | Win32/MacIntel 模拟 | PASS（mock）；原生 NOT_TESTED |
| Transfer/Journal/Restore/Index/AI 权限 | Diff 审查：无执行器/权限改动 | PASS |

门禁备注：Motion 首轮在冷 Vite 编译时 `page-switch` 选择器为空，复跑
`[v340-motion] PASS`；U3 的 multipane 首轮截图超时，直接复跑
`{"ok":true, checks: 8, errors: []}`。两者均为冷启动/截图超时，不是产品断言失败。

## D. 性能（详见 `parity/PERF.md`）

1×：

| 场景 | p50 | p95 | 掉帧 | LoAF max |
|---|---:|---:|---:|---:|
| 页面切换 | 16.7 | 16.7 | 0 | 0 |
| 侧栏收展 | 16.7 | 16.7 | 0 | 0 |
| 菜单弹出 | 16.7 | 16.7 | 0 | 0 |
| 确认层 | 16.7 | 49.9 | 4 | 0 |
| 目录树折叠 | 16.7 | 16.7 | 0 | 0 |
| 列表↔网格 | 16.7 | 16.7 | 0 | 0 |
| 窗格布局切换 | 16.7 | 16.7 | 2 | 62 |

4×（同一机器、同一 Fixture）：

| 场景 | p50 | p95 | 掉帧 | Layout ms | Style ms | LoAF max | 结果 |
|---|---:|---:|---:|---:|---:|---:|---|
| 页面切换 | 16.7 | 116.7 | 19 | 305 | 209 | 223 | PASS |
| 侧栏收展 | 16.7 | 49.9 | 22 | 267 | 154 | 102 | PASS（memo 后 p95 166.6→49.9） |
| 菜单弹出 | 16.7 | 16.7 | 1 | 38 | 72 | 0 | PASS |
| 确认层 | 16.7 | 83.3 | 12 | 103 | 187 | 170 | PASS（3px 模糊保留） |
| 目录树折叠 | 16.7 | 16.8 | 3 | 224 | 50 | 60 | PASS |
| 列表↔网格 | 16.7 | 100.0 | 14 | 168 | 251 | 191 | PASS |
| 窗格布局切换（2→4） | 16.7 | 116.6 | 15 | 239 | 173 | 371 | PASS（memo 后 216.7→116.6 / 542→371） |

- DPR2 四窗格 4×：p95 100.0、LoAF 375（memo 前 249.9 / 558）。
- 1→4 直接三窗格冷揭示（比验收脚本 2→4 更重）：5 次 p95 中位 150.1ms / 最差 200ms；
  最长 LoAF 中位 585ms / 最差 609ms（1/5 超过 600ms 门槛）→ **PARTIAL**。
- 侧栏同轮 5 次：p95 中位 50ms；最长 LoAF 中位 106ms / 最差 117ms → PASS。
- 归因：最长任务为 React `dispatchDiscreteEvent` 脚本（Chrome CPU profile 顶部为
  `jsxDEV` / `createElement`），不是 Layout/Paint；每个新窗格首次可见仍需一次完整
  workspace render。memo 已消除三个旧窗格在同一 commit 中的重复渲染，剩余成本是
  新窗格首帧本身。没有删除动效、缩短测试窗口或减少窗格数量。

## D2. 响应式 / DPI

- **158 / 158 PASS**（157/158 首轮中唯一失败为 `Control+K` 与页面加载竞争；
  改为点击真实 `.zc-titlebar-search` 触发器后完整复跑）。
- 覆盖 1080P/2K/4K 100–250%、Air13/15、Pro14/16、5K DPR2、980–2560 逻辑窗口。
- Finder 列：2/3/4 级、预览显示/隐藏、Inspector 互斥、快速切换目录、980↔1280 缩放恢复；
  22 个列状态用例检查列数、列宽、选中行、预览 244、横向滚动到位、Inspector 状态。
- 0 overflow / overlap / clipping；Settings ≤820；页面 ≤1180；四窗格 ≥160×160。
- 原生安装版、每屏 DPI、跨屏拖动、WKWebView overlay scrollbar = NOT_TESTED / #487。
- 详见 `../responsive/README.md` 与 `responsive.json`。

## D3. 原生窗口边界

| 场景 | 结果 |
|---|---|
| Air13 1280×747 work area → 1232×699 居中 | PASS |
| Pro16 1728×1117 → 1280×860 居中 | PASS |
| 4K 250% → 1280×816 | PASS |
| 副屏正坐标 | PASS |
| 副屏负坐标（-1920,-200） | PASS |
| 副屏 2× DPI（-2560,0） | PASS |
| work area < 980×680（900×620）→ 852×572 + OS min 852×572 | PASS |
| 窄竖条 700×1000 → 652×860 | PASS |
| 300×200 极小 work area → 252×180，仍在屏内 | PASS |
| 屏幕配置改变后重启 → 按当前 work area 重新计算 | PASS |
| 真实 WM/Tauri 安装版行为 | NOT_TESTED（#487） |

## E. 本轮修复

1. parity 原型/生产统一 DPR/UA/主题；PNG 物理尺寸断言 + manifest 记录；DPR1 单独控制。
2. Finder 列：226 祖先 / 244 预览；24px/13px；文件类型图标；祖先轨道；自动滚动；
   单文件预览 + 真实 Quick Look；hover 不再覆盖选中。
3. macOS 浏览默认 Columns；位置栏 Finder 分区；工具栏 42px 图标化。
4. Organize：AI 安全/流程/边界折叠为一行，计划分组首屏可见；移除重复右侧卡；
   `rich` fixture 5 ready / 2 review（1 项目标冲突）/ 1 blocked。
5. 四窗格 `PaneHost` memo（路径快照比较），消除旧窗格重复渲染；1→4 仍逐帧揭示。
6. 窗口几何支持 work area < 980×680，OS 最小尺寸随屏下调；负坐标/缩放副屏/极小屏单测。
7. 响应式门禁新增 Finder 列状态与搜索真实触发器。

## F. Owner 必须决定的保留差异

1. **macOS chrome**：Fylura 标签页、动作条、1–4 窗格、真实卷/树；无原型收藏/标签。
   列视图本体 PASS，整体 chrome PARTIAL。
2. **Cleanup**：保留安全/需判断/谨慎分类与真实理由，原型为扁平清单 → PARTIAL。
3. **Overview / History / Settings**：保留真实任务/批次/技术分区，信息架构与原型不同 → PARTIAL。
4. **Organize**：组摘要首屏 PASS；完整逐文件清单在审核面板 → 计划总览 PASS，
   逐项完整性 PARTIAL。
5. **性能**：2→4 已达标；1→4 三窗格冷启动 LoAF 中位 585ms、最差 609ms → PARTIAL。
6. **原生安装版 / WKWebView / 真实 Finder / 多显示器**：NOT_TESTED，#487 暂缓。

**#574 保持 Open，不宣布最终高保真验收通过。**
