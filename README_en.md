# Fylura

<div align="center">
  <img src="docs/banner_en.svg" width="100%" alt="Fylura Banner" />
</div>

<br />

<div align="center">
  <a href="README.md">
    <img src="https://img.shields.io/badge/切换到中文版本-0f172a?style=for-the-badge" alt="中文版本" />
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

## Introduction

> **A local-first personal file lifecycle assistant.**
> Fylura is not a file explorer replacement or a simple classifier. It connects scanning, fast indexing, explainable organization, safe preview execution, and restore records into one controlled local workflow.

## Supported Platforms

- Windows.
- macOS 13+, Apple Silicon (arm64) only. Intel Macs are unsupported; no Universal Binary or Rosetta compatibility guarantee is provided.

## Development Identity Migration

On first launch, Fylura checks for the unreleased development database and copies it into the new data directory with SQLite's Backup API, verifies integrity, and leaves the original untouched. Under each platform's application data root, the database moves from `com.startlan.zencanvas/zen-canvas.sqlite3` to `com.startlan.fylura/fylura.sqlite3`. The backup includes committed WAL state, preserving the file library, index authorization, History, Journal, and Restore records. An existing Fylura database always wins and is never overwritten; if a primary database is missing while WAL, SHM, or Journal sidecars remain, startup stops and leaves them for inspection. Migration does not start a new full-disk index.

Close the older app and its Windows index service before migrating. If the Windows preview handler still occupies the shared COM registration, the installer stops safely; uninstall the older development build first. Each uninstaller only removes its own app, service, and system registrations and leaves user data in place. If the old version enabled launch at login, Fylura migrates that setting only when both the startup value name and old executable path match exactly; ambiguous entries are preserved. After migration, run only Fylura so the old version cannot continue writing to a separate database. Existing Safe Trash paths and other user-file paths remain unchanged.

## Core Capabilities

- **Overview / Scan**: scan user space or selected folders through the Tauri system directory picker. Project directories are summarized as parent project assets, so configured engineering environments are not casually moved. The scanner's disk capacity is currently a reference value; a later version will report capacity by each scan root's disk.
- **Global Search**: stays centered in the title bar. Use `Ctrl + K` on Windows and `⌘ K` on macOS; when the main window is closed, the shortcut opens a standalone frosted search box.
- **Organize Files**: explains suggested destinations through four clear zones: In Use, Archive Ready, Private, and Cleanup. Suggestions do not bypass the preview and confirmation flow.
- **File Library**: browse managed files, status filters, and classification reasons. Use Global Search for finding a specific file.
- **Storage Cleanup**: analyzes durable cleanup findings and moves confirmed candidates through the Safe Trash path, preserving recovery records.
- **Preview & Execute**: groups plans by main folders and subfolders. Moves, renames, cleanup actions, and eligible permanent deletion all require an authoritative preview and explicit confirmation.
- **History / Restore**: review operations and cleanup records created by Fylura, then restore eligible outcomes after identity revalidation. Operation logs are persisted in SQLite, recent records load by default, and the saved retention setting controls automatic pruning.
- **Automation**: Manual / Schedule / Managed files changed triggers only prepare reviewable Organization Plans and remain review-required / never-auto-execute. Rule Repository / Rule Proposal stay available as Advanced Policies compatibility surfaces and are not semantic fallback for new Organize / Cleanup.
- **Content Understanding**: extract and understand managed content under explicit policy and consent; its results do not authorize filesystem mutation.

## Search

- Local SQLite WAL + FTS5 trigram indexing, with no dependency on Everything, Spotlight, or OS search backends.
- Supports filename search, path search, tokenized terms, and extension filters.
- Default search and paged queries exclude stale files, so transient delete events do not destroy the visible library state.
- Bulk scans and large watcher upserts run SQLite `PRAGMA optimize`, then emit a `search-index-optimized` event with trigger, duration, success, and error fields.
- Results can open files, reveal them in the system file manager, or open File Library details.
- Performance validation includes frontend architecture guards and a real SQLite/FTS benchmark.

## Incremental Indexing

- Watcher remove / delete events mark files stale instead of deleting `files` rows.
- The `files` table tracks `is_stale` and `last_seen_at`; search, pagination, stats, and rule execution exclude stale files by default.
- Create / modify / rename / change events are debounced and batch-upserted. Files that reappear can revive stale records.
- After watcher upserts, Fylura runs `execute_rules_for_paths` only for affected paths instead of re-running rules over the full library.
- Large watcher upserts trigger search index optimize at the existing threshold. Optimize failures only log warnings and do not fail the upsert.
- When watcher deep indexing reaches its safety limit for a large directory, the UI warns the user to run a full manual scan so a partial update is not mistaken for complete indexing.

## Safe Execution And Recovery

Every user-file mutation follows this controlled chain:

```text
intent
  -> authoritative Operation Preview
  -> explicit confirmation
  -> backend identity/path revalidation
  -> operation/cleanup journal or Safe Trash
  -> filesystem mutation
  -> durable outcome
  -> History / Restore
```

Eligible managed files may also enter a separate permanent-delete review. It is an explicit, confirmation-gated action and never an implicit result of scanning, indexing, organizing, or cleanup analysis.

- Operation and cleanup journals persist the requested action, its result, and the recovery state.
- Safe Trash moves eligible cleanup findings into the controlled recovery path; History / Restore exposes the durable outcome.
- Restore operations are identity-bound and update the managed index and FTS after successful file operations.

## Advanced Rules / Compatibility Classification

- Built-in and user rules still serve existing Rule/Policy, watcher-classification, and compatibility behavior. User rules persist in the SQLite `rules` table. After the AI-only migration they are **not** semantic fallback for new Organize / Cleanup and do not authorize filesystem mutation.
- `rule_version` uses a stable hash and no longer relies on `DefaultHasher`.
- The `files` table stores classification fingerprints: `last_classified_at`, `classified_rule_version`, `last_classified_mtime`, and `last_classified_size`.
- `execute_rules_on_inbox` only considers files where `lifecycle = Inbox` and `is_stale = 0`, and skips records whose rule version, mtime, and size have not changed.
- `RuleExecutionSummary` includes `skipped`, making candidate scans and real reclassifications visible separately.
- Planned rule work now focuses on versioning, import/export, conflict detection, and finer rule auditability.

## Safety

- The app does not scan automatically on launch. Scanning only creates an index and suggestions.
- User-file mutations use the authoritative preview, explicit confirmation, backend identity/path revalidation, operation or cleanup journaling, and the Safe Trash / Restore boundaries described above.
- Eligible permanent deletion is a separate explicit review and confirmation flow, not an automatic side effect of scanning, indexing, organizing, or cleanup analysis.
- Fylura skips selected system and generated directories by default, including `.git`, `node_modules`, `.venv`, `__pycache__`, `dist`, `build`, `target`, `coverage`, `vendor`, `Windows`, `Program Files`, and `System Volume Information`.
- Sensitive files show advice and reasons, but are not selected for execution.
- Conflicts, low-confidence items, and close rule scores enter manual confirmation by default.
- The Tauri command layer revalidates move, rename, and restore operation type, absolute paths, safe filenames, source-path consistency, protected system targets, and overwrite conflicts.
- Watcher delete events only mark stale records and do not directly destroy index history.
- Watcher updates for very large directories may be partial and can prompt the user to run a full manual scan.
- Execute / restore updates the `files` table and FTS after successful file operations.
- Search index optimize failures only log warnings and do not fail scans or upserts.
- Tauri CSP is configured. The frontend does not access the file system directly; scanning, indexing, moving, renaming, and restore are handled in Rust commands.

## Architecture

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

## Project Truth And Workflow

- [Current project status](docs/project/STATUS.md)
- [Product map](docs/project/PRODUCT_MAP.md)
- [Architecture map](docs/project/ARCHITECTURE_MAP.md)
- [Development workflow](docs/project/DEVELOPMENT_WORKFLOW.md)

## Development

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

`npm run test:performance` first runs the frontend architecture guard, then runs a Rust SQLite/FTS benchmark. By default, the benchmark inserts 100,000 simulated index rows into a temporary SQLite database, runs SQLite optimize after the bulk write, covers `resume` / `invoice` / `screenshot` / `project` / `身份证` / `report` / `archive` queries, and checks p95 query latency against a 1,000ms threshold. The benchmark uses a temporary DB and does not touch user data; the ignored Rust benchmark does not run during ordinary `cargo test`.

```bash
npm run test:performance
ZC_BENCH_ROWS=50000 ZC_BENCH_P95_MS=1000 npm run test:performance
```

In PowerShell:

```powershell
$env:ZC_BENCH_ROWS="50000"; $env:ZC_BENCH_P95_MS="1000"; npm run test:performance
```

Set `ZC_BENCH_EXPLAIN=1` to print SQLite query plans.

Full release verification:

```bash
npm run verify
```

## Packaging

Fylura has moved to Tauri 2. The current packaging entrypoint is the Tauri build, which produces the desktop app and installer for the current platform. Signing hooks are reserved for later.

```bash
npm run assets:brand
npm run build
```

Windows builds output the NSIS installer under `src-tauri/target/release/bundle/nsis/`. The cross-platform release matrix and signing flow will be refined alongside the Tauri release configuration.
