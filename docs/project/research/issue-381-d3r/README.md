# Issue #381 D3R SQL plan experiment

This isolated harness compares the production `IN (...) AND EXISTS (...)`
stale-row update with the equivalent set-based `UPDATE ... FROM` update. It
uses the full Schema 37 fixture, then applies the exact provisional Schema 38
`files_au` migration SQL from the #383 candidate. The temp candidate table has
the same columns, `WITHOUT ROWID` layout and `candidate_id` primary key as
production. The harness does not alter persistent schema outside the isolated
database files it creates.

Run from the repository root:

```sh
CARGO_TARGET_DIR=/tmp/issue381-d3r-target \
ZC_ISSUE381_D3R_OUTPUT_DIR=docs/project/research/issue-381-d3r/output \
cargo run --offline --release --locked \
  --manifest-path docs/project/research/issue-381-d3r/harness/Cargo.toml -- --plans
```

The command records paired three-repetition 10k and 100k runs in
`output/sql-plan-results.jsonl`. Each variant starts from a copy of the same
fixture database. It records `EXPLAIN QUERY PLAN`, update and commit time,
writer-lock residence, stale count, temp index/cardinality, FTS behavior,
rename behavior and WAL size. SQLite is bundled by rusqlite 0.39.0.

See [REPORT.md](REPORT.md) for results and limitations. Verify the preserved
inputs and results with `sha256sum -c MANIFEST.sha256` from this directory.
