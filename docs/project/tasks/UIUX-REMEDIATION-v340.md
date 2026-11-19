# Fylura UI/UX v3.40 remediation record

Date: 2026-11-19
Scope: the 25 items in the supplied UI/UX backlog. This is a source and Chromium browser-mock record; it does not qualify an installed Windows or macOS build.

Disposition: 21 done or verified against existing behavior; PERF-01 partial; ENG-02 deferred with bounded rationale; NATIVE-01 and NATIVE-02 unverified.

## Item disposition

| ID | Status | Change or evidence |
|---|---|---|
| UI-01 | Done | `SettingsSelect` now uses a vertically centered flex row with separate text and chevron sizing. Focused settings tests cover the shared primitive and both locales. |
| UI-02 | Done | The title-bar theme action uses Sun, Moon, and Monitor for Light, Dark, and System; its state and tooltip are localized. |
| UI-03 | Done | Replaced the visible `ZS` workspace mark with the Fylura brand mark and removed visible Zen Canvas product prose. Legacy storage keys and compatibility identifiers remain intact. A production-source scan found only compatibility/model references, not a visible old brand mark. |
| UI-04 | Done, no layout change | Reproduced at 1800×1100 with four panes and 850×680 with the compact one-pane strategy. The list owns its scroll area; the load-more control remains below it. Loading another page increased the mock list from 32 to 64 entries without covering the last row. |
| UX-01 | Done | Sidebar now has Files and Overview, an Organize & maintain group with Organize, Cleanup, and History, and Settings at the bottom. Three sidebar states and route behavior remain covered. |
| UX-02 | Done | Common file operations and task/results entry now render once per workbench. Selection, navigation, and destination follow the focused pane; pane paths, sorting, views, and selections stay independent. |
| UX-03 | Done | Kept pane-local path, history navigation, view, and sort controls. Moved shared operations out of pane-local bars and made the shared action row fit the compact window. |
| UX-04 | Done | Settings exposes six groups while preserving the existing child section IDs, deep links, search targets, and persisted values. Existing ten sections are regrouped rather than deleted. |
| UX-05 | Done | Watcher health moved into Platform Diagnostics. Overview technical coverage is collapsed under a diagnostic disclosure; task, file, review, and space actions remain in the normal view. |
| UX-06 | Done | History uses a single combined operation/restore status, emphasizes source → destination paths, and labels the detail action. Restore eligibility remains visible in the inspector and selection bar. |
| UX-07 | Done | Removed repeated cleanup safety banners from the workflow detail and removed an unrelated Organize-confirmation warning from the Settings log-retention description. The operation preview, actual scope, selected impact, final confirmation, and each workflow’s distinct eligibility guidance remain; execution boundaries did not change. |
| UX-08 | Done in browser fixture | Added `npm run test:browser:ux08` for folder search, multi-select, copy conflict with Keep Both, result review, cross-pane Move, Ctrl/⌘Z, rename, and History restore. Fixed the browser mock’s ISO timestamp comparison so undo selects the newest batch. |
| UX-09 | Done | Ctrl/⌘K idle state uses real saved/recent locations and current browse places, with keyboard hints and empty/error states. No synthetic history was added. |
| UX-10 | Verified existing | The Ready/Review/Blocked filters are already left aligned, with plan results and decision groups taking priority. Browser source inspection found no layout defect, so no code change was needed. |
| UX-11 | Done | Cleanup estimates now share the reclaimable-byte calculation and consistently mark estimates as approximate across selected, run, confirmation, and sticky summaries. |
| UX-12 | Done | Overview keeps the priority task, Files/Organize/Cleanup actions, and space summary prominent; detailed System Coverage is available in a collapsed technical section. |
| UX-13 | Done for touched pages | Reused existing v3.40 page/header, metric, status, and action-bar primitives. Chromium checks cover Overview, Settings, Organize, Cleanup, and History at compact width; no global restyle was introduced. |
| PERF-01 | Partial | See performance evidence below. Cold 1→4 was remeasured with a real Home folder and per-pane rows. The earlier 1→4 reference used an empty location state, so its numbers are not a like-for-like comparison. Any remaining long-frame result is recorded; no performance PASS is inferred from an animation-only check. |
| PERF-02 | Done in Chromium | Verified 850×680 and 1800×1100 with four panes. At compact width, controls remain inside the toolbar and document/body do not overflow horizontally; the existing one-pane responsive strategy remains active. |
| ENG-01 | Verified existing | Shared focus-visible grammar distinguishes ordinary, primary, danger, warning, and selected controls. Forced-colors and reduced-motion rules remain in the existing styles; focused tests cover the rules and keyboard separator resizing. No focus system was replaced. |
| ENG-02 | Deferred, bounded | No wholesale V26/W6-07/v3.40 stylesheet cleanup was attempted. The reproduced layout fixes use their owning component styles; deleting cross-generation overrides without a complete visual regression surface would exceed this local remediation. |
| ENG-03 | Verified existing | Overview, Settings, Cleanup, Organize, and History screenshots showed no clear glass/solid-surface or light-theme environment-light defect requiring a token change. |
| ENG-04 | Verified existing | No decorative motion was added. Existing reduced-motion handling and timing contracts remain covered by motion tests. |
| NATIVE-01 | Unverified | This Linux session did not run a real Windows installed build/WebView2. Font, DPI, monitor, native menu, drag, and file-operation qualification remains open. |
| NATIVE-02 | Unverified | This session did not run a real macOS installed build/WKWebView. Retina, Traffic Lights, Finder Columns, scrolling, and native file-operation qualification remains open. |

## Verification

- `npm run typecheck` — passed.
- Focused Vitest run for Settings, shell/navigation, History, Cleanup, Overview, command search, file operations, and browser-mock ordering — **9 files, 159 tests passed**.
- Final Settings/Cleanup copy check — **2 files, 63 tests passed** after UX-07 wording cleanup.
- `npm run build:frontend` — passed. Tailwind emitted its existing `var(...)` optimizer warning.
- Accessibility and motion tests — **4 files, 30 tests passed** (`w607Phase7FocusGrammar`, `phase10AccessibilityMotion`, `v340MotionSystem`, `designSystemV4`).
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH=/usr/bin/chromium npm run test:browser:ux08` — passed; no page or console errors.
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH=/usr/bin/chromium node scripts/runFilesMultiPaneBrowser.mjs` — passed earlier in this task, including 1–4 panes, pane-local state, cross-pane transfer, compact-window bounds, and persistence.
- Chromium browser-mock screenshots and route checks: Overview, command idle state with a real recent Home location, Settings, Cleanup, Organize, and History. These are UI checks, not native qualification.

## Performance evidence

The existing 1→4 reference in `docs/design/v340-evidence/parity/PERF.md` reported frame p95 150.1ms median / 200ms worst and longest LoAF 585ms median / 609ms worst. The old fixture opened no folder; this run opened Home and recorded four file rows across the panes. Keep the different fixture load visible when comparing.

Five-run development-server measurements are diagnostic only: p95 183.4ms median / 266.8ms worst; longest LoAF 613ms median / 648ms worst; click response 952ms median / 1,110ms worst; pane content shell 1,609ms median / 1,878ms worst. Each run displayed four file rows across the panes. Three of five development runs exceeded the existing 600ms LoAF reference. The dev trace attributes the long mount to React work; a sampled profile materially inflated LoAF and is not treated as a gate result. Production-preview measurement was attempted but is unavailable with this fixture because browser mocks are compiled out of production builds by design (`src/api/core.ts`); the app fell back to its database-unavailable screen. PERF-01 remains **PARTIAL**; see `docs/design/v340-evidence/parity/PERF.md`.

## Native scope

Windows and macOS native installed-build checks are **UNVERIFIED**, not PASS. Chromium, the browser mock, and source tests cannot verify WebView2 or WKWebView behavior. The available Browser `js` connector was not exposed in this session, so Chromium Playwright was used for browser fallback.
