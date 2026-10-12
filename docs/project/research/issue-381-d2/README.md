# Issue #381 D2 research artifact

This directory preserves the Linux feasibility experiment approved before D3. It is historical evidence against the frozen D2 input HEAD recorded in `INPUTS.json`; it is not CI evidence for the D3 candidate and does not qualify Windows/macOS.

- `REPORT.md`: methods, metrics, limitations, reproduction commands, and D3 design recommendation.
- `results.jsonl`: 20 raw A/B/C variant measurements at 10k and 100k.
- `failure-contracts.jsonl`, `fts-contract.json`: boundary, rollback, and FTS checks.
- `schema37-sqlite-schema.sql` plus `harness/`: real Schema 37 catalog and a portable standalone harness copy. Its only post-run source adjustment is configurable output-directory handling; measured SQL and benchmark logic are unchanged.
- `wal-syscalls-summary.json`: intrusive trace summary/checksums; the 21 MiB unfiltered trace is intentionally excluded from Git.
- `MANIFEST.sha256`: file integrity list for this directory, excluding the manifest itself.

Run `sha256sum -c MANIFEST.sha256` from this directory to verify the preserved inputs.
