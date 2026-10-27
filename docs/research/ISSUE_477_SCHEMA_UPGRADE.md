# Schema 42 → 43 visible startup upgrade

Owner contract: [Issue #477](https://github.com/ArdenZC/Zen-Canvas/issues/477).
Base master a47672182ea2d92784e226496b6f2d2be8cafe2c,
tree 765e9b3cd03b391def839e035948baeb89059d78 (Schema 43).
Source Facts/Snapshot/FTS work in #474 is an independent branch and delivery.

## Result and scope

Existing databases requiring upgrade enter a native Windows Task Dialog or
macOS AppKit modal loop before opening the database on a worker. The dialog is
painted before the worker starts; it stays responsive while SQLite runs. Cancel
asks SQLite to interrupt and keeps the dialog alive until the transaction exits.
Failures offer explicit Retry/Exit, the actual database location, storage/busy
guidance and instructions to exit all app processes before preserving any
existing database/WAL/SHM together for support. No automatic retry loop.
Background/autostart defers a pending upgrade to a foreground launch and starts
no indexer/watcher. A real Tauri background subprocess checks clean exit code0,
no migration/native dialog and retained Schema42. Acknowledged Exit/defer no
longer falls through the former Tauri build expect/panic. Second-instance
activation waits for database readiness.
No renderer, new IPC command, installer or durable recovery authority is added.

A cancellation racing a successful COMMIT may retain the successful upgrade.
Process termination cannot guarantee immediate rollback or progress UI; SQLite
recovers on restart and the app re-reads user_version under BEGIN IMMEDIATE.
No index removal, schema downgrade or INDEXED BY fallback is a recovery action.

## Resource policy

Migration uses FILE temp storage, an 8 MiB main-page cache, disabled mmap and
zero auxiliary SQLite sort threads; normal connection settings are restored
before the pooled connection is released. This bounds the configured cache and
avoids MEMORY external sorts; it is not a hard bound on total process RSS.

Before any Schema 42 migration DDL, under its existing write transaction, a
cancellable read sums actual UTF-8 name/ID/volume key bytes and records SQLite
page size/count/freelist. Per-row allowances cover integer/record/rowid bytes.
The storage reservation uses three payload copies for index allocation, two
index generations for WAL/checkpoint overlap, WAL frame headers, two generations
of the largest external sort and a 64 MiB reserve on each storage role. Free
pages are not credited. SQLite's VFS supplies the actual temporary filename;
space is checked on that volume and on the database volume, combined when they
are the same filesystem. No universal “database size × 2” guarantee is made.
A 128 MiB available-memory working reserve is checked before the key scan;
low memory asks users to close other apps and retry. This is a policy reserve,
not a prediction or hard limit for process RSS.
Other processes can consume free space after preflight: SQLITE_FULL still must
roll back and reach visible recovery. Native allocator and OS caches remain
outside these SQLite settings.

## Validation protocol and pending evidence

The existing Rust benchmark generator supplies names, paths and metadata.
One full-index/FTS/trigger fixture uses production stable_entry_id (67-byte
ge_ BLAKE3 IDs), actual 67-byte gv_ IDs, four sources including one disabled,
and stale rows. Controlled copies compare 100k/200k and short IDs; population,
copy/reshape and resource inspection are outside migration-open timing (the
production open includes its own preflight). No million-row query matrix is
repeated. The timed path is Database::open, including the actual migration and
pool configuration. All 12 complete snapshots, source health/revision and
index status compare across the upgrade; INDEXED BY plans are retained.

Process RSS and main/WAL/SHM sizes/free space are sampled with a 20 ms sleep
between polls; OS query overhead increases the actual interval. These are
observed process-wide samples, not a proven OS peak or universal upper bound.
The full per-index dbstat allocation before/after is retained in JSONL.

File-backed tests cover SQLITE_FULL, cancellation after DROP, rollback/retry,
pre-COMMIT process kill (without graceful unwinding), integrity_check and
restart. Existing Schema 43/future-version tests remain applicable.

Hosted Windows/macOS compiled and executed the actual native modal widgets,
including visibility before worker start, continued timer events, cancellation
and native Retry. The UI probe uses a deliberately delayed small fixture and
synthetic native button messages, not a simulated webview. Human keyboard,
screen-reader, contrast, packaged first-upgrade/tray behavior and Owner Windows
acceptance remain unverified. No release qualification follows from automation.


## Measured installed-schema cost

The bounded 100k/200k pair is run
[40372208959](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40372208959),
source 90034453bd3c9ef992908513593a210461e09cec / tree
66ee5af2d48c34de814ccf0d80688e207a630b21.
One 200k fixture per native platform supplies the controlled copies. Its
population costs 73.399s Windows / 79.959s macOS, outside upgrade timing.

| Platform / rows | Production ID open ms | Short ID open ms | Production sampled RSS MiB |
| --- | ---: | ---: | ---: |
| Windows x64 / 100k | 1236.684 | 805.418 | 35.53 |
| macOS arm64 / 100k | 1211.338 | 766.585 | 70.97 |
| Windows x64 / 200k | 2934.445 | 1618.695 | 30.70 |
| macOS arm64 / 200k | 2682.515 | 1783.055 | 145.80 |

These are single observed opens per configuration, not p50/p95 distributions or
upper bounds. The short-ID copies at these scales retain fragmentation/free
pages after rewriting keys; their negative net allocation delta is compaction
of an old fragmented Name index, not negative index cost. Do not treat this
contrast as a pure key-width ratio or as the original fresh short-ID fixture.
At 200k production shape: Name 33.37 MiB + Source 27.58 MiB, net +37.65 MiB;
observed WAL 61.32 MiB. macOS short-copy RSS reaches 231.44 MiB; process-wide
allocator/cache history differs by stage, so 8 MiB configured SQLite cache is
not a total memory cap. Volumes' free-space samples can include other runner
activity and OS allocation accounting.

### Single 1M controlled run

[40421058751](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40421058751)
completed SUCCESS on Windows x64, source
0cc1c30a5ed856032293a14ee2872b096a0d451e, tree
7c3aca0a6cff7ecf08985e46d58ddbf0a2955fdb.
One 1M fixture is generated (603.077s outside timing), with controlled long/short
copies; both copies are VACUUM-compacted before timing to control fragmentation.
Copy/reshape/compact/checkpoint costs are separately recorded (22.814s long,
103.192s short). No second million-row fixture or full search latency matrix is
run for #477. All 12 complete snapshots compare across each actual upgrade,
including BM25, ordering, source health/revision and index status.

| 1M configuration | Actual open ms | Name / Source MiB | Net index MiB | Sampled process RSS MiB |
| --- | ---: | ---: | ---: | ---: |
| Production 67-byte ge_/gv_ IDs | 10906.489 | 166.99 / 138.53 | 202.67 | 32.08 |
| Short-ID controlled copy | 6905.502 | 65.70 / 36.43 | 46.33 | 30.92 |

Production main file 1551.21→1753.89 MiB; observed WAL 307.33 MiB and SHM
0.625 MiB. The largest observed combined database/temp-volume free-space decline
is about 539.31 MiB; this is a sampled allocation observation, not an exclusive
measurement of SQLite temporary files (Unix temp files can be unlinked while
open). Preflight reserves 2329.77 MiB on the database role and 468.42 MiB on the
temp role, combined because they share a volume. Available memory was about
13.50 GiB. Short-copy WAL is 102.75 MiB. Full per-index allocations, page layout,
plans, samples and native UI event records are retained in
[evidence/schema-upgrade](evidence/schema-upgrade).

The measurement uses recently built/copied fixtures on Hosted storage. It is
not a cold-cache, slow disk, low-memory or separate-temp-drive guarantee. Older
~30s synthetic results and the external ~76s/217 MiB estimate are not paired
comparators and are not promoted to production limits. The single measured
10.906s result does not establish a maximum user upgrade duration.

## Failure and platform boundaries

File-backed tests preserve complete Global Index entry/volume fingerprints,
Schema42, original Name INDEXED BY query and integrity_check across actual
SQLITE_FULL (SQLite max_page_count), cancellation after DROP, rejected capacity,
read-only failure and forced child termination after uncommitted WAL frames are
spilled. Restart retries the original 42→43 migration; idempotent 43 reopen and
future-version rejection remain covered. Separate filesystem I/O and
cancel-before-open cases ensure parents are preserved and no file is created
when already cancelled. Hardware SQLite IOERR and actual system temporary-volume
exhaustion are not injected; neither receives a fictitious platform PASS.

Normal CI on earlier source 4811d6a encountered the existing macOS SIGBUS
signature tracked by #446 (40421964479, Rust macOS job121919000883), without an
identified crashing test/stack, and W2-11 browser ERR_FILE_NOT_FOUND. They are
retained as failures; no ignored gate or blanket native/full-validation PASS.
The prior source d08c9c0 passed both normal Rust quality jobs. This does not
prove the root cause of the later crash or negate #446's release blocker. No
unchanged-source full CI or SIGBUS investigation is started here.

The native Windows probe initially assumed custom Task Dialog buttons had child
HWNDs; the DirectUI control hierarchy invalidated that test assumption. It now
uses the documented Task Dialog button message after the recovery window is
visible. Ordinary foreground open errors use the same explicit recovery UI;
Windows labels the second recovery action Exit. An unexpected native window
failure requests cancellation and waits for the worker before returning.

The patched happy-dom21.0.4 dev dependency removes GHSA-834v-8h66-3h79 without
changing audit policy. Its expected native animation cancellation is handled
only in the mounted IME test fixture; unexpected errors remain failures.
All 179 frontend files / 1953 tests, typecheck and 30 architecture checks passed
locally. Applicable final CI and exact final HEAD/tree are recorded on Draft
#479. Native QA is not Owner Windows or human accessibility acceptance. #474
Schema44 integration/backfill resource qualification remains a separate decision.
