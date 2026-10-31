# Fylura

<div align="center">
  <img src="docs/banner_zh.svg" width="100%" alt="Fylura Banner" />
</div>

<br />

<div align="center">
  <a href="README_en.md">
    <img src="https://img.shields.io/badge/Switch_To_English_Edition-0f172a?style=for-the-badge" alt="English Edition" />
  </a>
</div>

<div align="center">
  <img src="https://img.shields.io/badge/Tauri_2-24C8DB?style=for-the-badge&logo=tauri&logoColor=white" alt="Tauri 2" />
  <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/React_19-61DAFB?style=for-the-badge&logo=react&logoColor=black" alt="React 19" />
  <img src="https://img.shields.io/badge/TypeScript-3178C6?style=for-the-badge&logo=typescript&logoColor=white" alt="TypeScript" />
  <img src="https://img.shields.io/badge/Vite_8-646CFF?style=for-the-badge&logo=vite&logoColor=white" alt="Vite 8" />
  <img src="https://img.shields.io/badge/Tailwind_CSS_4-06B6D4?style=for-the-badge&logo=tailwindcss&logoColor=white" alt="Tailwind CSS 4" />
  <img src="https://img.shields.io/badge/SQLite_FTS5-003B57?style=for-the-badge&logo=sqlite&logoColor=white" alt="SQLite FTS5" />
</div>

---

## 简介

> **本地优先的个人文件生命周期管理助手。**
> Fylura 不替代系统资源管理器，也不是简单文件分类器；它把空间扫描、快速索引、智能解释、整理预览、安全执行和恢复记录串成一个可控闭环。

## 支持平台

- Windows。
- macOS 13+，仅支持 Apple Silicon（arm64）。Intel Mac 不在支持范围内；不提供 Universal Binary 或 Rosetta 兼容保证。

## 开发版身份迁移

首次启动 Fylura 时，如果检测到旧开发版数据库，应用会在旧数据目录仍保留原件的情况下，将 SQLite 主库通过 SQLite Backup API 复制到新目录，并验证完整性。各平台对应数据根目录下的数据库从 `com.startlan.zencanvas/zen-canvas.sqlite3` 迁移到 `com.startlan.fylura/fylura.sqlite3`；复制包含 WAL 中已提交的状态，因此文件库、索引授权、History、Journal 和 Restore 记录继续可用。已有 Fylura 数据库始终优先，不会被旧库覆盖；若主库缺失但发现 WAL、SHM 或 Journal sidecar，应用会停止迁移并保留现场等待检查。迁移不会发起新的全盘索引。

迁移前请先关闭 Zen Canvas 旧开发版及其 Windows 索引服务。Windows 安装器发现旧预览处理器仍占用共享 COM 注册时会安全中止；请先卸载旧开发版再安装，新旧卸载流程只处理各自拥有的安装、服务和系统注册，不删除用户数据。如果旧版启用了登录时启动，Fylura 只会在启动项名称和旧可执行文件路径完全匹配时迁移该设置；无法确认归属时会保留旧启动项。迁移完成后只启动 Fylura，避免旧版继续写入旧数据库形成分叉。历史 Safe Trash 目录等用户文件路径保持原样。

## 核心能力

- **Overview / Scan**：支持扫描用户空间或通过 Tauri 系统目录选择器选择指定文件夹；项目目录会被识别为父级项目资产，默认不深入移动内部工程文件。扫描页当前展示的磁盘容量是参考值，后续会按扫描根目录所在磁盘统计。
- **Global Search**：常驻顶部中央，Windows 使用 `Ctrl + K`，macOS 使用 `⌘ K`；主窗口关闭时可唤起独立毛玻璃搜索框。
- **Organize Files**：用“正在使用 / 可归档 / 隐私敏感 / 临时清理”四区解释文件去向，整理建议不会绕过预览与确认执行链。
- **File Library**：用于查看已管理文件、状态筛选和分类原因；具体找文件优先使用 Global Search。
- **Storage Cleanup**：分析持久化清理发现，确认后的候选项通过 Safe Trash 路径处理，并保留恢复记录。
- **Preview & Execute**：按主文件夹和子文件夹展示整理方案；移动、重命名、清理和符合条件的永久删除都必须经过权威预览与显式确认。
- **History / Restore**：查看 Fylura 自己执行的操作与清理记录，并在身份重新校验通过后恢复仍可恢复的结果；记录持久化在 SQLite 中，并按设置的保留天数自动清理。
- **Automation**：Manual / Schedule / Managed files changed 触发只负责准备可审核的 Organization Plan，始终 review-required / never-auto-execute；Rule Repository / Rule Proposal 保留在 Advanced Policies，不能作为新的 Organize / Cleanup 语义 fallback。
- **Content Understanding**：在已管理文件范围内，根据明确的策略与同意提取和理解内容；结果不会成为文件系统变更权限。

## 搜索能力

- 本地 SQLite WAL + FTS5 trigram 索引，不依赖 Everything、Spotlight 或系统搜索服务。
- 支持文件名、路径、空格分词和扩展名过滤。
- 默认搜索和分页查询会排除 stale 文件，避免临时删除事件破坏用户可见状态。
- 批量扫描和大批 watcher upsert 后会执行 SQLite `PRAGMA optimize`，并通过 `search-index-optimized` event 暴露触发来源、耗时和错误信息。
- 结果支持打开文件、系统定位、进入文件库详情。
- 性能验证包含前端架构守卫和真实 SQLite/FTS benchmark。

## 增量索引

- watcher 的 remove / delete 事件只会把文件标记为 stale，不会直接删除 `files` 记录。
- `files` 表记录 `is_stale` 和 `last_seen_at`；搜索、分页、统计和规则执行默认排除 stale 文件。
- create / modify / rename / change 事件会 debounce 后批量 upsert；重新出现的文件会 revive stale 记录。
- watcher upsert 后只对受影响 paths 调用 `execute_rules_for_paths` 做轻量分类，不触发全库规则重跑。
- 大批 watcher upsert 达到阈值时会触发 search index optimize；失败只记录 warning，不影响 upsert。
- watcher 深度索引目录达到安全上限时会提示用户手动运行完整扫描，避免把部分更新误认为完整索引。

## 安全执行与恢复

所有涉及用户文件的变更都遵循以下受控链路：

```text
意图
  → 权威 Operation Preview
  → 显式确认
  → 后端身份与路径重新校验
  → operation/cleanup journal 或 Safe Trash
  → 文件系统变更
  → 持久化结果
  → History / Restore
```

对符合条件的已管理文件，永久删除会进入独立的显式预览与确认流程，不会作为扫描、索引、整理或清理分析的隐式结果执行。

- 执行请求只携带后端生成的预览 ID 和文件 ID；Rust 会重新读取权威路径、动作和文件身份，不信任前端提交的源/目标路径。
- 文件系统变更前先写入 pending journal；启动时会协调上次中断的操作，再将结果落为 success / failed / skipped。
- 恢复请求只携带日志 ID；Rust 仅允许恢复数据库中 success、can_restore 且尚未恢复的记录，并回写 `restore_status`、`restored_at`、`restore_error` 和 `can_restore`。
- 应用启动时会从 SQLite 读取最近 operation logs，恢复记录不再只是 React state。
- execute / restore 成功后会同步更新 `files` 表和 FTS，确保文件库和搜索结果指向真实路径。

## Advanced Rules / 兼容分类

- 内置规则与用户规则仍服务于既有 Rule/Policy、watcher 分类与兼容场景；用户规则持久化在 SQLite `rules` 表中。AI-only 迁移后，它们**不是**新的 Organize / Cleanup 语义 fallback，也不具有文件系统执行权。
- `rule_version` 使用稳定 hash，不依赖 `DefaultHasher`。
- `files` 表保存分类指纹：`last_classified_at`、`classified_rule_version`、`last_classified_mtime`、`last_classified_size`。
- `execute_rules_on_inbox` 只处理 `lifecycle = Inbox` 且 `is_stale = 0` 的文件，并跳过 rule version、mtime、size 都未变化的记录。
- `RuleExecutionSummary` 会返回 `skipped`，便于区分已扫描候选和实际重分类数量。
- 后续规则能力会优先补充版本管理、导入导出、冲突检测和更细的规则审计。

## 安全边界

- 启动不自动扫描，扫描只建立索引和建议。
- 涉及用户文件的变更统一经过：意图 → 权威 Operation Preview → 显式确认 → 后端重新校验 → operation/cleanup journal 或 Safe Trash → 文件系统变更 → 持久化结果 → History / Restore。
- 对符合条件的已管理文件，永久删除只作为独立的显式预览与确认流程出现，不是扫描、索引或整理的隐式副作用。
- 默认跳过部分系统目录和明显生成目录，例如 `.git`、`node_modules`、`.venv`、`__pycache__`、`dist`、`build`、`target`、`coverage`、`vendor`、`Windows`、`Program Files`、`System Volume Information`。
- 敏感文件只显示建议和原因，不生成默认可执行勾选。
- 冲突、低置信、规则接近项默认进入待确认队列。
- Storage Cleanup 只把精确白名单缓存路径默认视为安全候选，不会把整个 `.cargo`、`.m2` 或 `.gradle` 当作缓存清理；所有 Safe Trash 操作仍需预览确认。
- Tauri command 层会再次校验移动、重命名和恢复操作的权威 ID、文件身份、类型、绝对路径、安全文件名、系统目录和覆盖冲突。
- AI API Key 由系统凭据存储保存，生产接口不会把密钥回传给前端。
- watcher 删除事件只标记 stale，不直接破坏索引历史。
- watcher 对超大目录可能只做部分增量索引并提示手动完整扫描。
- execute / restore 后会同步更新 `files` 表和 FTS。
- search index optimize 失败只记录 warning，不会让扫描或 upsert 失败。
- Tauri CSP 已配置；前端不直接访问文件系统，扫描、索引、移动、重命名和恢复都在 Rust command 层处理。

## 技术架构

```text
React 19 + TypeScript + Tailwind CSS 4 UI
  -> Tauri 2 commands / events
    -> Rust backend
      -> SQLite WAL + FTS5 trigram
      -> r2d2 connection pool
      -> jwalk scanner + notify watcher
      -> stale/upsert incremental indexer
      -> operation log + restore journal
      -> guarded move / rename / restore executor
      -> rule classifier with stable rule version + file fingerprint
      -> PRAGMA optimize after bulk writes
```

## 项目文档

- [当前项目状态](docs/project/STATUS.md)
- [产品地图](docs/project/PRODUCT_MAP.md)
- [架构地图](docs/project/ARCHITECTURE_MAP.md)
- [开发工作流](docs/project/DEVELOPMENT_WORKFLOW.md)

## 开发

```bash
npm install
npm run dev
npm run typecheck
npm test
cd src-tauri && cargo test && cargo check --features desktop-runtime && cd ..
npm run test:performance
npm run build
npm run security:audit
```

`npm run test:performance` 会先执行前端架构守卫，再运行 Rust SQLite/FTS benchmark。默认 benchmark 会在临时 SQLite 数据库中插入 100,000 条模拟索引，批量写入后执行 SQLite optimize，覆盖 `resume` / `invoice` / `screenshot` / `project` / `身份证` / `report` / `archive` 查询，并检查 p95 查询耗时不超过 1,000ms。benchmark 使用临时 DB，不污染用户数据；ignored Rust benchmark 不会在普通 `cargo test` 中运行。

```bash
npm run test:performance
ZC_BENCH_ROWS=50000 ZC_BENCH_P95_MS=1000 npm run test:performance
```

在 PowerShell 中可用：

```powershell
$env:ZC_BENCH_ROWS="50000"; $env:ZC_BENCH_P95_MS="1000"; npm run test:performance
```

设置 `ZC_BENCH_EXPLAIN=1` 可输出 SQLite query plan。

完整发布前验证：

```bash
npm run verify
```

## 打包与发布

本项目已迁移到 Tauri 2，当前打包入口为 Tauri 构建。Release workflow 会执行前端与 Rust 质量门，生成 CycloneDX SBOM 和 SHA-256 checksums；公开构建目前仍是未签名安装包。

```bash
npm run assets:brand
npm run build
```

Windows 构建会输出 NSIS 安装包到 `src-tauri/target/release/bundle/nsis/`。生产签名需要在发布环境配置 Windows 代码签名证书以及 macOS signing/notarization 凭据。
