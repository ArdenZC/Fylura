# v3.40 响应式 / DPI 适配证据（PR A）

**范围**：基于窗口逻辑尺寸与容器宽度的自适应，覆盖 Windows 1080P / 2K / 4K 的
100–250% 缩放、macOS Air 13/15、Pro 14/16、Studio Display 5K，以及 980×680–2560×1440
的逻辑窗口梯度。**浏览器模拟，不等于原生安装版验收**；原生跨屏移动、真实 DPR
缓存与拖放坐标仍归 #487。

## 1. 结果

- **144 / 144 PASS**，0 overflow / overlap / clipping。
- Chromium `151.0.7922.34`，DPR 1 / 1.25 / 1.5 / 2 / 2.5。
- 原始数据：`responsive.json`；代表性截图：本目录 4 张 PNG。

| 显示 | 用例 | 说明 |
|---|---:|---|
| 1080P 100% (1920×1080) | 32 | 全部 8 个逻辑窗口 + 12 页面/状态 |
| 1080P 125% (1536×864) | 7 | fit 窗口 + Files/Inspector/Grid/Quad/Settings/Search/Preview |
| 1080P 150% (1280×720) | 7 | 同上 |
| 2K 100% (2560×1440) | 7 | 同上 |
| 2K 125% (2048×1152) | 7 | 同上 |
| 2K 150% (1707×960) | 7 | 同上 |
| 4K 150% (2560×1440) | 7 | 同上 |
| 4K 200% (1920×1080) | 7 | 同上 |
| 4K 250% (1536×864) | 7 | 同上 |
| MacBook Air 13 (1280×832, DPR2) | 20 | 含 Columns 980/1080/1280、四窗格、页面 |
| MacBook Air 15 (1440×932, DPR2) | 8 | fit + Columns |
| MacBook Pro 14 (1512×982, DPR2) | 12 | fit + 4 页面 |
| MacBook Pro 16 (1728×1117, DPR2) | 8 | fit + Columns |
| Studio Display 5K (2560×1440, DPR2) | 8 | fit 2512×1392 + Columns |

逻辑窗口梯度：980×680、1080×740、1280×720、1366×768、1440×900、1536×864、
1920×1080、2560×1440。

## 2. 每例检查

- `documentElement/body.scrollWidth` 无横向溢出。
- Workspace 边界不超出 viewport。
- 文件列表 ≥220px，名称单元格 ≥90px，表头/工具栏 `scrollWidth` 不溢出。
- 可见按钮矩形按最近裁剪祖先求交后无重叠（避免把被裁掉的面包屑误报）。
- Settings 内容 ≤820px；Overview/Organize/Cleanup/History `.v340-page` ≤1180px。
- Grid 在 ≥1280 窗口至少 3 列。
- 四窗格：宽布局必须 4 个可见窗格；窄布局允许 1 个活动窗格，但窗格菜单必须仍为 4。
- Search/Preview 矩形完整落在视口内。
- 宽/窄切换后 pane 数、选择、路径由现有 model 保存。

## 3. 本轮修复

1. **13″ MacBook 首次启动窗口超出工作区**：`app_control.rs` 以指针所在显示器的
   `work_area` 计算首次尺寸/位置，默认 1280×860 放不下时缩到 work area−48 并居中；
   保留 980×680 最小尺寸。新增 5 个纯函数单测（Air13、Pro16、4K 250%、副屏、最小窗口）。
2. **Finder 列视图最后一列被拉满**：原型为 226px 祖先列 + 244px 预览列。固定宽度后
   宽屏不再出现 840px 单列，窄窗由容器横向滚动。
3. **5K 下文件列表“大小”列被拉成 1400px**：名称列改为 `1fr` 吸收宽屏，类型/修改/大小
   保持测得宽度；2560 窗口下为 `22 / 1702 / 96 / 118 / 78`。
4. **内容页在 5K 无限拉宽**：`.v340-page` 最大 1180px 居中；Settings 保持 820px。
5. **四窗格窄窗行为**：≤宽布局阈值时显示活动窗格，窗格菜单保留 1–4 与活动窗格切换，
   不再出现按钮互相覆盖；宽布局四格均 ≥160×160。
6. **面包屑与编辑按钮几何重叠**：面包屑 `min-width:0; flex:0 1 auto`，按裁剪后矩形测量。
7. **窗格单元格 containment**：`contain: layout paint`，避免窗格内容参与外层重排。

## 4. 未验证 / 保留

- 原生窗口在真实多显示器间拖动、Windows 每屏 DPI、macOS 内屏↔外接切换：
  **NOT_TESTED**（#487）。
- 浏览器无法动态改变 DPR；DPR 变化后的缩略图/PDF Canvas 缓存刷新只能静态覆盖。
  现有 PDF 渲染 `pixelRatio = min(2, max(1, devicePixelRatio))`，图片缩略图由浏览器按 DPR 解码。
- macOS overlay scrollbar 使用 Chromium 模拟；原生 WKWebView 行为 **NOT_TESTED**。
- 13″ 默认高度采用 work area−48（测试用例 699px），实际菜单栏/Dock 高度由系统给出。

## 5. 复现

```bash
node scripts/runV340Responsive.mjs           # 全矩阵，约 9 分钟
node scripts/runV340Responsive.mjs --quick   # 8 例冒烟
```

矩阵大于 700 个 DOM 断言，不提交数百张 4K/5K 图片；只保留 4 张代表性截图。
