# #381 D4-S3 Manifest Authority Research

Status: `#381-D4-S3 READY_FOR_OWNER_AUTHORITY_PROOF_REVIEW` — research only; production remains NO-GO pending Owner review and the remaining gates in the ADR.

This branch is based on the frozen D4-S2 commit `0a29c5b0114744f810de90ad31146394ce7be399` (Tree `7de3c7478868dd17b53884e9481d1d6ce60b7b99`). It contains only research documentation, SQL views/triggers for disposable databases, a Python test Harness, and evidence. The working tree does not change production Schema, Scanner, File Library, FTS code, IPC, installer, master, PR #383, or PR #386. No schema version is assigned.

## Reproduce

From the repository root:

```sh
python3 docs/project/research/issue-381-d4-s3/harness/run.py
```

The Harness loads the captured Schema 37 DDL and the committed D4-S2 candidate SQL. It first executes the D4-S2 15 assertion groups unchanged, then runs the D4-S3 counterexamples and protections in fresh disposable databases. It does not open a user database or touch a project SQLite file.

The exact raw log, structured output, and SHA-256 manifest are under `evidence/`. The source SQL is in `harness/manifest-authority-oracle.sql` and `harness/manifest-authority-protection.sql`.

## Review order

1. [ADR-002-unified-manifest-authority.md](ADR-002-unified-manifest-authority.md) — proposed authority, publication, cursor, history, FTS, and #380 prerequisites.
2. [SQL-ORACLE.md](SQL-ORACLE.md) — exact bidirectional relation, source/path evidence checks, deterministic summaries, and reproduced failures.
3. [VALIDATION-CONTRACT.md](VALIDATION-CONTRACT.md) — repeatable command, retained D4-S2 assertions, additional tests, and scope limits.
4. [EVIDENCE-PROVENANCE.md](EVIDENCE-PROVENANCE.md) — original D4-S2 log recovery and byte-level provenance.
