# #380 K2-G1 native authority feasibility

**Terminal: K2_G1_PLATFORM_AUTHORITY_BLOCKED. Production NO-GO.** This is test-only platform research, not an implementation
selection or certification of a shipped Zen Canvas upgrade.

## Authority and exact inputs

[Owner G1 authorization](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6139836557),
[product safety boundary](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6139718764)
and [accepted K2-T0](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6139390521)
were reread along with master, Issue #380 and Draft PR #390. Their full bodies
are retained in [remote facts](../../tests/issue380_k2_g1/evidence/remote-facts.json).

Base master: `3275ac7c72df8ae4139a30f16c7f649971324e97`.
Base tree: `009576241cd4e8e2ba12cc644a368314f599faa6`.
Independent branch: `test/issue-380-k2-g1-platform-authority`.
The final HEAD/tree/diff and exact-head Actions URLs belong in the Draft PR
closeout; embedding this report's own resulting tree would be circular.

## Evidence grades and safety

The Python processes are **TEST_MODEL** actors using real native file APIs and
real SQLite WAL, not modified old applications, shipped binaries or reconstructed
Zen Canvas builds. Actual native capability and actual old-product qualification
are different claims. Linux smoke results never count as Windows/macOS proof.
Each raw result records checked-out HEAD/tree, every Python input SHA-256,
OS/image/architecture/Python/SQLite, parent/child PIDs, command/input/output,
handles/descriptors, physical main/WAL/SHM identities and native/SQLite errors.

All databases, aliases, copied test actors and processes are inside one unique
owned temporary directory. Cleanup removes that directory after evidence capture.
No app installation, SCM, login item, registry or OS security policy is touched.
Snapshot transform is a separate-generation toy table, not a production migration.
Owner-native acceptance and production migration are **NOT_RUN / NOT_AUTHORIZED**.
Cross-user/session, elevated broker, real installer/service, power loss and product
WAL-VFS behavior remain unverified. An 8-second actor timeout is recorded as
`BOUNDED_ACTOR_TIMEOUT`, never converted into an invented SQLite error code.

## G1-01 old binary provenance

[Official provenance](../../tests/issue380_k2_g1/evidence/old-binary-provenance.json)
records PR #322's installer `Zen Canvas_0.1.40_x64-setup.exe`, 9,811,183 bytes,
SHA-256 `02C9ECE0B6799947D8828E7BB0D0B9F52F95166DF3030FC2C503F1688EAD21F9`,
source candidate `843693ea7e3612a662a5331daaedef557d40fee1`.
The official Releases collection is empty. The two named PR #322 candidate CI
artifact collections contain source/integration/service evidence, not that installer.
The paginated installer/NSIS-named artifact search finds only later synthetic NSIS
research; these are not matching shipped bytes. Owner's external retained drive
has not been accessed. This bounded search does not prove that no private copy
exists elsewhere. No matching installer was obtained or executed: **NOT_RUN**.
No reconstructed product build was made. No product compatibility PASS is claimed.

## Native experiment contract (G1-02 through G1-07)

The normal-PR-only [workflow](../../.github/workflows/issue380-k2-g1-native.yml)
uses Windows and macOS Hosted runners, read-only repository permission, immutable
checkout/action pins, G1-only path filters and a ten-minute job timeout. It has
no dispatch, secrets, service registration, package or publication steps. Native
raw JSON uploads even on failure; normal existing CI success is not this proof.

| Gate | Actual bounded operation and interpretation |
| --- | --- |
| G1-02 | Windows `CreateFileW` on main/WAL/SHM: read handle with share 0, share READ, and share READ+WRITE negative control; acquire while existing WAL writer holds connections, attempt retained commit, terminate controlled writer, reacquire, then late independent writes. Every handle/error and commit result retained. |
| G1-03 | While guard process holds acquired handles, separate read-only SQLite connection runs bounded Online Backup; integrity and every known acknowledged pre-snapshot correction must match. Prepare separate generation and first acknowledged new write before releasing guard. Blocking the backup fails the candidate. Partial acquisition is never full exclusion. |
| G1-04 | Physical identities use Windows volume serial/file index/final handle path or POSIX device/inode/fstat. Main/WAL/SHM hardlinks, file symlinks and directory symlinks/Windows junctions are tested. All sidecars are explicitly aliased; this does not assume a main-file alias selects canonical WAL. Unavailable alias APIs retain errors and NOT_RUN. |
| G1-05 | Controlled old writer retained connection, abnormal exit, clean close/exit and restart, late startup, copied actor at another executable path; guard abnormal exit followed by old relaunch. No SCM/single-instance simulation substitutes for authority. |
| G1-06 | macOS separate guard holds `flock` or whole-file `fcntl.lockf` on all three files. Existing/late/alias writer and held-lock backup results are measured. Advisory flock is not assumed mandatory; fcntl conflicts may also block required SQLite reads/backup. |
| G1-07 | Confirmed old corrections compared to WAL backup, test-generation preparation, first new correction, abnormal process exits and fresh SQLite integrity/recovery. Old relaunch after guard death plus acknowledged new correction is two-authority NO-GO, never silent rollback. A real BEGIN IMMEDIATE control separately measures SQLITE_BUSY, concurrent WAL backup and writing after transaction release. |

`bounded_held_window_candidate` only means the observed acquired-handle window
blocked tested late/alias actors, allowed snapshot, included confirmed corrections
and allowed the new fixture's first write. It is **not** a lifetime/product GO:
existing writer acquisition, all product routes, guard failure, alias availability,
released binary provenance and durable activation still require independent proof.
Snapshot consistency alone cannot establish a last-possible-old-commit boundary.

## G1-08 inherited cross-domain constraints

Only accepted K1 and K2-T0 reports and their frozen dependency table are reused;
no K1/T0 suites or #381 inventory/consumer audit are rerun.

| Domain | Frozen input and required constraint |
| --- | --- |
| Installation generation | Owner contract: process installation/activation does not select or fence physical DB writers. Guard disappearance must block migration/activation, not silently select another DB. |
| Physical DB/write generation | K1 `bbdd3db254582ae622236237b87adb73b097bc42` / tree `6341a9fdf547fe85331121a412fcb35622415bda`; K2-T0 `e1e993893a87aa916c67f7cff41a102b269fcf88` / tree `c432f0738f5c846fc98fd31c0e76305c2ee6a586`. Mandatory preopen/per-write admission cannot retroactively constrain old released routes. |
| C2 Global Index publication | Track B `9e144da261b97a421b6a40f61280e149887f6204`: complete sealed generation, participating readers/writers, short pointer CAS and compatible DB epoch. No new semantic qualification. |
| #381 Root/Source publication | S8R `a48f8849b1ba1baadcf84e217d505e2298cf35a1`: source/path/physical identity epochs, seal and effective visibility must share compatible read truth. Frozen, no new audit or integration. |
| User state and recovery | Preserve latest confirmed corrections/history/consent/IDs and associations. Two confirmed generations require RECOVERY_REQUIRED/forward assessment, not rollback to old data. Miniature fixture does not certify all product relations or FTS/rank/cursor/GC. |

These three generation domains remain distinct. Schema v38 is unallocated;
#381 production, NSIS/U2 release and #380 automatic migration remain NO-GO.

## First genuine Hosted execution

[Native run 38359437737](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38359437737)
completed SUCCESS, attempt 1, both native jobs. Checked-out input HEAD
`e6afbf6f8b5311cfb5cfb5f0a327d5fbe2ccf788`, tree
`799a1692a2d8f0a6f48798071017ad74d97bd201`.
[Manifest](../../tests/issue380_k2_g1/evidence/hosted-first-manifest.json)
retains original Artifact IDs/digests and preserved JSON SHA-256. These are
historical inputs, not evidence for subsequently added probe commands.

| Platform | Actual image / interpreter / SQLite | Execution |
| --- | --- | --- |
| Windows | Server 2025 build 26100 AMD64; win25-vs2026 `20260925.250.1`; Python 3.12.10; SQLite 3.49.1 | COMPLETE; 294 events / 33 child launches |
| macOS | macOS 26.6.2 arm64; macos26 `20260907.0351.1`; Python 3.14.7; SQLite 3.53.4 | COMPLETE; 183 events / 21 child launches |

| Candidate | Existing writer / late writer | Snapshot while held | Outcome |
| --- | --- | --- | --- |
| Windows share 0 | Existing handles cause Win32 ERROR_SHARING_VIOLATION 32; retained old commit succeeds. After exit, gate acquires and late/alias opens fail SQLITE_CANTOPEN 14. | Backup reader also fails SQLITE_CANTOPEN 14. | NO-GO: backup blocked. |
| Windows share READ | Existing handles cause error 32 and retained old commit succeeds. After exit, all three handles acquire; late and all tested alias writes fail SQLITE_READONLY 8. | Online Backup/integrity succeeds, includes both acknowledged old corrections; fixture generation 2 and first new write succeed while guard holds. | Bounded held-window capability observed. Guard death allows copied old relaunch/confirmed correction while new correction survives recovery: lifetime/product NO-GO. |
| Windows share READ+WRITE | Gate acquires alongside writer; retained, late and all alias commits succeed. | Backup/integrity succeeds and includes six observed corrections. | NO-GO: no write exclusion. |
| macOS flock, all three files | Locks acquire. Retained write returns no SQLite result in 8 seconds (bounded timeout); late/alias SQLite opens return SQLITE_BUSY 5. | Read-only backup source also fails SQLITE_BUSY 5. | NO-GO: backup blocked; do not infer OS permission revocation from one SQLite implementation. |
| macOS whole-file fcntl | Same observed timeout/BUSY/read-only-backup blockage; locks acquire. | Backup source fails SQLITE_BUSY 5. | NO-GO: backup blocked. |

All tested hardlink/symlink aliases succeed in construction on both systems;
Windows junction creation succeeds. Raw JSON gives main/WAL/SHM identity equality,
not just matching paths. For example Windows READ gate main volume serial
`375717793`, file index `281474976712782`; macOS flock main device `16777226`,
inode `2960416`. Sidecar identities, every PID, descriptor and API result remain in
[Windows raw evidence](../../tests/issue380_k2_g1/evidence/hosted-Windows.json) and
[macOS raw evidence](../../tests/issue380_k2_g1/evidence/hosted-macOS.json).

The separate BEGIN IMMEDIATE control on both systems permits WAL backup, returns
SQLITE_BUSY 5 for the held-window writer and permits old writing after release.
It is a transaction capability, not installation or generation authority.
No new generation was opened in candidates whose held barrier blocks backup.
This is fail-closed experimental behavior, not a production gate implementation.

## Qualified native source and final matrix

[Native run 38360029127](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38360029127)
completed SUCCESS, attempt 1, both jobs, actual input HEAD
`581aef465a2dc35ec4d5f3d87d89f1c2c8646365`, tree
`96ae4765812bfe9925f86119d57c0168def97bb3`. Platform/image/Python/SQLite versions
match the first-run table. Windows now records 329 events / 38 child launches;
macOS 211 events / 25 launches. Revised code is frozen; final evidence/report-only
HEAD's natural native run is linked separately in the PR closeout.

[Qualified manifest](../../tests/issue380_k2_g1/evidence/hosted-qualified-manifest.json),
[Windows raw input/output](../../tests/issue380_k2_g1/evidence/hosted-qualified-Windows.json),
[macOS raw input/output](../../tests/issue380_k2_g1/evidence/hosted-qualified-macOS.json)
retain exact original Artifact bytes, hashes and every experiment exchange. Windows
checkout inputs have CRLF and macOS/Git blobs LF: all raw source SHA-256 values were
verified against precisely these bytes, without treating distinct hashes as equal.
Default SQLite VFS is `unix` on this macOS interpreter. Windows extension does not
export `sqlite3_vfs_find`; the missing symbol is explicitly recorded, not guessed.
Both compile-option arrays are preserved. Neither interpreter's SQLite is claimed
to be the product's bundled rusqlite SQLite or a shipped binary.

| Gate | Genuine Windows result | Genuine macOS result | Qualification |
| --- | --- | --- | --- |
| G1-01 | No matching installer bytes obtained; no installer execution. | No authentic old macOS product bytes obtained. | Shipped old Zen Canvas **NOT_RUN**; reconstruction **NOT_RUN**. |
| G1-02 | Existing READ/exclusive gates fail error 32 and retained commits succeed; READ+WRITE gate allows retained and late writes. | Existing locks acquired but SQLite writes time out; timeout is not an acknowledged commit or SQLite error code. | Real OS/SQLite evidence; retained writer cannot be retroactively assumed revoked. |
| G1-03 | READ-only sharing blocks tested old/alias writes with error 8 while WAL backup, integrity, last-confirmed preservation and new first write succeed; share 0 blocks backup error 14. | Both held native lock types block required backup source with error 5. | Windows READ has bounded fixture GO only; complete final-snapshot/lifetime authority **not qualified**. |
| G1-04 | Main/WAL/SHM file IDs, hardlinks, symlinks and junction reparse aliases captured; alias write outcomes follow candidate sharing mode. | Device/inode/fd identity and hardlink/file/directory symlinks captured. | Actual physical identity; all constructed alias types ran. Arbitrary reparse types, malicious namespace replacement, cross-volume and all product paths **UNVERIFIED**. |
| G1-05 | Retained writer, clean stop/restart, killed writer, late/copy launch and killed guard are actual separate processes. All guards' death allows old relaunch commits. | Same controlled lifecycle coverage; no permission withdrawal from ordinary old copies. | Actual controlled process evidence. Real SCM, Desktop/Direct/Worker, single-instance and cross-session **NOT_RUN**. |
| G1-06 | Raw O_RDWR probe fails errno 13 under share 0/READ, succeeds under READ+WRITE. | Under both flock/fcntl, raw O_RDWR writes identical 16 bytes and fsync succeeds while locks remain held. Actual SQLite attempts may block, but OS file write access remains. | Advisory locks are not general write revocation. No claims about untested ACL/broker/sandbox mechanisms. |
| G1-07 | READ window preserves old acknowledged data, new first write and crash recovery, then guard death yields old/new acknowledged divergence. Independent SQL gate also forks after release. | SQL gate permits WAL snapshot; separate-generation first correction survives abnormal exit/reopen with integrity ok; old correction after release yields two acknowledged generations. | **NO-GO / RECOVERY_REQUIRED**; no silent old rollback. Snapshot-blocked native candidates never open new generation. |
| G1-08 | Accepted K1/T0/C2/#381 inputs and constraints reused above. | Same frozen contract. | STATIC_DEPENDENCY_ONLY; no broad audit, repeat old suite or production integration. |

The identical-byte descriptor probe does not modify logical SQLite contents and
is **not** an acknowledged user correction. It demonstrates actual file-write
capability independent of SQLite's cooperative locking/VFS. macOS lock behavior
here is real, but does not generalize to mandatory enforcement over arbitrary
same-user old programs. The retained-connection write timeout remains preserved
as uncertain outcome, never relabeled successful cancellation or SQLITE_BUSY.

Both SQL transaction controls keep BEGIN IMMEDIATE held while Online Backup
preserves old-final and the clean-restart confirmed correction; new generation 2
accepts `new-first`, then its process is killed and reopened with integrity `ok`.
After old SQL gate release, `after-sqlite-release` is confirmed only in old DB.
This is the required two-authority counterexample on both native platforms.
It does not qualify crash-durable activation or power-loss recovery.

## CI, unverified boundaries and decision

The G1 native executions are genuine Windows/macOS Hosted evidence, not ordinary
CI proxy results. Normal CI runs 38359437781 and 38360029169 automatically selected
performance work because a workflow was added. They were cancelled to limit this
research scope; some source/routing/frontend steps had completed, performance had
started, and neither CI run is claimed PASS. Raw job dispositions are retained in
the qualified manifest. The final ordinary CI will likewise be cancelled if it
selects those broad tasks. No manual Hosted, Full Validation or test rerun was
requested; native runs came only from ordinary PR opened/synchronize events.

**Test-only execution GO; production NO-GO.** Windows READ sharing establishes a
useful held-window capability for this controlled SQLite fixture. It does not
acquire over existing writer handles, survive guard death or certify all released
product writers. Other tested native gates either allow writes or prevent the
required final snapshot. No persistent end-to-end upgrade exclusion has been
proved. Do not infer that every possible native mechanism is impossible.

Unverified: authentic released binaries and their actual routes/VFS; Owner-native
acceptance; real service/installer/login/session behavior; complete namespace/ACL
and privileged brokers; power loss, durable activation and recovery selection;
all production state preservation/FTS/Root+Source semantics. No production gate,
migration, activation ledger, Schema v38, #381 integration or NSIS change is made.

**Stop: K2_G1_PLATFORM_AUTHORITY_BLOCKED.** Draft PR only for Owner review. #380,
#381 production and NSIS/U2 release remain NO-GO; prior Draft holds unchanged.
No merge, issue closure, release or next research/development phase is started.
