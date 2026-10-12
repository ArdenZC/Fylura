#!/usr/bin/env python3
"""Independently validate S8R set coverage, provenance, scope and checksums."""
from __future__ import annotations

import csv
import hashlib
import json
import subprocess
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
REGRESSION_KIND = {
    "stale_group_ids": "stale_reference_group",
    "write_function_group_ids": "write_function_group",
    "files_read_group_ids": "files_read_group",
}
SERVICE_IDS = {"CMD-100", "CMD-101", "CMD-115", "CMD-117", "CMD-118", "CMD-124", "CMD-125", "CMD-127"}
FILE_WORKSPACE_IDS = {
    "CMD-168", "CMD-169", "CMD-170", "CMD-171", "CMD-172", "CMD-174", "CMD-175", "CMD-176", "CMD-177", "CMD-178", "CMD-179", "CMD-180", "CMD-181", "CMD-182", "CMD-183", "CMD-184", "CMD-186", "CMD-187", "CMD-188", "CMD-190", "CMD-191", "CMD-192"
}
STORAGE_ANALYZER_IDS = {"CMD-198", "CMD-199", "CMD-200", "CMD-202", "CMD-203", "CMD-204", "CMD-205", "CMD-206", "CMD-207", "CMD-208"}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def git(*args: str, check: bool = True) -> str:
    result = subprocess.run(["git", "-C", str(REPO), *args], check=check, capture_output=True, text=True)
    if check:
        return result.stdout.strip()
    return result.stdout.strip()


def git_bytes(*args: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(REPO), *args])


def read_csv(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as f:
        return list(csv.DictReader(f))


def id_list(value: str) -> list[str]:
    return [item for item in value.split(";") if item]


def id_key(value: str) -> tuple[str, int]:
    head, _, suffix = value.rpartition("-")
    return head, int(suffix or 0)


def memberships(rows: list[dict[str, str]], field: str) -> dict[str, list[str]]:
    result: dict[str, list[str]] = defaultdict(list)
    for row in rows:
        for entry_id in id_list(row[field]):
            result[entry_id].append(row["domain_id"])
    return dict(result)


def check_source_and_frozen_s8() -> tuple[str, str]:
    tree = git("rev-parse", f"{BASE_HEAD}^{{tree}}")
    require(tree == BASE_TREE, f"S8 tree changed: {tree}")
    head = git("rev-parse", "HEAD")
    branch = git("symbolic-ref", "--short", "HEAD")
    require(branch == "research/issue-381-d4-s8r", f"unexpected branch: {branch}")
    ancestor = subprocess.run(["git", "-C", str(REPO), "merge-base", "--is-ancestor", BASE_HEAD, head], check=False)
    require(ancestor.returncode == 0, "S8 baseline is not an ancestor of S8R HEAD")
    s8_paths = git("ls-tree", "-r", "--name-only", BASE_HEAD, "docs/project/research/issue-381-d4-s8").splitlines()
    require(len(s8_paths) == 17, f"frozen S8 file count changed: {len(s8_paths)}")
    for path in s8_paths:
        committed = git_bytes("show", f"{BASE_HEAD}:{path}")
        local = (REPO / path).read_bytes()
        require(local == committed, f"frozen S8 working file changed: {path}")
        final = subprocess.run(["git", "-C", str(REPO), "show", f"{head}:{path}"], capture_output=True)
        if final.returncode == 0:
            require(final.stdout == committed, f"S8R commit changed frozen S8 file: {path}")
    changed = git("diff", "--name-only", BASE_HEAD, head).splitlines()
    outside = [path for path in changed if not path.startswith("docs/project/research/issue-381-d4-s8r/")]
    require(not outside, f"S8R diff changes files outside new research directory: {outside}")
    production_diff = subprocess.run(["git", "-C", str(REPO), "diff", "--quiet", BASE_HEAD, head, "--", "src-tauri"], check=False)
    require(production_diff.returncode == 0, "S8R changes production/Schema files under src-tauri")
    return head, branch


def check_maps() -> dict[str, object]:
    original = read_csv(S8 / "ledgers/DOMAIN-TO-ENTRY-MAP.csv")
    production = read_csv(OUT / "PRODUCTION-DOMAIN-ENTRY-MAP.csv")
    regression = read_csv(OUT / "REGRESSION-ENTRY-MAP.csv")
    require(len(original) == len(production) == 17, "production map must retain exactly the 17 S8 domains")
    require([r["domain_id"] for r in production] == [r["domain_id"] for r in original], "production domain IDs/order differ from frozen S8")
    old_members: dict[str, dict[str, list[str]]] = {field: memberships(original, field) for field in DOMAIN_FIELDS}
    source_rows: dict[str, list[dict[str, str]]] = {}
    source_by_id: dict[str, dict[str, dict[str, str]]] = {}
    result_counts = {}
    proof = json.loads((OUT / "evidence/ID-SET-PROOF.json").read_text(encoding="utf-8"))
    proof_areas = proof["areas"]
    for field, (filename, id_field, scope_field) in DOMAIN_FIELDS.items():
        rows = read_csv(S8 / "ledgers" / filename)
        source_rows[field] = rows
        source_by_id[field] = {r[id_field]: r for r in rows}
        expected_prod = {r[id_field] for r in rows if r[scope_field] == "production/schema"}
        expected_test = {r[id_field] for r in rows if r[scope_field] == "test/fixture"}
        require(len(expected_prod) == {"stale_group_ids": 103, "write_function_group_ids": 121, "files_read_group_ids": 54}[field], f"unexpected production count for {field}")
        output_by_domain = {r["domain_id"]: r for r in production}
        actual_prod_by_domain = {r["domain_id"]: id_list(r[field]) for r in production}
        expected_prod_by_domain = {
            row["domain_id"]: [entry_id for entry_id in id_list(row[field]) if source_by_id[field][entry_id][scope_field] == "production/schema"]
            for row in original
        }
        require(actual_prod_by_domain == expected_prod_by_domain, f"{field}: per-domain production memberships were lost or added")
        actual_prod = {i for values in actual_prod_by_domain.values() for i in values}
        require(actual_prod == expected_prod, f"{field}: production set mismatch; missing={sorted(expected_prod-actual_prod)} extra={sorted(actual_prod-expected_prod)}")
        actual_test = {r["entry_id"] for r in regression if r["source_kind"] == REGRESSION_KIND[field]}
        require(actual_test == expected_test, f"{field}: regression test set mismatch; missing={sorted(expected_test-actual_test)} extra={sorted(actual_test-expected_test)}")
        for row in regression:
            if row["source_kind"] != REGRESSION_KIND[field]:
                continue
            entry_id = row["entry_id"]
            source = source_by_id[field][entry_id]
            require(row["source_scope"] == "test/fixture", f"production ID leaked to regression or wrong scope: {entry_id}")
            require(row["prior_s8_domain_memberships"] == ";".join(old_members[field].get(entry_id, [])), f"prior S8 test domain membership changed for {entry_id}")
            require(row["prior_s8_domain_map_status"] == ("mapped" if old_members[field].get(entry_id) else "not mapped in frozen S8 Domain Map"), f"prior-map status mismatch for {entry_id}")
            require(row["source_path"] == source["path"], f"source path mismatch for {entry_id}")
        area_proof = proof_areas[field]
        expected_prod_sorted = sorted(expected_prod, key=id_key)
        expected_test_sorted = sorted(expected_test, key=id_key)
        require(area_proof["production_expected_ids"] == expected_prod_sorted == area_proof["production_mapped_ids"], f"proof production IDs invalid for {field}")
        require(area_proof["production_expected_count"] == area_proof["production_mapped_count"] == len(expected_prod), f"proof production count invalid for {field}")
        require(area_proof["production_missing_ids"] == area_proof["production_extra_ids"] == [], f"production missing/extra IDs in proof for {field}")
        require(area_proof["test_expected_ids"] == expected_test_sorted == area_proof["test_mapped_ids"], f"proof test IDs invalid for {field}")
        require(area_proof["test_expected_count"] == area_proof["test_mapped_count"] == len(expected_test), f"proof test count invalid for {field}")
        require(area_proof["test_missing_ids"] == area_proof["test_extra_ids"] == [], f"test missing/extra IDs in proof for {field}")
        counts = Counter(entry_id for values in actual_prod_by_domain.values() for entry_id in values)
        result_counts[field] = {
            "production_unique": len(actual_prod),
            "test_unique": len(expected_test),
            "domain_memberships": sum(counts.values()),
            "ids_in_multiple_domains": sum(count > 1 for count in counts.values()),
            "prior_test_ids_mapped": len(set(old_members[field]) & expected_test),
            "prior_test_ids_not_mapped": len(expected_test - set(old_members[field])),
        }
    require(proof["baseline"] == {"s8_head": "4005073337f198a245773e30a76178749bfd2920", "s8_tree": "71e18895e9740f9a233c5cf56df3c723f626c727", "production_master": "3275ac7c72df8ae4139a30f16c7f649971324e97"}, "ID-set proof baseline mismatch")
    require(all(row["regression_lane"] == "REG-TEST" for row in regression), "non-regression row in test map")
    return {"domain_count": len(production), "area_counts": result_counts, "regression_rows": len(regression)}


def check_commands() -> dict[str, object]:
    source = read_csv(S8 / "ledgers/TAURI-COMMAND-REGISTRY.csv")
    ownership = read_csv(OUT / "TAURI-COMMAND-OWNERSHIP.csv")
    d16_only = read_csv(OUT / "D16-ONLY-COMMAND-TRACEABILITY.csv")
    original_map = read_csv(S8 / "ledgers/DOMAIN-TO-ENTRY-MAP.csv")
    old_members = memberships(original_map, "registered_command_ids")
    source_by_id = {r["command_id"]: r for r in source}
    owner_by_id = {r["command_id"]: r for r in ownership}
    require(len(source) == len(ownership) == 209, "all 209 command rows must be retained")
    require(set(source_by_id) == set(owner_by_id), "command ownership ID set differs from frozen command registry")
    original_fields = list(source[0])
    for command_id, row in owner_by_id.items():
        src = source_by_id[command_id]
        for field in original_fields:
            require(row[field] == src[field], f"original command registry field changed: {command_id}/{field}")
        domains = old_members.get(command_id, [])
        require(row["s8_domain_memberships"] == ";".join(domains), f"domain membership changed for {command_id}")
        expected_d16_only = domains == ["D-16"]
        require(row["d16_only"] == ("yes" if expected_d16_only else "no"), f"D-16-only flag incorrect for {command_id}")
        require(row["actual_owner"] and row["d4_scope_and_boundary"] and row["ownership_evidence_and_limit"], f"missing ownership disposition for {command_id}")
    expected_d16 = SERVICE_IDS | FILE_WORKSPACE_IDS | STORAGE_ANALYZER_IDS
    actual_d16 = {r["command_id"] for r in ownership if r["d16_only"] == "yes"}
    require(actual_d16 == expected_d16, f"D16-only set mismatch: missing={sorted(expected_d16-actual_d16)} extra={sorted(actual_d16-expected_d16)}")
    require({r["command_id"] for r in d16_only} == expected_d16 and len(d16_only) == 40, "D16-only review table must include exact 40 IDs")
    expected_class = {
        **{command_id: "independent service/system capability" for command_id in SERVICE_IDS},
        **{command_id: "independent File Workspace capability" for command_id in FILE_WORKSPACE_IDS},
        **{command_id: "other cross-track filesystem operation" for command_id in STORAGE_ANALYZER_IDS},
    }
    require(all(owner_by_id[c]["actual_owner_class"] == kind for c, kind in expected_class.items()), "D16-only owner class mismatch")
    require(Counter(owner_by_id[c]["actual_owner_class"] for c in expected_d16) == Counter({"independent service/system capability": 8, "independent File Workspace capability": 22, "other cross-track filesystem operation": 10}), "D16-only owner category counts mismatch")
    require("reveal_in_folder" in owner_by_id["CMD-202"]["d4_scope_and_boundary"], "CMD-202 raw reveal path risk was not documented")
    source_cache: dict[str, list[str]] = {}
    registration_source = git_bytes("show", f"{MASTER}:src-tauri/src/main.rs").decode("utf-8").splitlines()
    for command_id, row in source_by_id.items():
        registered_path, registered_line = row["registered_at"].rsplit(":", 1)
        require(registered_path == "src-tauri/src/main.rs", f"unexpected registration source for {command_id}: {registered_path}")
        registration_index = int(registered_line) - 1
        require(0 <= registration_index < len(registration_source), f"registration line is out of range for {command_id}")
        require(row["registered_symbol"] in registration_source[registration_index], f"registration symbol mismatch at {row['registered_at']}")
        definition_path, definition_line = row["definition"].rsplit(":", 1)
        if definition_path not in source_cache:
            source_cache[definition_path] = git_bytes("show", f"{MASTER}:{definition_path}").decode("utf-8").splitlines()
        definition_index = int(definition_line) - 1
        require(0 <= definition_index < len(source_cache[definition_path]), f"definition line is out of range for {command_id}")
        symbol = row["registered_symbol"].split("::")[-1]
        require(symbol in source_cache[definition_path][definition_index], f"definition symbol mismatch at {row['definition']}")
    for command_id in SERVICE_IDS:
        row = source_by_id[command_id]
        require(row["classification"] == "non-file service command", f"service classification mismatch for {command_id}")
        require(row["definition"].split(":", 1)[0] in {"src-tauri/src/settings.rs", "src-tauri/src/runtime_capabilities.rs", "src-tauri/src/app_control.rs"}, f"service module mismatch for {command_id}")
    for command_id in FILE_WORKSPACE_IDS:
        row = source_by_id[command_id]
        require(row["classification"] == "explicit user-selected workspace command", f"File Workspace classification mismatch for {command_id}")
        require(row["definition"].startswith("src-tauri/src/file_workspace/integration/commands.rs:"), f"File Workspace definition module mismatch for {command_id}")
    for command_id in STORAGE_ANALYZER_IDS:
        row = source_by_id[command_id]
        require(row["classification"] == "independent storage cleanup command", f"Storage Analyzer classification mismatch for {command_id}")
        require(row["definition"].startswith("src-tauri/src/storage_analyzer.rs:"), f"Storage Analyzer definition module mismatch for {command_id}")
    proof = json.loads((OUT / "evidence/ID-SET-PROOF.json").read_text(encoding="utf-8"))["commands"]
    expected_ids = sorted(source_by_id, key=id_key)
    require(proof["registered_ids"] == expected_ids == proof["mapped_ids"], "command proof ID arrays mismatch")
    require(proof["registered_count"] == proof["mapped_count"] == 209, "command proof count mismatch")
    require(proof["missing_ids"] == proof["extra_ids"] == [], "command proof missing/extra set not empty")
    require(proof["d16_only_count"] == 40 and set(proof["d16_only_ids"]) == expected_d16, "command proof D16-only set mismatch")
    return {"registered": 209, "d16_only": 40, "registration_locations_verified": 209, "definition_locations_verified": 209, "owner_classes": dict(Counter(owner_by_id[c]["actual_owner_class"] for c in expected_d16))}


def check_frontend() -> dict[str, int]:
    source = read_csv(S8 / "ledgers/FRONTEND-CONSUMER-LEDGER.csv")
    original_map = read_csv(S8 / "ledgers/DOMAIN-TO-ENTRY-MAP.csv")
    production = read_csv(OUT / "PRODUCTION-DOMAIN-ENTRY-MAP.csv")
    regression = read_csv(OUT / "REGRESSION-ENTRY-MAP.csv")
    old = memberships(original_map, "frontend_consumer_ids")
    src = {r["consumer_id"]: r for r in source}
    expected_prod = {r["consumer_id"] for r in source if "test/demo mock" not in r["classification"]}
    expected_test = {r["consumer_id"] for r in source if "test/demo mock" in r["classification"]}
    actual_prod = {item for row in production for item in id_list(row["frontend_consumer_ids"])}
    actual_test_rows = {r["entry_id"]: r for r in regression if r["source_kind"] == "frontend_consumer"}
    require(len(source) == 11 and len(expected_prod) == 10 and len(expected_test) == 1, "frontend ledger scope counts changed")
    require(actual_prod == expected_prod, "frontend production/test scope mismatch")
    require(set(actual_test_rows) == expected_test, "frontend demo/mock is not isolated in regression map")
    for entry_id, row in actual_test_rows.items():
        require(row["prior_s8_domain_memberships"] == ";".join(old.get(entry_id, [])), f"frontend prior domain membership changed: {entry_id}")
        require(row["source_path"] == src[entry_id]["source"], f"frontend source path mismatch: {entry_id}")
    proof = json.loads((OUT / "evidence/ID-SET-PROOF.json").read_text(encoding="utf-8"))["frontend"]
    require(proof["source_count"] == 11 and set(proof["source_ids"]) == set(src), "frontend proof source mismatch")
    require(proof["production_count"] == 10 and set(proof["production_ids"]) == expected_prod == set(proof["production_mapped_ids"]), "frontend production proof mismatch")
    require(proof["test_count"] == 1 and set(proof["test_ids"]) == expected_test == set(proof["test_mapped_ids"]), "frontend regression proof mismatch")
    return {"total": 11, "production": 10, "test_demo": 1}


def check_input_and_checksums() -> dict[str, object]:
    inputs = json.loads((OUT / "evidence/INPUTS.json").read_text(encoding="utf-8"))
    require(inputs["s8_head"] == BASE_HEAD and inputs["s8_tree"] == BASE_TREE and inputs["production_master"] == MASTER, "input pin mismatch")
    for rel, digest in inputs["input_files"].items():
        data = (S8 / rel).read_bytes()
        require(hashlib.sha256(data).hexdigest() == digest, f"frozen input digest mismatch: {rel}")
        require(data == git_bytes("show", f"{BASE_HEAD}:docs/project/research/issue-381-d4-s8/{rel}"), f"frozen input does not match S8 object: {rel}")
    sums_path = OUT / "evidence/SHA256SUMS"
    entries: dict[str, str] = {}
    for line in sums_path.read_text(encoding="utf-8").splitlines():
        if not line:
            continue
        digest, relative = line.split(None, 1)
        entries[relative.lstrip("* ")] = digest
    expected_paths = {p.relative_to(OUT).as_posix() for p in OUT.rglob("*") if p.is_file() and p != sums_path}
    require(set(entries) == expected_paths, f"SHA256SUMS path coverage mismatch: missing={sorted(expected_paths-set(entries))} extra={sorted(set(entries)-expected_paths)}")
    for relative, expected in entries.items():
        actual = hashlib.sha256((OUT / relative).read_bytes()).hexdigest()
        require(actual == expected, f"S8R checksum mismatch: {relative}")
    return {"input_files_pinned": len(inputs["input_files"]), "s8r_files_checksummed": len(entries)}


def main() -> None:
    head, branch = check_source_and_frozen_s8()
    maps = check_maps()
    commands = check_commands()
    frontend = check_frontend()
    checksums = check_input_and_checksums()
    # Pin the specific source observation behind the raw-path reveal warning.
    storage_source = git_bytes("show", f"{MASTER}:src-tauri/src/storage_analyzer.rs").decode("utf-8")
    require("pub fn reveal_storage_candidate(path: String) -> Result<(), String>" in storage_source, "CMD-202 source definition changed at pinned master")
    require("crate::file_ops::reveal_in_folder(path)" in storage_source, "CMD-202 raw path reveal delegation not found at pinned master")
    print(json.dumps({
        "result": "PASS",
        "branch": branch,
        "head": head,
        "s8_base": BASE_HEAD,
        "frozen_s8_files_unchanged": 17,
        "production_domain_count": maps["domain_count"],
        "production_and_regression_sets": maps["area_counts"],
        "regression_map_rows": maps["regression_rows"],
        "commands": commands,
        "frontend": frontend,
        "checksums": checksums,
        "production_schema_diff": "none",
        "historical_S8_tests_CI_hosted_benchmarks": "not run",
    }, indent=2))


if __name__ == "__main__":
    main()
