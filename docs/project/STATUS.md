# Fylura Project Status

Last verified: 2026-11-03 against GitHub and remote master. Dated baseline: `4e34b25` ([#542](https://github.com/ArdenZC/Fylura/pull/542)); M2 delivery: `7b6492b` (#541); M3 delivery: `4e34b25` (#542); always fetch latest master before work.

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
- The next mainline is **M4**: explicit Global Search authorization → truthful coverage → Smart Lean exclusions → usable results; then sustained Zen Zero resource measurements and supported native workflows. The 1M FTS candidates remain independent; no default-on/release claim follows from Cleanup delivery.
- [#512](https://github.com/ArdenZC/Fylura/issues/512) is the open product inventory; [#487](https://github.com/ArdenZC/Fylura/issues/487) owns remaining real Windows/macOS GUI acceptance.
- Search activation: **CODE_MERGED** (#518/#533 and platform-gated activation in #537); **WINDOWS_DEFAULT_ON=false**, **MACOS_DEFAULT_ON=false**, **RELEASE_READY=false**. Windows default-on remains pending candidate-bound installed-package GUI, resource and exit qualification in a profile-safe Windows environment that preserves the current service and installation; macOS GUI remains **UNVERIFIED**. #404 SQLite writer competition and #353 macOS resource evidence stay separate. This does not block M1.
- Independent Search work remains open in [#494](https://github.com/ArdenZC/Fylura/issues/494), PRs [#495](https://github.com/ArdenZC/Fylura/pull/495), [#497](https://github.com/ArdenZC/Fylura/pull/497), [#510](https://github.com/ArdenZC/Fylura/pull/510). These candidates are not the Files mainline. #518, #519, #521, #529, #533, #534, #535, #536, #537, #540, #541 and #542 are merged, not active Drafts.

## Real blockers

- Native whole-product GUI coverage remains incomplete under #487, including macOS [#270](https://github.com/ArdenZC/Fylura/issues/270) startup and [#446](https://github.com/ArdenZC/Fylura/issues/446) SIGBUS qualification. Browser/hosted checks do not establish Beta readiness.
- M2 uses metadata only; no live model-quality or installed-app GUI qualification is implied. M3 native installed-app GUI and live-model judgement quality remain unverified under #487; hosted filesystem tests and Chromium fixtures do not establish Beta readiness.
- No public Release/Beta qualification is claimed. Independent historical research is not a blanket blocker for bounded Files delivery.

## Next

Codex implements the authorized mainline: **M4: Search authorization/coverage, Smart Lean, Zen Zero resources and real Windows/macOS quality**. Reuse merged drag/clipboard/tabs/dual-pane/Basket/rename/task/conflict/preview/recovery owners; fix observed friction, preserve backend Preview/Revalidation/Journal. Interrupt ordinary transfers only for an actual user decision.

Existing specialist owners retain their boundaries: #487 owns native GUI QA/platform defects; #494 owns Search performance; neither expands this task into new audits or approval flows. No extra Owner gate or Release is introduced.

Historical implementation and technical evidence: [STATUS archive, 2026-11-02](STATUS-ARCHIVE-2026-11-02.md). Archives are optional evidence, not default startup context.
