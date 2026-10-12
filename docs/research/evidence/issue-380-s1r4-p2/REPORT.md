# #380 S1R4-P2 Windows Path Matrix Result

## Decision

`#380-S1R4-P2 READY_FOR_OWNER_WINDOWS_PATH_RESULT_REVIEW`

The one authorized Windows path matrix completed successfully. This proves only the frozen P1 path guard matrix on the recorded Windows runner. It does not authorize or prove NSIS runtime behavior, installation, U2 production work, or any database/Service/IPC behavior.

## Run and source identities

| Evidence | Value |
|---|---|
| Workflow Run | [38258879642](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38258879642) |
| Job | [114843958776](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38258879642/job/114843958776) |
| Attempt | 1 |
| Event | `push` |
| Runner | `windows-2022` |
| Workflow name | `Issue 380 S1R4-P2 Windows path guard matrix` |
| Workflow commit HEAD | `86c21c14dd65bd2bbdb3056513489e0d4139bb47` |
| Workflow commit Tree | `883a851cf8e75b4f39a4a46baea15ab109feb312` |
| Frozen P1 tested source HEAD | `7a6fa69e5521d9759702c1923a9ade60a75819c3` |
| Frozen P1 tested source Tree | `c5465ef93eae268e2c683876d959db90fac5da08` |
| Test source path guard SHA-256 (Windows checkout bytes) | `39aaa4db01d082b7e6596a6f47108ee0220f9c6f32fb4086d67a14905404d512` |
| Test script SHA-256 (Windows checkout bytes) | `47f5c6b3f6fabe1608fdb0e00b8c48c08cd2c969021ee9bb07a090b9536e6e69` |
| Workflow SHA-256 | `4efcbde0ddc46d651ff90d5038d7ec0d68b681ceb19c2dd71138d3d27477d90d` |
| Run conclusion | `success` |

P2 activation commit `86c21c14dd65bd2bbdb3056513489e0d4139bb47` has one-file diff from frozen P1: `.github/workflows/issue-380-s1r4-p2-windows-path-matrix.yml`. P1 source HEAD/Tree were checked before execution and the exact frozen P1 files were run from a detached worktree.

The P1 manifest records LF checkout hashes `d38865aba8eccb444daba77ec4de572e64bcf2530f8e4f16c09b3aa7e7225ba9` (path guard) and `e0a6620456a04e08e3ef62cd001a01d6511dcd05e677680556862181221d4610` (test). The Windows worktree hashes above are the CRLF checkout forms; converting the committed P1 LF bytes to CRLF reproduces both recorded Windows hashes. The frozen Git HEAD and Tree remained unchanged.

## Runtime outcome

- Windows: `Microsoft Windows NT 10.0.20348.0`
- PowerShell: `7.6.6`
- Actual test process: `C:\Program Files\PowerShell\7\pwsh.exe` (PID `6020`)
- Arguments: `-NoLogo -NoProfile -NonInteractive -File D:\a\_temp\issue380-s1r4-p2-frozen-p1-source\scripts\testIssue380S1R4PathGuard.ps1`
- Exit code: `0`; timed out: `False`
- Cases: `40/40`, all expected/actual values matched and every `passed` value was true.
- P1 test fixtures: scope root removed `True`, external fixture removed `True`, junction/symlink links removed `True`.
- Frozen P1 source worktree: `OWNED_WORKTREE_REMOVED`.
- NSIS and unrelated executables: not launched. The path-test result records `process_launch_count=0` for the test logic and explicit `nsis_launched=false` / `unrelated_executable_launched=false`.

## Per-case original results

The original JSON is preserved in `artifact/path-matrix.result.json`. The table reproduces each case's `expected`, `actual`, `passed`, and error/rejection reason from that JSON.

| Case | Expected | Actual | Passed | Error / rejection reason |
|---|---|---|---|---|
| `taskroot_working_directory` | ALLOW | ALLOW | True | — |
| `direct_child_working_directory` | ALLOW | ALLOW | True | — |
| `nested_child_working_directory` | ALLOW | ALLOW | True | — |
| `shared_prefix_sibling_working_directory` | REJECT | REJECT | True | Working directory is outside TaskRoot: root=D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task candidate=D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task-sibling |
| `taskroot_parent_working_directory` | REJECT | REJECT | True | Working directory is outside TaskRoot: root=D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task candidate=D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507 |
| `other_runner_temp_working_directory` | REJECT | REJECT | True | Working directory is outside TaskRoot: root=D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task candidate=D:\a\_temp\issue380-s1r4-external-d732a6b3e33e44cd91ba6656783cd507 |
| `taskroot_outside_runner_temp_rejected` | REJECT | REJECT | True | TaskRoot is outside RUNNER_TEMP: runner_temp=D:\a\_temp task_root=D:\ |
| `traversal_that_resolves_back_inside` | REJECT | REJECT | True | WorkingDirectory path contains a parent-directory traversal segment: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\direct\..\direct |
| `traversal_to_parent` | REJECT | REJECT | True | WorkingDirectory path contains a parent-directory traversal segment: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\..\external |
| `uppercase_path_variant` | ALLOW | ALLOW | True | — |
| `forward_slash_path_variant` | ALLOW | ALLOW | True | — |
| `uppercase_forward_slash_path_variant` | ALLOW | ALLOW | True | — |
| `uppercase_taskroot_variant` | ALLOW | ALLOW | True | — |
| `forward_slash_taskroot_variant` | ALLOW | ALLOW | True | — |
| `trailing_separator_taskroot_variant` | ALLOW | ALLOW | True | — |
| `existing_in_root_executable` | ALLOW | ALLOW | True | — |
| `uppercase_executable_path_variant` | ALLOW | ALLOW | True | — |
| `forward_slash_executable_path_variant` | ALLOW | ALLOW | True | — |
| `taskroot_is_not_an_executable` | REJECT | REJECT | True | Executable must be a strict descendant of TaskRoot: root=D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task candidate=D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task |
| `missing_executable` | REJECT | REJECT | True | Executable path component does not exist or cannot be inspected: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\missing.exe; Exception calling "GetAttributes" with "1" argument(s): "Could not find file 'D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\missing.exe'." |
| `directory_cannot_be_executable` | REJECT | REJECT | True | Executable is not an ordinary file: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\direct\nested |
| `outside_taskroot_executable` | REJECT | REJECT | True | Executable must be a strict descendant of TaskRoot: root=D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task candidate=D:\a\_temp\issue380-s1r4-external-d732a6b3e33e44cd91ba6656783cd507\outside.exe |
| `another_runner_temp_directory_is_not_executable` | REJECT | REJECT | True | Executable must be a strict descendant of TaskRoot: root=D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task candidate=D:\a\_temp\issue380-s1r4-external-d732a6b3e33e44cd91ba6656783cd507 |
| `unc_path_rejected` | REJECT | REJECT | True | Executable path uses UNC syntax; only local drive paths are allowed: \\example.invalid\share\tool.exe |
| `unc_taskroot_rejected` | REJECT | REJECT | True | TaskRoot path uses UNC syntax; only local drive paths are allowed: \\example.invalid\share |
| `extended_device_path_rejected` | REJECT | REJECT | True | Executable path uses a Windows device namespace, which is not allowed: \\?\C:\Windows\System32\tool.exe |
| `device_namespace_taskroot_rejected` | REJECT | REJECT | True | TaskRoot path uses a Windows device namespace, which is not allowed: \\?\C:\isolated\task |
| `win32_device_path_rejected` | REJECT | REJECT | True | Executable path uses a Windows device namespace, which is not allowed: \\.\C:\Windows\System32\tool.exe |
| `nt_device_path_rejected` | REJECT | REJECT | True | Executable path uses a Windows device namespace, which is not allowed: \??\C:\Windows\System32\tool.exe |
| `relative_candidate_rejected` | REJECT | REJECT | True | Executable path must be fully qualified: relative\tool.exe |
| `drive_relative_candidate_rejected` | REJECT | REJECT | True | Executable path must be fully qualified: C:relative\tool.exe |
| `alternate_data_stream_rejected` | REJECT | REJECT | True | Executable path contains an alternate data stream or embedded colon: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\synthetic-compiler.exe:stream |
| `junction_working_directory_rejected` | REJECT | REJECT | True | WorkingDirectory path traverses a reparse point: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\junction-to-external |
| `junction_descendant_rejected` | REJECT | REJECT | True | Executable path traverses a reparse point: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\junction-to-external |
| `directory_symlink_working_directory_rejected` | REJECT | REJECT | True | WorkingDirectory path traverses a reparse point: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\symlink-to-external |
| `file_symlink_executable_rejected` | REJECT | REJECT | True | Executable path traverses a reparse point: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\task\linked.exe |
| `taskroot_symlink_rejected` | REJECT | REJECT | True | TaskRoot path traverses a reparse point: D:\a\_temp\issue380-s1r4-path-tests-d732a6b3e33e44cd91ba6656783cd507\taskroot-symlink |
| `taskroot_reparse_ancestor_rejected` | REJECT | REJECT | True | TaskRoot path traverses a reparse point: D:\a\_temp\issue380-s1r4-parent-link-d732a6b3e33e44cd91ba6656783cd507 |
| `compiler_version_working_directory_is_taskroot` | ALLOW | ALLOW | True | — |
| `compiler_hdrinfo_working_directory_is_taskroot` | ALLOW | ALLOW | True | — |

## Original outputs and artifact

- Original stdout: `artifact/path-matrix.stdout.raw.txt`; SHA-256 `2d6323b54961a37e2712bfbb6db7c9a7d128d9209a73bb1eda19d39f287ac3d9`.
- Original stderr: `artifact/path-matrix.stderr.raw.txt`; SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (empty stream).
- Test process record: `artifact/path-matrix-process.json`; exit code and timeout are retained.
- Full original GitHub Artifact ZIP: `artifact-original.zip`, ID `11751421846`, name `issue380-s1r4-p2-38258879642-attempt-1`, size `6876` bytes, GitHub digest `sha256:d4c139e3447a2ecb9c2e9bc376b7d36742d36aacbd07ed394db0d693c2436782`; local ZIP SHA-256 `d4c139e3447a2ecb9c2e9bc376b7d36742d36aacbd07ed394db0d693c2436782`.
- The Artifact's internal `SHA256SUMS` and `SHA256SUMS.sha256` were independently verified against every extracted file.
- Complete Actions logs: `actions-run.log`, 86607 bytes, SHA-256 `3bbd14395d873a307d604f4df4480a32b2bb54a3c1fb8242fe3fb5fd02ef522b`. Raw log download stderr is preserved separately and empty.

## Notes and limits

The Actions log contains non-failing GitHub annotations about actions being forced to Node.js 24 and dependency deprecation warnings. The workflow and job concluded successfully. No P2 rerun occurred, and the P2 branch was not changed after its activation push. No NSIS Runner, NSIS download/compiler, Probe, installer, uninstaller, application, Service, IPC, database, full CI, or performance job was run.

## Hash manifest

See [`SHA256SUMS`](SHA256SUMS) for hashes of this report, raw Artifact ZIP, extracted Artifact files, full Actions log, and GitHub run/job/artifact metadata. `SHA256SUMS.sha256` contains the manifest hash.
