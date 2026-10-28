# Corrected Candidate B native evidence

These raw JSONL files were emitted by the branch-scoped native job in workflow run
[40666549498](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40666549498). Both runners checked
out source SHA `be835b09f1c7e8f2c7aca1997db3b76214d9c867`, tree
`579b050ccf8d1ec5493b5be740b440014e9a93ad`; the Candidate B code and test source in the follow-up
commit are unchanged from that run. The run executed all Candidate B adversarial cases, then the
official 100k and 1M Release fixtures on Windows and macOS.

Every fixture has 13 query records, each with 5 warmups and 30 measured calls, plus fixture, compact
view build and incremental write/rebuild records. Query percentiles use the empirical order
statistic `ceil((n - 1) * q)` with zero-based indexing and no interpolation. For 30 samples p99 is
the maximum observed call, not a stable tail estimate.

## macOS

- Artifact: [issue494-native-ab-macos-latest-40666549498](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40666549498)
- Artifact ID: `12807507521`
- Archive SHA-256: `b482b23d43e250e54674106ddc2c9dcd847721c9753a7915ed1f3df6d5f95332`
- 100k JSONL SHA-256: `57f1e6253bf0672cb044395ee56643c60d8ddd9795f499d4aedb0b8519950ffa`
- 1M JSONL SHA-256: `3aaf92a9f75a93116ce828ee85ffc495b35278acdf1298f12d6ded47d565ae0e`
- Correctness cases: 23 passed, 0 failed.

## Windows

The Windows runner completed the correctness cases and 100k fixture. Its 1M fixture is still
running; this section will record its artifact and file hashes after upload.

## Files

- `issue494-macos-latest-100k.jsonl`
- `issue494-macos-latest-1m.jsonl`
- `issue494-windows-latest-100k.jsonl` (pending Windows artifact)
- `issue494-windows-latest-1m.jsonl` (pending Windows artifact)

From the repository root, regenerate the complete performance, resource and write/rebuild tables
with `python3 docs/research/evidence/issue-494/summarize.py`.
