# D4-S4 validation contract

## Exact command and inputs

From the repository root:

    python3 -B docs/project/research/issue-381-d4-s4/harness/run.py

The runner uses the committed D4-S0 captured Schema 37 DDL, D4-S2 physical candidate SQL, D4-S2 runner, D4-S3 Oracle/protection SQL, and the unchanged D4-S3 harness runner. D4-S4 SQL is applied only to in-memory or temporary test databases. The default Schema pragma remains 37; no formal schema migration is run.

Runtime observed: SQLite 3.53.1, source ID recorded in evidence/transitive-evidence-results.json.

## Preserved regression suite

The runner first calls D4S3.run() unchanged, which itself calls the D4-S2 run_main_experiment unchanged. It asserts:
- 15 D4-S2 history/identity/retention/failure-recovery assertion groups;
- 7 D4-S3 original authority counterexamples;
- 4 invalid publication cases rejected and rolled back;
- 16 DML immutability/CAS changes rejected;
- overlapping-root membership union;
- rename, old/new FTS term, pinned WAL reader, cursor expiry, rollback, and integrity checks.

D4-S3 evidence and checksums are read but not rewritten.

## New original-model counterexamples

Each case uses the old D4-S3 candidate plus its D3 protections, without the D4-S4 SQL. It publishes a valid single-source Manifest, applies one direct SQL mutation, and records the raw D3 view and post-mutation evidence checks:

1. scan_runs.coverage_complete changes from 1 to 0;
2. scan_runs.status changes from completed to failed;
3. file_sources.status changes from observed to conflicted;
4. file_sources.identity_confidence changes from native_verified to path_only;
5. file_physical_identity_epochs.evidence_digest is replaced after publication;
6. a published explicit source_identity_claims row is deleted;
7. scan_roots.normalized_path changes so the member falls outside its root.

The raw effective view and pointer remain unchanged for all seven. The record distinguishes D3 run/identity/set evidence failures from physical-digest/root-boundary checks absent in the old Oracle. Since D3 Oracle labels only pre-publication candidates as valid, the post-publication status predicate is not treated as the evidence verdict; the result records its component checks separately.

## New digest bypass counterexamples

For each of member_digest and root_map_digest:
- stage a membership-correct candidate;
- deliberately corrupt the stored digest;
- record that the Python Oracle rejects;
- use direct D3 SQL to set generation and Manifest status to published and insert the initial pointer;
- prove the pointer advances despite the wrong digest.

This is the raw behavior being closed; it is not inferred from the Python Oracle.

## New protections and success controls

The D4-S4 SQL adds:
- a research-only authority clock and immutable Manifest/root/source evidence seal;
- guards on published scan-run terminal evidence;
- guarded source semantic changes with mandatory source_revision step and authority invalidation;
- root ownership/Watcher invalidation while keeping display_name writable;
- immutable referenced physical identity epochs and source claims;
- append-only claim revocation with source_revision and authority clock change;
- DML publication capability guards on generation/Manifest status and pointer insert/update;
- publisher capability guard on current files.path/name changes;
- an authority-filtered view that compares seal/source/root/runtime evidence versions;
- no changes to user context, user_tags, file_user_tags or independent saved-view metadata.

Tests verify:
- valid trusted initial publication creates root/member/source evidence counts and hashes;
- invalid digest calls fail before seal/status/pointer and leave ready/complete/absent-pointer state;
- raw status SQL on the compatible connection fails with capability false;
- raw connection without the UDF fails with “no such function”;
- direct transitive evidence DML rejects or rolls back;
- authorized source, root, Watcher and claim-revocation changes advance authority revision and remove rows from the current authority view while leaving the pointer and historical seal intact;
- a candidate prepared before a Watcher change fails final preflight, leaves old pointer/old Manifest intact and leaves the successor ready but unsealed;
- user file context and tag association writes remain available without authority revision changes;
- foreign_key_check, integrity_check and FTS5 external-content integrity-check pass.

## Hash ownership and SQL scope

Python computes the exact SHA-256 row summaries after acquiring BEGIN IMMEDIATE. SQLite trigger SQL verifies relational set equality, identity joins, immutability and call authorization. No SQLite SHA-256 UDF is assumed or added. The connection-local publication authorization UDF returns only a private capability flag; it does not hash data.

## Not covered

No native Windows/macOS identity, symlink/Junction/ACL/TOCTOU verification, SQLite 3.51.3 FTS/BM25, large dataset, full CI, Hosted runners, production migration, arbitrary schema-owner attack, GC/long-lived readers, WAL growth or space exhaustion test was run. These remain later gates; no production GO follows from this research result.
