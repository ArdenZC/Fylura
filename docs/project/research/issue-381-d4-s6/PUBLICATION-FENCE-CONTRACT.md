# D4-S6 Publication Fence Contract

Research-only contract for the Schema 37-derived prototype. This is not product DDL.

## Trusted transaction route

1. The designated D4-S6 publisher enters D4-S4 `trusted_publish` with the D4-S5 and D4-S6 connection-local capabilities enabled.
2. D4-S4 starts `BEGIN IMMEDIATE` and runs the Python/SQL preflight against the staged Manifest, selected root generations, source/path identities, claims, root and Watcher revisions, and the active pointer parent.
3. D4-S4 inserts its Manifest, Root, and Source Evidence seals. SHA-256 values are computed by this Oracle while the transaction is open.
4. The publisher marks the selected generations and Manifest `published`. A `BEFORE UPDATE` trigger requires D4-S4, D4-S5, and D4-S6 authorization. D4-S5's existing `AFTER UPDATE` trigger inserts Root/Source authority rows and per-Manifest seals in this same transaction.
5. Before initial pointer insertion or successor pointer CAS, D4-S6 triggers compare the complete sets and revisions described below. D4-S3 continues to enforce parent/revision CAS.
6. Only after those checks succeed does the pointer statement run. COMMIT makes the new Manifest, all seals, and pointer visible atomically.
7. Every exception before COMMIT executes ROLLBACK. No post-COMMIT assertion is used as the publication proof.

## Relational Oracle

Let `R` be `manifest_root_heads(manifest_id)` and `RM` be `d4s5_manifest_root_seals(manifest_id)`. Require:

```sql
R EXCEPT RM = empty
RM EXCEPT R = empty
```

The tuple is `(root_id, generation_id)`. Each seal must match D4-S4 Root Evidence and current Root Authority, root revision, and Watcher revision.

Let `M` be the distinct `(source_id, path_entry_id)` selected by all Root Heads joined to their Generation Members, `P` the same identity from `manifest_file_paths`, `E` from D4-S4 Source Evidence, and `S` from D4-S5 Source Seals. Require:

```sql
M EXCEPT P = empty   AND   P EXCEPT M = empty
M EXCEPT E = empty   AND   E EXCEPT M = empty
E EXCEPT S = empty   AND   S EXCEPT E = empty
```

Each Source Seal must match the D4-S4 evidence row's source revision and the current Source Authority revision. Counts in the D4-S4 Manifest Authority Seal, Root Seals, Source Seals, and selected relations must agree. Empty `M` is valid when each selected root has its matching empty Generation and Root Seal.

## Hash responsibility

The trusted application Oracle computes SHA-256 for root maps, generation members, and source evidence inside the open write transaction. SQLite has no built-in SHA-256 in the tested environment. SQL requires the D4-S4 authority row, checks its root-map field against the staged Manifest field, checks relational counts and membership, and depends on the in-transaction Oracle to establish that the digest fields match the underlying ordered rows. A future implementation must preserve this boundary explicitly or provide an approved deterministic hash extension.

## Failure behavior

| Failure | Required result |
| --- | --- |
| D4-S5 or D4-S6 capability absent | Reject before Manifest publication; rollback D4-S4 inserts and generation status |
| Root Seal omitted or extra | Pointer fence rejects; rollback Manifest, all seals, authority bootstrap, and trace |
| Source Seal omitted or extra | Pointer fence rejects; rollback all publication state |
| Root/source identity or revision mismatch | Pointer fence rejects; rollback |
| D4-S3 Oracle digest/member/path mismatch | Preflight rejects before pointer; rollback |
| SQL error while inserting evidence or seals | Roll back all writes in the publication transaction |
| Stale parent or pointer CAS rowcount miss | Reject; old pointer and old Manifest remain unchanged |
| Empty Generation | Commit if Root Seal set is exact and member/evidence/source-seal sets are all empty |

The event trace is part of the disposable harness only. It records Seal insertion and pointer CAS ordering in the same transaction, and therefore disappears on rollback.
