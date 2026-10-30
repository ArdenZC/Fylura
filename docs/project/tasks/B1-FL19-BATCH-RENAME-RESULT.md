# B1 FL-19 Batch Rename — Acceptance Result

Date: 2026-10-30
PR: [#514](https://github.com/ArdenZC/Zen-Canvas/pull/514)
Current decision: **MERGE_GO**

## Integration and fixes

- Integrated the latest master `8f197a7753a9f3f546ef214b617f36511f1be5ef` into the existing PR branch with merge commit `d813d32ab4ce92a1b0f94ec185de6e9ccb88b916`. The merge had no conflicts and keeps the U5 changes from #486.
- Extracted batch rename command-bar ownership into `BatchRenameAction` and `useBatchRenameAction`. `LibraryMode.tsx` is 497 lines, below the existing 500-line gate.
- Fixed the mixed-preview handoff: every included row must have a pending, executable `rename` preview for the same file ID and exact current name. Blocked rows disable Continue until explicitly skipped; a skipped row is not passed to the final Operation Preview.
- Fixed a render loop found in the native Tauri session. A command-bar memo depended on the freshly reconstructed source-owner object, repeatedly re-registering the command-bar surface. The action now receives a stable selection projection and preview callback.

The previous exact-head run on `59bcec7050bd6ee14b7e7ba8e910dd614dd3a0e3` failed because the file-length contract observed 510 lines. Windows/macOS aggregate Quality failures were downstream of that frontend gate. The corrected code candidate `d7c3b702ad4019a6f60d1f58538618afc2ee699c` passed [Actions run 41009173921](https://github.com/ArdenZC/Zen-Canvas/actions/runs/41009173921): frontend/format quality, performance profile, U3/U4, W2-01, W2-10, W2-11, and Windows/macOS aggregate Quality all succeeded. The validation plan skipped lane-specific package/native-preview jobs; no macOS GUI claim is made. This report-only closeout is the only follow-up change after that code candidate; its exact-head checks are recorded in the PR.

## Automated verification

- Targeted FL-19 and Library maintainability tests: **22 passed** across 3 files. Coverage includes number/text/regex name generation, Unicode, final-extension preservation, invalid and duplicate names, explicit 2–500 regular current-file selection, blocked plus executable previews, explicit skip, exact file/name/current preview matching, edited-name re-preview, Escape during an in-flight request, selection identity changes, focus restoration, and larger-batch preview progress.
- `npm run typecheck`: **PASS**.
- `npm run build:frontend`: **PASS**. The existing Tailwind selector optimizer warning remains.
- Targeted Rust manual-rename, authorization and Smart Lean tests passed before these frontend-only changes. The corrected candidate's hosted Windows/macOS Rust quality result remains subject to the exact-head Actions run.

## Windows 11 native acceptance

The test used the isolated profile `F:\_qa\zen-canvas-b1-fl19\profile` and dedicated fixture root `H:\U5\fixtures\B1-FL19-run-20261030` on volume `ZCU5TEST`. The profile had one enabled managed root at that exact fixture. All six global volumes remained disabled and `not_authorized`; global-index entries were zero. The historic QA profile and other worktree scratch were not opened.

- The native file library displayed the fixture root and its seven files. Three files were selected for numbered rename. The existing `01 source-one.txt` conflict blocked Continue while the other two rows had executable previews.
- Explicit Skip left only the two executable previews in the final Operation Preview. The preview was left without confirmation; read-only hashes showed no change.
- Keep Both requested a fresh backend preview for `01 source-one (2).txt`. All three targets then appeared in Operation Preview. The confirmed operation completed 3/3 with no skips or failures. History showed three restorable rename entries.
- Restore was run through History's native UI and confirmation. The UI reported 3 restored, 0 failed, 0 skipped, 0 canceled. Read-only SHA-256 verification showed the original seven fixture files restored byte-for-byte and no generated rename targets left behind.
- Text mode was exercised in the native dialog: replacing `report` with `档案` produced `alpha-档案.txt` and `beta-档案.txt`, and both backend per-item previews were executable. This pass stopped before Operation Preview confirmation, so Text-mode execution is not claimed.
- Regex mode was exercised with `^(.*)-report$` → `$1-档案`. It produced `alpha-档案.txt` and `beta-档案.txt`; both backend per-item previews were executable. Both exact fixture destinations were reviewed in Operation Preview, then confirmed through the existing execute confirmation. The Tauri result reported 2 successful, 0 skipped and 0 failed. History exposed both entries as restorable; Restore reported 2 restored, 0 failed, 0 skipped, 0 canceled and 0 unexecuted. Read-only SHA-256 verification confirmed all seven original fixture files byte-for-byte and no generated rename targets remained.

## Platform and remaining evidence

- Windows 11 Tauri/WebView2: **PASS** for numbered rename and regex execution, Text and Regex secure previews, conflict/Keep Both, mixed blocked/executable explicit skip, Operation Preview, History and Restore. The native Text-mode pass stopped before execution.
- macOS GUI: **UNVERIFIED**. No native Apple Silicon run is claimed.
- Corrected code candidate exact-head GitHub Actions: **PASS** at `d7c3b702ad4019a6f60d1f58538618afc2ee699c`; the final report-only commit's checks are recorded on PR #514.

No merge or release was performed. FL-20 has not started; it follows only after FL-19 reaches `MERGE_GO`.
