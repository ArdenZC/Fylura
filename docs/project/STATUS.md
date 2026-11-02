# Fylura Project Status

Last verified: 2026-11-02 against GitHub and remote master. Dated baseline: `8a6a87f` ([#537](https://github.com/ArdenZC/Fylura/pull/537)); always fetch latest master before work.

## Current product goal

Fylura v1 delivers **Files, Global Search, AI Organize and AI Cleanup** on Windows and macOS 13+ Apple Silicon. Files works without a managed root or AI provider; indexing needs explicit source authorization and AI runs only under its separate consent. Linux is not a product target. Intel Macs are not product targets; Universal binaries are not product targets; Rosetta is not a product target.

## Current initiative

**File-first product delivery**

Status: **ACTIVE — IMPLEMENTATION**

Scope: [delivery record](initiatives/file-first-product-delivery.md), [ADR-0016](DECISIONS/0016-file-first-four-core-jobs.md), [ROADMAP](ROADMAP.md).

- **M0-A complete:** #534 merged; [#527](https://github.com/ArdenZC/Fylura/issues/527) closed. Skippable onboarding, immediate Browse, no ordinary Library/Browse switch.
- **M0-B/C complete:** [#535](https://github.com/ArdenZC/Fylura/pull/535) merged as `6392e66`, required CI passed; stop legacy startup Managed Scan submissions, preserve all managed data/services, archive old status and align milestones. Dormancy review: **2027-01-04**.
- **M1 active:** a file-first compact workspace and multi-select Copy/Move → results → destination → History/Restore; then **M2** AI Organize, **M3** AI Cleanup, **M4** Search/Zen Zero/real quality, **M5** Windows/macOS Beta acceptance.

## Work in progress

- The sole mainline task is [M1 #536](https://github.com/ArdenZC/Fylura/pull/536): Files simplification (one location/search row, on-demand tools/comparison, file rows first; Global Search only through the top search bar/shortcut) and transfer-result navigation: root paths were truncated, partial failures opened the failed item's directory, and all-failed batches still offered destination navigation. Fix the renderer on existing transfer/Browse/History owners; no engine or recovery changes.
- [#512](https://github.com/ArdenZC/Fylura/issues/512) is the open product inventory; [#487](https://github.com/ArdenZC/Fylura/issues/487) owns remaining real Windows/macOS GUI acceptance.
- Search activation: **CODE_MERGED** for #518/#533; **WINDOWS_DEFAULT_ON=false**, **MACOS_DEFAULT_ON=false**, **RELEASE_READY=false**. Windows activation awaits representative installed-app resource/exit qualification; macOS GUI remains **UNVERIFIED**. #404 SQLite writer competition and #353 macOS resource evidence stay separate. This does not block M1. Merged delivery: [#537](https://github.com/ArdenZC/Fylura/pull/537).
- Independent Search work remains open in [#494](https://github.com/ArdenZC/Fylura/issues/494), PRs [#495](https://github.com/ArdenZC/Fylura/pull/495), [#497](https://github.com/ArdenZC/Fylura/pull/497), [#510](https://github.com/ArdenZC/Fylura/pull/510). These candidates are not the Files mainline. #518, #519, #521, #529 and #534 are merged, not active Drafts.

## Real blockers

- Native whole-product GUI coverage remains incomplete under #487, including macOS [#270](https://github.com/ArdenZC/Fylura/issues/270) startup and [#446](https://github.com/ArdenZC/Fylura/issues/446) SIGBUS qualification. Browser/hosted checks do not establish Beta readiness.
- One-off Browse sources and grouped proposal review still need completion for M2/M3; a durable Managed Library is not an acceptable prerequisite.
- No public Release/Beta qualification is claimed. Independent historical research is not a blanket blocker for bounded Files delivery.

## Next

Codex implements the authorized mainline: **M1 multi-select cross-directory Copy/Move → progress/results → destination → History/Restore → M2 one-off AI Organize**. Reuse merged drag/clipboard/tabs/dual-pane/Basket/rename/task/conflict/preview/recovery owners; fix observed friction, preserve backend Preview/Revalidation/Journal. Interrupt ordinary transfers only for an actual user decision.

Existing specialist owners retain their boundaries: #487 owns native GUI QA/platform defects; #494 owns Search performance; neither expands this task into new audits or approval flows. No extra Owner gate or Release is introduced.

Historical implementation and technical evidence: [STATUS archive, 2026-11-02](STATUS-ARCHIVE-2026-11-02.md). Archives are optional evidence, not default startup context.
