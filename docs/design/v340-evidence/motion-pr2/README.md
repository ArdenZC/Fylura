# v3.40 Motion PR 2 — 菜单、Modal 与状态反馈（核心范围）

基准：`MOTION.md` §3.3 / §3.4。复用 PR 1 的 `utils/motion.ts`、`v340-motion.css`、`PressMotion`、`ModalPortal`，没有新建第二套 Motion 系统。

## 逐项结果

| 对象 | 原型 | 生产实现 | 行为验证 | 结果 |
|---|---|---|---|---|
| Settings 下拉 | 240ms pop / 130ms out；origin 随翻向 | `SettingsPrimitives`：受控 `closing`，WAAPI `zc-menu-in/out`，锚点 top/bottom | 连续 5 次打开均 240ms；Esc 后 `data-closing=true` + 130ms，再卸载；outside 焦点不抢 | PASS |
| 多窗格布局菜单 | 同上；action 立即关闭 | `FileLibraryWorkbench` 由 `<details>` 改为受控按钮 + 面板，保留原控件 | 5 次 240ms；Esc 130ms；操作按钮同帧关闭；outside/Esc 焦点正确 | PASS |
| Files 右键菜单 | 240/130；origin 指向锚点 | `FileLibraryContextMenu` + library/browse 两个 hook 的 `closing` 状态 | 5 次 240ms；Esc 130ms 后卸载；action/dialog-handoff 同帧关闭 | PASS |
| 全局搜索浮层 | 卡片 240ms pop；遮罩 200ms；退出 130ms | `CommandModal` 240ms Motion 入场；`AppShell.closeCommand` 先取消入场再播 `zc-modal-out`，130ms 后卸载；`useAppChrome` 不再抢 Esc | 进入 240ms；Esc 后仍挂载并运行 130ms out；最终卸载；ModalPortal 焦点归还路径未改 | PASS |
| 搜索遮罩 | 200ms 淡入 | `zc-scrim-in` | 入场 1 个动画实例 | PASS |
| Quick Preview | 260–280ms 入场；关闭先收 | 卡片 260ms pop 入场；关闭仍由 `controller.close()` 同步释放资源，组件立即卸载 | 连续 5 次打开每次 260ms；Esc 后 80ms 内释放；Pin 后浮动层关闭且仍只 1 个预览壳，Unpin 恢复且按钮 `aria-pressed` 正确 | PASS（入场）；退出见保留差异 |
| Toast | 320ms bounce / 130ms out | `ToastContainer` 本地 presence：进入 320ms，退出 130ms；持久 watcher/索引错误同帧解析，不新增清除计时器 | `motionOverlayPresence`：成功 Toast 320ms + bounce；持久告警无退出滞留 | PASS |
| 工具栏 More / 文件篮 / 位置 / 结果 / 任务面板 | 240/130 | `FileManagementToolbar`：受控 `panelClosing`，Esc/外部/关闭按钮 130ms 退出；动作点击同帧关闭 | 连续 5 次 240ms；Esc 后 `data-closing` + 130ms 卸载；动作不等待 | PASS |
| 确认层 / 文件操作弹窗 | 淡入 + 轻微缩放 + Y 位移 | `ConfirmDialog` 与新建/冲突弹窗标记 `data-confirm-backdrop/card`，200ms 遮罩 + 240ms 卡片 | `zc-dialog-card-in` 0.24s、`zc-scrim-in` 0.2s；Esc 立即执行关闭 | PASS（入场）；退出立即见保留差异 |
| 任务中心 | 展开/关闭 | 即工具栏 `tasks` 面板，同上 | 同上 | PASS |
| Badge 数量变化 | 320ms 单次轻弹 | `CountBadge` 仅值变化时 `zc-badge-pop`；侧栏整理徽标与文件篮计数接入 | 组件测试：变化 1 次、同值不重播 | PASS |
| 多选底部浮条 | 浮条进出 | 生产版没有该浮条（选择状态由状态栏表达），本轮不新增假 UI | — | N/A |
| PR 1 回归：窗格重播 | 只让新窗格落位 | `paneMotionRef` + id 同步，只对 `index >= previous.count` 或新 id 播放 | 1→2 只有新 pane 260ms；旧 pane 无动画 | PASS |
| PR 1 回归：带 transform 的按钮 | 保持定位 | 树把手已单独保留 `translateY(-50%)`；其余无按钮定位 transform | 源码审计 + 手动按压 | PASS |

## 实测指标

`metrics.json`（Chromium browser mock，1440×900，DPR 1）：

- Settings 下拉：`zc-menu-in` 240ms；5 次重复均存在。
- 窗格菜单：`zc-menu-in` 240ms；Esc 后 `zc-menu-out` 130ms。
- 右键菜单：`zc-menu-in` 240ms；Esc 后 130ms 退出。
- 搜索：卡片 240ms；遮罩 1 个动画；Esc 后 `zc-modal-out` 130ms。
- 工具栏面板：`zc-menu-in` 240ms；Esc 后 `zc-menu-out` 130ms。
- 新建文件夹确认层：卡片 `zc-dialog-card-in` 0.24s、遮罩 `zc-scrim-in` 0.2s，各 1 个动画实例。
- Quick Preview：`zc-preview-in` 260ms；Esc 后离开 `data-preview-shell`。
- Reduced Motion：Settings 下拉无动画、Esc 后 20ms 内卸载。

## 复现

```bash
node scripts/runV340OverlayBrowserGate.mjs   # 菜单/搜索/工具栏/确认层/预览/弱动效/macOS
node scripts/runV340MotionBrowserGate.mjs   # PR 1 回归：按压/侧栏/树/视图/窗格/选中
npx vitest run tests/fileLibraryW210ContextMenu.test.tsx tests/settingsComponentSystem.test.tsx tests/motionPressFeedback.test.tsx
```

## 保留差异 / 未完成

- **Quick Preview 退场**：关闭必须同步释放预览资源（来源身份、Pinned/Floating、安全规则优先）。不延迟 `controller.close()`，因此没有可观察的 130ms 退出；入场已实现。若后续要做退出，只能渲染静态快照，不能在释放后继续挂载真实预览。
- **确认层 / 文件操作弹窗的退出**：与 `ConfirmDialog` 的语义一致，确认/取消必须同帧执行真实操作，因此没有可观察的 130ms 退出；入场已实现。
- **多选底部浮条**：生产版没有该浮条，选择数量与范围由底部状态栏真实表达；按“不新增假 UI”的约束保留差异。
- **Toast 的浏览器触发**：持久错误来自真实的 watcher/index 状态，浏览器 mock 下难以稳定制造失败；进入/退出行为由组件测试覆盖，浏览器只提供视觉截图。
- 本 PR 不包含 PR 3 的视觉对照、4× CPU 七项性能与跨平台最终走查。
