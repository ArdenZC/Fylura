#!/usr/bin/env python3
"""Generate the S8R production-only domain map from frozen D4-S8 ledgers.

This is a deterministic ledger transformation. It does not scan source or claim
AST, call-graph, SQL-expansion, or runtime authorization completeness.
"""
from __future__ import annotations

import csv
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
S8 = REPO / "docs/project/research/issue-381-d4-s8"
OUT = REPO / "docs/project/research/issue-381-d4-s8r"
BASE_HEAD = "4005073337f198a245773e30a76178749bfd2920"
BASE_TREE = "71e18895e9740f9a233c5cf56df3c723f626c727"
MASTER = "3275ac7c72df8ae4139a30f16c7f649971324e97"
DOMAIN_FIELDS = {
    "stale_group_ids": ("STALE-GROUP-LEDGER.csv", "group_id", "reconciled_scope"),
    "write_function_group_ids": ("WRITE-FUNCTION-LEDGER.csv", "group_id", "reconciled_scope"),
    "files_read_group_ids": ("FILES-READ-GROUP-LEDGER.csv", "group_id", "reconciled_scope"),
}
REGRESSION_FIELDS = [
    ("stale_reference_group", "STALE-GROUP-LEDGER.csv", "group_id", "reconciled_scope", "path", "function_or_schema_owner", "owner_line", "test_contract"),
    ("write_function_group", "WRITE-FUNCTION-LEDGER.csv", "group_id", "reconciled_scope", "path", "function_or_schema_owner", "owner_line", "test_contract"),
    ("files_read_group", "FILES-READ-GROUP-LEDGER.csv", "group_id", "reconciled_scope", "path", "function_or_module", "function_line", "test_contract"),
]

SERVICE_IDS = {"CMD-100", "CMD-101", "CMD-115", "CMD-117", "CMD-118", "CMD-124", "CMD-125", "CMD-127"}
FILE_WORKSPACE_IDS = {
    "CMD-168", "CMD-169", "CMD-170", "CMD-171", "CMD-172", "CMD-174", "CMD-175", "CMD-176", "CMD-177", "CMD-178", "CMD-179", "CMD-180", "CMD-181", "CMD-182", "CMD-183", "CMD-184", "CMD-186", "CMD-187", "CMD-188", "CMD-190", "CMD-191", "CMD-192"
}
STORAGE_ANALYZER_IDS = {"CMD-198", "CMD-199", "CMD-200", "CMD-202", "CMD-203", "CMD-204", "CMD-205", "CMD-206", "CMD-207", "CMD-208"}
D16_ONLY_SPECIAL = SERVICE_IDS | FILE_WORKSPACE_IDS | STORAGE_ANALYZER_IDS


def read_csv(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as f:
        return list(csv.DictReader(f))


def write_csv(path: Path, rows: list[dict[str, str]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if not rows:
        raise ValueError(f"refusing empty CSV: {path}")
    with path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def id_list(value: str) -> list[str]:
    return [value for value in value.split(";") if value]


def id_key(value: str) -> tuple[str, int]:
    head, _, suffix = value.rpartition("-")
    return head, int(suffix or 0)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def grouped_domain_memberships(domain_rows: list[dict[str, str]], field: str) -> dict[str, list[str]]:
    result: dict[str, list[str]] = defaultdict(list)
    for row in domain_rows:
        for entry_id in id_list(row[field]):
            result[entry_id].append(row["domain_id"])
    return dict(result)


def owner_details(command: dict[str, str], memberships: list[str]) -> tuple[str, str, str, str]:
    command_id = command["command_id"]
    symbol = command["registered_symbol"]
    definition = command["definition"]
    if command_id in SERVICE_IDS:
        if command_id in {"CMD-100", "CMD-101"}:
            owner = "Settings service"
        elif command_id == "CMD-115":
            owner = "Runtime capabilities service"
        else:
            owner = "Application control service"
        return (
            "independent service/system capability",
            owner,
            "No Managed File consumer is identified by this command's registry classification and source module. Exclude it from D4 Effective Visibility implementation. Apply #380 process/database-generation admission wherever the service opens shared state.",
            "The registry definition and module classification are the basis; no runtime call-path claim is made.",
        )
    if command_id in FILE_WORKSPACE_IDS:
        return (
            "independent File Workspace capability",
            "File Workspace integration service",
            "This is an explicit user-selected workspace flow, not Managed File Library membership. Preserve its own request/window authorization and native path/object identity checks. Any operation that crosses into a Managed File action must resolve current source identity and Effective Visibility; File Workspace classification is not a blanket security exemption.",
            f"Registered definition {definition}; symbol {symbol}. S8R static mapping does not prove every native identity check or runtime authorization path.",
        )
    if command_id in STORAGE_ANALYZER_IDS:
        if command_id == "CMD-202":
            treatment = "Cross-track Storage Analyzer reveal operation. The registered handler accepts a path and delegates directly to file_ops::reveal_in_folder; the command entry does not bind a durable finding ID/revision. Require an explicit admitted cleanup scope and current native target identity before reveal; if the target is a Managed File, also resolve its current source and Effective Visibility."
        elif command_id == "CMD-205":
            treatment = "Cross-track destructive Storage Analyzer cleanup. Preserve approved-cleanup run/finding selection, finding revision CAS, physical identity recheck, safe-trash authorization, and operation journal. If a target is also a Managed File, require current source and Effective Visibility before mutation."
        elif command_id == "CMD-208":
            treatment = "Cross-track destructive Storage Analyzer restore. Preserve trash-item ownership and operation-journal authorization; validate destination identity/collision at execution time. If the destination is also a Managed File, require current source and Effective Visibility before mutation."
        elif command_id in {"CMD-203", "CMD-204"}:
            treatment = "Cross-track Storage Analyzer preview. Keep selection bound to the durable run/finding revision and current target identity; do not infer Managed File membership from path equality."
        elif command_id in {"CMD-206", "CMD-207"}:
            treatment = "Cross-track Storage Analyzer trash-history/restore preview. Preserve operation-journal identity and history; a later restore must revalidate destination identity and authorization."
        elif command_id == "CMD-198":
            treatment = "Cross-track Storage Analyzer scan admission. Preserve explicitly approved cleanup roots and run ownership; this is not a Managed Scanner root generation."
        else:
            treatment = "Cross-track Storage Analyzer run/finding read. Keep its independent durable run and finding authority; history remains readable without treating a path as a Managed File identity."
        return (
            "other cross-track filesystem operation",
            "Storage Analyzer + cleanup operation journal",
            treatment,
            f"Registered definition {definition}; symbol {symbol}; owner track {command['responsible_module_or_track']}. S8R does not assert runtime/native identity completeness.",
        )
    if "D-16" not in memberships:
        raise ValueError(f"unexpected command classification path: {command_id}")
    other_domains = ";".join(d for d in memberships if d != "D-16")
    return (
        "source-ledger domain owner; IPC registration is cross-cutting",
        command["responsible_module_or_track"],
        "Keep the source-ledger visibility/exemption contract in the mapped domain(s). D-16 records the IPC registration surface, not a call-graph edge or an independent D4 implementation owner. The exact consumer path still requires the formal source-level tests named in the ledgers.",
        f"Mapped inventory domains: {other_domains}. Domain co-membership is not proof that this command directly calls every entry in those domains.",
    )


def main() -> None:
    domain_rows = read_csv(S8 / "ledgers/DOMAIN-TO-ENTRY-MAP.csv")
    stale = read_csv(S8 / "ledgers/STALE-GROUP-LEDGER.csv")
    writes = read_csv(S8 / "ledgers/WRITE-FUNCTION-LEDGER.csv")
    reads = read_csv(S8 / "ledgers/FILES-READ-GROUP-LEDGER.csv")
    commands = read_csv(S8 / "ledgers/TAURI-COMMAND-REGISTRY.csv")
    frontend = read_csv(S8 / "ledgers/FRONTEND-CONSUMER-LEDGER.csv")
    required_counts = {
        "stale source": (len(stale), 149),
        "production stale": (sum(r["reconciled_scope"] == "production/schema" for r in stale), 103),
        "write source": (len(writes), 236),
        "production writes": (sum(r["reconciled_scope"] == "production/schema" for r in writes), 121),
        "direct-read source": (len(reads), 99),
        "production direct reads": (sum(r["reconciled_scope"] == "production/schema" for r in reads), 54),
        "registered commands": (len(commands), 209),
        "frontend consumers": (len(frontend), 11),
        "domain rows": (len(domain_rows), 17),
    }
    changed_counts = {name: pair for name, pair in required_counts.items() if pair[0] != pair[1]}
    if changed_counts:
        raise ValueError(f"frozen S8 count contract changed: {changed_counts}")
    source_rows = {
        "stale_group_ids": stale,
        "write_function_group_ids": writes,
        "files_read_group_ids": reads,
    }
    source_by_id: dict[str, dict[str, dict[str, str]]] = {}
    for field, (_, id_field, _) in DOMAIN_FIELDS.items():
        source_by_id[field] = {row[id_field]: row for row in source_rows[field]}
    frontend_by_id = {row["consumer_id"]: row for row in frontend}

    production_rows: list[dict[str, str]] = []
    prior_memberships: dict[str, dict[str, list[str]]] = {}
    for field in DOMAIN_FIELDS:
        prior_memberships[field] = grouped_domain_memberships(domain_rows, field)
    prior_frontend_memberships = grouped_domain_memberships(domain_rows, "frontend_consumer_ids")
    prior_command_memberships = grouped_domain_memberships(domain_rows, "registered_command_ids")

    for row in domain_rows:
        out = dict(row)
        for field, (_, id_field, scope_field) in DOMAIN_FIELDS.items():
            out[field] = ";".join(
                entry_id
                for entry_id in id_list(row[field])
                if source_by_id[field][entry_id][scope_field] == "production/schema"
            )
        out["frontend_consumer_ids"] = ";".join(
            entry_id
            for entry_id in id_list(row["frontend_consumer_ids"])
            if "test/demo mock" not in frontend_by_id[entry_id]["classification"]
        )
        out["mapping_semantics"] = (
            "D-16 is the complete registered IPC-command inventory and transport surface; it does not by itself assign all handlers to D4. See TAURI-COMMAND-OWNERSHIP.csv and D16-ONLY-COMMAND-TRACEABILITY.csv for actual owner/disposition, especially the 40 commands with no other S8 domain membership."
            if row["domain_id"] == "D-16"
            else "Production/schema IDs only for stale, write and direct-files-read groups. Cross-domain references are preserved as separate responsibilities; the test/demo mock consumer is kept only in the regression map."
        )
        production_rows.append(out)
    write_csv(OUT / "PRODUCTION-DOMAIN-ENTRY-MAP.csv", production_rows)

    regression_rows: list[dict[str, str]] = []
    for kind, filename, id_field, scope_field, path_field, owner_field, line_field, test_field in REGRESSION_FIELDS:
        source = read_csv(S8 / "ledgers" / filename)
        field = {
            "stale_reference_group": "stale_group_ids",
            "write_function_group": "write_function_group_ids",
            "files_read_group": "files_read_group_ids",
        }[kind]
        memberships = prior_memberships[field]
        for row in source:
            if row[scope_field] != "test/fixture":
                continue
            entry_id = row[id_field]
            domains = memberships.get(entry_id, [])
            regression_rows.append({
                "regression_lane": "REG-TEST",
                "source_kind": kind,
                "entry_id": entry_id,
                "source_scope": row[scope_field],
                "source_path": row[path_field],
                "source_owner": row[owner_field],
                "owner_line": row[line_field],
                "prior_s8_domain_memberships": ";".join(domains),
                "prior_s8_domain_map_status": "mapped" if domains else "not mapped in frozen S8 Domain Map",
                "required_test_contract": row[test_field],
                "treatment": "Regression-only fixture; excluded from production migration/visibility mapping.",
            })
    for row in frontend:
        if "test/demo mock" in row["classification"]:
            domains = prior_frontend_memberships.get(row["consumer_id"], [])
            regression_rows.append({
                "regression_lane": "REG-TEST",
                "source_kind": "frontend_consumer",
                "entry_id": row["consumer_id"],
                "source_scope": "test/demo mock",
                "source_path": row["source"],
                "source_owner": row["current_use"],
                "owner_line": "",
                "prior_s8_domain_memberships": ";".join(domains),
                "prior_s8_domain_map_status": "mapped" if domains else "not mapped in frozen S8 Domain Map",
                "required_test_contract": row["required_test_contract"],
                "treatment": "Test/demo mock only; keep synchronized with production identity/visibility contracts, but exclude from production migration mapping.",
            })
    regression_rows.sort(key=lambda row: (row["source_kind"], id_key(row["entry_id"])))
    write_csv(OUT / "REGRESSION-ENTRY-MAP.csv", regression_rows)

    owner_rows: list[dict[str, str]] = []
    for command in commands:
        command_id = command["command_id"]
        memberships = prior_command_memberships.get(command_id, [])
        if not memberships:
            raise ValueError(f"registered command is absent from S8 domain mapping: {command_id}")
        owner_class, owner, treatment, evidence = owner_details(command, memberships)
        out = dict(command)
        out.update({
            "s8_domain_memberships": ";".join(memberships),
            "d16_only": "yes" if memberships == ["D-16"] else "no",
            "actual_owner_class": owner_class,
            "actual_owner": owner,
            "d4_scope_and_boundary": treatment,
            "ownership_evidence_and_limit": evidence,
        })
        owner_rows.append(out)
    write_csv(OUT / "TAURI-COMMAND-OWNERSHIP.csv", owner_rows)
    d16_only_rows = [row for row in owner_rows if row["d16_only"] == "yes"]
    if {row["command_id"] for row in d16_only_rows} != D16_ONLY_SPECIAL:
        raise ValueError(
            "D16-only command set differs from explicit owner classifications: "
            f"expected={sorted(D16_ONLY_SPECIAL)} got={sorted(row['command_id'] for row in d16_only_rows)}"
        )
    write_csv(OUT / "D16-ONLY-COMMAND-TRACEABILITY.csv", d16_only_rows)

    # Exact source-vs-map set proof, including every test/fixture row in a
    # separate regression lane and the FE-011 test/demo mock.
    proof: dict[str, object] = {
        "baseline": {"s8_head": BASE_HEAD, "s8_tree": BASE_TREE, "production_master": MASTER},
        "areas": {},
    }
    for field, (filename, id_field, scope_field) in DOMAIN_FIELDS.items():
        source = source_rows[field]
        expected_prod = sorted((r[id_field] for r in source if r[scope_field] == "production/schema"), key=id_key)
        mapped_prod = sorted({entry_id for row in production_rows for entry_id in id_list(row[field])}, key=id_key)
        expected_test = sorted((r[id_field] for r in source if r[scope_field] == "test/fixture"), key=id_key)
        mapped_test = sorted((r["entry_id"] for r in regression_rows if r["source_kind"] == {
            "stale_group_ids": "stale_reference_group",
            "write_function_group_ids": "write_function_group",
            "files_read_group_ids": "files_read_group",
        }[field]), key=id_key)
        proof["areas"][field] = {
            "source_ledger": f"ledgers/{filename}",
            "production_expected_count": len(expected_prod),
            "production_expected_ids": expected_prod,
            "production_mapped_count": len(mapped_prod),
            "production_mapped_ids": mapped_prod,
            "production_missing_ids": sorted(set(expected_prod) - set(mapped_prod), key=id_key),
            "production_extra_ids": sorted(set(mapped_prod) - set(expected_prod), key=id_key),
            "production_id_set_sha256": sha256(("\n".join(expected_prod) + "\n").encode()),
            "test_expected_count": len(expected_test),
            "test_expected_ids": expected_test,
            "test_mapped_count": len(mapped_test),
            "test_mapped_ids": mapped_test,
            "test_missing_ids": sorted(set(expected_test) - set(mapped_test), key=id_key),
            "test_extra_ids": sorted(set(mapped_test) - set(expected_test), key=id_key),
            "test_id_set_sha256": sha256(("\n".join(expected_test) + "\n").encode()),
        }
    command_ids = sorted((row["command_id"] for row in commands), key=id_key)
    mapped_commands = sorted((row["command_id"] for row in owner_rows), key=id_key)
    frontend_prod = sorted((r["consumer_id"] for r in frontend if "test/demo mock" not in r["classification"]), key=id_key)
    frontend_test = sorted((r["consumer_id"] for r in frontend if "test/demo mock" in r["classification"]), key=id_key)
    proof["commands"] = {
        "registered_count": len(command_ids), "registered_ids": command_ids,
        "mapped_count": len(mapped_commands), "mapped_ids": mapped_commands,
        "missing_ids": sorted(set(command_ids)-set(mapped_commands), key=id_key),
        "extra_ids": sorted(set(mapped_commands)-set(command_ids), key=id_key),
        "d16_only_count": len(d16_only_rows),
        "d16_only_ids": sorted((r["command_id"] for r in d16_only_rows), key=id_key),
        "d16_only_owner_class_counts": dict(sorted(Counter(r["actual_owner_class"] for r in d16_only_rows).items())),
    }
    prod_front_map = sorted({i for row in production_rows for i in id_list(row["frontend_consumer_ids"])}, key=id_key)
    test_front_map = sorted((r["entry_id"] for r in regression_rows if r["source_kind"] == "frontend_consumer"), key=id_key)
    proof["frontend"] = {
        "source_count": len(frontend), "source_ids": sorted((r["consumer_id"] for r in frontend), key=id_key),
        "production_count": len(frontend_prod), "production_ids": frontend_prod,
        "production_mapped_ids": prod_front_map,
        "test_count": len(frontend_test), "test_ids": frontend_test,
        "test_mapped_ids": test_front_map,
    }
    (OUT / "evidence/ID-SET-PROOF.json").write_text(json.dumps(proof, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    # Inputs are byte-pinned to the frozen S8 tree. This records ledger hashes
    # without changing or regenerating any S8 file.
    input_paths = [
        "ledgers/DOMAIN-TO-ENTRY-MAP.csv", "ledgers/STALE-GROUP-LEDGER.csv",
        "ledgers/WRITE-FUNCTION-LEDGER.csv", "ledgers/FILES-READ-GROUP-LEDGER.csv",
        "ledgers/TAURI-COMMAND-REGISTRY.csv", "ledgers/FRONTEND-CONSUMER-LEDGER.csv",
        "evidence/SHA256SUMS",
    ]
    inputs = {
        "s8_head": BASE_HEAD,
        "s8_tree": BASE_TREE,
        "production_master": MASTER,
        "input_files": {path: sha256((S8 / path).read_bytes()) for path in input_paths},
        "frozen_counts": {"domains": len(domain_rows), "production_stale_groups": 103, "production_write_groups": 121, "production_files_read_groups": 54, "registered_commands": 209, "frontend_consumers": 11},
        "scope_note": "Generated from committed S8 CSV ledgers only. The independent ID-set proof compares exact source-scope sets to the S8R maps; this is not a fresh source scan.",
    }
    (OUT / "evidence/INPUTS.json").write_text(json.dumps(inputs, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    # Compute summary values for human-readable report.
    old_unique: dict[str, tuple[int, int, int]] = {}
    for field, (_, id_field, scope_field) in DOMAIN_FIELDS.items():
        all_ids = {row[id_field] for row in source_rows[field]}
        prod_ids = {row[id_field] for row in source_rows[field] if row[scope_field] == "production/schema"}
        test_ids = all_ids - prod_ids
        mapped = prior_memberships[field]
        old_unique[field] = (len(set(mapped) & prod_ids), len(set(mapped) & test_ids), len(test_ids - set(mapped)))
    prod_occurrences = {
        field: sum(len(id_list(row[field])) for row in production_rows)
        for field in DOMAIN_FIELDS
    }
    old_occurrences = {
        field: sum(len(id_list(row[field])) for row in domain_rows)
        for field in DOMAIN_FIELDS
    }
    production_overlap = {}
    for field in DOMAIN_FIELDS:
        membership_counts = Counter(
            entry_id
            for row in production_rows
            for entry_id in id_list(row[field])
        )
        production_overlap[field] = {
            "unique_ids": len(membership_counts),
            "domain_memberships": sum(membership_counts.values()),
            "ids_in_multiple_domains": sum(count > 1 for count in membership_counts.values()),
            "additional_domain_memberships": sum(count - 1 for count in membership_counts.values()),
        }
    command_membership_counts = Counter(
        command_id
        for row in production_rows
        for command_id in id_list(row["registered_command_ids"])
    )
    production_overlap["registered_command_ids"] = {
        "unique_ids": len(command_membership_counts),
        "domain_memberships": sum(command_membership_counts.values()),
        "ids_in_multiple_domains": sum(count > 1 for count in command_membership_counts.values()),
        "additional_domain_memberships": sum(count - 1 for count in command_membership_counts.values()),
    }
    old_front_test = [r for r in frontend if "test/demo mock" in r["classification"]]
    s8_files = sorted(p for p in OUT.rglob("*") if p.is_file() and p.name != "SHA256SUMS")
    sums_lines = [f"{sha256(p.read_bytes())}  {p.relative_to(OUT).as_posix()}" for p in s8_files]
    (OUT / "evidence/SHA256SUMS").write_text("\n".join(sums_lines) + "\n", encoding="utf-8")

    report = f'''# #381 D4-S8R — Production Domain Traceability Reconciliation

**Status:** traceability research only; D4 production remains **NO-GO**.
**Frozen S8 input:** `{BASE_HEAD}` / tree `{BASE_TREE}`.
**Production source audit pin:** master `{MASTER}`.
**Scope:** this package adds new S8R files only. The 17 S8 files remain byte-identical and are not regenerated.

## Exact source-to-map coverage

The S8R production map is a filtered projection of the S8 domain map, with source `reconciled_scope` as the production/test boundary. The test/fixture rows go to a distinct regression lane. `evidence/ID-SET-PROOF.json` lists both complete sorted sets and reports missing/extra IDs; the validator independently recomputes them.

| Inventory | Frozen S8 source ledger | Source production | Source test/fixture | Prior S8 map: prod / test unique | Prior S8 test IDs omitted | S8R production map | S8R regression map |
|---|---|---:|---:|---:|---:|---:|---:|
| stale groups | `{DOMAIN_FIELDS['stale_group_ids'][0]}` | 103 | 46 | {old_unique['stale_group_ids'][0]} / {old_unique['stale_group_ids'][1]} | {old_unique['stale_group_ids'][2]} | 103 exact | 46 exact |
| write groups | `{DOMAIN_FIELDS['write_function_group_ids'][0]}` | 121 | 115 | {old_unique['write_function_group_ids'][0]} / {old_unique['write_function_group_ids'][1]} | {old_unique['write_function_group_ids'][2]} | 121 exact | 115 exact |
| direct files-read groups | `{DOMAIN_FIELDS['files_read_group_ids'][0]}` | 54 | 45 | {old_unique['files_read_group_ids'][0]} / {old_unique['files_read_group_ids'][1]} | {old_unique['files_read_group_ids'][2]} | 54 exact | 45 exact |
| registered Tauri commands | `TAURI-COMMAND-REGISTRY.csv` | — | — | 209 | — | 209 exact | — |
| frontend consumers | `FRONTEND-CONSUMER-LEDGER.csv` | {len(frontend)-len(old_front_test)} | {len(old_front_test)} demo/mock | 11 | — | {len(frontend)-len(old_front_test)} production | {len(old_front_test)} test/demo |
| domains | `DOMAIN-TO-ENTRY-MAP.csv` | — | — | 17 | — | 17 | separate REG-TEST lane |

The old S8 map had all 103/121/54 production IDs, but also carried 42 stale tests, 115 write tests and 30 direct-read tests. Four stale and fifteen read test groups were absent from that map; S8R lists all 46 and 45 source-ledger fixtures in the regression lane. The 121 production write groups retain S8's accepted source-scope reconciliation: 20 groups previously labeled production by the S1 path heuristic were identified in `#[cfg(test)]` modules and remain in the 115-row test/fixture set. S8R does not rerun or alter that classification. No source inventory counts were altered.

The production map retains the prior domain membership of every production ID. Cross-domain repeated references remain separate memberships; they represent multiple domain responsibilities, not duplicate source entries. Their exact production multiplicities are: stale groups {production_overlap['stale_group_ids']['unique_ids']} IDs across {production_overlap['stale_group_ids']['domain_memberships']} domain memberships ({production_overlap['stale_group_ids']['ids_in_multiple_domains']} IDs recur across domains); write groups {production_overlap['write_function_group_ids']['unique_ids']} IDs / {production_overlap['write_function_group_ids']['domain_memberships']} memberships ({production_overlap['write_function_group_ids']['ids_in_multiple_domains']} recurring); direct reads {production_overlap['files_read_group_ids']['unique_ids']} IDs / {production_overlap['files_read_group_ids']['domain_memberships']} memberships ({production_overlap['files_read_group_ids']['ids_in_multiple_domains']} recurring). Commands retain {production_overlap['registered_command_ids']['unique_ids']} IDs / {production_overlap['registered_command_ids']['domain_memberships']} domain memberships ({production_overlap['registered_command_ids']['ids_in_multiple_domains']} occur in more than one domain). D-16 retains all 209 commands as the cross-cutting IPC registration inventory; it is not a claim that every handler is a D4 implementation target.

## D-16-only command disposition

Exactly 40 registry IDs occur only in D-16 in the frozen S8 domain map. The row-level `D16-ONLY-COMMAND-TRACEABILITY.csv` preserves each command ID, registered symbol, registration location and definition location, and classifies each from its S8 source-registry classification and module.

| Actual owner category | Count | IDs |
|---|---:|---|
| Independent service/system capability | 8 | {', '.join(sorted(SERVICE_IDS, key=id_key))} |
| Independent explicit File Workspace capability | 22 | {', '.join(sorted(FILE_WORKSPACE_IDS, key=id_key))} |
| Other cross-track Storage Analyzer filesystem/operation-journal capability | 10 | {', '.join(sorted(STORAGE_ANALYZER_IDS, key=id_key))} |
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
'''
    (OUT / "COVERAGE-RECONCILIATION.md").write_text(report, encoding="utf-8")
    readme = '''# #381 D4-S8R — Traceability-only correction

This package corrects the production/test boundary in the D4-S8 domain map without changing any frozen S8 file or source count.

- `PRODUCTION-DOMAIN-ENTRY-MAP.csv`: the same 17 domains, with production/schema stale, write and direct `files`-read IDs only; all 209 registered command IDs remain inventoried; the FE-011 browser mock is excluded.
- `REGRESSION-ENTRY-MAP.csv`: every stale/write/read test or fixture ID from the frozen ledgers plus the FE-011 test/demo consumer.
- `TAURI-COMMAND-OWNERSHIP.csv`: all 209 command IDs and original registration/definition locations, domain memberships, ownership and visibility boundary.
- `D16-ONLY-COMMAND-TRACEABILITY.csv`: the exact 40 commands mapped only to D-16 in frozen S8, classified by source module and registry metadata.
- `evidence/ID-SET-PROOF.json`: exact source and map ID arrays, counts, missing/extra checks and deterministic set digests.
- `tools/`: deterministic ledger projection and independent static validator.
- `evidence/INPUTS.json`, `evidence/SHA256SUMS`: frozen inputs and checksums of all S8R files other than the checksum manifest itself.

D-16 is an IPC registration/transport surface, not a blanket D4 implementation owner. Consult the reconciliation report for the 40 handler dispositions and limits. D4 production remains NO-GO; #380 database-generation isolation is still a prerequisite.
'''
    (OUT / "README.md").write_text(readme, encoding="utf-8")

    # Recompute once so README/report, generator, evidence, CSVs are all sealed.
    s8_files = sorted(p for p in OUT.rglob("*") if p.is_file() and p.name != "SHA256SUMS")
    sums_lines = [f"{sha256(p.read_bytes())}  {p.relative_to(OUT).as_posix()}" for p in s8_files]
    (OUT / "evidence/SHA256SUMS").write_text("\n".join(sums_lines) + "\n", encoding="utf-8")
    print(json.dumps({
        "s8_base": BASE_HEAD,
        "domains": len(domain_rows),
        "production_counts": {"stale": 103, "write": 121, "direct_files_read": 54},
        "regression_counts": {"stale": 46, "write": 115, "direct_files_read": 45, "frontend_demo": len(old_front_test)},
        "commands": len(commands),
        "d16_only": {"total": len(d16_only_rows), "service_system": len(SERVICE_IDS), "file_workspace": len(FILE_WORKSPACE_IDS), "storage_analyzer_cross_track": len(STORAGE_ANALYZER_IDS)},
        "sha256_manifest_entries": len(s8_files),
    }, indent=2))


if __name__ == "__main__":
    main()
