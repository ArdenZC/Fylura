# ADR-006 — Publication Fence and Cursor Scope

**Status:** research candidate for Owner review. **Production remains NO-GO.**

## Context

D4-S5 accepted root-scoped visibility and atomic source revocation, but its seal trigger only inserted seals when both D4-S4 and D4-S5 publication capabilities were enabled. It did not reject a published status transition when the D4-S5 capability was disabled. The older D4-S4 `trusted_publish` therefore remained a second publication route. The D4-S5 convenience wrapper also checked seal counts in Python after the D4-S4 helper had committed.

The D4-S6 harness reproduces both defects against the unchanged D4-S5 Schema 37 prototype:

1. Calling D4-S4 `trusted_publish` with the D4-S5 capability false changed a ready Manifest to `published`, inserted the active pointer, created zero D4-S5 seals, and made the effective file view empty.
2. With two roots and two files, an incomplete D4-S4 preflight containing one of two source-evidence rows still committed two Root Seals, one Source Seal, a published Manifest, and a pointer. Only one of two selected files remained visible. The wrapper's post-commit check compared the Seal count with the same incomplete return value and did not catch the mismatch.

These outcomes make the post-commit seal assertion too late to protect the pointer.

## Decision proposed

Keep the D4-S4 publication transaction and D4-S5 root/source Seal model. Add a research-only D4-S6 fence that requires the D4-S4, D4-S5, and designated D4-S6 publisher capabilities, then validates all relational seal sets before the pointer insert or CAS.

The supported successful entry is the D4-S6 `fenced_publish` wrapper. It enables the D4-S5 and D4-S6 connection-local capabilities and delegates to D4-S4's transaction owner. D4-S4 starts `BEGIN IMMEDIATE`, performs the digest and identity preflight, inserts D4-S4 authority evidence, changes generation and Manifest status, lets D4-S5 create its seals, and attempts the pointer insert/CAS. D4-S6 SQL triggers reject status publication unless all capabilities are active and reject pointer movement unless the new Manifest is completely sealed. A single COMMIT follows; any error rolls the transaction back.

The old D4-S4 entry remains callable in the research process but cannot successfully publish: it lacks D4-S5 and D4-S6 capabilities. Calling the D4-S5 wrapper without the D4-S6 entry is rejected as well.

## Commit barrier

Before pointer CAS, SQL checks:

- D4-S4 seal row exists; its root/member/source counts match the selected relations, and its root-map digest equals the staged Manifest value.
- Root Heads and Root Seals are equal in both directions by `(root_id, generation_id)`; each Root Seal agrees with its D4-S4 Root Evidence and current root authority revision, root revision, and Watcher revision.
- Selected Generation Members and the Manifest path projection are equal in both directions by `(source_id, path_entry_id)`.
- The selected member set, D4-S4 Source Evidence, and D4-S5 Source Seals are equal in both directions.
- Source Seal revision fields agree with D4-S4 Source Evidence and current source authority revision.
- Seal counts equal their corresponding complete selected sets. Empty generations are valid: one Root Seal for each selected root, zero Source Seals when the selected member set is empty.
- The candidate Manifest is already `published` inside the uncommitted transaction and satisfies the D4-S3 initial-pointer or parent CAS rules.

The Oracle and SQL checks run while the same `BEGIN IMMEDIATE` transaction is open. The D4-S6 pointer trigger is the last relational gate before pointer DML. A research trace table records Root Seal and Source Seal inserts before the pointer event; that trace rolls back with the publication.

SQLite core in this environment has no SHA-256 SQL function. The trusted application Oracle therefore computes and checks generation member digests, root-map digest, and source-evidence commitments while holding the write transaction. SQL verifies the available relational equality, field, count, and revision conditions before the pointer. It does not claim to recompute hashes inside SQLite.

## Atomicity and retention

The status transition and D4/D5 Seal inserts are uncommitted until pointer CAS succeeds and COMMIT completes. Missing permission, root/source Seal omission, SQL failure, incomplete evidence, or stale parent must restore the pre-publication state: generation status, Manifest status, D4-S4 evidence/seals, D4-S5 authority/seals, trace rows, and pointer. Existing published Manifests and user rows remain immutable and retained.

The harness verified initial and successor publication, overlapping roots, an empty generation, user context/tag retention, a direct legacy-call rejection, missing Root/Source Seal injection, SQL failure, stale parent rejection, and a simulated pointer CAS that affects zero rows. Each failed case retained an identical pretransaction publication snapshot. The existing D4-S5 runner also passed unchanged.

## Cursor scope

Global File Library cursors bind the active Manifest pointer, complete effective Root/Source Seal vector, normalized query/filter, sort definition, and current ranked result digest. Any effective root-set change invalidates a global cursor, including a Watcher change to root A.

A root-B-only cursor may survive a root-A-only Watcher change or publication when the query is actually filtered by root B, root B's current Root Seal and Source Seals are unchanged, and its exact visible FTS result rows and BM25 scores are unchanged. Its token excludes the global Manifest pointer and hashes those root-B authority rows plus the ranked results. A shared source identity change alters B's effective Source Seal relation and result rows, so the old B cursor is rejected and cannot reintroduce that source.

BM25 scores are computed against the SQLite FTS corpus, not a root-local corpus. The root-scoped token includes the actual ranked result rows and scores; an unrelated FTS change that changes B's BM25 order or score therefore invalidates the token. The harness only proves a small correctness case under SQLite 3.53.1, where A's Watcher-only change left the FTS corpus unchanged. SQLite 3.51.3 product ranking and performance remain a separate gate.

## Failure and limitation

The SQL/UDF capabilities are connection-local controls for the service-owned writer connection, not SQLite user roles. A process with arbitrary write access to the database and code able to replace registered functions or alter triggers is outside this prototype's protection boundary. Production must establish one owned publication connection and a controlled service API; this ADR does not define that IPC or client-generation boundary and does not approve production implementation.

Other outstanding gates remain: native Windows/macOS identity proof, SQLite 3.51.3 FTS/BM25 behavior, GC/space-failure recovery, cross-process recovery, and coordination with #380 database-generation isolation. No Schema v38 is allocated.

## Reproducible evidence

Run from the repository root:

```sh
python3 -B docs/project/research/issue-381-d4-s6/harness/run.py
```

Raw outcomes and the observed SQL are in `evidence/publication-fence-results.json` and `evidence/publication-fence-run.log`. Input hashes and artifact hashes are recorded in `evidence/INPUTS.json` and `evidence/SHA256SUMS`.
