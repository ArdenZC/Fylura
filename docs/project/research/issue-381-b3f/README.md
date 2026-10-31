# #381-B3-F — bundled trigram options probe

Authorization: [Owner comment 6143106540](https://github.com/ArdenZC/Zen-Canvas/issues/381#issuecomment-6143106540). This is a single paired **test-only** F0/F1/F2 comparison from master `3275ac7c72df8ae4139a30f16c7f649971324e97`. Only new files in `tests/issue381_b3_fts/` and this research directory are included. It is not a production schema/indexer/search rewrite, native qualification, or merge GO.

Read [REPORT.md](REPORT.md) for the decision and source-integration unknowns and [RESULT.md](RESULT.md) for actual final-run numbers. The final commit/tree, full diff and natural CI are in the independent Draft PR closeout. Committed qualification runs bind the immutable source-candidate commit; exact final-head replay verifies unchanged source SHA-256. A Git commit cannot contain its own final hash.

## Replay in the recorded Codex environment

The existing Rust 1.97.1 toolchain is selected without changing the repository toolchain file. From the repository root:

```sh
PYTHONDONTWRITEBYTECODE=1 python tests/issue381_b3_fts/run.py --output .tmp-tests/issue381_b3f/owner-replay
```

Use a fresh output directory. The runner sets the recorded existing Cargo/Rustup cache selectors, builds the isolated crate with `--locked`, verifies every registry package against the product lock, checks format and executes 0/one/small/10k/100k paired fixtures. The fixture generator and mathematical reference are portable stdlib Python; the orchestrator records this executor's cache paths. Native process-crash tests use Linux SIGKILL and do not pretend to be a Windows/macOS harness. There is no root Cargo/npm manifest registration or CI workflow change.

## Evidence layout

- `evidence/results.jsonl`: derived measurement/plan summary with explicit case/phase/scope/query IDs; no invented percentiles.
- `evidence/B3F-*-native.jsonl.gz`: **unmodified native stdout JSONL bytes**, losslessly compressed. Full source IDs, ranks, float bits, schedules, pages, failures, native reference values, GC batches and crash commands remain inside. Decompress with `gzip -dc`; the decompressed byte count/hash is recorded in `results.jsonl`.
- `evidence/independent-oracle-checks.jsonl`: full-result comparisons against independent fixture-string BM25; counts, exact-order/float hashes and differences. Large native oracle indexes are never rebuilt.
- `evidence/source-parity.json`: complete locked registry/version/checksum equivalence, bundled SQLite crate/amalgamation/header hashes and product lock provenance.
- `evidence/environment.json`, commands and adjacent stdout/stderr: source hashes, Git identity, Python/OS, real commands/exit codes and feature graph. Actual SQLite version/source ID/compile options/trigram capability come from the **native connections** in the raw/summary files, not Python SQLite.
- `evidence/owner-authorization.json`, `blueprint-pr.json`, `blueprint.md`, `product-packages.json`: reread authorization and immutable source inputs.
- `MANIFEST.sha256`: full-byte hashes of source/documents/evidence, excluding the manifest itself.

No binary SQLite snapshots are committed. Portable source fixtures plus full native rows and small inspectable state exports support replay. Temporary databases/builds stay in the worktree-local ignored `.tmp-tests/issue381_b3f/` directory. Natural PR CI does not register or run this standalone harness; any routed product checks are reported separately.
