# U5 File Library integration result

## 2026-10-30 final acceptance addendum — latest evidence (supersedes earlier status below)

### Integration and exact-head CI

- PR #486 remains Draft and unmerged. Its source integration is based on `master@5a52de0280f52f3e485e4cde7b2ad941a47d7d32`, using merge commit `9bb2c3737bb2856ac0927d5e8c7bb7299aa652d9`. At reviewed source head `86c77a9de14f1e75f7b7543985d442e06ffd4817`, the code diff from master was 27 files, +600/-41. The merge retained the U5 Inspector, comparison, duplicate-action and Browse-lifecycle work while incorporating current master changes; it did not replace U1–U4 or #489, #509, #501 or #504.
- The earlier macOS `clippy::let_and_return` failure was fixed in `src-tauri/src/main.rs`. Exact source-head Actions run [40995784773](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40995784773) completed successfully: 27 jobs, 16 success, 11 conditional skips, zero failures. Windows and macOS Rust quality (including Clippy/tests), Windows and macOS release compile, frontend/product browser gates, dependency audit, performance profile and Library & Content lane passed. The arm64 native macOS performance lane was skipped; this is not native GUI evidence.
- A documentation-only acceptance follow-up adds the batch evidence at [batch-operations.json](evidence/U5/windows-native/batch-operations.json). It makes no production-code change. The live PR head and check state remain the authority for the exact final documentation commit.

### Windows native GUI: batch progress, cancellation and partial failure

- The genuine Tauri window was interactive on Windows 11 Pro build 26300. The isolated app identity/profile was `com.startlan.zencanvas.u5b0` / `F:\_qa\zen-canvas-u5-b0\profile`; the dedicated test volume was `ZCU5TEST (H:)`. Before launch, the sole managed browse root was `H:\U5\fixtures`, global-volume authorization was absent and `global_entries=0`. AI was off. The historic 561,664-entry QA profile and other worktree scratch were left untouched.
- The 256-item native Copy showed progress `20/256` then `37/256`. Cancel Remaining ended with 37 completed and 219 canceled/skipped. History showed partial success and 219 skipped, not all-success. Read-only post-check found 256/256 source files and 37/37 destination files with SHA-256 matching the fixture manifest; there were no extra destination files. Copy retained all sources and correctly reported zero restorable items.
- A second controlled native Copy contained a 48 MiB file and two small files. A helper held `B-lock.bin` open with `FileShare.None` only after the first large copy completed. The real Operation Preview showed 3/3; execution correctly ended `2/3` with `operation_preview_stale` for the locked file. A and C were copied; B was absent at the destination. History recorded one failure and partial success. After releasing the test lock, a fresh one-item Copy Preview and retry completed 1/1. Read-only verification found all three original files unchanged and all three destination files matching their exact SHA-256 values. No overwrite, source deletion, or false all-success was observed. Since these were copies, Restore eligibility correctly remained zero; prior native Move/Restore acceptance remains as recorded below.

### Internal dual-pane drag/drop and startup classification

- The actual UI drag was attempted from a selected, `draggable=true` file row in `H:\U5\fixtures\Right\B0BatchPartialFailRun4` to the empty peer pane at `H:\U5\fixtures\Right\B0BatchPartialFailRun3`. The native CUA pointer drag produced no Operation Preview and no app error; the source selection remained and no file/History mutation occurred. Do not count this as a drag/drop PASS.
- Source inspection confirms the production chain: `dragstart` captures a session-local TransferTicket and opaque dataTransfer token; `drop` validates that token plus source/target pane identity and signatures, then sets `pendingTransfer`; the existing toolbar effect opens the authoritative Transfer Preview. With the same source and target folders, the visible “Copy to other pane” control opened that shared Preview, generated a `1/1` plan, and was canceled before execution. This proves the preview handoff path, not that the native WebView delivered `dragstart`/`dragover`/`drop`. Root cause is therefore classified as **native CUA/WebView drag-event delivery not observable in this environment; production defect not isolated; drag remains UNVERIFIED**. No synthetic event was used and no drag PASS is claimed.
- The earlier blank WebView startup obstruction was not reproduced: the current Vite endpoint returned HTTP 200 and the same Tauri window accepted native interaction. This run does not prove why the historical failure happened. No Windows service or global-disk scan was started, so indexing activity was not the cause of this run’s responsiveness.

### Platform decision

- Windows File Library native acceptance: PASS for previously documented U1–U4 workflows and the batch flows above. Internal pane drag is UNVERIFIED because native drag events were not observed; toolbar-to-peer Preview and all other tested file operations remain available.
- macOS Apple Silicon native GUI: UNVERIFIED; no interactive macOS host was available. macOS compile, Clippy and Rust tests passed in hosted CI, which is a separate claim.
- Formal cross-platform release qualification: NOT AUTHORIZED. A code merge would not authorize installer publication; no release was installed or published.

**Owner review recommendation: `MERGE_GO` for code integration.** Exact-head CI is green, the requested batch failure/cancel paths preserve identities and accurate partial status, and no high-risk data regression was found. Keep PR #486 Draft for Owner review. Treat internal drag and macOS native GUI as explicit, non-release-qualified UNVERIFIED boundaries.

---

## 2026-10-30 B0 integration addendum — preceding evidence (superseded by final addendum above)

- Integrated from `master@5a52de0280f52f3e485e4cde7b2ad941a47d7d32` via merge commit `9bb2c3737bb2856ac0927d5e8c7bb7299aa652d9`; initial integrated U5 candidate `3e48612c1e13e15b0c59c3f4871e6736dff920a1`.
- A subsequent **Clippy-only repair** removed an unnecessary `let profile_root` binding in `src-tauri/src/main.rs` (commit `3ef66d53c23256cb73154e7e5fbcf7bf578dcbf3`). Mac CI [40994822867](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40994822867), on the **preceding** head `3e48612c...`, failed the `cargo clippy --features "desktop-runtime native-qa" --all-targets -- -D warnings` job on `clippy::let_and_return` at `main.rs:174`. This is a deterministic code-quality failure, **not a reproduced macOS runtime failure**. The amended head needs a **new** exact-HEAD CI result before any merge recommendation changes.
- `native-qa` requires absolute `ZC_NATIVE_QA_PROFILE_ROOT` and scopes Quick Look, File Library thumbnails and native preview staging under that root. Normal builds continue using `app_data_dir`. The feature does not itself authorize an index source.
- **Windows native GUI — Codex/Owner-run *reported* evidence in PR #486 description, not independent new artifact reproduced by this GitHub documentation update:** the isolated `H:\\U5\\fixtures` profile loaded an interactive Tauri window; Browse/path navigation, tabs, dual-pane independent browse, sort/filter/compare, Inspector note/tag/favorite, Quick Preview, Basket, Copy/Move/Cut-Paste, conflict Skip/Keep Both, History/Restore, restart metadata and source-authorization checks were reported as PASS. PR description records fixture hashes, file identity and environment precautions. Those reports do **not** retroactively change the historical blocked-at-startup evidence below.
- **Current native coverage gaps:** Windows running batch progress/cancel/partial failure and actual internal drag/drop remain `UNVERIFIED`; macOS GUI remains `UNVERIFIED`. Two Windows automated drag/drop attempts did not reach a preview, without proof of a production drag/drop defect. No fresh macOS GUI E2E or actual Global Index full-scan qualification is claimed.
- **Historical safety exception (unchanged):** initial U5 QA profile indexed 5 mounted volumes/561,664 entries in an earlier worktree. The later fixture-only profile disabled volume authorization and had `global_entries=0`. The old profile/scratch was not deleted, audited as safe to delete, or implicitly covered by the later QA evidence.

**Current decision: `MERGE_HOLD`** pending amended-head CI and a truthful disposition of the remaining targeted native gaps. This addendum is the latest test-status authority; the following initial U5 record is preserved as time-stamped historical evidence.

---

Base: `472b850da3dfb38c8d50c786b9a4074ba240584b`, tree
`78a677d852f1d7de3ea1f9e9b679c7f492a6a306`. The connected GitHub API
confirmed this was still `master` when the U5 work was prepared.

U5 integrates the U1–U4 experience and corrects the layout and lifecycle issues
found during review. Browser and backend evidence is separated from native GUI
acceptance below. No Global Search, Windows Service, schema, migration, AI
organization, or file-operation executor was changed.

## User-visible changes

- Comparison batch actions now start collapsed, leaving more room for the
  comparison results. The results area is taller, its header and pagination
  remain visible while scrolling, and compact layouts use a bounded height.
- A wide dual-pane Inspector overlays only its owning pane. It no longer takes
  width from both panes or blocks the peer pane. Escape returns focus to its
  own trigger.
- Library Quick Preview and Show in folder have one shared action entry each;
  the Inspector retains its separate thumbnail Quick Look. Previous/Next is
  hidden for a single selected file.
- Hidden or suspended panes stop Browse enumeration. A canceled enumeration
  invalidates its publication generation and key, so resuming starts fresh
  instead of reusing a request that can no longer publish.

## U1–U4 acceptance matrix

`PASS` means the named evidence completed. `BLOCKED` means the native attempt
stopped before that workflow could run; feature-level Windows GUI acceptance
therefore remains `UNVERIFIED`.

| Area | Chromium browser mock | Windows backend automation | Windows Tauri GUI | macOS |
| --- | --- | --- | --- | --- |
| U1 selection, basket, preview, transfer conflicts, cancel and History entry | PASS | PASS for backend regression set; targeted real-temp-file Restore PASS | BLOCKED before interaction | UNVERIFIED |
| U2 tabs, independent panes, peer selection and pane ownership | PASS | Not exercised by backend tests | BLOCKED before interaction | UNVERIFIED |
| U3 sorting, pagination, comparison filters, batch selection and refresh | PASS | Not exercised by browser-independent file-operation tests | BLOCKED before interaction | UNVERIFIED |
| U4 Inspector, notes/favorites presentation, Quick Preview and Show in folder | PASS | PASS: four `u4_` SQLite/identity tests | BLOCKED before interaction | UNVERIFIED |
| Responsive layouts at 820/980/1440/1600, light/dark and long names | PASS | Not applicable | BLOCKED before interaction | UNVERIFIED |
| Hidden-pane Browse cancellation/resume guard | PASS: structural regression plus browser integration run | Not applicable | UNVERIFIED | UNVERIFIED |
| Long-lived native responsiveness, resource growth and handle cleanup | Not measured | Not measured | UNVERIFIED | UNVERIFIED |

The browser workflow passed 19 checks with zero page errors. It used deterministic
mock data, browser storage for mocked notes/favorites, and a simulated Win32
`navigator.platform`. It does not establish real filesystem changes, SQLite
durability, Restore, or Windows UI behavior. The [checks manifest](evidence/U5/browser-mock/checks.json)
states these boundaries in the artifact itself. A real Tauri partial-success or
locked-file failure flow was not executed; these remain `UNVERIFIED`.

## Evidence and regression results

- `npm run typecheck`: PASS.
- `npm run build:frontend`: PASS; Vite emitted two Tailwind selector
  optimizer warnings for `.text-[var(...)]` selectors.
- `npm run test:browser:u5`: PASS, 19 checks, zero errors. This runs the existing
  U3 and U4 Playwright mock journeys together. The [comparison capture](evidence/U5/browser-mock/compare-1600-light.png),
  [Inspector light capture](evidence/U5/browser-mock/inspector-1600-light.png),
  [Inspector dark capture](evidence/U5/browser-mock/inspector-1600-dark.png),
  and [dual-pane captures at 1440](evidence/U5/browser-mock/dual-pane-inspector-1440-light.png)
  and [1600](evidence/U5/browser-mock/dual-pane-inspector-1600-light.png) are
  browser mock screenshots.
- Focused Windows Rust `desktop-runtime` tests: PASS, including
  `restore_moves_core_restores_successful_move_log` (1/1), which creates a real
  temporary source file, executes a move and Restore, then checks source and
  target existence and the Restore journal result. `cargo test ... u4_` passed
  4/4, covering Browse Reveal ownership and SQLite reopen, stale identity,
  rename/move and replacement isolation.
- Full Windows Rust `desktop-runtime` suite: 1,327 passed, 36 ignored, 3 failed.
  The failures are `current_assessment_needs_no_credential_or_new_enqueue`,
  `stale_assessment_checks_fresh_readiness_and_keeps_plan`, and
  `trigger_tests::service::automatic_current_and_stale_semantics_reuse_shared_admission_without_mutation`
  in `db::automation`, outside U5 scope. File Library production paths were
  not modified to address them.
- Full Windows `npm test`: 1,955 tests passed, no failed assertions; four
  research-only `.mjs` suites failed to parse (`preference-frozen-tree`,
  `preference-signal-adjudication`, `preference-signal-b4`, and
  `preference-signal-b1`). These research files were not changed. Hosted PR CI
  is the authoritative Linux result for the normal workflow.
- `git diff --check` and `node --check scripts/runW2-11BrowserGate.mjs`: PASS.
- Hosted CI run [40502736115](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40502736115)
  on source head `7ac37438a90c13ed1da7c15d911dab1b117f47ec`: source/scope checks,
  validation planning, dependency audit, Windows and macOS release compile,
  package metadata, Library & Content performance, and the performance profile
  passed. W2-11 failed at its final console-error assertion with one
  `Failed to load resource: net::ERR_FILE_NOT_FOUND`; its first failure artifact
  did not retain the request URL. The Windows/macOS quality validation jobs
  then failed because the required merge-integration frontend result was a
  failure, not because their release compiles failed.
- The same PR adds bounded W2-11 diagnostics for console locations and missing
  request URLs. Diagnostic rerun [40506580823](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40506580823)
  on source head `fedbb23aac50f3bc43b30a09c2be8d222fc9e348` passed W2-11 and the
  complete normal PR CI, including frontend, W2-01/W2-10, both release
  compiles, Library & Content performance, performance profile, and Windows and
  macOS quality validation. The first run's one-off `ERR_FILE_NOT_FOUND` did
  not recur; no assertion or threshold was relaxed. A local run of the same
  W2-11 command was `BLOCKED` before the scenario: Vite showed a blank page and
  Playwright timed out after 60 seconds waiting for the Files button. This
  local environment failure is not evidence that the UI workflow passed or
  failed.
- No million-file, Full Validation, F1/T10, or new performance campaign was run.

### Windows native startup attempt

The available machine was Windows 11 Pro build 26300. A real Tauri process
reached `setup_complete` and created its main window, but never reported
`main_window_ready`. The Vite listener printed ready, while a bounded HTTP probe
to `127.0.0.1:5173` returned no headers within five seconds. No File Library
control was operated and no real GUI file operation or Restore was attempted;
there is no native GUI screenshot or PASS claim. The startup checkpoints and
fixture hashes are recorded in [startup diagnostics](evidence/U5/windows-native/startup-diagnostics.json)
and [fixture manifest](evidence/U5/windows-native/fixture-manifest.json).

Startup also showed that a fresh native-QA profile automatically starts Global
Index over mounted volumes. The isolated database accumulated 561,664 indexed
entries across five mounted volumes (1,262,473,216 bytes). Only aggregate counts
were inspected; no database contents were exported. The native process was
stopped without interacting with owner files. The ignored scratch directory
`.tmp-tests/u5-native` still contains 679 task-generated files, including this
profile, temporary/npm caches and logs, and seven disposable fixture files.
Exact-path cleanup commands were blocked by tool policy. This scratch directory
is on the test machine, is not part of the PR diff, and needs owner-side
cleanup. Do not relaunch the native app on a host with owner volumes mounted.
The Global Index implementation is out of scope and was not changed.

The database growth and Vite non-response overlapped because the attempted
profile was under the Vite project root. This is a correlation, not a proven
single root cause. Native qualification should continue in a disposable Windows
11 VM with no owner data mounted and all profile/temp paths outside the checkout.

## Concentrated Windows 11 Owner run

Use a disposable Windows 11 VM or equivalent isolated machine. Do not mount or
share owner work folders. The app starts Global Index automatically; keep all
mounted volumes disposable. Keep the app profile, TEMP/TMP, Vite config and
fixture root outside the checkout. Stop if the main window does not become
interactive; record startup logs and do not count a browser mock as GUI evidence.

1. Create a fresh `F:\U5` root in the isolated VM. In PowerShell, set up the
   app identity and profile outside the checkout, then launch the candidate:

   ```powershell
   $qaRoot = 'F:\U5'
   $profile = Join-Path $qaRoot 'profile'
   $tempRoot = Join-Path $qaRoot 'temp'
   $config = Join-Path $qaRoot 'tauri.u5.conf.json'
   New-Item -ItemType Directory -Force -Path $qaRoot | Out-Null
   New-Item -ItemType Directory -Force -Path $profile | Out-Null
   New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
   $env:ZC_NATIVE_QA_PROFILE_ROOT = $profile
   $env:TEMP = $tempRoot
   $env:TMP = $tempRoot
   [System.IO.File]::WriteAllText($config, '{"identifier":"com.startlan.zencanvas.u5"}', [System.Text.UTF8Encoding]::new($false))
   ```

   Keep this PowerShell session and profile for the later restart check.
2. Before launching the app, make `F:\U5\fixtures\Left` and
   `F:\U5\fixtures\Right`, then seed and hash the disposable files:

   ```powershell
   $fixtureRoot = Join-Path $qaRoot 'fixtures'
   $left = Join-Path $fixtureRoot 'Left'
   $right = Join-Path $fixtureRoot 'Right'
   New-Item -ItemType Directory -Force -Path $left | Out-Null
   New-Item -ItemType Directory -Force -Path $right | Out-Null
   $utf8 = [System.Text.UTF8Encoding]::new($false)
   [System.IO.File]::WriteAllText((Join-Path $left 'Copy-source.txt'), 'U5 disposable copy source', $utf8)
   [System.IO.File]::WriteAllText((Join-Path $left 'Move-source.txt'), 'U5 disposable move source', $utf8)
   [System.IO.File]::WriteAllText((Join-Path $left 'Cancel-source.txt'), 'U5 disposable cancel source', $utf8)
   [System.IO.File]::WriteAllText((Join-Path $left 'Conflict.txt'), 'U5 left conflict payload', $utf8)
   [System.IO.File]::WriteAllText((Join-Path $right 'Conflict.txt'), 'U5 right conflict payload', $utf8)
   [System.IO.File]::WriteAllText((Join-Path $right 'Keep.txt'), 'U5 disposable keep-both payload', $utf8)
   Get-ChildItem -LiteralPath $fixtureRoot -File -Recurse | Get-FileHash -Algorithm SHA256
   ```

   Record this manifest, then start the app in the same PowerShell session:
   `npm run dev -- --features native-qa --config $config`. Stop if the Tauri
   main window is not interactive.
3. In the real Tauri window, exercise Library/Browse, path entry, breadcrumbs,
   Back/Forward, Ctrl+L, list/grid, several tabs, dual-pane switching, sidebar
   hide/restore, close/reopen, and window sizes 820, 980, 1440 and 1600 pixels.
   Confirm pane-local selection/sort state and that returning to a prior folder
   cannot publish stale results.
4. Use the existing toolbar, basket, cut/copy/paste, drag/drop and cross-pane
   actions for single- and multi-file Copy/Move. For the collision, capture
   Preview, try Skip and Keep both, cancel one preview, and re-confirm a fresh
   preview. Change a disposable source after preview and verify identity
   revalidation refuses stale execution. After each operation, compare source,
   target and hash manifests. Use only the disposable fixture files for Restore;
   verify History, the restored status, original content/hash and target
   disappearance together.
5. Compare the two fixture folders: sort by name/type/size/modified both
   directions, folder-first, change a file and refresh, filter differences,
   then select a difference and start the existing transfer preview. Confirm
   statistics describe current-layer metadata only and test the documented
   result cap without raising it.
6. Admit only these fixture files through the existing managed indexing flow.
   Add/edit/delete a note and toggle a favorite in Inspector; restart the app
   with the same isolated profile and verify persistence. Rename, move and
   Restore a fixture, then put a different physical file at its old path and
   verify the note never follows the replacement. Check unavailable/stale
   display, dual-pane Inspector ownership, Quick Preview Previous/Next and
   Show in folder. Browse selections must remain ephemeral and offer no durable
   note/favorite write.
7. For a short non-benchmark soak, keep several tabs open, switch repeatedly,
   toggle single/dual pane, scroll the fixture directory, open/close Inspector,
   preview multiple files and perform several small transfers. Record any
   growing resource count, visible stalls or errors. Capture GUI screenshots and
   the final fixture/hash and History evidence. Mark unexecuted items
   `UNVERIFIED`; do not use Restore on files outside `F:\U5\fixtures`.

No macOS GUI environment was available; macOS remains `UNVERIFIED`.

After the Owner run, discard the disposable VM or revert its snapshot to remove
the profile, index and fixtures together.
