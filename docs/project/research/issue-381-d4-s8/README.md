# #381 D4-S8 — Read-only Consumer Coverage Reconciliation

**Status:** research/documentation only; no production D4 authorization.  
**Review target:** `#381-D4-S8 READY_FOR_OWNER_CONSUMER_COVERAGE_REVIEW` if the Owner accepts the row-level reconciliation.  
**Research parent:** D4-S7 `004fc6a671d11b8833ac734282f14f0732ca153e` / tree `3df87ad2701635510744c86ce3b185d6bc52a30e`.  
**Production source:** master `3275ac7c72df8ae4139a30f16c7f649971324e97` / tree `009576241cd4e8e2ba12cc644a368314f599faa6`, Schema 37.

## Contents

- [Consumer coverage reconciliation](CONSUMER-COVERAGE-RECONCILIATION.md): source classifications, 17-domain map, bypasses and trigger gaps.
- [Test contracts and gates](TEST-CONTRACT-AND-GATES.md): entry-level contract IDs and dependencies.
- [Migration and rollback dependencies](DEPENDENCY-AND-ROLLBACK-MATRIX.md): Schema 37 preservation risks and current #380 blockers.
- [FTS and Cursor admission](FTS-CURSOR-ADMISSION.md): exact next-stage inputs and Owner decisions required before measurement.
- `ledgers/`: row-level stale, write, direct `files` query, Tauri command and frontend consumer inventories.
- `tools/build_coverage_ledgers.py`: deterministic regeneration from the frozen S1 CSVs and the byte-identical production source tree.
- `evidence/`: exact input IDs, source inventory digests and generated ledger digests.

## Rebuild

Run from the repository root at this research branch:

```sh
python3 docs/project/research/issue-381-d4-s8/tools/build_coverage_ledgers.py
```

The builder first requires `src-tauri/src` to be identical to the fixed master object. It reads the S1 committed inventories, parses the 209 registered `generate_handler!` entries from that source, resolves each command definition, and emits the ledgers. It does not execute the application, parse Rust/SQL semantically, or claim dynamic SQL/call-graph completeness. Every generated row includes a unique ID, source location, domain classification, visibility treatment or explicit exemption, preservation rule, test contract and owner/track.

## Count reconciliation

| Inventory | S1/S7 recorded | S8 source-reconciled | Disposition |
|---|---:|---:|---|
| Stale-reference function/schema groups | 103 production/schema + 46 test | 103 production/schema + 46 test | All 149 groups and 262 exact source lines have IDs; Global Index and schema/DTO references are separated from managed-file readers. |
| Literal write function groups | 141 labeled production/schema + 95 test | 121 production/schema + 115 test | All 236 original groups remain identified. Twenty groups / 30 SQL lines were in `#[cfg(test)]` child modules and were mislabeled production by the S1 path heuristic. |
| Literal write references | 433 total | 210 production/schema + 223 test/fixture | All 433 exact lines have IDs and a parent function group. |
| Direct `FROM/JOIN files` references | not included in the 103-group stale scan | 99 groups / 125 lines; 54 production/schema groups / 72 lines, 45 test groups / 53 lines | Additional targeted query inventory for by-ID/path and reader paths; each line is linked to its function and contract. |
| Registered Tauri commands | summarized in S7 | 209 registered commands; 0 unresolved definitions | Every handler entry is mapped to a definition location and disposition. |
| Frontend stale consumers | 11 entries in S1 | 11 entries | Each component, DTO or browser mock is preserved with a concrete contract. |

The 20 write-group reclassifications are not dropped. The original S1 scope and each original reference remain in the ledger. The evidence and exact `#[cfg(test)]` parent declarations are documented in the reconciliation report.

`DOMAIN-TO-ENTRY-MAP.csv` maps all 17 S7 domains to row IDs. Its rows intentionally overlap because a command, trigger and repository function participate in multiple domains. The detailed per-row ledgers remain authoritative for the disposition.

## Limits

This phase does not run CI, hosted platform tests, product SQLite experiments, query-plan checks or benchmarks. It changes no production code or Schema, allocates no Schema version, and leaves master, PR #383 and PR #386 unchanged. D4 production remains **NO-GO** even if this coverage report is accepted.
