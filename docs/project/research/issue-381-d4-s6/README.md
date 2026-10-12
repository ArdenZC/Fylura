# Zen Canvas #381 — D4-S6 Offline Research

**Gate:** `#381-D4-S6 READY_FOR_OWNER_PUBLICATION_FENCE_REVIEW` (pending review). D4 production remains **NO-GO**.

This research branch is based on frozen D4-S5 HEAD `59e71da8e37dc7b1f1828b71af06596d178e5611`, Tree `9439b7264c34130f5716e482a35fffc0b225ce01`. It changes only `docs/project/research/issue-381-d4-s6/`. It does not modify D4-S5, D4-S0 through D4-S4, production code, master, PR #383, PR #386, official schema, IPC, installer, or Schema v38.

## Contents

- `ADR-006-publication-fence-and-cursor-scope.md`: publication barrier proposal, cursor scope, failure evidence, and limits.
- `PUBLICATION-FENCE-CONTRACT.md`: trusted transaction sequence, SQL set equalities, hash boundary, and rollback contract.
- `CURSOR-SCOPE-CONTRACT.md`: global and root-filtered cursor rules.
- `harness/publication-fence.sql`: test-only SQL fence, relational validation triggers, and transactional event trace.
- `harness/run.py`: baseline failure reproduction, repaired publication checks, Cursor experiments, and unchanged D4-S5 regressions.
- `evidence/publication-fence-results.json`: structured output, exact executed DML, transaction outcomes, seal order, and cursor observations.
- `evidence/publication-fence-run.log`: raw stdout from the reproducible run.
- `evidence/INPUTS.json`: frozen base and input hashes.
- `evidence/PROVENANCE.md`: run command, runtime identity, and evidence handling.
- `evidence/SHA256SUMS`: hashes of review artifacts and raw test output.

## Reproduce

From the repository root, with Python 3 and SQLite FTS5 available:

```sh
python3 -B docs/project/research/issue-381-d4-s6/harness/run.py
```

The command first reproduces the old S4 bypass and partial-Seal commit against the unchanged S5 prototype. It then installs only D6 research SQL and runs legacy-entry rejection, rollback/failure injection, initial/successor/overlap/empty-generation publication, root-scoped and global Cursor cases, and the unchanged D4-S5 runner. That retained runner executes its accepted D4-S2/S3/S4 predecessor cases as well.

Regenerate the artifact hashes from this directory with:

```sh
sha256sum ADR-006-publication-fence-and-cursor-scope.md CURSOR-SCOPE-CONTRACT.md PUBLICATION-FENCE-CONTRACT.md README.md evidence/INPUTS.json evidence/PROVENANCE.md evidence/publication-fence-results.json evidence/publication-fence-run.log harness/publication-fence.sql harness/run.py > evidence/SHA256SUMS
```

## Findings

- Old D4-S4 publication with S5 capability false committed a published Manifest and pointer with no D4-S5 seals; effective visibility became empty.
- An incomplete source evidence vector committed 2 Root Seals and only 1 Source Seal for 2 selected paths; only one source became visible.
- D6 rejects both the old S4 path and the S5 wrapper alone. The pointer triggers require complete Root and Source Seal relations before pointer DML. Permission errors, missing seals, injected SQL failure, stale parent, and a simulated zero-row pointer CAS all leave the complete publication snapshot unchanged.
- Global cursors invalidate when root A changes. A root-B cursor remains valid across root-A-only Watcher changes and publication only while root B's effective seal vector and exact filtered FTS ranked rows stay unchanged. Shared-source invalidation rejects the old B cursor.
- The harness used Schema `user_version=37` and local SQLite `3.53.1`. SQLite 3.51.3 product FTS/BM25, native platform identity, GC/space recovery, and #380 database-generation isolation remain separate gates.

No production implementation is authorized by this research result.
