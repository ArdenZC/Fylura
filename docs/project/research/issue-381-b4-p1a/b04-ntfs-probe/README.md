# B04 Windows NTFS file API probe

**State:** `T10_B04_NTFS_PROBE_R1_FAIL_CLOSED_SOURCE_READY_FOR_OWNER_REREVIEW /
T10_B04_NATIVE_EXECUTION_NO_GO / WINDOWS_NAMESPACE_DURABILITY_UNPROVEN /
T10_REAL_ENOSPC_DEFERRED_NOT_TESTED / T10_PHYSICAL_POWER_LOSS_DEFERRED_NOT_TESTED /
P1A_FULL_NO_GO`.

This standalone Windows-only binary records NTFS file API behavior for the
directory-open/sync question behind T10-02. It is not part of the existing
T10-A runner. No test, build script, workflow, or application path invokes the
Windows probe automatically. The binary returns `NOT_EXECUTED` without
filesystem work unless `T10_B04_PROBE_ENABLE=1` is set.

## Cases and reported evidence

| Case | Call sequence | Requested access and flags |
| --- | --- | --- |
| A | `File::open(directory)` then `sync_all()` if open succeeds | Rust default; no custom flags |
| B | Open directory then `sync_all()` | `GENERIC_READ`, `FILE_FLAG_BACKUP_SEMANTICS` |
| C | Open directory then `sync_all()` | `GENERIC_WRITE`, `FILE_FLAG_BACKUP_SEMANTICS` |
| D | Create one small file, capture identity, write and sync, recheck identity, no-replace same-directory rename, read-only reopen, compare identity and content/hash | One file, one rename, `MOVEFILE_WRITE_THROUGH`; no replace/copy/cross-volume flags |

Each open, sync, rename, identity, content, and cleanup stage has its own JSON
observation. Rust I/O errors include `error_kind`, `rust_raw_os_error`, and the
Windows code surfaced by Rust as `win32_error_code`. The raw Win32 calls record
the immediate `GetLastError()` value separately and use a null Rust raw error.
Paths, user names, host names, arbitrary command output, and error messages
are not emitted.

## R1 fail-closed behavior

- The file identity is queried from the newly created handle before writing.
  A zero or placeholder volume/file identity is rejected. A failed identity
  query is `ENVIRONMENT_UNAVAILABLE`.
- The handle identity is rechecked after file sync, and the source pathname is
  reopened read-only and checked against the creation identity immediately
  before rename. `MoveFileExW` is not called unless the identity is valid and
  equivalent. Missing identity blocks rename; changed identity is a
  `SAFETY_REJECTION`.
- After rename, the destination is reopened read-only and its volume/file ID is
  compared with the creation identity. A query failure or mismatch stops the
  remaining read/hash step.
- Cleanup checks both generated names. It removes a path only after a live
  read-only handle reports the exact captured identity and a final path handle
  independently reports the same identity. Unknown, invalid, or changed
  identity leaves that path untouched and yields `CLEANUP_INCOMPLETE`. Cleanup
  never recursively traverses the sandbox. The original probe status is kept
  separately from any cleanup failure.
- The aggregate requires all three directory-open outcomes, a sync outcome for
  every successful open, and all dependent file/rename/identity/content phases.
  A directory API error such as NTFS OS error 5 is retained as an observed
  stage result; it is not mislabeled as a missing phase. Missing/skipped
  required phases become `PROBE_INCOMPLETE`; content mismatch or identity
  rejection cannot produce `FILE_OPERATION_OBSERVED`.
- Enabled fatal outcomes use nonzero process exit codes: safety rejection 2,
  environment unavailable 3, incomplete probe 4, cleanup incomplete 5.
  `NOT_EXECUTED` remains exit 0 when consent is absent or on non-Windows, but
  the machine-readable status is never a qualification pass. A fully recorded
  run uses `FILE_OPERATION_OBSERVED`, not `PASS`.

The eight original in-memory guard tests were expanded with focused tests for
missing/zero/placeholder/changed identity, rename authorization, required phase
completeness, wrong content digest, cleanup failure, API error observations,
and nonzero preflight outcomes. They do not call Windows APIs or touch files.

## Provenance and execution gates

An enabled Windows invocation requires `T10_B04_EXPECTED_HEAD`,
`T10_B04_EXPECTED_TREE`, and `T10_B04_EXPECTED_BINARY_SHA256`. It checks that
the checkout is clean and matches the expected Git source identity, then
streams and hashes its own executable and rejects a mismatch before creating a
sandbox. A future separately approved runner must build this binary fresh
from that exact clean checkout and set the expected executable SHA-256 from
the just-built executable in the same job. Point `CARGO_TARGET_DIR` at a new,
empty runner-local directory for each approved run so the executable cannot
come from a retained target cache. The required build command is:

```text
cargo build --release --locked --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml --bin t10_b04_ntfs_probe
```

The source checks its monotonic 30-second budget between probe API stages, but
cannot interrupt a blocked Win32 call. Preflight `git`, PowerShell, fsutil, and
executable hashing are also not cancellable by that in-process timer. R1
therefore still requires a separately approved runner-level 30-second
process/job timeout, same-job source/build/executable hash evidence, and a
reviewed safe timeout evidence/cleanup procedure before any native
authorization. No such wrapper or workflow is added here, and this source-only
revision does not claim timeout cleanup proof.

The probe rejects administrator or high-integrity tokens. GitHub documents
standard Windows hosted runners as running with administrator privileges and
UAC disabled ([hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners));
therefore a standard `windows-latest` job is expected to be rejected by
preflight. Do not remove that guard or reconfigure a privileged machine. A
separately Owner-approved, credibly non-elevated execution principal is
required; otherwise report `ENVIRONMENT_UNAVAILABLE`. No Windows execution is
approved by this source review.

After preflight the probe creates one unique direct child under verified
`RUNNER_TEMP`, uses only two fixed child names for one regular file, writes a
payload below 1 MiB, attempts each API at most once, and does not retry. Its
policy ceiling is 1 MiB combined and 10 disposable files. No UAC/admin action,
volume handle, system path, user data, disk image, quota, format, mount, real
ENOSPC injection, or physical power-loss test is used.

## Durability boundary

`FlushFileBuffers` requires a write-capable handle. Opening a directory with
backup semantics does not establish POSIX parent-directory `fsync`
equivalence. `MoveFileExW(MOVEFILE_WRITE_THROUGH)` has limited documented
semantics and is not proof of SQLite, manifest, and namespace atomicity. A
successful API response means only `API_RETURN_OBSERVED`; it proves neither
namespace durability nor survival of power loss. Real ENOSPC and physical
power loss remain deferred and untested.

## API references

- [Microsoft: Obtaining a Handle to a Directory](https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory)
- [Microsoft: CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
- [Microsoft: FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
- [Microsoft: `FILE_ID_INFO`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info)
- [Microsoft: `GetFileInformationByHandleEx`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandleex)
- [Microsoft: MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
- [Rust: `OpenOptionsExt`](https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html)
- [Rust: `File::sync_all`](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all)

The only raw Win32 operations are `GetFileInformationByHandleEx(FILE_ID_INFO)`,
`MoveFileExW`, and immediate `GetLastError()` reads. The identity buffer uses
the documented 64-bit volume serial plus 128-bit file ID and is queried from a
live `File` handle. Rename arguments are NUL-checked; the destination must be
absent; replace, copy/cross-volume, and delayed-reboot flags are omitted. These
test-only FFI calls are not a product durability adapter. This source review
adds no FFI entry point; the Windows-target type-check remains separate from
any native execution evidence.

## Branch and change scope

- Existing branch: `test/issue-411-t10-b04-ntfs-probe`.
- Verified starting HEAD/tree: `89b1fad5a47397c6409c014fe2fb329a5410530d` /
  `92ca5a6d5e7c6a0728eb233c030d69f7da0b3498`.
- Allowed changes: this binary and documentation/evidence beneath this
  `b04-ntfs-probe/` directory only. Cargo files, T10-A source/evidence,
  inherited workflow, production code, CI/router, Schema, and master remain
  outside the change.
- Branch lineage is the existing archive-only T10-A research branch; its
  inherited workflow is unchanged and targets a different branch name.
- Source-only logs and exit codes for this revision are recorded append-only
  under `evidence/attempt-002/`; `SHA256SUMS` covers the updated source/report
  and the new receipts.
- The source-hash-bound final check summary is
  [attempt 002](evidence/attempt-002/STATIC-CHECKS.md).
- No Windows probe, hosted-runner job, full CI, real ENOSPC, or physical
  power-loss test ran.

Stop state: `T10_B04_NTFS_PROBE_R1_FAIL_CLOSED_SOURCE_READY_FOR_OWNER_REREVIEW`.
