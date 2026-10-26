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
Failures offer explicit Retry/Exit, storage/busy guidance and instructions to
preserve the database/WAL/SHM together for support. No automatic retry loop.
Background/autostart defers a pending upgrade to a foreground launch and starts
no indexer/watcher. Second-instance activation waits for database readiness.
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

Process RSS and main/WAL/SHM sizes/free space are sampled every 20 ms. These are
observed process-wide samples, not a proven OS peak or universal upper bound.
The full per-index dbstat allocation before/after is retained in JSONL.

File-backed tests cover SQLITE_FULL, cancellation after DROP, rollback/retry,
pre-COMMIT process kill (without graceful unwinding), integrity_check and
restart. Existing Schema 43/future-version tests remain applicable.

Hosted Windows/macOS compilation, execution, migration timings and interactive
native UI evidence are pending. No production/release qualification is claimed
from static inspection or a headless build. Owner Windows remains acceptance-only.
