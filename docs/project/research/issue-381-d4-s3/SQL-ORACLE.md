# SQL Oracle and counterexample results

The SQL prototype is under `harness/`. `manifest-authority-oracle.sql` creates independent expected/actual views and missing/extra/invalid-identity views. `manifest-authority-protection.sql` adds disposable-database publication and immutability triggers. Neither file is production DDL.

## Independent comparison

The Oracle derives `expected` by joining selected `manifest_root_heads` to `source_generation_members` and then to the exact composite `(path_entry_id, source_id)` row in `file_path_entries`. It does not use `effective_file_sources`. The actual relation is read independently from `manifest_file_paths`.

For each candidate, `run.py::oracle_report` records:

- expected and actual set counts and deterministic SHA-256 summaries;
- `expected EXCEPT actual` and `actual EXCEPT expected`, including the full differing rows;
- each selected root's generation status, run coverage, declared/actual member count, and declared/recomputed member digest;
- the root-map digest and current-pointer parent check;
- source/path identity evidence failures and sources with more than one distinct selected path entry.

The publication trigger rechecks the two set differences and identity/path joins at the status transition. The application-side Oracle additionally recomputes SHA-256 summaries over the locked candidate rows. Pointer CAS then publishes the new manifest in the same write transaction.

## Original-model failures

The exact D4-S2 candidate SQL was run without D4-S3 protections first. It accepted each invalid Manifest:

| Case | Original candidate behavior | Oracle evidence |
|---|---|---|
| Omitted valid member | Published | Expected 2, actual 1; one missing tuple |
| Wrong projected path | Published | Same counts, one expected tuple missing and one wrong-path tuple extra |
| Extra phantom projection | Published | Expected 1, actual 2; one extra tuple, even though the effective view filtered it |
| Unverified `path_only` member | Published | Set counts equal, but identity Oracle reports an invalid path identity |
| Post-publication identity update | Update succeeded without pointer revision | Member digest/evidence no longer matches stored commitment |
| Post-publication member delete | Delete succeeded without pointer revision; visible list became empty | Expected 0, projection still 1; generation count/digest mismatch |
| Cross-transaction old Cursor | No revision validation; list continuation, detail lookup, and FTS term silently resolved against current rows | Token said old manifest/revision 1; pointer was new manifest/revision 2; old term matched 0, new term 1, while old manifest retained the old path |

These are reproduced in `evidence/manifest-authority-results.json`, not inferred from the proposed SQL.

## Remediation results

The protected prototype rejected and rolled back all four publication failures: omitted member, wrong path, extra projection, and unverified path identity. The pointer stayed absent and generation stayed complete on failed initial publication. The valid nested-root case reported one expected member and one actual projection because both roots selected the same source/path-entry tuple.

The DML guard rejected 16 attempted changes across the five named published-object tables and the pointer CAS. These include insert/update/delete attempts for each of `source_generation_members`, `source_generations`, `manifest_root_heads`, `manifest_file_paths`, and `visibility_manifests`, plus a direct published-generation insert and a non-CAS pointer revision. The active pointer remained `(manifest-a, 1)` and visibility remained unchanged.

## Deterministic digest encoding

Each row is represented as a JSON array. The encoding uses UTF-8, `ensure_ascii=false`, compact separators `,` and `:`, and no object key ordering dependency. Rows are ordered by their canonical encoded bytes; SHA-256 is then computed over the compact JSON array of those ordered row arrays. Empty set digest is SHA-256 of `[]`.

The generation summary hashes ordered `(source_id, path_entry_id, identity_digest)` rows. The Manifest root map hashes `(root_id, generation_id)` rows. Projection summaries hash `(source_id, path_entry_id, path_key, path_text)` rows. All output includes count plus digest so a count collision cannot pass equality on its own.

The SQL `EXCEPT` comparisons remain the publication authority for exact membership. SHA-256 summaries are independently recomputed in the application Oracle in the same write transaction; the trigger also checks stored-vs-actual member counts. SQLite has no built-in SHA-256 function in this runtime, so the prototype does not pretend the trigger itself validates the digest algorithm.

## FTS and query-plan probe

After a same-source rename, a pinned reader kept its old WAL snapshot and saw the old pointer and old FTS token. After commit, a fresh reader saw the new pointer and path; the old FTS token returned zero and the new token returned one. `file_path_entries` and the old Manifest retained the old path for history.

On SQLite 3.53.1, the small `MATCH 'txt'` query returned 2 raw `files_fts` rows and 1 active-manifest row. The EXPLAIN QUERY PLAN lines are recorded in the JSON output. The BM25 query ranks the raw external-content FTS corpus before the active visibility join, so hidden rows affect corpus statistics. This is a correctness/plan observation, not a performance benchmark. Exact product SQLite 3.51.3 was unavailable and remains a separate qualification gate.
