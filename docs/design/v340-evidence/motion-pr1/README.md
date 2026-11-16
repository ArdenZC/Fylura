# v3.40 Motion PR 1 — 基础与高频交互

基准：`MOTION.md` §3（按压 / 侧栏 / 目录树 / 文件行 / 视图与窗格 / 页面）+ §6 弱动效。
本目录只覆盖 PR 1；菜单、搜索浮层、Quick Preview、确认/执行层、Toast 与徽标属于 PR 2，状态为 `NOT_TESTED`，不得据此宣布全量动效通过。

## 逐项对照

| 对象 | 原型规范 | 生产实现 | 验证 | 结果 |
|---|---|---|---|---|
| 全部按钮 / 导航 / 工具 / 标签页 / 分段 | 按下 90ms `spring-out`；松手 260ms `spring-pop` | `components/motion/PressMotion.tsx`（pointerdown/up/cancel/out、keydown/up、blur） | `motionPressFeedback.test.tsx` 4 项；`test:browser:motion:v340` 实测 `data-zc-press` 与 scale | PASS |
| 禁用 / 菜单项 / 文件数据行 | 不缩放 | `PressMotion` 排除 `role=option/menuitem`、`data-file-row`、disabled、inert | 同上（disabled/option/file-row 断言） | PASS |
| 文件行 Hover | 120ms 颜色 | `v340-motion.css`：仅未选中行 hover 过渡 | 浏览器实测 `transitionDuration=0.12s` | PASS |
| 文件行 Selection | 瞬切 | 选中态 `transition: none !important` | 浏览器实测 `0s`；选中底色同帧到位 | PASS |
| 文件复选框勾 | 260ms `spring-bounce` | `zc-file-check-in` 只作用于勾的 SVG | 浏览器实测 260ms 动画 | PASS |
| 应用侧栏 展/轨/隐 | 200ms 容器宽度；文字先退场；徽标数字→6px 点 | `v340-shell.css`：grid 轨道 + `max-width/opacity`，文字不 `display:none`；侧栏始终挂载 | 216→75→64ms 采样；隐藏 4.7→0；快速反向切换回 216/216；`aria-hidden`+`inert` | PASS |
| 目录树面板 | 200ms 宽度 + 160ms 透明度；内容不重排 | 保持大窗导航挂载；slot 宽度过渡，panel 固定 `--file-tree-width` | 196→14.4→0；关闭中 panel 保持 196；重开 181.5→196 | PASS |
| 目录树节点 / 导航分组 | `grid-template-rows 1fr→0fr` + 内容淡出 | `DirectoryTree` `.file-library-tree-branch`；`FileLibraryNavigation` `.file-library-navigation-group-body` | `v340MotionSystem.test.ts`；树节点展开动画与真实高度实测 | PASS |
| 窗格 1–4 | 每格 opacity + `scale(.982→1)` 260ms；不动 grid 轨道 | `FileLibraryWorkbench` effect + `playPaneEnter` WAAPI | 浏览器实测 2 个可见 pane 各 260ms `zc-pane-in` | PASS |
| 列表 ↔ 网格 | 整块 opacity .45→1 + 5px，240ms；不逐行 | `FileLibraryWorkspace` + `playViewSwap` WAAPI | 浏览器实测 `zc-view-swap` 240ms | PASS |
| 页面切换 | 220ms 淡入 + 4px 上浮；不横向推屏 | `AppShell` + `playPageEnter` WAAPI | 浏览器实测 `zc-page-in` 220ms；结束 opacity 1 / transform none | PASS |
| 窗格分隔条 | 拖动 1:1，无过渡 | 保持原 pointer/键盘实现，未加过渡 | 现有 W-10/U3 门禁 | PASS（未改） |
| 焦点环 | 180ms 扩散 | `v340-motion.css` outline/box-shadow 过渡 | 现有可访问性测试 + 手动键盘 | PASS |
| 滚动条 | overlay、内缩、悬停加深 | `v340-motion.css` 全局滚动条 | 视觉核对 | PASS |
| 弱动效 | CSS + WAAPI + JS 全部等价 | 全局 CSS 归零；`utils/motion.ts` 每个入口读 `prefersReducedMotion` | 浏览器 reduced-motion：侧栏 30ms 直达 64、页面 0 个 WAAPI、按压无 transform | PASS |

## 实测指标

见 `metrics.json`（Chromium，browser mock，1440×900，DPR 1）：

- 侧栏：216 → 75.2（80ms）→ 64；隐藏 4.7（80ms）→ 0；文字 opacity 80ms 时 0.003。
- 目录树：196 → 14.4（80ms）→ 0；关闭中 panel 仍为 196；重开 181.5 → 196。
- 文件选中：`transitionDuration=0s`；勾 260ms。
- 视图切换：`zc-view-swap` 240ms；窗格：2×`zc-pane-in` 260ms；页面：`zc-page-in` 220ms。
- Reduced Motion：侧栏 30ms 内 64，transition `1e-06s`。

## 证据文件

`sidebar-expanded/rail-mid/rail/hidden-mid/hidden.png`、`tree-open/closing-mid/reopened.png`、`file-selected.png`、`metrics.json`。
均为 browser mock 下的 Chromium 截图，不是安装版原生验收。

## 复现

```bash
node scripts/runV340MotionBrowserGate.mjs
node scripts/captureV340MotionEvidence.mjs
npx vitest run tests/motionPressFeedback.test.tsx tests/v340MotionSystem.test.ts
```

## 尚未做 / 下一批

- PR 2：文件右键菜单、工具栏/多窗格菜单、设置下拉、全局搜索、Quick Preview、确认层、执行层、Task Center、Toast、多选浮条、页面内浮层退场与打断。
- PR 3：同状态视觉对照、七项性能测量（普通 / 4× CPU）、跨平台与完整交互状态。
- 未做安装版原生验收；#487 继续暂缓。
