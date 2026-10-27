# U3 File Library: sorting, compare and batch management

Outcome: within Files, sort a whole Browse folder, compare two admitted folders,
select differences and review a batch transfer to the opposite pane. Issue
[#480](https://github.com/ArdenZC/Zen-Canvas/issues/480); base
`1098af35727cdd3bea042deba41bb59238be586e` / tree
`e0346ea07d8a917f634516c5848daa564d10d0cd`. Latest master integrated for final CI:
`f302f5da8ab86f7d121b5cad9a71e5e552b720b6` (upstream #479 unchanged).

## Workflow and boundaries

- Select Name, Type (extension), Size or Modified, direction and folders-first.
  The existing backend enumeration collects up to 1,024 raw children per call
  before publishing globally sorted pages. Maximum 20,000 raw children; above
  that, sorting fails explicitly and Enumeration order remains available.
  Equal keys use exact filename and native path as stable tie-breaks. Sorting
  is metadata observation, not an atomic filesystem snapshot. Refresh re-reads
  metadata, resets selection and invalidates old enumeration refs.
- Sort lives in each WorkspaceSession presentation/history. New Browse folders
  inherit sort while clearing text/type filters. Other panes and tabs retain
  their own query, selection and preview ownership. Library Query V2 is unchanged.
- In dual panes, open Browse folders and Compare folders. Clear name/type filters
  first. Comparison completes source-owned pages on demand; Pause/Close stops
  additional comparison-driven reads. Maximum 10,000 visible entries per side.
  Results render 40 per page, with a scrollable table and per-status counts.
  Compact windows collapse batch controls by default to retain visible results.
- Names match exactly, including case and Unicode spelling. This comparison
  condition deliberately does not infer volume lookup rules from Windows/macOS.
  A case-only pair stays visibly different; backend transfer preview decides
  actual target conflicts. Missing metadata stays unknown; same-name directories
  remain unvisited rather than being declared equal. Colliding display names
  retain every source ref as uncertain instead of overwriting a row. Size and optional mtime
  (existing second precision) are compared, with no content reads or hashes.
- Skipped/unreadable/unsupported children carry cumulative coverage warnings.
  Failed enumeration, observed pending changes, active filters or capacity
  overflow suppress a complete comparison. Results are observations of mutable
  folders, not proof of simultaneous equality; refresh after external changes.
- Select individual rows or either complete filtered side, locate in that pane,
  add to shared basket, or Copy/Move to the peer. Selection remains source-owned.
  Batches over 1,000 cannot be submitted through comparison. Every operation
  continues through Transfer Ticket → Operation Preview → explicit confirmation
  → identity revalidation → Execute → journal/History/Restore. Unsupported
  directory/cross-volume operations retain backend rejection. Skip/Keep both,
  progress, cancellation and partial outcomes reuse the existing review surface.

## Validation

- Final full frontend suite: **184 files / 1,974 tests pass**, with no unhandled
  errors. Typecheck, frontend build, documentation/governance, performance
  architecture (30 tests), Cargo format and diff checks pass. Existing CSS
  optimization warning about `var(...)` is unrelated to this feature.
- The existing high-severity dependency audit exposed
  [GHSA-834v-8h66-3h79](https://github.com/advisories/GHSA-834v-8h66-3h79).
  Current master supplies development-only Happy DOM 21.0.4; `npm run security:audit`
  reports zero vulnerabilities. The IME test teardown observes expected WAAPI
  cancellation and still fails on any other error. No Global Search production
  code changes. Existing W2-08/W2-10 browser assertions now exercise backend
  sorting instead of requiring the pre-U3 unavailable control; gate thresholds
  and CI routing are unchanged.
- Rust regressions cover 138 entries over 17-entry pages, all four sort fields
  and both directions, tie-breaking, stale refs, empty progress before EOF,
  capacity rejection and skipped-child coverage. Local Cargo execution is
  blocked at `glib-sys` by missing GLib; normal Windows/macOS PR CI owns results.
- Browser evidence uses the deterministic in-memory `C:/U3/Left` and `Right`
  fixture (138 entries each), Playwright Chromium and simulated Windows renderer
  platform. This is **browser mock**, never Windows/macOS native GUI evidence.
- No Global Search production code, schema, AI, indexing/watcher scheduling,
  CI policy, mutation executor or recovery strategy changes. No Codex Review,
  Full Validation or million-file campaign. No release/installer qualification.

## Browser evidence

`npm run test:browser:u3` passes on the final production sources. The Browser
plugin has no callable control tool in this session; the repository's Playwright
path supplies cloud browser evidence. [Checks and environment](evidence/U3/checks.json).
Screenshots cover English/Chinese and light/dark at 1600×1000 and 820×800;
980/1280 widths additionally pass the no-horizontal-overflow checks. No framework
overlay, blank app or relevant browser console error was observed.

| Flow | Screenshot |
| --- | --- |
| Whole-folder sort | [Light sorting](evidence/U3/01-sorting-light.png) |
| Two-folder metadata comparison | [Light comparison](evidence/U3/02-compare-light.png) |
| Filtered batch selection / basket | [Selected differences](evidence/U3/03-filtered-selection.png) |
| Conflicts block execution | [Conflict preview](evidence/U3/04-conflict-preview.png) |
| Keep both + Skip, fresh consent | [Cross-pane preview](evidence/U3/05-cross-pane-preview.png) |
| Durable outcome projection | [Transfer results](evidence/U3/06-operation-results.png) |
| Difference filter in dark theme | [Dark filter](evidence/U3/07-filter-dark.png) |
| Compact pane and compare table | [Dark 820px](evidence/U3/08-narrow-dark.png), [Light 820px](evidence/U3/09-narrow-light.png) |
| Existing History entry point | [History](evidence/U3/10-history.png) |
| Chinese sorting and dark theme | [Chinese dark](evidence/U3/11-chinese-dark.png) |

The scripted path also cancels a prepared Move, then confirms Move and observes
updated comparison; unsupported directory preview stays non-executable. Existing
U1 focused tests cover cancellation, partial failure/progress and preview-handle
release. No real cross-volume or native Restore operation ran in the browser.
