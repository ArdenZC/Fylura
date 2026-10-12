# D4-S4 trusted publication transaction contract

This is a research contract only. It is exercised by harness/run.py against the captured full Schema 37 DDL plus unchanged D4-S2/D4-S3 candidate SQL. It does not change the production DB API.

## State transition authority

Only one service repository method may publish: a future private publish_visibility_manifest(expected_parent, expected_pointer_revision, manifest_id) operation. The service connection registers zero-argument d4s4_publish_authorized() and d4s4_evidence_change_authorized() functions backed by private connection state. They return false by default. Only the private repository operation can arm the publish capability after its Oracle has completed; ordinary query/command handlers never receive setters or a writer connection escape hatch.

The test trigger rejects:

- complete generation -> published without the publication capability;
- ready Manifest -> published without the publication capability;
- active pointer insert/update without the publication capability;
- evidence-clock writes outside the evidence-change or publication capability;
- current files.path/name updates outside publication.

The existing D4-S3 state, exact-set, identity, published-object immutability, parent-pointer and CAS triggers remain active. A raw SQLite connection without the registered function fails with “no such function: d4s4_publish_authorized”. On the compatible connection, direct status and pointer SQL fails because the private capability remains false.

The capability is not an SHA-256 proof. A local process with unrestricted database access could register a forged function or remove triggers; #380's approved compatible-client/database-generation gate must prevent arbitrary old writers from reaching the upgraded DB. This is a service trust boundary, not hostile-local-user protection.

## Single transaction sequence

The publisher runs the following under one BEGIN IMMEDIATE / COMMIT transaction:

1. Read and retain the old pointer (manifest_id, revision). Require the proposed Manifest parent to match it (or no parent for first publish).
2. Load candidate root heads, root generations, members, file path entries, source rows, physical identity epochs, explicit claims, roots and scan runs.
3. Recompute application-side SHA-256 summaries over canonical JSON row arrays for:
   - all selected (root_id, generation_id) pairs;
   - all exact expected (source_id, path_entry_id, path_key, path_text) members;
   - all source revision, identity, path/physical digest and claim evidence tuples.
4. Require the existing D3 Oracle's expected/projection and projection/expected EXCEPT sets to both be empty. Recheck counts, per-generation member digest/count, root-map digest, identity confidence, physical epoch completeness, exact explicit claim, source uniqueness, and root/path boundary.
5. Require each run is terminal completed with coverage complete; generation member count/digest matches; generation root and watcher snapshots match the current root and run-start revisions; the Watcher is fully applied; root is enabled, healthy and reconciled.
6. Insert immutable Manifest authority/root/source seal rows at the current authority revision.
7. Arm the private publish capability. Seal complete generations, set the ready Manifest to published and insert/update the pointer using parent plus revision CAS.
8. Commit and disarm the capability. Any exception, SQL trigger rejection, disk error or CAS mismatch rolls back all transaction changes. The previous pointer and previous published rows remain.

The harness starts publication only after staging rows exist. If a wrong stored digest is supplied, the trusted Oracle rejects before a seal or pointer write. If raw SQL attempts status transition, the authorization trigger rejects. If the root or Watcher changes after candidate preparation, final preflight rejects and no successor seal is written.

## Trigger/application boundary

| Invariant | Application Oracle in locked transaction | SQL trigger |
|---|---|---|
| SHA-256 values | Computes and compares bytes for member, root map and source-evidence summaries. | Does not call or assume a native SHA-256 UDF. SQLite in this environment has no such built-in function. |
| Exact Manifest relation | Independently derives the expected set; checks both EXCEPT directions and all digest/count values. | D3 trigger rechecks both directions and join/identity constraints. |
| Source and root identity | Re-reads source revision/status/confidence, physical epoch, path evidence, claim, root path/config and Watcher/run revision snapshots. Performs platform-specific root containment in future native implementation. | Stops unauthorized post-publication evidence changes; allows only evidence API changes that advance the authority clock. |
| Publish sequencing | Owns the only callable publisher, arms the private capability only after Oracle success, and performs one transaction. | Blocks direct DML publication and enforces D3 parent/revision CAS. |
| Atomicity | All seal/status/pointer and any current path/name/FTS changes share the transaction. | RAISE(ABORT) causes the wrapper to roll back the full transaction. |

## Published evidence changes and invalidation

- Run terminal status and coverage evidence is sealed against rewrite. Recovery or correction uses another run.
- Source state changes require a one-step source_revision increase. The authorized mutation increments the global authority revision.
- Root semantic ownership/configuration and Watcher revision changes use the authorized evidence path and increment the authority revision.
- Physical identity epochs and source claim rows are append-only. A new physical observation adds a new epoch/path entry. Claim revocation appends a revocation event, increments source_revision and authority revision, and retains the original claim.
- A fresh current read is valid only if the active Manifest seal revision equals the current authority revision. A cross-transaction Cursor includes that authority revision as well as pointer ID/revision. It is rejected on any mismatch.
- Root display_name, files.context and user tag changes are outside the authority domain. They do not move the Manifest pointer or authority clock. The Harness verifies the tag association remains writable after publication.
- Root revision remains the Schema 37 broad CAS for publication preparation. It is stored in the seal, but not continuously used as the authority clock because normal scan admission increments it. Production must separate root configuration revision from lease/lifecycle revision.

The current prototype intentionally leaves the D3 effective_file_sources as a raw relational view. After an evidence change, that raw view can still list rows. The new d4s4_effective_file_sources is the authority-filtered view and returns zero rows until a new Manifest publishes. This reproduces why all product consumers must be migrated before the Manifest can become production authority.

## D4-S4 result

The test-only publication API rejects wrong member/root SHA values and stale root/Watcher snapshots. A connection without the private UDF cannot publish. A valid source seal records exact root, member and transitive source-evidence counts/digests. Old published state is retained on failure, and a prior Manifest stays pointed to after a post-preflight Watcher event; its authority view fails closed while the prepared successor remains ready and unsealed.

No schema version, production API or migration is approved by this contract.
