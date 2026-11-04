# Fylura Project Status

Last verified: 2026-11-04 against GitHub and remote master at `e4e8391` (#544). M2 delivery: `7b6492b` (#541); M3 delivery: `4e34b25` (#542); always fetch latest master before work.

## Current product goal

Fylura v1 delivers **Files, Global Search, AI Organize and AI Cleanup** on Windows and macOS 13+ Apple Silicon. Files works without a managed root or AI provider; indexing needs explicit source authorization and AI runs only under its separate consent. Linux is not a product target. Intel Macs are not product targets; Universal binaries are not product targets; Rosetta is not a product target.

## Current initiative

**File-first product delivery**

Status: **ACTIVE — IMPLEMENTATION**

Scope: [delivery record](initiatives/file-first-product-delivery.md), [ADR-0016](DECISIONS/0016-file-first-four-core-jobs.md), [ROADMAP](ROADMAP.md).

- **M0-A complete:** #534 merged; [#527](https://github.com/ArdenZC/Fylura/issues/527) closed. Skippable onboarding, immediate Browse, no ordinary Library/Browse switch.
- **M0-B/C complete:** [#535](https://github.com/ArdenZC/Fylura/pull/535) merged as `6392e66`, required CI passed; stop legacy startup Managed Scan submissions, preserve all managed data/services, archive old status and align milestones. Dormancy review: **2027-01-04**.
- **M1 code complete:** #536 and #540 merged; final workflow closeout distinguishes partial failure/recovery-required results and verifies cancel → reissue on the existing engine. No new P0/P1 product defect was found in the exercised code workflows; native qualification remains #487. **M2 code complete:** #541 merged as `7b6492b`, exact-head required CI passed; one-off folder → metadata AI → grouped review/edit → batch execution → History/Restore; **M3 code complete:** supported folder → deterministic Finding groups → explicit AI review and exceptions → qualified Safe Trash batch → focused History/Restore; then **M4** Search/Zen Zero/real quality, **M5** Windows/macOS Beta acceptance.

## Work in progress

- M3 reuses the existing Analysis Finding, provider/consent, Preview and Safe Trash/Restore owners. Group totals/categories cover the complete backend run, not a loaded page. Safe candidates work without an AI key; Review retains its current-assessment and separate confirmation conditions; protected items remain excluded. Final cleanup confirmation is mandatory. Existing M2 retains its 100-direct-entry, metadata-only/no-recursion boundary.
- **M4 code delivery:** [PR #545](https://github.com/ArdenZC/Fylura/pull/545) continues `codex/m4-coverage-workflow` from current master `e4e8391`; production/test commit [`ab05fed`](https://github.com/ArdenZC/Fylura/commit/ab05fedce42e93d1c5c49ef58bea62df279aafb4). It keeps backend policy CAS authoritative, invalidates stale previews until refresh/re-preview, reloads durable state after failed or partially completed saves, and adds native Smart Lean coverage for identity-verified OS temporary roots inside an authorized source. Smart Lean still does not exclude generic same-name folders. Exact-head CI and merge state are recorded in the current #512 update.
- **Smart Lean/user workflow boundary:** focused Settings/search tests cover source opt-in separation, Preview → Apply, stale revisions, save/recovery states, observation categories and ready/partial feedback. The hosted Apple Silicon source qualification reported `spotlight_external_not_indexed` for the task-owned APFS image, so Smart Lean is **not cross-platform verified** and macOS remains Default OFF. Windows NTFS VHD/service qualification is routed by #545 exact-head CI; the final outcome is recorded in the current #512 update. Installed-app coverage Settings GUI remains in #487; the Windows #544 installed-package evidence covers explicit source authorization, top-bar queries, create/rename/move/delete, source disable and unauthorized system-volume exclusion, but not the full Settings preview/apply flow.
- **Zen Zero:** Windows has the bounded #544 31:23 idle sample (809 samples) and GUI/WebView2 exit evidence; the index service remained running. The complete startup/first-index, continuous-change, Search/Preview alternation, pause/resume/restart/exit and long-duration CPU/memory/I/O/wake matrix is not qualified. Apple Silicon installed-app resources and exit remain **UNVERIFIED**. Keep those exact gaps in #487; budgets were not relaxed.
- The next mainline is **M5 Beta acceptance** under #487. The 1M FTS candidates remain independent; code merge does not grant Release authorization.
- [#512](https://github.com/ArdenZC/Fylura/issues/512) is the open product inventory; [#487](https://github.com/ArdenZC/Fylura/issues/487) owns remaining real Windows/macOS GUI acceptance.
- Search activation remains **CODE_MERGED** (#518/#533 and platform-gated activation in #537); **WINDOWS_DEFAULT_ON=true**, **MACOS_DEFAULT_ON=false**, **RELEASE_READY=false**. This is the already-qualified Windows Filename Candidate subfeature from #544; M4 does not change either platform default or enable automatic source indexing. The Windows installed-package evidence and 31:23 idle measurements are detailed below; they do not establish the full Zen Zero matrix or Release readiness. macOS default-on and installed-app GUI acceptance remain **UNVERIFIED/OFF**; #404 SQLite writer competition and #353 macOS resource evidence stay separate.
- Independent Search work remains open in [#494](https://github.com/ArdenZC/Fylura/issues/494), PRs [#495](https://github.com/ArdenZC/Fylura/pull/495), [#497](https://github.com/ArdenZC/Fylura/pull/497), [#510](https://github.com/ArdenZC/Fylura/pull/510). These candidates are not the Files mainline. #518, #519, #521, #529, #533, #534, #535, #536, #537, #540, #541 and #542 are merged, not active Drafts.

## Real blockers

- Native whole-product GUI coverage remains incomplete under #487, including macOS [#270](https://github.com/ArdenZC/Fylura/issues/270) startup and [#446](https://github.com/ArdenZC/Fylura/issues/446) SIGBUS qualification. Browser/hosted checks do not establish Beta readiness.
- M2 uses metadata only; no live model-quality or installed-app GUI qualification is implied. M3 native installed-app GUI and live-model judgement quality remain unverified under #487; hosted filesystem tests and Chromium fixtures do not establish Beta readiness.
- No public Release/Beta qualification is claimed. Independent historical research is not a blanket blocker for bounded Files delivery.

## Next

Codex continues directly with the authorized mainline: **M5 Windows/macOS Beta acceptance**. Reuse merged drag/clipboard/tabs/dual-pane/Basket/rename/task/conflict/preview/recovery owners; complete installed-app Search coverage Settings and the existing Zen Zero lifecycle/resource matrix under #487. Preserve backend Preview/Revalidation/Journal. Interrupt ordinary transfers only for an actual user decision.

Existing specialist owners retain their boundaries: #487 owns native GUI QA/platform defects; #494 owns Search performance; neither expands this task into new audits or approval flows. No extra Owner gate or Release is introduced.

Historical implementation and technical evidence: [STATUS archive, 2026-11-02](STATUS-ARCHIVE-2026-11-02.md). Archives are optional evidence, not default startup context.
