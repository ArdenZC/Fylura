# U3 File Library: sorting, compare and batch management

Outcome: within Files, sort a whole Browse folder, compare two admitted folders,
select differences and review a batch transfer to the opposite pane. Issue
[#480](https://github.com/ArdenZC/Zen-Canvas/issues/480); base
`1098af35727cdd3bea042deba41bb59238be586e` / tree
`e0346ea07d8a917f634516c5848daa564d10d0cd`.

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
- Names match exactly, including case and Unicode spelling. This comparison
  condition deliberately does not infer volume lookup rules from Windows/macOS.
  A case-only pair stays visibly different; backend transfer preview decides
  actual target conflicts. Missing metadata stays unknown; same-name directories
  remain unvisited rather than being declared equal. Size and optional mtime
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

- Full frontend suite: 183 files / 1,972 tests passed before final focused
  interaction refinements. Focused final checks and hosted CI are recorded in
  the PR. Typecheck and frontend build pass; existing CSS optimization warning
  about `var(...)` remains unrelated to this feature.
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
