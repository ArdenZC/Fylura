# v3.40 同状态视觉对照与最终证据（聚焦整改）

本目录是本轮高保真收敛的验收材料。原型来自仓库
`docs/design/prototypes/Fylura-proto-v3.40.zip`（`DESIGN-SPEC.md`、`MOTION.md`、
`HANDOFF.md`）；生产来自当前 `master` 上的真实 React 组件与 browser fixture。

**结论口径**：这里证明的是 Chromium 浏览器模拟下的结构、密度、材质和真实交互状态。
macOS 使用 MacIntel UA + DPR2，不是原生 WKWebView / Finder 验收；原生安装版整体验收
仍归 #487。PARTIAL 不计入 PASS。

## 1. 方法与本轮修正

- 原型和生产使用**同一个** Chromium context 参数：viewport、`deviceScaleFactor`、
  `colorScheme`、locale、时区和平台 UA（MacIntel / Win32）。
- 每对截图写入前读取 PNG IHDR，断言两侧物理像素尺寸完全一致；不一致直接抛错，
  不写入 manifest。manifest 记录 viewport、DPR、物理像素、平台、主题和场景。
- 原型仍然从仓库 ZIP 自动解压（`fflate`），不依赖本机路径。
- `mac-columns-dpr1` 是独立的 1× 对照，**不与 Retina 证据混用**。
- 生产 Fixture 只走现有 Browse / Plan / Finding 展示契约，不接入模拟执行器。

上一轮 P0 的错误：原型用 `browser.newPage({viewport})`（DPR1），生产用
`deviceScaleFactor: 2`，导致同名“Retina 对照”一边 1280×832、一边 2560×1664。
本轮两侧统一 DPR2，并保留 DPR1 单独控制组。

## 2. 场景（33 组）

| 场景 | 数量 | 状态 |
|---|---:|---|
| Files + Inspector / Overview / Organize plan / Cleanup / History / Settings / 4 panes / menu / search | 18 | 1440×900，light/dark，DPR1，Win32 |
| Files 1080×740 / 850×680 | 2 | light，DPR1 |
| macOS Finder Columns 1440×900 | 2 | light/dark，DPR2 |
| macOS Finder Columns Air13 1280×832 | 2 | light/dark，DPR2 |
| macOS Finder Columns Pro14 1512×982 | 1 | light，DPR2 |
| macOS Finder Columns Pro16 1728×1117 | 1 | light，DPR2（三级列 + 中文长文件名预览） |
| macOS Finder Columns 5K 2560×1440 | 1 | light，DPR2（中文长文件名） |
| macOS Finder Columns 980×680 | 1 | light，DPR2 |
| macOS Finder Columns DPR1 控制组 | 1 | light，DPR1，单独标注 |
| Organize AI 不可用状态 A | 1 | 1440×900，DPR1 |
| macOS 四窗格（生产独有） | 1 | 1512×982，DPR2 |
| macOS 四级列 / 横向滚动（生产独有） | 2 | 980×680 与 1512×982，DPR2 |

`manifest.json` 是唯一索引；`index.html` 由脚本按 manifest 生成，包含每对的
物理像素尺寸。所有截图均为真实页面状态，不靠 CSS 声明推断 PASS。

## 3. macOS Finder Columns

### 3.1 对齐的状态

- 视图模式：Columns。
- 目录层级：`C:/U3/Finder` → `Documents` → `Workspace`，三级祖先列。四级列
  （`Workspace/Assets`）原型没有对应演示数据，作为生产独有证据单独提交。
- 选中状态：当前列选中一个真实文件（`assets-2025.zip` / `Design tokens v2.pdf` /
  `poster.png` / 中文长文件名），祖先列显示通向当前目录的 Finder 轨道高亮。
- 预览：单文件选中时在右侧渲染 244px 文件信息列；多选或打开 Inspector 时不渲染。
- 尺寸：Air13 1280×832、Pro14 1512×982、Pro16 1728×1117、1440×900、980×680、
  5K 2560×1440，全部 DPR2。

### 3.2 本轮落地

- 祖先列 226px，当前文件列 226px，预览列 244px；≤1320 / ≤1200 / ≤1040 按原型
  收缩到 206 / 190 / 182px。
- 行高 24px、13px 字号、5px 圆角、6px 图标间距；文件夹蓝色图标 + chevron，
  文件按扩展名着色。
- 文件夹单击导航、祖先列回退、双击文件无副作用；文件通过真实选择身份选中。
- 选中/祖先轨道使用系统蓝填充 + 白字；hover 不再覆盖选中背景。
- 列区域水平滚动，并在目录变化时自动滚到当前列；位置栏面包屑仍可回退。
- 预览列显示真实名称、类型/大小、修改时间、大小、位置，并提供真实 Quick Preview
  入口（复用现有 `previewController.open`）。
- macOS 浏览模式默认 Columns；标签页、1–4 窗格、跨窗格传输、选择身份未改。
- macOS 文件位置栏分区改为 Finder 语义：位置 / 文件夹 / 其他位置；工具栏改为
  42px、图标按钮、分段视图切换。

### 3.3 保留差异（PARTIAL）

- 生产仍保留 Fylura 标签页、动作条（新建/复制/剪切/粘贴/任务中心/更多）和
  1–4 窗格；原型是单 Finder 视图。这是产品身份，不是遗漏。
- 生产左侧是真实卷/目录树/可浏览位置，没有原型的“个人收藏/标签”演示数据；
  不伪造收藏或标签。
- 列头保留 24px 名称/已加载计数，用于说明分页真相；原型没有列头。
- 以上差异在 `MATRIX.md` 记为 **PARTIAL**，由 Owner 决定是否接受。

## 4. AI Organize 两个状态

### 状态 A：AI 未配置 / 不可用

`prod-organize-unavailable-light-1440x900@1x.webp`。保留真实 AI 不可用状态：
顶部安全条明确写出 AI 提供商未就绪、新的分析不可用、受阻项目不会进入 Dry Run 或执行；
详细 provider / scope / workflow / 执行边界在可展开区域完整保留，没有伪造执行资格。

### 状态 B：已有合法展示的整理计划

`prod-organize-plan-light/dark-1440x900@1x.webp`，query 为
`pm01-organize=rich&pm01-readiness=ready`：

- 8 项计划、4 格真实 KPI（8 文件 / 0 已纳入 / 2 待决定 / 1 暂不可处理）。
- 目标目录分组完整出现在首屏：Brand / Assets / Archive / Notes 四组；每组显示
  真实样本文件名、来源目录、目标目录、大小、风险、审核原因与组级动作，全部来自
  现有 plan/group 契约。完整逐项清单仍在“查看分组”面板。
- 安全条压缩为一行：真实阻断数 + 待审数；展开后仍有 provider、scope、四步流程、
  执行边界与阻断说明。
- 右侧重复的“执行边界”卡已移除，分组恢复全宽；执行条固定在底部，Dry Run /
  执行仍由原有后端资格与确认链路决定。
- `rich` fixture 为 5 ready / 2 review（其中 1 项带目标冲突）/ 1 blocked；
  冲突计数在“需要我决定”页签的分组行显示（计划页签只列 ready/reviewed 组）。
  冲突数来自 plan/group 契约，不绕过任何执行资格，也不模拟执行成功。

## 5. 明确保留的信息架构差异（PARTIAL）

| 页面 | 生产保留 | 原型差异 | 结果 |
|---|---|---|---|
| Cleanup | 安全候选 / 需判断 / 谨慎处理分类、真实风险理由、批量选择 | 原型为扁平 findings 清单 | PARTIAL：保留风险分类，不为了截图删除 |
| Overview | 真实任务、系统覆盖、后台状态、索引/权限分支 | 原型有最近文件/问候语节奏 | PARTIAL：无装饰性问候语 |
| History / Restore | 批次 + 明细双栏、恢复入口、失败/部分成功 | 原型更简洁时间线 | PARTIAL：保留可恢复性细节 |
| Settings | 212px 分区导航 + 60px 搜索头 + 820px 内容列、单分区、深链/搜索定位 | 原型内容分区更少 | PARTIAL：技术分区与 AI 管理范围是生产必要内容 |

这些差异不是“相关功能存在即 PASS”；`MATRIX.md` 单独列出并交 Owner 接受。

## 6. 复现

```bash
node scripts/captureV340Parity.mjs          # 全部 33 组
node scripts/captureV340Parity.mjs --mac-only
```

脚本会删除旧的 `proto-*/prod-*` 图片后重新生成；`manifest.json` 与 `index.html`
同步重建。DPR 不一致或物理尺寸不一致会直接失败。

## 7. 测量限制

- 所有数据来自 Chromium browser fixture，非安装版 / WKWebView / WebView2。
- macOS 系统字体、overlay scrollbar、真实 Finder 拖放和 Quick Look 由 #487 验收。
- `index.html` 中的图片为 WebP 质量 0.9，由 PNG 转换以控制仓库体积；物理尺寸断言
  在 PNG 上完成，转换不改变尺寸。
