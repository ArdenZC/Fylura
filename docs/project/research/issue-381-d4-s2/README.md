# Zen Canvas #381 D4-S2 research branch

This branch is an independent successor to the frozen D4-S1 design branch. It contains a physical source-incarnation ADR amendment and a one-shot disposable SQLite Harness. It does not change production application code or schema, and it does not allocate schema version 38.

## Reproduce the Harness

From the repository root:

```sh
python3 docs/project/research/issue-381-d4-s2/harness/run.py
```

The command reads the captured full Schema 37 `sqlite_master` DDL from the D4-S0 evidence, creates disposable databases, seeds synthetic user/history relations, rebuilds only the test database's `files` table with an opaque `source_id` and without `path UNIQUE`, and applies the candidate relations in `harness/physical-candidate.sql`. It writes JSON and a concise run log to `evidence/`. It deletes its temporary databases on completion.

The Harness demonstrates:

- Schema 37 direct `files(id)` foreign-key preservation and rowid/FTS integrity through a table rebuild.
- Legacy source defaulting to read-only `legacy_unverified`, followed by an explicit audited user claim.
- One-root membership loss while another root still exposes the same source; loss of the final root hides it while retaining all related rows.
- A different native identity occupying the same path, with no transfer of tags, notes/classification, history, fingerprint, duplicate membership, content, findings, or Global Index AI state.
- Same-source rename/path-entry history and separate hard-link sources sharing a physical-identity epoch.
- Idempotent retry, injected constraint rollback, process exit before commit, startup recovery, pointer conflict rejection, and FK/integrity/FTS checks.

It uses workspace Python SQLite 3.53.1. It is a synthetic migration feasibility proof, not a performance test or Windows/macOS/native filesystem qualification. The exact input and output hashes are in `INPUTS.json` and `evidence/SHA256SUMS`.

## Scope boundary

The branch does not modify PR #383 or #386, master, official Schema 37 code, the installer, or IPC. It runs no full CI, Hosted Windows/macOS test, broad benchmark, Codex Review, merge, release, or Issue mutation. The D4-S1 source-audit inventories remain the frozen reader/writer reference; this amendment focuses on physical identity and the legacy mapping.

## Files

- `ADR-001-physical-source-incarnation.md` — revised physical model and logical-contract amendments.
- `MIGRATION-TRUTH-TABLE.md` — path reuse, move, hard link, roots, and retained data outcomes.
- `VALIDATION-CONTRACT.md` — future physical, migration, visibility, concurrency, search, GC, and #380 gates.
- `harness/physical-candidate.sql` and `harness/run.py` — test-only DDL and reproducible assertion Harness.
- `evidence/schema37-physical-harness-results.json` — raw named assertions and measured environment IDs.
- `evidence/schema37-physical-harness-run.log` — exact concise command output.
