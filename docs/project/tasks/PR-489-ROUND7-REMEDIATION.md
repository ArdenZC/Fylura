# PR #489 round 7 source remediation evidence

Date: 2026-10-28. Scope: Owner review [5681685333](https://github.com/ArdenZC/Zen-Canvas/pull/489#pullrequestreview-5681685333).
The PR body records the final exact candidate HEAD/tree, CI links and reconciled acceptance matrix.
This record is source/test evidence, not native release or merge approval.

## Reconciled starting identity

- Requested and observed remote PR HEAD: `31a98d710ee44ee81eac2ed616405ed28846bcc0`.
- Starting tree: `ac7cb03cd60ecd003d2e8b660181cf401c664946`.
- Observed remote master/base: `df940915084b810101a8a3eb78796cdf8730d55b`.
- Original `/workspace/Zen-Canvas` checkout was clean at master. Work was isolated in
  `/workspace/Zen-Canvas-round7`, branch `fix/pr489-round7`, then pushed to the
  existing `fix/issue-487-global-index-opt-in` PR branch without force.
- Baseline and full diff: `df940915084b810101a8a3eb78796cdf8730d55b...<final HEAD>`;
  this round alone: `31a98d710ee44ee81eac2ed616405ed28846bcc0..<final HEAD>`.

## Failure-first reproduction

Commit `443394aa842c90e47594acc08046b1048ae52808` adds regressions before the
production repair. [CI 40678847956](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40678847956)
retains the failing source candidate. The macOS Rust test step reports
1259 passed, 3 failed, 37 existing conditional/ignored tests. Its failures are:

- `reused_letter_does_not_hide_active_or_actionable_history`: one row instead of two.
- `capability_downgrade_recovery_requires_new_consent_and_preserves_history`:
  unsupported instead of restored MFT provider.
- `concurrent_demotion_between_enable_check_and_write_cannot_enable`:
  the concurrent enabler incorrectly succeeds.

Windows-only regressions additionally exercise the real Windows request route,
real DatabaseIndexSink and Coordinator against an isolated SQLite database.
The native watcher startup fails only on a unique nonexistent fixture path;
MFT/USN work is injected as an admitted checkpoint write, never a real-drive scan.
The failure-first Windows candidate also checks failed Journal creation 5/1314
followed by 1179. Windows failure-first CI reports 1372 passed, 6 failed,
37 pre-existing ignored tests. The additional failures explicitly show missing
full-index timestamp, watcher_unavailable instead of paused, and lost create
permission error. These are the six intended regressions; no unrelated Rust
failure occurs on that Windows candidate. Final Windows outcomes are linked in
the PR acceptance matrix.
The separate frontend consent test first fails on missing `NTFS USN` disclosure.
No assertion or performance threshold was removed to obtain a pass.

## P1-A: operation ownership and watcher recovery

The provider previously called the terminal sink operation after successful
MFT/USN work when watcher startup failed. That consumed the owner before the
Coordinator could save timestamps or complete rebuild intent.

The provider now calls `set_source_state`. Only the Coordinator finalizes the
operation. Admission changes source health to indexing/syncing, so any degraded
state reported during the admitted operation is preserved, even when equal to
last cycle's state. `watcher_unavailable` is a recognized operation degradation,
but not a persistent degradation carried into a successful recovered cycle.

`coordinator_watcher_lifecycle_tests.rs` verifies:

1. Failed watcher + successful initial work persists full/incremental timestamps,
   checkpoint and degraded status, releases exactly one owner.
2. Later cycles dispatch incremental work, not repeated initial work.
3. A successful rebuild completes its pending intent despite watcher failure.
4. A healthy watcher restores Ready and disables extra degradation polling.
5. The actual worker audit/wake loop performs incremental cycles while degraded
   and stays idle after recovery with unchanged topology. Only the test clock
   uses 25ms; Windows production remains `5 * 60` seconds.
6. Pause wins terminal status, disable fences old completion, and a rebuild
   requested during initial indexing remains pending for a later rebuild.

These are production sink/lifecycle tests with injected native work. They do not
claim real Windows watcher-failure-to-MFT-to-USN GUI E2E.

## P1-B: capability state machine and retained data

The UPSERT preserved every unauthorized Windows row's old provider, stranding a
recovered source behind stored unsupported capability. It now preserves only
the historical recursive fallback marker pending explicit reconfirmation;
other rows track freshly discovered capability independently of consent.

The on-disk regression repeats three complete cycles:

`MFT/authorized/enabled -> unsupported/not_authorized/disabled -> MFT/not_authorized/disabled -> explicit confirmation -> MFT/authorized/enabled`.

Each cycle checks retained source identity, one indexed entry, journal ID/cursor,
full timestamp, AI state, managed scope and AI job. Enabling and authorization
without confirmation fail after recovery; reopen retains the indexed entry.
The existing actual Schema44 migration/reconfirmation fixture remains in the
regression suite, retaining its fallback marker and one-time confirmation policy.

Windows discovery/database tests combine the production classifier with:
GUID loss, colliding serials, different GUIDs reusing a letter, letter changes,
FAT32 cloud substitution, virtual NTFS without USN, recovery and explicit
reauthorization. All historical identities remain stored. Unsupported/cloud
sources cannot authorize the native path. File Library code is unchanged.

## P2-A: atomic enable authorization

The enable UPDATE includes the current Windows authorization predicate. A later
SELECT chooses only a rejection message and cannot authorize a write. Disable
remains unconditional. A controlled channel handshake uses two Database instances
on the same file: demotion commits after the old check boundary and before the
write. The rejected enable leaves revision unchanged and enabled false.
A second test verifies idempotent enable retains the live owner, disable increments
revision and clears it, stale completion cannot write timestamps, and re-enable
of an authorized source increments revision with no inherited owner.

## P2-B: journal recovery and consent

If Journal creation fails and requery still returns 1179, the typed creation error
is returned. Diagnostic context retains Journal absence. A successful competing
creator is accepted; a different requery error is retained. Tests cover creation
codes 5, 1314 and non-permission 19, requery 21, competing creation and successful
creation. The recovery error is serialized/deserialized through the existing v4
service response taxonomy: 5/1314 remain `permission_required`, 19 remains error.
Existing service-host/client authorization and IPC tests also remain active.
No ACL, UAC or Windows security mechanism is changed.

The existing Settings authorization dialog uses these translations:

> 授权后，Zen Canvas 将为整个选定的本地 NTFS 磁盘建立文件名、路径和属性索引，包括系统和其他用户文件。可能建立或配置 NTFS USN 变更日志，这是持久化的卷级文件系统元数据；某些操作需要管理员或服务权限。未授权磁盘和 Google Drive 云盘不会执行此操作。是否授权此磁盘？

> Authorize metadata indexing across the entire selected NTFS volume (local disk), including system and other users’ files? Zen Canvas may create or configure the NTFS USN change journal (persistent volume-level filesystem metadata). Some operations require administrator or service privileges. Unauthorized disks and Google Drive cloud drives are excluded.

## P2-C: duplicate mount projection

Only inert unsupported, unauthorized, disabled, zero-entry rows with no journal
checkpoint and discovered/unavailable health may be hidden behind a newer row.
Enabled, permission-degraded, legacy-confirmation and indexed historical sources
remain visible. Sorting has a deterministic identity tie-break. No stored row,
index, identity, AI state or historical G: record is deleted or merged.
No native G: discovery or filesystem access was performed.

## Local validation and separate failures

Environment: Linux x86_64 cloud workspace, task-owned Ubuntu 24.04 Docker container,
Rust stable, Node 24.19.0. Missing GTK/WebKit development dependencies were installed
in that container; production dependency manifests were not changed.

- Frontend: 188 files / 1997 tests PASS; focused consent/Settings/coverage 49 PASS.
- Typecheck, frontend production build, 30 performance-architecture checks,
  documentation/governance and Cargo format PASS.
- Local Global Index Rust run: 99 PASS, 4 FAIL, 12 pre-existing ignored tests.
  New capability/TOCTOU/owner-fencing/projection regressions PASS. No ReadOnly
  occurs in this run. Windows-only tests require Windows CI.
- All four failures are Managed AI fixture timeouts/no requests at the local
  Ollama fixture: disabled-control wake, scheduler-policy wake, queue refill,
  and live assessment consumption. On unchanged master `df940915`, in the same
  container, those same four fail (3 other Managed AI tests PASS). The independent
  `ai::settings::validation_tests::system_credential_store_fails_closed_on_unsupported_platforms`
  test also PASSes. Source tracing identifies the cause: fixtures save with an
  InMemoryCredentialStore, but the live worker calls `get_ai_settings_for_db`,
  which reads SystemCredentialStore even for Ollama. On Linux that deliberately
  returns `system_credential_store_unsupported_platform`; the worker converts
  the settings error to None and never contacts the local fixture. Installing
  GTK/WebKit does not change this intentional unsupported-platform policy.
  This baseline AI limitation is separate from SQLite ReadOnly. No AI/security
  behavior was changed to make these failures green.

Local raw logs are retained in `/workspace/scratch/round7-*.log`; hosted test
logs are retained by their linked Actions runs. The original missing-glib build
failure and the initially omitted `DOCS_DIFF_BASE` invocation are retained;
installing libraries and supplying the documented diff base resolved them.

### Historical 15 SQLite ReadOnly failures: UNVERIFIED

The prior host's failing database/WAL/SHM files and original per-test extended
SQLite codes are not available in this workspace. Current source inspection
finds pooled connections (max 8, min idle 1), WAL configuration, PID/counter temp
names in Global Index tests, task-local coordinator names, and explicit removal
at fixture cleanup. A still-live Database clone, checked-out connection or worker
can outlive a local Database binding; cleanup/name reuse is therefore a lifecycle
hypothesis, not an established explanation for the historical failures.
Current parallel production-database tests, two-connection contention tests and
read-only WAL observer checks ran without ReadOnly. A non-reproduction and green
hosted CI do not establish the old cause or prove it unrelated. Root-cause
acceptance remains UNVERIFIED pending original same-host logs/fixtures or a
controlled reproduction with extended code and connection/cleanup timing.

## Retained historical and native boundaries

W2-11 historical intermittent `blob:` resource failures remain
HISTORICAL_INTERMITTENT / MONITOR, root cause UNVERIFIED. Preserve the existing
bounded diagnostics and artifacts
[12801564933](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40656903855/artifacts/12801564933)
and [12807343226](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40656903855/artifacts/12807343226).
A later pass does not prove a lifecycle race cause. No gate threshold was changed.

Still UNVERIFIED: actual installed v3-to-v4 service upgrade; genuine 5/1314 through
native service/IPC/GUI; Owner-installed Windows GUI and regular-user permission
acceptance; real watcher failure/recovery with native MFT/USN and GUI. Source and
injected lifecycle tests are separate evidence. #371 Smart Lean remains NOT
IMPLEMENTED. No release, merge, master update, real C/D/E/F/G scan, user-data
cleanup or performance implementation/threshold change is authorized or claimed.
