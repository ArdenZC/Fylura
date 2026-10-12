# #381 D4-S4 — Transitive Evidence Seal Research

Status: #381-D4-S4 READY_FOR_OWNER_TRANSITIVE_AUTHORITY_REVIEW. This is research-only. D4 production remains NO-GO.

Frozen parent D4-S3:
- HEAD: ad85df097d7a8425df01e1a95829c38fd17dd4d6
- Tree: 58b0e5854237189275d54224d5ed80237d34c526

D4-S4 changes are confined to docs/project/research/issue-381-d4-s4/. No production source, formal schema, Scanner, Service, IPC, installer, File Library, FTS implementation, master, PR #383 or PR #386 was modified. Schema v38 is unassigned.

## Review order

1. ADR-003-transitive-evidence-seal.md — decision, evidence, revision and #380 contracts.
2. FIELD-DEPENDENCY-MATRIX.md — per-field dependencies, actual product readers/writers, revisions and current is_stale consumers.
3. PUBLICATION-TRANSACTION-CONTRACT.md — sole trusted publication entry and trigger/application split.
4. harness/transitive-evidence-protection.sql — disposable SQL protection, seal and authority-filtered view.
5. INPUTS.json — frozen parent, exact input files and environment.
6. evidence/transitive-evidence-results.json — structured old-failure/new-protection output.
7. evidence/transitive-evidence-run.log and evidence/SHA256SUMS — raw summary and exact artifact digests.

## Reproduce

From the repository root:

    python3 -B docs/project/research/issue-381-d4-s4/harness/run.py

The harness imports the frozen D4-S3 runner unchanged and executes it first. That reruns the retained D4-S2 15 assertion groups, D4-S3 7 original counterexamples, 4 invalid publication cases, 16 DML guards, overlapping roots, rename/FTS, cursor, and rollback checks. It then creates fresh disposable Schema 37 databases for D4-S4 counterexamples and protection. It never opens a user database.

Verify committed evidence:

    sha256sum -c docs/project/research/issue-381-d4-s4/evidence/SHA256SUMS

## Principal observations

- Prior S3 SQL allowed all seven tested post-publication transitive mutations. The pointer stayed unchanged while evidence could become invalid; the raw D3 relation kept exposing the source.
- The old Oracle did not bind physical identity epoch evidence_digest into its invalid-identity view.
- With deliberately wrong member_digest or root_map_digest, direct D3 status updates plus pointer insert succeeded despite the Python Oracle rejecting the candidate.
- The D4-S4 prototype rejects direct changes to published terminal run evidence, source identity state, physical identity rows, claims, root ownership and Watcher revisions. Authorized source/root/Watcher changes advance an explicit authority revision, so current reads and cross-transaction Cursors fail closed until a successor Manifest publishes.
- Claims remain as history; revocation is an appended event. File context and tag writes remain available and do not change authority revision.
- Wrong digest values fail the trusted locked publisher; direct status/pointer SQL on the compatible connection is blocked. A raw connection without the private authorization UDF fails closed.
- On failure, the prior Manifest and pointer remain; a post-preflight Watcher change makes the prepared successor fail while preserving the old pointer and an unsealed ready candidate.

## Runtime and limits

Harness runtime: SQLite 3.53.1 in this execution environment; the product SQLite 3.51.3 FTS/BM25 gate was not run. No large benchmarks, full CI, Hosted Windows/macOS, Codex Review, Schema version allocation or production code change occurred.

The authority clock is global in this prototype and temporarily hides all roots after one evidence invalidation. Root boundary is only a synthetic POSIX check. Native platform identity, filesystem TOCTOU, arbitrary local schema replacement, #380 generation isolation, GC/leases, WAL/space pressure, and product query migration remain production blockers.
