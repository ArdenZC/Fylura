# B1 FL-19 Batch Rename — Acceptance Result

Date: 2026-10-30
PR: [#514](https://github.com/ArdenZC/Zen-Canvas/pull/514)
Current decision: **MERGE_HOLD**

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

## Final native follow-up — 2026-10-30

Live GitHub state was refreshed before this follow-up. `master` is still `8f197a7753a9f3f546ef214b617f36511f1be5ef`; Draft PR #514 is still `f5b9fafc317c1c038190f2d7b0c97ac689e22fb2`, with no merge conflicts. Its exact-HEAD Actions run [41009844128](https://github.com/ArdenZC/Zen-Canvas/actions/runs/41009844128) completed successfully. Frontend/format and the Windows/macOS aggregate Quality jobs succeeded; the run plan skipped lane-specific Rust/package and native-preview jobs. It is hosted CI evidence, not native GUI acceptance.

The follow-up used the separate QA profile `D:\_qa\zen-canvas-b1-fl19\profile` and only the task fixture `H:\U5\fixtures\B1-FL19-run-20261030-r2\cancel-test-200` on `ZCU5TEST`. The real Windows Tauri/WebView2 window was running against the Vite development server, not a packaged release build. AI remained off and no scan-source authorization was changed. No user volume or historical QA profile was used.

- Read-only fixture inventory found all 200 expected files (`cancel-000.txt` through `cancel-199.txt`), each 43 bytes. No extra claim entry was present in the directory.
- History Restore for the remaining 49 rename records reported **44 restored, 0 failed, 0 skipped, 0 canceled, 0 unexecuted**. Five records were separately marked **manual review** with the message “目标已提交，但无法确认持久化状态，需要人工复核。” The five files were physically present at their original names and their baseline SHA-256 values matched: `011` `A610FEAFC9D05E54CB1C905274C560FB4F548DFDFE53330DC03699D99D8B36D4`; `021` `26C972DCD0C5083CABFAC6164EFEA873F81960F11442752F9A18A696033A9286`; `025` `FC089EC200D051F93A3186AD61EBF019D8B1A85FE3AFC51C5935B32DA988C89E`; `026` `999340FCA3DD783483E7E727219F63436D63B948D56BB177CC09F4115BA86E5C`; `035` `6B21841D26A2BA0F03AADBE6E96D08A29A994DFE230951ABB5610DDF8374D7CD`.
- This is a History/Restore outcome mismatch, not evidence of file-content loss. The UI's fail-closed manual-review state was preserved; no retry, journal deletion, or direct database edit was attempted. The available evidence does not isolate whether final journal persistence or post-commit identity verification produced the five states.
- The final 150-item in-progress cancellation attempt was **not executed**: exact explicit row selection could not be established reliably through the available desktop automation, so no batch operation was started. Cancellation and cancellation/retry History consistency remain **UNVERIFIED**. This is not a product PASS or FAIL finding.

## Final disposition

- Windows 11 native GUI: prior rename, conflict/Keep Both, preview, execution and successful Restore flows remain PASS for their recorded fixture cases. The 49-item Restore follow-up exposed five manual-review records despite matching physical file names and sampled hashes. Batch cancellation is UNVERIFIED.
- macOS native GUI: **UNVERIFIED**; Apple Silicon GUI was unavailable.
- Exact-HEAD hosted CI: **PASS** for `f5b9fafc317c1c038190f2d7b0c97ac689e22fb2` at run `41009844128`.
- Merge recommendation: **MERGE_HOLD** until the restore outcome discrepancy is explained and cancellation is captured on a real operation. No source-code fix was made in this follow-up because the observed persistence/identity cause was not isolated; do not treat the five manual-review items as restored in History.

The report was committed as documentation-only HEAD `9b6d866dc976c979c8b75603e40b287d43f76d7b`; its exact-head Actions run [41060625158](https://github.com/ArdenZC/Zen-Canvas/actions/runs/41060625158) completed successfully (7 jobs succeeded, 20 were skipped by the validation plan). The FL-19 source code is unchanged from the tested code candidate above.

PR #514 remains Draft. No merge, release, issue closure, or FL-20 PR was performed.
