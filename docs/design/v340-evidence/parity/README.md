# v3.40 同状态视觉对照与最终证据

本目录是 PR 3 的验收材料。原型来自 `docs/design/prototypes/Fylura-proto-v3.40.zip`
（`DESIGN-SPEC.md`、`MOTION.md`、`HANDOFF.md`），生产来自本仓库 `master` 上的真实 React 组件。

## 1. 方法

- 同一 Chromium、同一窗口尺寸、DPR 1、同一主题和同一交互状态。
- 原型直接从仓库 `docs/design/prototypes/Fylura-proto-v3.40.zip` 解压（`fflate`，
  不依赖本机路径）；Windows 用 `redesign-v2.html`，macOS 用 `redesign-v2-macos.html`。
- 生产使用 `?browser-mock=1&u3-folder-move=1&devices-fixture=1` 的展示 Fixture：
  Files 四个窗格分别为 `C:/U3/Left`、`C:/U3/Right`、`C:/U1/Source`、`C:/U1/Organize`；
  Organize 为 8 项真实计划；Cleanup 为 9 条 finding；侧栏含只读 SSD 与离线 NAS。
- Fixture 只走现有 Browse/Plan/Finding 契约，不接入任何模拟执行器；原型与生产的文件名/数量
  仍不同，因此比较结构、密度、材质和控件排布，不做像素差。
- Files 场景在两种环境都选中首项并打开 Inspector；四窗格场景切到 4 栏；
  菜单场景展开布局菜单；搜索场景打开空查询浮层。
- 截图为普通 1× 渲染；提交的 WebP（质量 0.9）由 PNG 转换，仅用于减小仓库体积。

## 2. 场景

| 场景 | 原型 | 生产 | 尺寸 |
|---|---|---|---|
| Files + Inspector | `proto-files-*.webp` | `prod-files-*.webp` | 1440/1080/850 × light/dark |
| macOS Files / Columns | `proto-mac-*` | `prod-mac-*` | 1440 DPR2 及 Air13 1280×832 |
| Overview | `proto-overview-*` | `prod-overview-*` | 1440 × light/dark |
| AI Organize | `proto-organize-*` | `prod-organize-*` | 1440 × light/dark |
| AI Cleanup | `proto-cleanup-*` | `prod-cleanup-*` | 1440 × light/dark |
| History / Restore | `proto-history-*` | `prod-history-*` | 1440 × light/dark |
| Settings | `proto-settings-*` | `prod-settings-*` | 1440 × light/dark |
| Files 四窗格 | `proto-panes4-*` | `prod-panes4-*` | 1440 × light/dark |
| 布局菜单 | `proto-menu-*` | `prod-menu-*` | 1440 × light/dark |
| 全局搜索 | `proto-search-*` | `prod-search-*` | 1440 × light/dark |

`index.html` 是 25 组双图画廊；`manifest.json` 记录文件、DPR、平台与 Chrome 版本。

## 3. 本轮修复的结构差异（有截图/几何证据）

1. **Inspector 打开时列表被压到 370px**：工作区网格错误地保留了第三个空列
   （`196px 710px 288px`），而 Inspector 实际渲染在 content 内部。移除空列后
   列表恢复 `658px`，列宽 `22 / 282 / 96 / 118 / 78`，与原型
   `22 / 293 / 96 / 118 / 78` 基本一致。
2. **多余的「创建时间」列**：v3.40 文件表只有名称 / 类型 / 修改时间 / 大小。
   已从表格移除创建列（仍保留在 Inspector 事实行与排序菜单），并迁移旧的
   五列本地列宽快照。
3. **类型列过早消失**：原阈值 900px 使 1440 宽 + Inspector 打开时丢失类型列；
   按原型 660px 内容宽仍显示类型，阈值改为 540px。
4. **搜索浮层 720px / 20vh**：原型为 560px / 14vh，且遮罩带 3px 模糊。已对齐。
5. **Inspector 标题「上下文」**：原型为「信息」，已对齐文案。
6. **设备行缺少容量条/只读徽标**：侧栏设备行现在使用真实
   `totalBytes/availableBytes/readOnly` 渲染 3px 容量条、只读/云盘/离线 chip、
   hover 弹出按钮；browser mock 无挂载设备，因此截图里看不到，组件测试覆盖。
7. **5K 下文件列表大小列被拉宽**：名称列改为 `1fr`，类型/修改/大小保持实测宽度；
   2560 窗口为 `22/1702/96/118/78`。
8. **Finder 列视图最后一列被拉满**：固定 226/244px 后宽屏不再出现 840px 单列。
9. **多窗格菜单 1/2/3/4 分段控件丢失样式**：PR 2 重建弹层时只留下布局钩子，
   数字渲染为裸文本。补齐 segmented 背景/选中态/焦点环（268px 菜单卡片）。
10. **主区 20px `backdrop-filter` 造成每帧全屏重栅格**：1× CPU 下所有浮层动画
   只有 33.3ms p50。主区面板本身 93% 不透明，模糊几乎不可见；移除后浮层
   p50 回到 16.7ms（详见 `PERF.md`）。侧栏/标题栏玻璃保留。

## 4. 保留差异（明确、有理由）

| 差异 | 原因 | 状态 |
|---|---|---|
| Overview 无「下午好，…」问候 | 产品既有决定：不做个人化问候；状态带/任务行保留 | 保留 |
| 原型演示文件与生产 mock 文件不同 | 生产不得注入假文件操作；Fixture 只走展示层 | 保留 |
| 生产没有多选底部浮条 | 选择范围/数量由真实状态栏表达；不新增假 UI | N/A |
| 设备行使用展示 Fixture | 只读 SSD/离线 NAS 可见容量条与 chip；真实凭据/弹出仍由原生验收覆盖 | PASS（展示） |
| Inspector 关闭时面板直接消失，列表宽度 200ms 收回 | 避免延迟释放选中详情；列表形变有动画，面板内容不做假快照 | PARTIAL |
| Quick Preview 关闭立即释放资源 | 预览身份/安全/资源释放优先，不退场 | PARTIAL |
| 原生安装版、真实 macOS overlay scrollbar | #487 暂缓，本轮只做 Chromium 平台模拟 | NOT_TESTED |

## 5. 复现

```bash
node scripts/captureV340Parity.mjs   # 自动从仓库 ZIP 解压原型
node scripts/captureV340MotionFrames.mjs
node scripts/runV340Perf.mjs
node scripts/runV340OverlayBrowserGate.mjs
node scripts/runV340MotionBrowserGate.mjs
```

验收矩阵见 `MATRIX.md`，性能原始值见 `perf/perf-1x.json`、`perf/perf-4x.json`，
动效关键帧与动画 id/时长见 `motion-frames/timeline.json`。
