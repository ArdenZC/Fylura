# Zen Canvas #381 D4-S0 research harness

This directory contains only an isolated feasibility harness and evidence. It is not part of PR #383 and adds no production schema, IPC, Global Index, AI, or CI changes.

## Reproduce

From the D4-S0 worktree:

```sh
ZC_ISSUE381_D4_OUTPUT_DIR=target/issue-381-d4-s0-output \
  cargo run --offline \
  --manifest-path docs/project/research/issue-381-d4-s0/harness/Cargo.toml \
  -- --d4-s0
```

The harness uses bundled SQLite 3.51.3 and the copied Schema 37 migration source identified in [INPUTS.json](INPUTS.json). It creates disposable databases with full project tables, indexes, triggers, FTS, and associations, then runs three repetitions for each model at 10k and 100k candidate rows. It also writes failure/recovery contracts and a schema dump. The test-only `d4_s0_*` tables are created inside these disposable databases and do not change `user_version`.

## Evidence

- [REPORT.md](REPORT.md): findings, limitations, and recommendation.
- [consumer-matrix.csv](consumer-matrix.csv): reviewed production readers and writers.
- [results.jsonl](evidence/results.jsonl): 12 raw scale/model repetitions.
- [failure-contracts.json](evidence/failure-contracts.json): restart, fence, rollback, crash, and multi-root checks.
- [broad-ftsrank-diagnostic.json](evidence/broad-ftsrank-diagnostic.json): separate broad FTS ranking sensitivity measurement.
- [schema37-sqlite-schema.sql](evidence/schema37-sqlite-schema.sql): schema and trigger dump from the fixture.
- [MANIFEST.sha256](MANIFEST.sha256): evidence and input source hashes.

The harness exercises pointer semantics in a reduced model; it does not establish that current application consumers share one visibility authority. Source and filesystem limits are stated in the report.
