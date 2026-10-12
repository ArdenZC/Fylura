# #381 D4-S8R — Production Domain Traceability Reconciliation

**Status:** traceability research only; D4 production remains **NO-GO**.
**Frozen S8 input:** `4005073337f198a245773e30a76178749bfd2920` / tree `71e18895e9740f9a233c5cf56df3c723f626c727`.
**Production source audit pin:** master `3275ac7c72df8ae4139a30f16c7f649971324e97`.
**Scope:** this package adds new S8R files only. The 17 S8 files remain byte-identical and are not regenerated.

## Exact source-to-map coverage

The S8R production map is a filtered projection of the S8 domain map, with source `reconciled_scope` as the production/test boundary. The test/fixture rows go to a distinct regression lane. `evidence/ID-SET-PROOF.json` lists both complete sorted sets and reports missing/extra IDs; the validator independently recomputes them.

| Inventory | Frozen S8 source ledger | Source production | Source test/fixture | Prior S8 map: prod / test unique | Prior S8 test IDs omitted | S8R production map | S8R regression map |
|---|---|---:|---:|---:|---:|---:|---:|
| stale groups | `STALE-GROUP-LEDGER.csv` | 103 | 46 | 103 / 42 | 4 | 103 exact | 46 exact |
| write groups | `WRITE-FUNCTION-LEDGER.csv` | 121 | 115 | 121 / 115 | 0 | 121 exact | 115 exact |
| direct files-read groups | `FILES-READ-GROUP-LEDGER.csv` | 54 | 45 | 54 / 30 | 15 | 54 exact | 45 exact |
| registered Tauri commands | `TAURI-COMMAND-REGISTRY.csv` | — | — | 209 | — | 209 exact | — |
| frontend consumers | `FRONTEND-CONSUMER-LEDGER.csv` | 10 | 1 demo/mock | 11 | — | 10 production | 1 test/demo |
| domains | `DOMAIN-TO-ENTRY-MAP.csv` | — | — | 17 | — | 17 | separate REG-TEST lane |

The old S8 map had all 103/121/54 production IDs, but also carried 42 stale tests, 115 write tests and 30 direct-read tests. Four stale and fifteen read test groups were absent from that map; S8R lists all 46 and 45 source-ledger fixtures in the regression lane. The 121 production write groups retain S8's accepted source-scope reconciliation: 20 groups previously labeled production by the S1 path heuristic were identified in `#[cfg(test)]` modules and remain in the 115-row test/fixture set. S8R does not rerun or alter that classification. No source inventory counts were altered.

The production map retains the prior domain membership of every production ID. Cross-domain repeated references remain separate memberships; they represent multiple domain responsibilities, not duplicate source entries. Their exact production multiplicities are: stale groups 103 IDs across 136 domain memberships (31 IDs recur across domains); write groups 121 IDs / 277 memberships (95 recurring); direct reads 54 IDs / 88 memberships (23 recurring). Commands retain 209 IDs / 437 domain memberships (169 occur in more than one domain). D-16 retains all 209 commands as the cross-cutting IPC registration inventory; it is not a claim that every handler is a D4 implementation target.

## D-16-only command disposition

Exactly 40 registry IDs occur only in D-16 in the frozen S8 domain map. The row-level `D16-ONLY-COMMAND-TRACEABILITY.csv` preserves each command ID, registered symbol, registration location and definition location, and classifies each from its S8 source-registry classification and module.

| Actual owner category | Count | IDs |
|---|---:|---|
| Independent service/system capability | 8 | CMD-100, CMD-101, CMD-115, CMD-117, CMD-118, CMD-124, CMD-125, CMD-127 |
| Independent explicit File Workspace capability | 22 | CMD-168, CMD-169, CMD-170, CMD-171, CMD-172, CMD-174, CMD-175, CMD-176, CMD-177, CMD-178, CMD-179, CMD-180, CMD-181, CMD-182, CMD-183, CMD-184, CMD-186, CMD-187, CMD-188, CMD-190, CMD-191, CMD-192 |
| Other cross-track Storage Analyzer filesystem/operation-journal capability | 10 | CMD-198, CMD-199, CMD-200, CMD-202, CMD-203, CMD-204, CMD-205, CMD-206, CMD-207, CMD-208 |
| **Total** | **40** | |

The eight settings/runtime/app-control commands have no Managed File consumer evidenced by their registry classification and source module; S8R therefore excludes them from D4 visibility implementation. Where they access shared process/database state, #380 generation admission still applies.

The 22 `file_workspace::integration::commands` handlers are a separate explicit user-selected workspace flow. They are not silently treated as Managed File Library membership, and that classification does not waive authorization or native path/object identity checks. Any future route into Managed File actions must resolve the current source and Effective Visibility.

The ten Storage Analyzer commands belong to durable analysis findings and cleanup operation-journal flows, not Managed Scanner roots. They are cross-track filesystem operations. The `reveal_storage_candidate(path)` registered handler delegates a raw path to `reveal_in_folder` without binding that command argument to a durable finding revision in the handler. Cleanup moves and restore actions need run/finding or trash-item ownership, revision/identity checks and explicit authorization at execution. When a target overlaps a Managed File source, that integration must additionally validate current source identity and Effective Visibility. S8R flags these gates; it does not claim the existing runtime implementation has already passed them.

`TAURI-COMMAND-OWNERSHIP.csv` retains all 209 original IDs and definition locations, gives each the full S8 domain memberships, and distinguishes D-16 transport inventory from source-ledger implementation ownership. Domain co-membership is not represented as a function call edge.

The static validator resolves all 209 registered-command lines in the pinned production `main.rs` and all 209 definition lines in the pinned master source tree, matching the recorded handler symbols at each location. This confirms the ledger coordinates against the frozen source object; it does not validate runtime dispatch or permission checks.

## Coverage statement and limits

S8R establishes that every entry in the **frozen static inventories** is assigned to either the production map or the isolated regression map, and that the 40 D-16-only command registrations have an explicit source-grounded owner/disposition. Production/test ID equality is verified from S8 ledger fields; no IDs are re-created from scan counts.

The underlying S8 inventories used deterministic text/regular-expression heuristics and source-location reconciliation. They are not a complete Rust/TypeScript AST, SQL expansion, macro expansion, dynamic SQL inventory, interprocedural call graph, or runtime authorization test. Indirect calls and generated SQL remain outside this proof. The command ownership classifications are based on registered symbols, definition locations and S8 ledger classifications. Runtime native identity and authorization remain formal implementation/test gates, especially for File Workspace and Storage Analyzer path actions.

No S8 historical tests, CI, hosted jobs, benchmark, production code, Schema, master, PR #383 or PR #386 were changed or rerun by S8R. #380 database-generation isolation remains a blocking prerequisite; this package does not allocate Schema v38 or authorize D4-S9/production work.

## Reproduction

From repository root:

```sh
python3 docs/project/research/issue-381-d4-s8r/tools/generate_s8r_traceability.py
python3 docs/project/research/issue-381-d4-s8r/tools/validate_s8r_traceability.py
(cd docs/project/research/issue-381-d4-s8r && sha256sum -c evidence/SHA256SUMS)
```
