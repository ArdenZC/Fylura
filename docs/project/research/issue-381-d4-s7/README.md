# Zen Canvas #381 — D4-S7 Integration Readiness Research

This research branch is based on frozen D4-S6 HEAD `895acaa2f0afc0f2c3f554e799107fdd55b05eb4`, Tree `8770037a9f9428066532e00cc8218d3c1f8485c7`. The read-only production source audit is pinned to `master@3275ac7c72df8ae4139a30f16c7f649971324e97`, Tree `009576241cd4e8e2ba12cc644a368314f599faa6`, Schema 37.

This folder contains only the D4-S7 integration readiness report, a consumer matrix, a migration dependency graph, an FTS/Cursor validation contract, and input provenance. It does not change production code, production schema, master, PR #383, PR #386, installer or IPC contracts. It allocates no Schema version and authorizes no production implementation.

## Review documents

- [Integration readiness report](INTEGRATION-READINESS-REPORT.md) — production source facts, migration and rollback risks, #380 gates, and GO/NO-GO checklist.
- [Consumer matrix](CONSUMER-MATRIX.csv) — exact current production files/functions/SQL, call routes, proposed authority target, compatibility risks, and required tests.
- [Migration dependency graph](MIGRATION-DEPENDENCY-DAG.md) — required ordering and client/database generation barriers.
- [FTS and Cursor validation contract](FTS-CURSOR-VALIDATION-CONTRACT.md) — exact SQLite 3.51.3 correctness and future scale qualification. No large benchmark was run.
- [Input provenance](evidence/INPUTS.json) — frozen heads, source-audit artifacts, source hashes, and non-execution declarations.

## Reproduce the static audit

The audited master HEAD exactly matches the source baseline already captured by D4-S1. Its complete direct stale-reference and function-group inventories are therefore reused rather than regenerated:

```sh
python3 docs/project/research/issue-381-d4-s1/source-audit/build_inventory.py \
  3275ac7c72df8ae4139a30f16c7f649971324e97 \
  docs/project/research/issue-381-d4-s1/source-audit
python3 docs/project/research/issue-381-d4-s1/source-audit/build_write_inventory.py \
  3275ac7c72df8ae4139a30f16c7f649971324e97 \
  docs/project/research/issue-381-d4-s1/source-audit
```

The D4-S1 source audit records 262 literal `is_stale` references across 40 Rust files, 103 grouped production/schema entries, and a separate literal-SQL write inventory. The inventories are source-text heuristics, not Rust or SQL parsers. This report manually traces the relevant public commands, service calls, SQL triggers/views, frontend uses and independent authorities. Exact source references are tied to the pinned master SHA.

No tests, CI, Hosted Windows/macOS job, benchmark, Codex Review, PR/Issue mutation, merge, or release was run for D4-S7.

**D4 production remains NO-GO.** The requested endpoint is `#381-D4-S7 READY_FOR_OWNER_INTEGRATION_READINESS_REVIEW`.
