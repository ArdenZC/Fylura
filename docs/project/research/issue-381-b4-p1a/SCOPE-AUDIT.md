# Scope audit

The source commit was created from the exact Schema 38 `master` base recorded in [source-provenance.json](evidence/final/source-provenance.json). Its eight changed paths are all beneath `tests/issue381_b4_p1a_f1/`. The only other changed paths are beneath `docs/project/research/issue-381-b4-p1a/`, the evidence directory allowed for this task.

The test harness uses synthetic fixtures and disposable temporary SQLite files. It does not use the application database. The schema-38 sentinel and product files are unchanged. No production source, schema, migration, workflow, CI gate, release file, or lockfile outside the isolated harness is changed.

The machine-readable path inventory and allowlist result are in [evidence/final/scope-audit.json](evidence/final/scope-audit.json). The isolated source patch is [evidence/final/source-harness.patch](evidence/final/source-harness.patch), with its digest in `source-provenance.json`. The final complete branch diff, exact PR HEAD/tree, and GitHub CI run IDs are recorded in the Draft PR description after natural PR checks complete.
