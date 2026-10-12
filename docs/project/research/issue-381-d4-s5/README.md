# Zen Canvas #381 — D4-S5 Offline Research

**Gate:** `#381-D4-S5 READY_FOR_OWNER_SCOPED_AUTHORITY_REVIEW` (research only). D4 production remains **NO-GO**.

This independent branch starts at frozen D4-S4 commit `ecbf19d5fb68b7b59777056e63e26cc95922b439`, Tree `f31ed1f2b8e959e690f427f273aa7a3fa0b0ed28`. It changes only this research directory. It does not touch PR #383, PR #386, master, production code, official Schema, IPC, installer, or Schema v38.

## Contents

- [ADR-004 — Root-Scoped Visibility Authority](ADR-004-root-scoped-authority.md): proposed authority domains, overlap behavior, cursor rules, atomic revoke, risks, and production preconditions.
- [Root-Scoped Visibility and Evidence Mutation Contract](ROOT-SCOPED-VISIBILITY-CONTRACT.md): relation, revision, cursor, and transaction contract.
- `harness/scoped-authority-prototype.sql`: disposable SQL tables, guards, publication-time seals, root-aware effective views, and diagnostic projection view.
- `harness/run.py`: reproducible test harness based on the accepted D4-S2/S3/S4 research fixtures.
- `evidence/scoped-authority-results.json`: structured raw outcomes and input hashes.
- `evidence/scoped-authority-run.log`: exact stdout summary from the successful focused run.
- `evidence/archive/d4-s4-transitive-evidence-run.log`: byte-exact D4-S4 original log restored from the retained local raw file; see `evidence/PROVENANCE.md`.
- `evidence/SHA256SUMS`: hashes for this research branch's review artifacts and archived S4 log.

## Reproduce

From the repository root, with Python 3 and SQLite FTS5 available:

```sh
python3 -B docs/project/research/issue-381-d4-s5/harness/run.py
```

The command runs the unchanged S4 runner, which re-executes the accepted 15 D4-S2 assertion groups, 7 D4-S3 failure counterexamples, 4 invalid-publication cases, 16 DML guards, and 11 D4-S4 transitive mutation guards. It then runs the D4-S5 root, overlap, same-path reuse, FTS, cursor, failure-injection, reader, and reconnect cases. The current environment reports SQLite 3.53.1; SQLite 3.51.3 product FTS/BM25 validation remains a separate gate.

The run writes its JSON and log under `evidence/`. Recompute hashes after a run with:

```sh
cd docs/project/research/issue-381-d4-s5
sha256sum ADR-004-root-scoped-authority.md ROOT-SCOPED-VISIBILITY-CONTRACT.md README.md evidence/INPUTS.json evidence/PROVENANCE.md evidence/archive/d4-s4-transitive-evidence-run.log evidence/scoped-authority-results.json evidence/scoped-authority-run.log harness/run.py harness/scoped-authority-prototype.sql > evidence/SHA256SUMS
```

The archived historical log is ignored by the repository's `*.log` rule; it is force-added in this research branch and included in the SHA manifest.

## Findings at a glance

- D4-S4's global clock hides root B when only root A's Watcher or source identity changes.
- With independent revisions and a per-root membership relation, root B remains visible/searchable; A's old projection row is retained but excluded from current results. A cursor carrying the old seal vector is rejected.
- For overlapping roots, A's Watcher invalidation leaves the shared file visible through B only while B's own root seal and the shared source/path/physical evidence remain valid. Invalidating the shared source hides that source from both roots while an independent B companion remains visible.
- Publishing complete replacement generations that remove the last source membership hides only that source. The prior Manifest, claim, file tags, user context, and history remain.
- Same-path reuse yields distinct old/new source and file IDs, and the old tags/context do not transfer.
- D4-S4's split autocommit claim-revocation sequence has a reproducible half-completion. D4-S5's explicit `BEGIN IMMEDIATE` transaction rolls back at all five injected failure points; success advances the source and source-authority revisions once, appends the revoke in the same commit, and survives reconnect.
