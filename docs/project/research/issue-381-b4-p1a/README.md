# Issue #406 — B4-P1A isolated test package

This package records the test-only B4-P1A investigation requested by [Issue #406](https://github.com/ArdenZC/Zen-Canvas/issues/406), using the three design contracts in [Draft PR #405](https://github.com/ArdenZC/Zen-Canvas/pull/405) and its cited research evidence in [PR #399](https://github.com/ArdenZC/Zen-Canvas/pull/399) and [PR #400](https://github.com/ArdenZC/Zen-Canvas/pull/400).

## Locked source

- Base: Schema 38 `master`, commit `ab3915e3cdcc7c729317498b3b97501862832a91`, tree `ca22989646575bba9a7224ce10becbb89c01e0ee`.
- Test harness source commit: `fd5fec9595baca1c62ae781ac5cf5e2d67d117f8`, tree `05168efc9c05f614a5a33defd98632179b4337ae`.
- Branch: `test/issue-381-b4-p1a-f1-lifecycle`.
- `src-tauri/src/db/schema.rs` remains Schema 38. The existing `library_generation_authority` default remains `('legacy', 0, NULL, 1)`.

## Running the isolated harness

From the repository root:

```sh
python3 tests/issue381_b4_p1a_f1/run.py
```

The runner uses a separate Cargo manifest and lockfile, bundled SQLite, synthetic fixtures, and disposable temporary databases. It does not open the application database. It saves complete raw JSONL output as deterministic gzip with both compressed and uncompressed SHA-256 values, and keeps stdout/stderr for the toolchain, Cargo feature tree, formatting, build, and independent oracle commands.

## Results and evidence

- [REPORT.md](REPORT.md) summarizes the result and measured limits.
- [MATRIX.md](MATRIX.md) maps all 12 positive gates and 4 negative gates to their evidence.
- [DESIGN-INPUTS.md](DESIGN-INPUTS.md) lists the reviewed Issue, contracts, and cited research inputs.
- [SCOPE-AUDIT.md](SCOPE-AUDIT.md) records the path allowlist and prohibited-file audit.
- [evidence/final/test-matrix.json](evidence/final/test-matrix.json) contains machine-readable case results.
- [evidence/final/raw/matrix-native.stdout.jsonl.gz](evidence/final/raw/matrix-native.stdout.jsonl.gz) preserves native harness output, including 300 benchmark samples.
- [evidence/final/raw/matrix-oracle-checks.jsonl](evidence/final/raw/matrix-oracle-checks.jsonl) preserves all 12 independent query comparisons.
- [evidence/final/environment-and-commands.json](evidence/final/environment-and-commands.json) binds tool output and hashes to the source HEAD and tree.
- [evidence/previous-formal-run/README.md](evidence/previous-formal-run/README.md) preserves the earlier complete run before raw T06 lock waits were added to the output stream.
- [evidence/previous-formal-run-raw-lock-samples/README.md](evidence/previous-formal-run-raw-lock-samples/README.md) preserves the complete run before the final whitespace-only source cleanup.
- [evidence/development/README.md](evidence/development/README.md) explains and indexes both retained zero-sample smoke attempts.

The recorded result is `P1A_BLOCKED_ENVIRONMENT`: every runnable expectation passed, but T10 remains unavailable because this executor is Linux and cannot provide the required native Windows and macOS file-handle runs. The process-kill and injected-capacity cases do not establish sudden power-loss behavior.
