# PR #489 round 9 — Windows v3→v4 install upgrade acceptance plan

**Qualification status: UNVERIFIED (2026-10-29).** The active executor is Linux;
no disposable Windows VM or verified v3 installer is attached to this task.
This plan does not block the independent watcher code fix. No host service or
user volume has been touched.

## Scope and exact versions

The upgrade under test is an installed Windows desktop and its installed
`ZenCanvasGlobalIndex` service, moved from IPC v3 to IPC v4 by a real NSIS
installer. The PR base `df940915084b810101a8a3eb78796cdf8730d55b` contains
`IPC_PROTOCOL_VERSION = 3` and pipe `\\.\pipe\ZenCanvas.GlobalIndex.v3`;
the v4 candidate contains protocol 4 and pipe
`\\.\pipe\ZenCanvas.GlobalIndex.v4`. Capture the actual final candidate
commit/tree after code review and use those exact identities throughout the
run. The service name is `ZenCanvasGlobalIndex`.

The October 10 Release Build artifact at run
[38081362340](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38081362340)
is not a suitable v3 package: its source commit predates the Global Index
service code. The ordinary PR `Package NSIS` job verifies a build but does not
retain the installer executable. Neither artifact may be promoted to an
installed v3→v4 result.

## Obtain and verify the v3 installer

1. First look for an unexpired artifact from the official
   `Build Release Installers` workflow whose `head_sha` is exactly the chosen
   v3 commit. Inspect the commit source and require both the v3 protocol
   constant and v3 pipe name. Download its Windows NSIS executable and matching
   `installers-windows.sha256` from that same workflow run. Verify the checksum
   before placing it inside the test VM.
2. If no matching retained artifact exists, build the old installer inside the
   disposable Windows VM from the official repository at the exact v3 source
   commit, with the checked-in Node/Rust lockfiles and `rust-toolchain.toml`.
   Use the repository's Windows package command (`npm ci`, then `npm run
   build`). Record the checked-out SHA/tree, dependency lockfile identities,
   toolchain versions, installer file version and SHA-256. This produces an
   actual NSIS package from the v3 application/service source; it is not a
   source contract test or an emulated upgrade.
3. Do not use a package selected only by display version. Reject any installer
   whose source identity, embedded service behavior, or checksum cannot be
   tied to the v3 commit. If neither the exact official artifact nor a
   reproducible exact-commit build is available, stop with `UNVERIFIED`.

## Disposable VM and disk boundary

1. Create a fresh Windows 11 x64 VM with a disposable OS VHDX and one separate,
   disposable fixed NTFS VHDX for Global Index test data. Use a local test
   account and Hyper-V checkpoint. Do not attach physical disks, host drive
   pass-through, SMB/OneDrive/Google Drive locations, or host user profiles.
   Disable shared folders and clipboard file transfer for the run.
2. Assign the test VHDX a drive letter (for example `D:`), place a unique
   owner marker at `D:\ZC-Upgrade-Fixture\`, and record its virtual disk ID,
   volume serial, filesystem, drive kind and mount path. Verify all visible
   volumes belong to the VM before launching either installer. Only the
   fixture volume may be enabled or authorized; keep the VM's OS volume
   disabled and unauthorized in Global Index.
3. Keep source checkouts, both installers, logs, database backups and fixture
   files on the VM's disposable disks. No step may address a host-mounted path.
   If an unexpected disk, share or cloud volume appears, stop before indexing.

## Install v3 and preserve a real database/index state

1. Install the verified v3 NSIS package from inside the VM. Record the install
   path, application file version/hash, `sc qc ZenCanvasGlobalIndex`, service
   image path, start type and SCM state. Confirm the service executable is the
   installed product executable running with `--index-service`.
2. Launch that installed v3 application. In its UI, authorize only the
   disposable NTFS fixture volume. Create a unique sentinel file under
   `D:\ZC-Upgrade-Fixture\`, allow the v3 service to index it, and confirm the
   sentinel is searchable and the source is `ready`. Confirm no other source
   became enabled or authorized.
3. Record the exact database location returned by Tauri's
   `app_data_dir()` (the database filename is `zen-canvas.sqlite3`), without
   assuming a profile path. Stop the application and service cleanly, run
   SQLite `PRAGMA integrity_check`, checkpoint the WAL, and create a byte-for-
   byte or SQLite-backup copy plus SHA-256 on the disposable VM disk. Record
   the fixture source row, authorization, operation/checkpoint fields, entry
   identity for the sentinel, and an application setting seeded for the test.
   Retain the live database in place for the upgrade.

## Run the v4 NSIS upgrade and verify the installed handshake

1. Check out the reviewed v4 candidate by its exact SHA/tree inside the VM and
   build the Windows NSIS installer with `npm ci` and `npm run build`. Capture
   the resulting installer SHA-256 and embedded version. Keep it outside the
   installed product directory, on a disposable VM disk.
2. Run that installer elevated over the installed v3 product. Capture the
   installer exit code/log. Verify the old `ZenCanvasGlobalIndex` service
   reaches `STOPPED` before its image is replaced, and that the installer
   converges the same service name to the new installed executable path. Check
   the new service's file hash/version against the v4 installer payload,
   `ImagePath` (`--index-service`), SCM state, and running process image. There
   must be one product service, with no surviving v3 process or v3 service
   image.
3. Launch the newly installed desktop executable and trigger its normal Global
   Index incremental path through its built-in service client. Capture the
   installed app and service image paths, a successful connection to
   `\\.\pipe\ZenCanvas.GlobalIndex.v4`, the successful operation and absence
   of a protocol/fallback error. The exact candidate source must show request
   protocol 4 and `exchange` must reject a response with a different protocol
   or request ID. If a raw request/response record is required, capture the
   serialized pipe buffers from the installed client/service processes with a
   debugger attached only inside the VM; record protocol 4 and the matching
   request ID in both frames. Do not substitute a standalone client that
   bypasses the installed-executable caller check. If the exact wire fields
   cannot be observed, keep the raw-handshake subclaim `UNVERIFIED`.
4. Verify the database remains at the same app-data path and passes
   `integrity_check`. Compare the saved setting, source identity and prior
   sentinel entry with the backup; allow only expected schema/application
   metadata changes. Confirm no data directory or historical entry was deleted.
5. Check authorization before any post-upgrade indexing. If the v4 contract
   requires fresh consent, the source must remain disabled/unapproved until an
   explicit confirmation in the v4 UI; then authorize only the test VHDX and
   confirm the prior sentinel remains searchable and a new fixture sentinel is
   picked up by an incremental USN operation. No automatic consent migration
   or full-volume baseline is accepted as a substitute.

## Inject interruption and verify recovery

Run each case from a fresh VM checkpoint and retain the failed attempt's
installer log, service observations and database integrity result:

1. Cancel before product mutation. The installer must report cancellation;
   the v3 application/service image, service state and live test database must
   remain usable and unchanged.
2. Inject a deterministic file-mutation failure after the installer has
   stopped the old service, using a test-only ACL or locked file on the VM's
   install directory. Do not modify the installer binary or hooks. Verify the
   installer reports failure and follows its existing rollback path: product
   files/service are either restored to a coherent v3 state or left in the
   documented recoverable state, and the user database remains intact.
3. In a separate checkpoint, terminate the installer process after observing
   the old service stopped and before the new service reaches `RUNNING`. Verify
   the VM can be restarted and the supported installer retry/repair path
   reaches one coherent v4 service without deleting the database. Record this
   as crash-recovery evidence separately from NSIS rollback; a forced process
   kill is not a normal installer exit.
4. If any interruption leaves an unowned service, silently authorizes the test
   volume, loses the database, or cannot reach a coherent repair state, mark
   that case FAIL and stop. Revert only the disposable VM checkpoint and test
   VHDX; never clean or repair a host installation.

## Acceptance record

| Gate | Required evidence | Current status |
| --- | --- | --- |
| Trusted v3 NSIS source/artifact | Exact source SHA/tree, v3 pipe/protocol, artifact provenance and SHA-256 | **UNVERIFIED** — no qualifying installer attached |
| Isolated Windows VM and disposable disks | VM/checkpoint identity, virtual disk IDs, no host/shared/cloud mounts | **UNVERIFIED** — no Windows VM attached |
| Installed v3 app + service | NSIS result, installed image path, service state, successful v3 client exchange | **UNVERIFIED** |
| Preserved v3 data/index | DB backup/hash, integrity check, source/entry/settings comparison | **UNVERIFIED** |
| Installed v4 NSIS replacement | Exact candidate SHA/tree, installer hash, old stop/new image/new SCM state | **UNVERIFIED** |
| Real IPC v4 handshake | Installed app's v4 pipe request/response, protocol 4, matching request ID | **UNVERIFIED** |
| Reauthorization and incremental resume | Explicit UI consent, one test source, preserved sentinel, incremental checkpoint | **UNVERIFIED** |
| Installer error and interruption recovery | Cancel, injected mutation failure, interrupted-process repair records | **UNVERIFIED** |

Only completion of these installed-package gates is a v3→v4 upgrade PASS.
Source contracts, a simulated upgrade, a fresh v4 install, or CI's standalone
Service qualification remain separate evidence and cannot satisfy this matrix.
