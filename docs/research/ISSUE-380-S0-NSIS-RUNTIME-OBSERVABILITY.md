# #380-S0 NSIS Runtime Observability Feasibility

**Disposition: `#380-S0 OBSERVABILITY_UNPROVEN`**

The source and package chain explain why Gate 1 had no usable diagnostic record. An independent NSIS 3.11 probe compiles, including a direct `FileOpen`/`FileWrite`/`FileClose` sink placed before `Abort`, but this environment cannot execute Windows NSIS binaries. The probe therefore does not prove that any log mechanism survives runtime `Abort` and process exit. No next Hosted Windows run is authorized by this report.

## Frozen inputs and scope

- Master candidate: `3275ac7c72df8ae4139a30f16c7f649971324e97`, tree `009576241cd4e8e2ba12cc644a368314f599faa6`.
- U1R-FIX candidate: `20c37a726d596466767bf95e4c5c989e9260470c`, tree `6944b875aa13b157a2072a5ffe1c87bd6ed0a286`.
- D1-A packaged candidate: `d260131f9d6ec354f2361e6c7460c79e62e38391`, tree `4765285668095fb3d7c0dcefd8a8ba5f2bd4fb69`.
- Existing Hosted run `38211883187`, attempt 1, is read-only evidence. It was not rerun.
- D1-A PR #386 and all production installer, IPC, Coordinator, and Schema files remain unchanged.
- The working branch is isolated as `research/issue-380-s0-nsis-observability`, based on the frozen U1R-FIX commit.

## What Gate 1 actually captured

The normalized evidence for run `38211883187` says the in-place installer was started with `/S` and `/LOG=<run-temp-path>`, returned exit code 2, and left `logExists=false`, `diagnosticTraceExists=false`, `oldUninstallerLogExists=false`, empty stdout/stderr, no `firstAbortMessage`, and no first failing condition. The result is recorded as `ROOT_CAUSE_UNRESOLVED`; it does not identify a Registry, Preview Handler, Service, or installer-page predicate.

The evidence archive's package logs show that both frozen candidates went through `npm run build`, Tauri's NSIS bundle path, and `makensis`. The package artifacts were real outputs, not merely source-file inspections:

| Candidate | Installer SHA-256 | Packaged `zen-canvas.exe` SHA-256 | NSIS package-log SHA-256 |
|---|---|---|---|
| Master `3275ac7c…` | `2d1d3d33095347984cb18f2889a4fe3bb12f16697027499fff16afdf36f72554` | `380dd4f04133f5d0dbf3be3daf1bfed8ea972ea267d2834c553df54e67a33050` | `cc11c9a109c124b938569fe8285dd9fc68bbb267243719cd1510f8da7f84a86f` |
| D1-A `d260131f…` | `9bc3fa75681baec8e0cf0ba87b3d4235545ad2f8b738aa5ca580dd8a6bbd8d41` | `7236b2edf0b6f13f0a3507a386dcea47fe64ace5ab3d936ebbb1164b39ec76ea` | `f5ea167bcc0a56eb2063e2221f028e56bb1f916670312fde77a6ae278b01824d` |

The package and binary hashes above are from the normalized Gate 1 evidence and its ZIP entries. The complete ZIP SHA-256 is `d7274ffc4795d3053a4f655080f91e51e04b0d9986c1beb4cc7570db214e143c`; the normalized JSON SHA-256 is `ab0a3f836ca9800cf9e1c1eeab1708b545a55f3c196dafc6d36419b02dedf2a2`.

## Why the trace was absent

### Proven by the harness source

1. `Install-NSIS` in `scripts/issue380U1HostedWindows.ps1:554-584` launches with `UseShellExecute=false`, `CreateNoWindow=true`, and redirected stdout/stderr. It passes `/S` and `/LOG=<path>`, then only reads the redirected streams and requested log file.
2. `Convert-U1RDetailPrintsToFileTrace` in `scripts/issue380U1HostedWindows.ps1:936-950` does not convert DetailPrint into a file trace. It validates that marker lines exist and writes the original source back unchanged. Its comment assumes NSIS `/LOG` will capture them. The function adds no `FileOpen`, `FileWrite`, or other durable sink.
3. The D1-A instrumentation in `scripts/issue380U1HostedWindows.ps1:1108-1155` and `1254-1255` edits the generated NSIS template and hooks to add `DetailPrint`. The old uninstaller also receives a separate `/LOG` argument through the generated maintenance path, but no direct file sink is added there either.
4. Consequently, stdout/stderr redirection is not a substitute for a NSIS DetailPrint log. The observed empty redirected streams and absent files are consistent with the instrumentation having no proven sink.

### Strong evidence about `/LOG`, but not a runtime proof

- In the earlier exact Windows package attempt `38191176125`, the D1-A package failed while `makensis` compiled the generated installer at line 481 with: `LogSet specified, NSIS_CONFIG_LOG not defined`. This build-time error is preserved in `docs/research/evidence/issue-380-u1r/run-38191176125/u1-evidence.json` (SHA-256 `7e5e34faa827391d6d822127811db4f0f86dc1831aee4cf0c80eea5b7ecf4fa3`).
- The independent NSIS 3.11 compiler reports no `NSIS_CONFIG_LOG` symbol in `-HDRINFO`; a negative fixture using `LogSet on` fails with the same diagnostic. The exact negative compile log is retained under `docs/research/evidence/issue-380-s0/harness/compiler-recheck/`.
- This proves the package compiler path does not support the in-script `LogSet` command used by that prior diagnostic attempt. It makes `/LOG` an unsafe assumption for Gate 1. It does **not**, by itself, prove the runtime behavior of `/LOG` when `LogSet` is absent; the current container could not execute the fixture.

### Installer, uninstaller, and UAC boundary

The Tauri 2.11.2 upstream NSIS template sets `RequestExecutionLevel admin` for `perMachine` installs. The checked-in template has no `UAC::RunElevated` call; `RunAsUser` is used later when launching the installed application. The Gate 1 runner directly starts the installer with `ProcessStartInfo` and redirected pipes. Its snapshot field `elevated=true` checks the PowerShell host's Administrator role; it does not record installer PID, parent PID, token elevation, UAC broker/consent child, or whether any elevated child inherited the caller's output handles. Those process/UAC details remain unverified.

Installer and uninstaller are different NSIS execution contexts (`Function` and `un.Function`, plus `.onInit`/`un.onInit`). The old-uninstaller `/LOG` path and top-level installer `/LOG` path were both absent in Gate 1. No process sample proves whether the old uninstaller was launched before the top-level abort. The next test must exercise both contexts separately.

## Actual package generation chain

This review follows the candidate build path rather than treating the `.nsh` source as the package:

1. `package.json:12` maps `npm run build` to `node scripts/buildWindowsPackage.mjs`.
2. On Windows, `scripts/buildWindowsPackage.mjs:48-82` builds the native Preview Handler, calls `prepareWindowsNsisTemplate()`, and invokes Tauri CLI 2.11.2 with `src-tauri/tauri.windows.package.conf.json`.
3. `src-tauri/tauri.windows.package.conf.json:7-11` selects `windows/installer-lifecycle-wrapper.nsh` and the generated template. `scripts/prepareWindowsNsisLifecycleTemplate.mjs:21-33` composes the pinned Tauri template (`a48a46149f6d6bdc76a0bf13f53e4acdfedb310b`) with metadata hardening, lifecycle orchestration, and hook relocation; `scripts/prepareWindowsNsisTemplate.mjs:51-60,379-384` checks and writes the pinned source. Tauri then generates and compiles the actual installer; the builder verifies the generated Preview resource path and removes its temporary generated template.
4. The Hosted harness created exact-clean worktrees for both candidate SHAs, ran `npm ci`, frontend and QA builds, injected test-only instrumentation, ran `npm run build`, and copied/hash-checked the actual Tauri-produced installer. Gate 1 logs and package hashes above verify this chain completed for both candidates.

The generated `installer.nsi` files and Windows `makensis.exe` binary/hash were not retained in the Gate 1 artifact. Therefore this review can verify the pinned source and actual package outputs, but cannot recompile byte-identical package scripts offline or identify the precise Windows compiler build from Gate 1.

The following production inputs are identical between master and D1-A by Git object comparison: `package.json`, `scripts/buildWindowsPackage.mjs`, `scripts/prepareWindowsNsisTemplate.mjs`, `src-tauri/tauri.conf.json`, `src-tauri/tauri.windows.package.conf.json`, `src-tauri/windows/installer-lifecycle-wrapper.nsh`, `installer-lifecycle-final.nsh`, `installer-lifecycle-synchronous.nsh`, and `installer-hooks.nsh`. The Gate 1 instrumentation differs by candidate, but it only adds test diagnostics and was restored before evidence reported the frozen source identity.

## Static product-check order (source facts only)

The generated installer replaces the upstream `Section Install` preinstall hook (`src-tauri/windows/tauri-2.11.2-installer.upstream.nsi:632-642`) with `Call ZCPrepareInstallLifecycleFinal`. In `src-tauri/windows/installer-lifecycle-final.nsh:106-132`, `ZCInitializeInstallLifecycle` runs first. In `src-tauri/windows/installer-lifecycle-synchronous.nsh:441-481`, it validates product metadata, then Preview Handler state, then Service ownership; it captures Service state only after those checks. The service stop, main-app gate, and Preview quiesce happen after that validation call. The upstream `EarlyChecks` and `WebView2` sections (`src-tauri/windows/tauri-2.11.2-installer.upstream.nsi:521-555`) occur before `Section Install`; the failed run did not show which page/section was reached.

Within the lifecycle checks:

1. **Pre-existing product / Registry metadata** (`src-tauri/windows/installer-hooks.nsh:403-478`). `DetectZenCanvasPreexistingProduct` computes expected fields first: quoted InstallLocation/UninstallString/DisplayIcon, package version/publisher/URLs, and `EstimatedSize = GetSize($INSTDIR, uninstall.exe) + ${ESTIMATEDSIZE}`. It then queries the uninstall key and manufacturer key. An unknown key open returns product state 2 immediately. If either key or the existing uninstaller file signals a product, it checks `DisplayName`, `MainBinaryName`, `DisplayIcon`, `DisplayVersion`, `Publisher`, `InstallLocation`, `UninstallString`, `NoModify`, `NoRepair`, `EstimatedSize`, optional URL fields, then the manufacturer key's default value. Most field mismatches set a shared invalid flag and the function reaches one final inconsistent-metadata state; it does not abort at the first bad value. `ValidateZenCanvasPreexistingProduct` then displays a generic message and aborts.
2. **Preview Handler** (`src-tauri/windows/installer-hooks.nsh:174-250`). `ZC_VALIDATE_PREVIEW_CORE` checks, in order: CLSID default friendly name; CLSID `AppID`; `InprocServer32\ThreadingModel`; the production CLSID under `PreviewHandlers`; and the `InprocServer32` default DLL path. A present value must have the required string type and exact value. Foreign/wrong-type/unreadable values abort immediately; a partially present core aborts after the sequence.
3. **Service ownership** (`src-tauri/windows/installer-hooks.nsh:491-535`). The exact Service key is queried. Absence is allowed; unknown key state is rejected. If present, `ImagePath` must be `REG_EXPAND_SZ` and exactly equal the canonical quoted install binary plus `--index-service`. Ownership state 2 aborts.
4. **Other possible earlier stops.** The upstream template runs `EarlyChecks` and `WebView2` sections before `Install`; the D1-A maintenance/version page and nested old-uninstaller path also precede or surround lifecycle execution. The Gate 1 evidence has no page, section, process, or Abort trace, so none can be eliminated as the first runtime failure from static order alone.

## What the existing snapshots do and do not show

The old fresh-install snapshot in run `38211883187` shows `EstimatedSize` as `REG_DWORD 35150`; the Preview CLSID/AppID, ThreadingModel, PreviewHandlers entry and canonical DLL path are present; Service `ImagePath` is `REG_EXPAND_SZ` with the canonical path/argument, and the Service is running. The same-path no-update attempt records unchanged pre/post snapshots for the old installation.

Those are external snapshots of the old installation. They do not reveal the D1-A installer's runtime `$ZC_EXPECTED_ESTIMATED_SIZE`, the `GetSize` return values, the raw type/value state returned inside `ZC_REG_QUERY_*`, or which validation function ran. They are not enough to declare any one predicate correct or failed. An EstimatedSize discrepancy is a plausible candidate because the expected value includes generated package metadata, but it is only a hypothesis; no product root cause is located.

## Independent NSIS probe

`scripts/issue380S0NsisObservabilityProbe.nsi` is a synthetic, isolated HKCU-only probe. It compares a synthetic `REG_DWORD` (expected 35150, actual 35149), emits DetailPrint, sets a nonzero exit code, and calls `Abort`. The `file` variant also calls a separate installer or uninstaller-context function that writes the synthetic predicate record with `FileOpen`/`FileWrite`/`FileClose` before Abort. It does not read or write product Registry keys or user data.

`scripts/issue380S0NsisObservabilityHarness.sh` is the runner contract for a Windows-capable NSIS execution environment. In this Linux environment, NSIS v3.11 compiler-only validation completed:

- Both `detail` and `file` variants compiled in `stage` and `abort` forms as x86 Unicode fixtures.
- The file-stage compile emitted warning 6010 for an installer trace function that its non-failing stage mode does not call; it still exited successfully. The abort variant calls that function.
- `LogSet on` negative compile failed as expected with `NSIS_CONFIG_LOG not defined`.
- Fixture SHA-256 values are summarized in `harness/compiler-recheck-summary.txt`; raw compiler output and hashes are in `harness/compiler-recheck/`.
- An initial x64 compile attempt failed because this Linux package has no `zlib-amd64-unicode` stub. The raw failure is retained in `harness/detail/compile-stage.log`; the x86 Unicode compile is the successful compiler-only result.
- Runtime could not start: `/usr/bin/wine --version` exits 126 because `/usr/lib/wine/wine` fails with `Exec format error` in this container. The fixture installer and generated uninstaller were not run.

Thus compiler acceptance and the negative compile are proven. The requested success/failure runtime assertions—DetailPrint persistence, `/LOG`, direct file trace after Abort/exit, uninstaller logging, and UAC behavior—are not proven. This is why the phase cannot be marked READY.

## Next experiment: minimum evidence contract (not an authorization)

Any later same-path experiment should be a single disposable Hosted Windows run only after Owner/main-dialog approval. Use a run-specific temporary evidence directory and synthetic/test profile; do not upload real user paths, filenames, Registry data, or database content. Required artifacts:

1. Retain the exact installer SHA, generated `installer.nsi` SHA, NSIS compiler version and binary SHA, source/tree identity, and instrumentation patch SHA. Capture the NSIS parent/elevated child PID chain, image paths, file hashes, start times, command lines, and token elevation. The runner's own token flag is not a substitute.
2. Before replacement, record the old mapped client PID, process creation time, process image path, command line, and the old candidate binary identity. After replacement, independently record the new on-disk image SHA and the still-running old process's mapped-image identity; do not infer mapped bytes from the path now holding the new file.
3. Record the new Service's SCM ImagePath type/value, Service PID/state/start time, process image path/hash, and the Service process identity observed by the client connection.
4. Write the first failed predicate synchronously to a run-owned file before every relevant `Abort`: phase/function, ordered predicate number, Registry key/value name, raw type, expected value, actual test value, and the NSIS error/return state. Close the file before Abort. Use separate installer and uninstaller trace functions. Keep `/LOG` only as a comparison channel, not the sole evidence sink. Make a missing/unwritable trace an explicit failed test.
5. Capture the actual client request and every Service response/event in order, including `protocol_version`, `request_id`, command, source/volume identity, refusal category, connection endpoint, and any Direct fallback decision. Redact unrelated machine/user fields. Record that a trusted refusal happened before any mutating fallback.
6. Snapshot the isolated index database immediately before and after the refused request. Include Global Entries/FTS, source checkpoint/state, Managed rows, AI state, and counts as hashes or scoped test values; keep snapshots closed before the client request to avoid file locking.
7. Assert the concrete expected first failure from the durable trace, nonzero installer exit, no product mutation before the ownership checks, exact old/new process identities, and database snapshot equality on refusal. If the check sequence reaches no failure, report that fact and do not substitute a guessed predicate.

This contract is a proposal for review. It does not start or approve another Hosted Windows run.

## Evidence index and hashes

- Gate 1 ZIP: `d7274ffc4795d3053a4f655080f91e51e04b0d9986c1beb4cc7570db214e143c`.
- Gate 1 normalized JSON: `ab0a3f836ca9800cf9e1c1eeab1708b545a55f3c196dafc6d36419b02dedf2a2`.
- Earlier Windows compile failure evidence JSON: `7e5e34faa827391d6d822127811db4f0f86dc1831aee4cf0c80eea5b7ecf4fa3`.
- NSIS 3.11 compiler fixture hashes and `-HDRINFO`: `docs/research/evidence/issue-380-s0/harness/compiler-recheck/`.
- SHA-256 manifest for all S0 report, harness, and compiler evidence: `docs/research/evidence/issue-380-s0/SHA256SUMS`.
- Wine runtime attempt and exact environment failure: `docs/research/evidence/issue-380-s0/harness/runtime-attempt.txt`.
- Harness sources: `scripts/issue380S0NsisObservabilityProbe.nsi` and `scripts/issue380S0NsisObservabilityHarness.sh`.

## Final status

`#380-S0 OBSERVABILITY_UNPROVEN`

The instrumentation defect is located: DetailPrint was left without a verified durable sink, while the requested `/LOG` path is not sufficient evidence. The package build path and static validation sequence are documented. The real runtime behavior of the candidate logging mechanisms at `Abort`/exit remains untested, so the first product predicate remains `UNKNOWN` and no Hosted Windows verification is started from this task.
