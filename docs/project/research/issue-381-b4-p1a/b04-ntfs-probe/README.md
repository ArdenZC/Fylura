# B04 Windows NTFS file API probe

**State:** `T10_B04_NTFS_PROBE_SOURCE_ONLY / T10_B04_NATIVE_EXECUTION_NO_GO /
WINDOWS_B04_NAMESPACE_DURABILITY_UNPROVEN / T10_REAL_ENOSPC_DEFERRED_NOT_TESTED /
T10_PHYSICAL_POWER_LOSS_DEFERRED_NOT_TESTED / P1A_FULL_NO_GO`.

This is a new, standalone test binary at
`tests/issue381_b4_p1a_f1/src/bin/t10_b04_ntfs_probe.rs`. It records Windows
file API return behavior for the exact directory open/sync question behind
T10-02. It is not part of the existing T10-A runner and no test, build script,
workflow, or application path invokes it automatically.

## Probe cases

| Case | Call sequence | Requested access and flags |
| --- | --- | --- |
| A | `File::open(directory)` then `sync_all()` if open succeeds | Rust standard default; no custom flags |
| B | `OpenOptionsExt::open(directory)` then `sync_all()` if open succeeds | `GENERIC_READ`, `FILE_FLAG_BACKUP_SEMANTICS` |
| C | `OpenOptionsExt::open(directory)` then `sync_all()` if open succeeds | `GENERIC_WRITE`, `FILE_FLAG_BACKUP_SEMANTICS` |
| D | Create one small file, write and `sync_all()`, capture handle identity, same-directory `MoveFileExW`, read-only reopen, compare identity and bytes/hash | One file, one rename, `MOVEFILE_WRITE_THROUGH`; no replace/copy/cross-volume flags |

Each attempted stage gets its own JSON observation. A Rust I/O error records
`error_kind`, `rust_raw_os_error`, and the corresponding Windows code surfaced
by Rust as `win32_error_code`. `MoveFileExW` records its immediate
`GetLastError()` value separately; its `rust_raw_os_error` is null because that
call bypasses Rust's I/O wrapper. Paths, user names, host names, command output,
and arbitrary error messages are not emitted. The only path labels are fixed
probe-owned basenames and `RUNNER_TEMP/<unique-direct-child>`.

`FILE_OPERATION_OBSERVED` and `API_RETURN_OBSERVED` mean only that a particular
API call returned and was recorded. They never mean durable namespace state,
power-loss survival, multi-file atomicity, or product acceptance. A Windows
directory sync error such as OS error 5 remains an observed error. Missing
filesystem or identity capability is recorded as unavailable; safety proof
failure is a rejection; failed cleanup is `CLEANUP_INCOMPLETE` and retains the
original operation observations.

## Execution gates and limits

The Windows executable returns `NOT_EXECUTED` without creating a sandbox
unless `T10_B04_PROBE_ENABLE` is exactly `1`. Non-Windows builds always return
`NOT_EXECUTED`, even when that variable is present. An enabled invocation also
requires exact `T10_B04_EXPECTED_HEAD` and `T10_B04_EXPECTED_TREE` values from a
separately Owner-approved source revision. Before creating a directory, it
checks that the current Git HEAD/tree match those values, the repository is
clean, the working directory is the repository root, `RUNNER_TEMP` is an
absolute local path with no reparse-point component, the root is outside
protected user/system/workspace roots, the current token is not elevated, the
runner owns `RUNNER_TEMP`, the filesystem is verified as NTFS, and at least
1 MiB is available.

The source check cannot self-attest which source produced an already-built
binary. A future approved runner must build the binary fresh from this exact
clean checkout with the existing locked manifest, then run that newly produced
executable in the same job; it must not launch a retained executable artifact.
Use `cargo build --release --locked --manifest-path
tests/issue381_b4_p1a_f1/Cargo.toml --bin t10_b04_ntfs_probe`, with a fresh
runner-local target directory. The runner should capture the executable
SHA-256 with its evidence.

After those checks it creates one unique direct child of `RUNNER_TEMP`, checks
the new directory owner and reparse status, and uses only two fixed child names
for one regular file. The payload is under 1 MiB, at most 10 disposable files
are allowed by the policy guard, and the probe performs no retry. Cleanup
removes only those exact regular files and the empty sandbox directory; it
never recursively traverses a path. Existing paths are not overwritten. No
admin/UAC request, volume handle, system path, user data, VHD/VHDX/DMG, quota,
format, mount, or disk-full injection is used.

The code checks the 30-second monotonic budget before each probe operation and
skips later operations when it is exhausted. Synchronous operating-system
calls cannot be preempted by the same thread. Any future execution therefore
also requires the separately approved limited runner mechanism to enforce a
30-second process timeout and report timeout cleanup accurately. That runner,
its arguments, exact source identity, sandbox, and execution approval do not
exist in this source-only stage.

## Durability boundary

`FlushFileBuffers` requires a write-capable handle, while opening a directory
with backup semantics does not document POSIX parent-directory `fsync`
equivalence. `MoveFileExW(MOVEFILE_WRITE_THROUGH)` has limited documented
semantics; the observed return does not prove SQLite, a separate manifest, and
a directory entry are one durable transaction. No successful API response is
promoted to a power-loss claim. Real ENOSPC and physical power loss remain
deferred and untested.

## API references

- [Microsoft: Obtaining a Handle to a Directory](https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory)
- [Microsoft: CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
- [Microsoft: FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
- [Microsoft: `FILE_ID_INFO`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info)
- [Microsoft: `GetFileInformationByHandleEx`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandleex)
- [Microsoft: MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
- [Rust: `OpenOptionsExt`](https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html)
- [Rust: `File::sync_all`](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all)

The rename and file-identity calls are narrow, reviewed FFI calls. Rust's
portable rename contract does not provide the required no-replace plus
`MOVEFILE_WRITE_THROUGH` flags, and the repository-pinned Rust version does not
stabilize the Windows metadata methods that expose these identity fields. The
`FILE_ID_INFO` buffer has the documented 64-bit volume serial plus 128-bit file
ID layout and is queried through the live file handle. Rename UTF-16 arguments
are NUL-checked; the destination is checked absent;
`MOVEFILE_REPLACE_EXISTING`, `MOVEFILE_COPY_ALLOWED`, and delayed-reboot flags
are not used; and `GetLastError()` is read immediately on either failed API.
Both calls remain test-only and are not product durability adapters.

## Source-only review record

- Requested branch: `test/issue-411-t10-b04-ntfs-probe`.
- Verified starting HEAD: `b9c44a0a05fc8dc34744563b43d88eeab7509b35`.
- Verified starting Tree: `2becfa34995d1872df74115adc4c7c941b353334`.
- Starting branch lineage: the existing archive-only T10-A research branch;
  its inherited workflow is unchanged and is restricted to another branch.
- Allowed additions: this binary and files beneath this `b04-ntfs-probe/`
  directory only.
- No Windows probe, complete CI, workflow, Cargo manifest/lockfile, prior T10-A
  source/evidence, production code, or other repository path was changed.
- Rust formatting/type-check and pure guard-test outcomes are recorded below.
- Source and report SHA-256 values are in `SHA256SUMS`; final HEAD/Tree and
  exact branch diff are delivered in the task handoff.

### Checks run in the source-only stage

- `rustfmt --edition 2024 --check tests/issue381_b4_p1a_f1/src/bin/t10_b04_ntfs_probe.rs` — exit 0.
- `cargo test --locked --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml --bin t10_b04_ntfs_probe` — exit 0; 8 pure in-memory policy tests passed. No filesystem probe is called by these tests.
- `cargo check --locked --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml --bin t10_b04_ntfs_probe` on Linux — exit 0.
- Windows-only source type-check — exit 0 for `x86_64-pc-windows-gnu`, using a temporary `/tmp` manifest with only the same pinned direct `serde_json` and `sha2` dependencies. The generated Windows program was not run.
- Full-manifest Windows cross-check was attempted but could not start its bundled SQLite C build because `x86_64-w64-mingw32-gcc` is not installed. The isolated Windows source type-check above covers this new binary's Windows-specific Rust code without altering the repository dependency graph.
- No Windows native probe, no GitHub Runner job, no full CI, no real ENOSPC test, and no physical power-loss test ran.

Stop state: `T10_B04_NTFS_PROBE_SOURCE_READY_FOR_OWNER_REVIEW`.
