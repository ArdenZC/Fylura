# T10-A hosted native research

## Source and execution boundary

- Branch: `test/issue-381-b4-p1a-native-t10`
- Frozen start: HEAD `d5ba2b07b7f6c1be6dce5f58b67718bb5b7ef84f`, Tree `ba664783eb69c388c5846cd1e05ad76a315da8d4`
- Hosted workflow: `.github/workflows/issue-381-b4-p1a-native-t10.yml`
- Trigger: push to this branch only; matrix: `windows-latest` and `macos-latest`
- Permissions: `contents: read`; checkout and artifact actions use the repository's pinned immutable SHAs
- Build: `cargo build --release --locked --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml`
- Entry point: `issue381-b4-p1a-f1 --t10-native`; the legacy all-suite runner is not invoked
- All fixtures and Cargo build output are rooted under each hosted runner's temporary directory

## Evidence rules

The executable emits eight per-platform rows (`T10-01` through `T10-08`) with one of `PASS`, `EXPECTED_REJECTION`, `FAIL`, or `ENVIRONMENT_UNAVAILABLE`. It records the exact pushed HEAD and Git Tree, OS build, filesystem type, architecture, Rust target and versions, SQLite `sqlite_source_id()`, Cargo lock parity, raw filesystem outcomes and OS error codes, child exit/restart observations, manifest and slot digests, and cleanup results.

Windows directory-sync errors are preserved with their `ErrorKind` and raw OS code. The T10-only sealer may continue using disposable fixtures only after the SQLite file and manifest files have passed real `sync_all`; a failed directory sync remains `ENVIRONMENT_UNAVAILABLE` and never counts as persistence success. Linux execution, child-process termination, simulated disk-full, and unsupported directory sync are not power-loss or native persistence proof.

Raw per-platform stdout, stderr, exit codes, environment records, per-case JSONL and SHA-256 manifests are uploaded as immutable GitHub Actions artifacts even when build or test steps fail. No user files are opened, scanned, indexed or logged.

## Hosted execution record

Pending first exact-head Hosted Windows/macOS run. Run IDs, attempt, tested HEAD/Tree, per-platform outcomes, artifact digests and any blocker will be recorded here after the push workflow completes.
