# Issue #381-B2 — isolated effective-visibility Oracle

Owner authorization: [comment 6141964429](https://github.com/ArdenZC/Zen-Canvas/issues/381#issuecomment-6141964429). This is a disposable test model and independent reference implementation. It is not a production fix, migration, native identity qualification, or proof of all product consumers.

Only new files under `tests/issue381_b2/` and this research directory are included. Master, production Schema/Scanner/Watcher/IPC, default policies, manifests and CI workflows remain unchanged. PR #383 stays DESIGN BLOCKED; v38 is unallocated; PR #394 and frozen S8/S8R remain unchanged. No Codex Review, Full Validation dispatch, merge, release, or issue closure is authorized.

## Replay

From the repository root, use a **fresh output directory**:

```sh
PYTHONDONTWRITEBYTECODE=1 python tests/issue381_b2/run.py --output /tmp/zen-canvas-b2-replay
```

The runner refuses to overwrite existing `results.jsonl` and rejects optimized Python (`-O`). A full run returns zero only for exactly 16 positive passes and 6 observed expected rejections. `--case B2-T13` runs one diagnostic, but cannot produce the full ready status. Missing FTS5 returns `B2_ENVIRONMENT_UNAVAILABLE`; LIKE is never substituted.

`model.py` owns the disposable SQLite schema and model operations. `oracle.py` independently computes root-claim union, incarnation/observation eligibility and BM25 using Python sets and token frequencies; it imports neither the model nor SQL visibility helpers. `run.py` owns deterministic fixtures, interleavings, assertions, subprocess crashes, contention and evidence. These files are not registered in production or the repository's test manifest. Natural PR CI does **not** run this Python harness.

## Evidence

- [REPORT.md](REPORT.md): model semantics, case mapping, provenance and limitations.
- `evidence/results.jsonl`: unmodified runner output, one record per T/N case, seed, inputs, schedules, assertions, independent expected/actual results and differences.
- `evidence/environment.json`: Python/SQLite/source ID/compile options, OS, FTS5, exact command, source hashes and source-candidate Git identity.
- `evidence/commands.json`: actual validation commands, exit codes and logs.
- `evidence/summary.json`: full-run disposition and counts.
- `evidence/*.sqlite`: SQLite backup snapshots for inspecting retained rows and recovery outcomes. Backup snapshots are separate from the live WAL measurements in JSON; they are not original live WAL files.
- `evidence/input-*`: verbatim reread GitHub snapshots and frozen FTS/cursor contract inputs; `inputs.json` binds the contract paths to immutable commits.
- `MANIFEST.sha256`: full byte hashes of source, documents and evidence, excluding the manifest itself.

The final commit/tree and exact-head CI result are recorded in the Draft PR body/closeout. They cannot be embedded into their own Git tree without a circular hash. The committed raw run binds to the source-candidate commit; final-head replay independently verifies unchanged source hashes. Final CI evidence is an external exact-head record, not a later untested evidence commit.
