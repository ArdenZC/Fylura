# Issue #329 V2 — Native Failure Root-Cause Remediation

Last verified: 2026-10-11

## Native failure follow-up — 2026-10-11

Owner reported the frozen installer result as **RECOVERED (PARTIAL), NOT PASS**. The Cloud workspace cannot access `D:/Install_Package/Zen-Canvas-Windows-11658618364/NATIVE-QUALIFICATION-REPORT.md`; this follow-up uses only the observations repeated in the Owner request and does not claim to have read the Windows report or logs.

The starting PR candidate was `8fe826238052846df37414944af732f200224d23`, tree `5d8aafd39fe08ead5f0d246440d8bd02ddf99aa7`. The latest fetched master was `58062c5c356969f332f19c7458028bf2e097595e`. It was merged into the existing PR branch by `52650e4fc440dcfe46afcea1ff6cf5a23c6d64ee`, tree `6b3d25e73b94875d0c217face71e079b2ffa65a6`, without conflicts. Master changes did not overlap the existing Settings / Onboarding files. The current source candidate still requires exact-head Hosted CI; PR #335 remains OPEN / Draft and Issue #329 remains OPEN.

The native sequence remains: first Settings save showed the generic first-use error; retry was followed by `rollback_reconciliation_failure`; Settings later showed an enabled root while File Library was initially empty until the same folder was selected again. Global Index showed C: permission required and D: unavailable. No file move, delete or rename was observed. The native Settings failure stage, Settings revision sequence, root-sync outcome, active watcher owner, reconciliation admission outcome, clean-first-run reproducibility, and Global Index service logs were not supplied to Cloud. The original failure therefore remains **unattributed**.

Two source-level issues are now covered by bounded changes:

- `useAppSettings` exposes the currently persisted Settings snapshot separately from optimistic editor state. `AppRuntimeProviders` now derives scan roots, custom search roots, startup background-index admission and scanner defaults from that persisted snapshot. A failed root save no longer admits background indexing using a root that exists only in optimistic renderer state. Backend CAS / watcher ordering and existing fail-closed errors are unchanged.
- When persisted Settings contain an enabled scan root but the saved File Library scope is still the empty default `current_scan`, the File Library adopts its existing `all_enabled_roots` scope and persists it. Explicit scopes and completed scan-session scopes are preserved. This matches the observed “Settings enabled, Library empty until selecting the same folder” path in the source; the actual installer sequence was not independently reproduced in Cloud.

For the next authorized native qualification, `native-qa` builds can opt in to `ZC_NATIVE_QA_SETTINGS_TRACE=1`. The bounded trace records Settings revisions, enabled-root counts, save/rollback stage, runtime-restore outcome, and SQLite primary/extended result codes without paths or raw database errors. Watcher trace separately records restart and reconciliation-scheduling outcomes. This is diagnostic evidence for a future run, not proof about the frozen run.

Global Index behavior was not changed. The reported C: permission-required and D: unavailable states are source/provider availability signals; no lock-owner, SQLite contention, or index-service startup trace was supplied. There is no evidence that the #329 Settings/Onboarding failure and #328 Global Index issue share a root cause. #328 remains independent.

| Validation | Result |
| --- | --- |
| Focused Settings, Onboarding, Library scope, background-index tests | 5 files, 38 tests PASS |
| Full frontend suite | 176 files, 1,874 tests PASS |
| Frontend typecheck | PASS |
| Performance architecture | 3 files, 30 tests PASS |
| Frontend production build | PASS; existing CSS optimizer and PDF dynamic-import warnings remain |
| Governance and documentation | PASS (`DOCS_DIFF_BASE=origin/master`) |
| Cargo format | PASS |
| `git diff --check` | PASS |
| Local Rust Settings unit tests | BLOCKED before test execution by the existing Linux-only missing `keyring` target dependency; no source compile error remains in the edited Settings code |
| Exact-head Hosted CI | Pending on the pushed candidate |
| Windows native qualification / installer | Not run in Cloud |

No package version, Schema, IPC, or global SQLite timeout change was made. No #328 Global Index production behavior was modified.

## Prior latest-master reconciliation — 2026-10-10

The actual master baseline was `9ac78cf86ed99deed16caaddab4164bdd631c3be`. Existing PR #335 HEAD `8c38e12676e77f72d206f3ab81e99ba14cbeb3ea`, tree `59f31f68e0fcb8166ef40145540c18762860e3d5`, was integrated by history-preserving merge `8b9a7805734e47cd8bda9d52a196bfccea58954c`, tree `93cb937263ae6fb983d05a86a4491891f98844f7`. The merge's first parent is the previous PR HEAD and second parent is the fetched master baseline. It completed without conflicts and preserved all V2 commits.

Relative to that master, the PR still changes the same 16 files: 14 Settings, watcher, Onboarding, and regression source/test files plus `docs/project/STATUS.md` and this report. No `package.json`, lockfile, database Schema, Tauri command/IPC contract, or #328 Global Index production behavior was changed by the reconciliation. Current master changes from CI validation-plan governance, the #345 contention investigation, and Global Search benchmark/query work are inherited from master; they were not copied into the #329 diff. Draft PRs #356 and #358 were checked for scope overlap.

The V2 invariants remain unchanged: the CAS starts `BEGIN IMMEDIATE` before its in-transaction prior-settings read; the configured SQLite busy timeout remains 5,000 ms; persistence requires an explicit successful save result; the selected initial folder intent and normalized path comparison remain; watcher reload, reconciliation scheduling, compensation, and stable failure classification remain fail-closed; and a failed save cannot mark Onboarding complete or add a Managed Scope. No second settings/watcher/SQLite coordination authority or generic retry layer was added.

Local checks on the reconciled source passed: focused Settings/Onboarding/error tests (4 files, 26 tests), frontend typecheck, full frontend suite (176 files, 1,870 tests), remediation tests (14), performance architecture checks (30 tests), frontend build, Cargo format, governance, documentation, YAML parsing, helper syntax, and diff check. The frontend build retained its existing CSS optimizer and PDF chunking warnings. A Linux attempt to run the Rust Settings/CAS tests stopped during crate compilation because the existing Linux target does not include the `keyring` dependency referenced in `src/ai/settings.rs`; no Rust test binary ran locally. Windows/macOS Hosted Rust quality and the real-pool SQLite CAS contention tests remain required evidence.

The prior exact-head failures, including CI `37863698017` and `37861489174`, remain preserved as failures. The fresh exact-head Hosted CI and its Windows/macOS Rust, Settings CAS contention, Performance profile, Windows Global Index qualification, and aggregate validation-plan outcomes must be read from PR #335's checks on the reconciled candidate; no older run is promoted as a pass. This reconciliation does not change the original Windows event's evidence: no exact SQLite stage or lock owner was retained, so its historical root cause remains unproved. #328 remains independent, and no Windows local build, installer, or native qualification was run here.

## Historical V2 disposition

## Disposition

**IMPLEMENTATION COMPLETE — EXACT-HEAD SOURCE VALIDATION LANES PASS; THREE AGGREGATE CI CHECKS FAIL ON A SEPARATE VALIDATION-PLAN INPUT DEFECT; COMPLETED REVIEW FINDINGS RESOLVED.** PR [#335](https://github.com/ArdenZC/Zen-Canvas/pull/335) remains **OPEN / Draft**. Issue #329 remains **OPEN**. Codex CLI follow-up review after the finding fix was blocked by its usage limit; the user then instructed that no further codex-review be run, so no post-fix review pass is claimed. This report does not authorize Ready, merge, native qualification, or issue closure.

The frozen Windows native failure installation was not touched. No installer build or Owner-driven native product qualification was initiated. The automated disposable Windows Global Index service gate is reported separately as hosted CI.

## Source identity

| Item | Identity |
| --- | --- |
| Starting PR head | `3b303ea54227fc4ce6711f770b0e51475d568648` |
| Starting tree | `b2a2666546309cca20ae8aa94ca505a961f10719` |
| Current master observed | `0eda3e0d232a22eba116a99567b7bdf6735c5104` |
| Source candidate | `c85257940c591035d1a5520b50df33292fd38142` |
| Source tree | `61f7a64be9bb232cb2b952177bef4464d367212f` |
| Branch | `fix/issue-329-onboarding-scan-scope-save` |
| PR | #335, OPEN / Draft |
| Documentation closeout | Docs-only commit on the same PR branch, after the source candidate |

The branch was continued from the existing PR head without rebasing for the docs-only master advance.

## Implementation and root-cause evidence

The production authority chain remains:

`OnboardingDialog.saveFolderSetup` → `setDefaultScanFolders` → `updateSettingsWithResult` → Tauri `save_settings` → versioned Settings CAS → watcher root sync → watcher restart and reconciliation scheduling → success or Settings/watcher compensation.

`save_app_settings_cas` now acquires `TransactionBehavior::Immediate` before reading the prior Settings row. This is limited to that CAS transaction. It does not change the global five-second SQLite busy timeout, add retries, or change catalog-revision, serialization, normalization, or revision-conflict rules. A process-local Settings save guard covers the previous snapshot, CAS, watcher reload, and compensation; it does not hold a SQLite transaction while runtime work runs. Lock order is the Settings save guard followed by the catalog execution guard inside the CAS transaction; the catalog guard is released before watcher-root synchronization. No reverse acquisition path was found.

The real-pool regression compares the previous deferred boundary with the repair:

- Deferred baseline: a second pooled connection holds `BEGIN IMMEDIATE`; the real CAS performs its read then fails at the read-to-write upgrade with `SQLITE_BUSY` primary/extended code `5`, before the 300 ms holder release. Persisted Settings remain unchanged.
- Repaired short contention: the real CAS waits for the holder to release inside the existing timeout, then saves exactly one revision.
- Over-timeout contention: the real CAS remains fail-closed with `SQLITE_BUSY(5)` around the existing five-second bound. The test also confirms no Settings or root mutation before acquisition; after release, one explicit save and root sync succeed.
- A stale expected revision remains `RevisionConflict`; the winning Settings row and single catalog revision increment remain authoritative.

The watcher-root synchronization path already uses `BEGIN IMMEDIATE`. Its acquisition failure happens before mutations and no retry was added. A transaction-triggered failure after root insertion is covered; transaction drop rolls back the inserted root, the Settings command persists a compensating Settings revision, and the command returns the root-sync support code. A bounded retry at the root-sync acquisition boundary could be safe before transaction/mutation begins, but it was not added: the existing five-second busy handler already bounds the wait, and a retry policy was not authorized. Errors after transaction start are not generically replayed.

Watcher runtime failure is separate from SQLite contention. A regression supplies a valid existing directory and injects a watcher-manager state-lock failure without a database operation; no watcher owner is installed. The production reload preserves the shared startup/wake behavior of attempting reconciliation scheduling even if restart fails, while keeping restart failure as the reported stage. Scheduling failure is separately classified and tested through the Settings compensation boundary. Settings compensation remains fail-closed: failure to restore runtime state yields `rollback_reconciliation_failure`, never success.

The V2 tests prove a Settings CAS writer-contention mechanism that matches a deferred-CAS mechanism investigated separately in #328. This is implementation-mechanism overlap only. It does **not** prove that the frozen Windows native event or the currently reported Global Index states shared that cause; the original native failure remains unattributed, and no shared historical root cause is claimed.

The exact historical Windows native event remains **INFERRED**, not PROVED: the frozen run exposed only a generic save failure and did not record its stage or SQLite lock owner. The patch proves and repairs a real Settings CAS failure mechanism on the current production path; it does not retrospectively prove that the native event reached that stage.

## Diagnostics and preserved behavior

The Settings error path now exposes only stable, non-sensitive stage strings:

- `settings_save_failure:database_failure`
- `settings_save_failure:revision_conflict`
- `settings_save_failure:watcher_root_sync_failure`
- `settings_save_failure:watcher_runtime_failure`
- `settings_save_failure:watcher_reconciliation_schedule_failure`
- `settings_save_failure:rollback_reconciliation_failure`
- `settings_save_failure:unknown_failure`

The localized user message includes the support code. Backend errors, paths, file content, credentials, and SQLite details are not included in the support code or watcher event payload. No telemetry or logging database was added.

The existing normalized-root semantic comparison, `persisted: true` requirement, `/` handling, delayed-load intent and single conflict rebase remain in place. Onboarding keeps its failure alert and cannot advance or mark completion after a failed root persistence. Settings add/enable/disable/delete continue to use the same persisted versioned Settings and watcher-root authority.

## Changed files

- `src-tauri/src/settings.rs`
- `src-tauri/src/watcher.rs`
- `src-tauri/src/global_index/lock_contention_tests.rs`
- `src/components/AppRuntimeProviders.tsx`
- `src/components/OnboardingDialog.tsx`
- `src/hooks/useAppSettings.ts`
- `src/i18n/dictionary.ts`
- `src/utils/viewHelpers.ts`
- `tests/appSettings.test.ts`
- `tests/onboardingSettingsPersistence.test.tsx`
- `tests/stableErrors.test.ts`
- `docs/project/STATUS.md`
- this report

## Validation

| Check | Result |
| --- | --- |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | PASS |
| `git diff --check` | PASS |
| `npm run typecheck` | PASS |
| Focused Settings / delayed-load / Onboarding / error tests | 4 files, 26 tests PASS |
| Full `npm test` | 176 files, 1,810 tests PASS |
| `npm run test:remediation` | 1 file, 14 tests PASS |
| `npm run test:performance:architecture` | PASS; 3 files, 30 tests PASS |
| `npm run build:frontend` | PASS; existing CSS optimizer and PDF dynamic-import warnings remain |
| Local Rust unit tests | BLOCKED before test execution on Linux: the current crate references `keyring::Entry`, while Cargo declares `keyring` only for Windows/macOS targets. No Linux test binary ran. New snapshot assertions were corrected to compare serialized settings after the first compile exposed the absent `AppSettings: PartialEq` implementation. |
| Hosted Windows/macOS CI for source candidate | Source validation lanes PASS; overall run FAILS on three aggregate validation-plan checks; [run 37861489174](https://github.com/ArdenZC/Zen-Canvas/actions/runs/37861489174). |
| Native qualification | Not run, by scope |

The default Cargo source cache (`/home/agent/.cargo/registry`) and npm cache (`/home/agent/.npm`) persist across work in this managed environment; those package managers keep separate formats. The previously missing Linux native build dependencies are now cached together: 127 Debian packages (61.8 MB downloaded) in `/home/agent/.cache/zen-canvas/apt/cache/archives`, extracted to a reusable 285 MB user-owned sysroot at `/home/agent/.cache/zen-canvas/sysroot`. `/home/agent/.cache/zen-canvas/prepare-linux-native-cache.sh` recreates the cache and `/home/agent/.cache/zen-canvas/native-deps.sh` activates it for builds. Running the preparation script again used **0 B** of package archives, confirming reuse. This avoids system installation privileges; `node_modules` and Cargo build output remain workspace-local. With the sysroot active, pkg-config resolves GLib, ATK, GTK and WebKit. Linux Rust test execution then reaches the existing platform-specific `keyring` compile error described above; supported Windows/macOS hosted lanes remain the validation target.

The hosted validation sequence exposed three test-only issues, each corrected on the next source candidate:

- Run `37815479031` on `dbc7ed3` found that the short-contention Windows fixture expected backslashes while production serializes normalized scan roots with forward slashes. The expected fixture now normalizes its path in `170e202`.
- Run `37820064428` on `170e202` found the existing rollback integration test still expected the old internal error text. Its assertion now checks `settings side-effect reconciliation failed` in `3ac7264`.
- Run `37821143157` on `e71f6e7` found that the over-timeout CAS test had a redundant strict eight-second ceiling. The test includes waiting for the process-wide catalog execution guard, so parallel macOS test scheduling made the observed elapsed time `9.548870375s` even though it returned the expected `SQLITE_BUSY(5)`. `c852579` removes only that redundant assertion; the test still verifies the configured 5,000 ms SQLite busy timeout, `SQLITE_BUSY`, unchanged persisted Settings/root state while locked, the shared 4.5–20 second bounded-wait guard, and successful save/root synchronization after releasing the holder. The three downstream validation-plan failures in run `37821143157` reported missing head-validation success after this Rust failure and were dependent gates.

The current exact-head run is `37861489174` on `c852579`. Its source checkout, scope and lane-plan contracts, Windows disposable Global Index service gate, Windows/macOS release compile, Rust head and merge lanes, frontend/browser head and merge lanes, and Search/Scan performance shards passed. The overall run is red because `Performance profile`, `Quality (windows-latest)`, and `Quality (macos-latest)` fail at `Verify validation plan` with `non-equivalent trees require head validation success, got missing.` Rerunning those failed jobs after head validation completed reproduced the same three failures.

The CI workflow invokes `scripts/ciValidationPlan.mjs --aggregate` in those jobs without the required `HEAD_VALIDATION_RESULT`, `INTEGRATION_VALIDATION_RESULT`, or `LANE_JOB_RESULT` inputs. For non-equivalent PR trees, the helper therefore has no head-validation result to evaluate. This is an existing workflow/helper call-contract defect, separate from the #329 source changes. The workflow and helper are outside this bounded remediation scope, so they were not modified. The aggregate failures remain a separate Owner gate; no CI green result is claimed. Earlier source-specific failures occurred only on superseded candidates and are listed above.

## Independent review history and disposition

Completed `codex review` passes were separate from implementation and focused on CAS ordering, rollback/reconciliation, semantic roots, privacy/fail-closed behavior, #328 overlap, and cross-boundary tests. The user has since prohibited any further codex-review runs; none were run after that instruction.

1. **P1 — watcher error return type broke native Rust callers.** Resolved by retaining the public `Result<bool, String>` watcher reload boundary and adding a private typed stage-returning function for the Settings command. Native startup/wake callers remain source-compatible.
2. **P2 — early return after watcher restart failure skipped reconciliation scheduling for startup/wake callers.** Resolved by preserving the existing behavior that attempts both restart and scheduling and reports the restart error first. Added a regression that ensures scheduling is attempted after restart failure.
3. **Final focused review after the serialized-settings assertion correction:** no actionable substantive defects. The serialized-settings assertions compare the full persisted contract without adding `PartialEq` to the production settings type; the review also ran the 26 focused frontend tests successfully. Rust/native execution was not independently validated by codex-review.
4. **Independent review on commit `3ac7264`: P2 — rollback absence assertion used an unnormalized Windows path, allowing a false negative.** Resolved in `e71f6e7` by normalizing the query parameter to forward slashes before checking persisted roots. This was a test-only correction; no production defect was identified by that pass.
5. **Follow-up review after `e71f6e7`:** the attempt returned a Codex CLI usage-limit error and terminated without a review result. The user then prohibited further codex-review runs, so no review was attempted after `c852579`. That source change only removes the brittle eight-second timing assertion described above; production behavior is unchanged. The earlier P2 finding is resolved in source.

Every substantive finding returned by a completed review has an explicit resolution above; none is known to remain open. The attempted follow-up produced no result, and `c852579` has no independent post-fix review signoff. The changed watcher stage does not alter Tauri command registration, IPC shape, or the public watcher reload Rust return type.

## Package, schema, IPC, #328, and remaining limits

- Package remains `0.1.40`; database schema remains `37`; IPC remains `v3`.
- The `save_settings` command and its success payload are unchanged. Its error string now uses a stable support classification; no new IPC field or command was introduced.
- #328 remains a separate issue and authority. No Global Index behavior, global SQLite mutex, timeout, or retry framework changed.
- SQLite contention codes in the authored Rust regressions are `SQLITE_BUSY` primary/extended `5`; the injected post-mutation root trigger expects primary `ConstraintViolation`, extended `1811`. Rust execution of these tests awaits hosted CI.
- Exact historical native failure attribution remains inferred. The manager runtime-fault regression is deterministic injection, not a native watcher qualification.
- Reconciliation scheduling can admit durable scan work before a later scheduling error is returned. The existing scheduler remains its authority; this patch does not add cancellation or replay. Settings still rolls back and surfaces a failure code. Owner review should consider this existing partial-admission boundary when interpreting a scheduling-stage failure.
- PR #335 remains Draft; issue #329 remains OPEN. No Ready, merge, installer build, native qualification, or closure is authorized here.
