#!/usr/bin/env python3
"""Rebuild D4-S8 row ledgers from the frozen S1 inventories and Schema 37 source.

Run from the repository root. The source read is pinned by git object ID; this
script produces static reconciliation evidence, not a Rust/SQL semantic proof.
"""
from __future__ import annotations

import csv
import re
import subprocess
from collections import defaultdict
from pathlib import Path

ROOT = Path("docs/project/research/issue-381-d4-s8")
S1 = Path("docs/project/research/issue-381-d4-s1/source-audit")
MASTER = "3275ac7c72df8ae4139a30f16c7f649971324e97"
SRC = "src-tauri/src"
SOURCE_CACHE: dict[str, list[str]] = {}


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], text=True)


def read_csv(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as f:
        return list(csv.DictReader(f))


def write_csv(path: Path, rows: list[dict[str, str]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fields = list(rows[0]) if rows else []
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=fields, lineterminator="\n")
        w.writeheader()
        w.writerows(rows)


def source_file(path: str) -> list[str]:
    if path not in SOURCE_CACHE:
        SOURCE_CACHE[path] = Path(path).read_text(encoding="utf-8").splitlines()
    return SOURCE_CACHE[path]


def in_test_module(path: str, line: int | None = None, lines: list[str] | None = None) -> bool:
    # S1 misses cfg-gated `tests.rs`, `trigger_tests.rs`, and trigger_tests/ children.
    if path.endswith("/tests.rs") or path.endswith("/trigger_tests.rs") or "/trigger_tests/" in path or "/tests/" in path:
        return True
    if line is None or lines is None:
        return False
    for i, text in enumerate(lines[:line]):
        if "#[cfg(test)]" not in text:
            continue
        opener = None
        for j in range(i + 1, min(i + 6, len(lines))):
            if re.search(r"\bmod\s+\w+\s*\{", lines[j]):
                opener = j
                break
            if lines[j].strip() and not lines[j].lstrip().startswith("#["):
                break
        if opener is None or opener + 1 >= line:
            continue
        depth = 0
        for j in range(opener, min(line, len(lines))):
            code = re.sub(r'"(?:\\.|[^"\\])*"', '""', lines[j].split("//", 1)[0])
            depth += code.count("{") - code.count("}")
            if j > opener and depth <= 0:
                break
        else:
            return True
        if depth > 0:
            return True
    return False


def scope(path: str, old_scope: str = "") -> str:
    if old_scope == "test/fixture" or in_test_module(path):
        return "test/fixture"
    return "production/schema"


def nearest_fn(lines: list[str], line: int) -> tuple[str, int]:
    for i in range(line - 1, -1, -1):
        m = re.search(r"\b(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(", lines[i])
        if m:
            return m.group(1), i + 1
    return "<module-or-schema-trigger>", 0


def domain_for(path: str, function: str, tables: str = "", original: str = "") -> str:
    p = path.lower()
    f = function.lower()
    t = set(tables.lower().split(";"))
    if original == "test/fixture" or in_test_module(path):
        return "test/fixture (excluded from production migration)"
    if "/global_index/" in p:
        if "enqueue_managed_ai_for_library_files" in f:
            return "Global Index / explicit managed bridge"
        return "independent Global Index"
    if p.endswith("/db/schema.rs"):
        return "Schema / trigger / migration"
    if "scan" in p or "watcher" in p or "scanner" in p:
        return "Scanner / Watcher / root authority"
    if "content" in p:
        return "Content / content FTS"
    if "dedupe" in p:
        return "Dedupe / fingerprints"
    if "analysis" in p:
        return "Storage analysis / findings"
    if "organization" in p:
        return "Organization plans"
    if "rule_proposals" in p or "classification" in p or "automation" in p or "learning" in p:
        return "Rules / Classification / AI / automation"
    if "file_ops" in p or "file_workspace" in p:
        return "Filesystem action / explicit workspace"
    if "library/tags" in p or "user_tags" in t or "file_user_tags" in t:
        return "Tags / independent user metadata"
    if "library" in p:
        return "File Library V2"
    if "global_entries" in t or "managed_entries" in t or "managed_scopes" in t or "ai_jobs" in t:
        return "independent Global Index"
    return "Legacy File Library / shared DB"


def visibility_disposition(domain: str, tables: str, source_text: str = "") -> tuple[str, str, str, str]:
    t = set(tables.lower().split(";"))
    if domain.startswith("test/fixture"):
        return ("No production migration; keep as regression fixture.", "Fixture state is disposable test data.", "TC-STATIC-01", "Source audit only")
    if domain == "independent Global Index":
        return ("Exempt from Managed File Effective Visibility; use Global Index source/volume authority. Managed AI bridge entry points additionally resolve the current Managed File source and manifest.", "Preserve Global Index rows, its own stale state, FTS and queue state.", "TC-GLOBAL-10; TC-DBGEN-11", "Global Index track; #381 owns only the explicit bridge")
    if domain == "Global Index / explicit managed bridge":
        return ("Bridge must validate Global Index identity and current Managed File Effective Visibility in one admitted generation; path equality is insufficient.", "Keep Global Index and Managed File identities separate; preserve queued/history state.", "TC-GLOBAL-10; TC-VIS-01; TC-DBGEN-11", "Global Index bridge + #381 File Library")
    if "files_fts" in t or "files_fts" in source_text.lower():
        return ("Derived index only. All public File Library search/count/page paths must constrain rows by the same effective manifest; pointer publish must revise query authority explicitly.", "Keep rowid/content sync for real name/path/row changes; hidden FTS terms cannot leak IDs or silently define BM25 corpus.", "TC-FTS-07; TC-FTSCURSOR-12; TC-TRIGGER-04", "#381 File Library / FTS")
    if "file_user_tags" in t or "user_tags" in t:
        return ("User metadata is not visibility authority. List counts and selection-bound writes resolve current visible source IDs; unscoped tag catalog CRUD remains independent.", "Never delete/reassign tag rows during publication or path reuse.", "TC-TAG-05; TC-IDENT-03", "#381 File Library tags")
    if "scan_roots" in t or "scan_runs" in t or "scan_sessions" in t or "scan_seen" in t or "scan_session_roots" in t:
        return ("Scanner/root authority. Changes affecting coverage, source ownership, Watcher sequence or root revision invalidate that root's candidate/seal; only complete successful generation becomes visible through pointer publication.", "Retain run/session/event evidence; no partial root publication or cross-root global stale propagation.", "TC-SCAN-08; TC-DBGEN-11; TC-SCHEMA-13", "#381 Scanner / Watcher / publication")
    if "content_artifact_fts" in t or "content_artifacts" in t:
        return ("Active artifact/detail/search uses Effective Visibility plus source/content/policy revision. Run/history remains readable as history; background completion must CAS current source and manifest.", "Retain artifact and historical run rows; do not rely on files-only trigger for pointer changes.", "TC-WORKER-09; TC-FTS-07; TC-TRIGGER-04; TC-HISTORY-06", "#381 Content")
    if "file_fingerprints" in t or "duplicate_groups" in t or "duplicate_group_members" in t:
        return ("Candidate and active group projections resolve through Effective Visibility; worker publication CASes source/run revisions. Root A loss cannot invalidate a still-valid root B source.", "Preserve fingerprints/groups/evidence as history; no same-path inheritance.", "TC-WORKER-09; TC-IDENT-03; TC-HISTORY-06", "#381 Dedupe")
    if "analysis_findings" in t or "analysis_finding_evidence" in t:
        return ("Current candidates/actions use Effective Visibility and source/manifest/run revisions; historical findings/evidence/decisions are read through their history API without current visibility filtering.", "Preserve decisions, notes, evidence and findings; path alone never rebinds to replacement.", "TC-WORKER-09; TC-HISTORY-06; TC-IDENT-03", "#381 Analysis")
    if "organization_plans" in t or "rule_proposals" in t or "classification_history" in t or "classification_feedback" in t:
        return ("Active plan/proposal/classification admission and execution re-resolve Effective Visibility and CAS captured source/manifest/policy revisions; history remains retained.", "Keep plan/history/user decision state; no stale result can attach by path to a replacement source.", "TC-WORKER-09; TC-HISTORY-06; TC-IDENT-03", "#381 Organization / Rules / AI")
    if "files" in t:
        return ("Current File Library identity/payload is not itself membership authority. Readers and actions resolve Effective Visibility; scanner changes publish root membership. Independent metadata writes retain their own revision and require source-ID CAS when selection-bound.", "Preserve files.id/rowid and all FK/tag/history/artifact/fingerprint associations until jointly approved migration; path reuse must create a distinct source without inheritance.", "TC-VIS-01; TC-IDENT-03; TC-IPC-02; TC-SCHEMA-13", "#381 File Library / scanner / operation owner")
    return ("No direct Managed File visibility authority in this SQL target; if it carries a managed file ID/path, caller must validate Effective Visibility and source identity. Otherwise keep its owning subsystem authority.", "Preserve durable rows; never infer ownership from equal path.", "TC-STATIC-01; TC-DBGEN-11", "Owning module listed in entry")


def stale_disposition(domain: str, roles: str, source_text: str) -> tuple[str, str, str, str]:
    if domain == "test/fixture (excluded from production migration)":
        return visibility_disposition(domain, "", source_text)
    if domain == "independent Global Index":
        return visibility_disposition(domain, "global_entries", source_text)
    if domain == "Global Index / explicit managed bridge":
        return visibility_disposition(domain, "managed_entries", source_text)
    if domain == "Schema / trigger / migration" and "trigger or view definition" not in roles:
        return ("Schema/migration field or compatibility SQL, not a standalone active reader. Any future migration must be jointly versioned with #380; no v38 allocation or path/stale-bit ownership backfill is implied.", "Preserve Schema 37 files.id/path, rowid, FKs, user metadata, histories and FTS until a jointly approved migration crosswalk proves otherwise.", "TC-SCHEMA-13; TC-DBGEN-11", "Joint #380/#381 database migration owner")
    if domain == "Schema / trigger / migration" and "trigger or view definition" not in roles:
        return ("Schema/migration field or compatibility SQL, not a standalone active reader. Any future migration must be jointly versioned with #380; no v38 allocation or path/stale-bit ownership backfill is implied.", "Preserve Schema 37 files.id/path, rowid, FKs, user metadata, histories and FTS until a jointly approved migration crosswalk proves otherwise.", "TC-SCHEMA-13; TC-DBGEN-11", "Joint #380/#381 database migration owner")
    if "trigger or view definition" in roles or re.search(r"create\s+trigger|create\s+view", source_text, re.I):
        return ("Schema/derived-object definition, not an application reader. Its current row-event trigger must be inventoried separately; manifest-pointer publication does not fire a files-row trigger and must explicitly revise dependent visibility/search state.", "Keep rowid/FTS/content/duplicate-derived records consistent and retained; no pointer-only assumption.", "TC-TRIGGER-04; TC-FTS-07", "#381 database/trigger owner")
    if "managed file DTO or row projection" in roles:
        return ("Projection field rather than a standalone SQL visibility predicate. Active DTO availability must be derived from the same Effective Visibility snapshot/token; historical DTOs remain read-only history.", "Preserve source ID, rowid and associations; do not expose a path action from a raw stale boolean.", "TC-VIS-01; TC-IPC-02; TC-HISTORY-06", "#381 repository projection + IPC DTO owner")
    return visibility_disposition(domain, "files", source_text)


def build_stale() -> tuple[list[dict[str, str]], list[dict[str, str]]]:
    refs = read_csv(S1 / "stale-reference-inventory.csv")
    groups = read_csv(S1 / "function-inventory.csv")
    group_id: dict[tuple[str, str, str], str] = {}
    group_rows: list[dict[str, str]] = []
    for n, g in enumerate(groups, 1):
        gid = f"STG-{n:03d}"
        key = (g["path"], g["function"], g["function_line"])
        group_id[key] = gid
        sc = scope(g["path"], "test/fixture" if g["domains"] == "test/fixture" else "")
        domain = domain_for(g["path"], g["function"], original="test/fixture" if sc == "test/fixture" else "")
        target, preserve, test, owner = stale_disposition(domain, g["roles"], "")
        group_rows.append({
            "group_id": gid, "source_rev": g["source_rev"], "path": g["path"],
            "function_or_schema_owner": g["function"], "owner_line": g["function_line"],
            "reference_lines": g["reference_lines"], "reference_count": g["reference_count"],
            "s1_domain_heuristic": g["domains"], "s1_roles": g["roles"],
            "reconciled_scope": sc, "reconciled_domain": domain,
            "Effective_Visibility_or_exemption": target, "data_and_compatibility_preserved": preserve,
            "test_contract": test, "responsible_module_or_track": owner,
            "classification_note": "One source function/schema owner; individual raw lines are in STALE-REFERENCE-LEDGER.csv.",
        })
    raw = []
    for n, r in enumerate(refs, 1):
        key = (r["path"], r["enclosing_function"], r["function_line"])
        gid = group_id[key]
        sc = scope(r["path"], r["domain"] if r["domain"] == "test/fixture" else "")
        domain = domain_for(r["path"], r["enclosing_function"], original="test/fixture" if sc == "test/fixture" else "")
        target, preserve, test, owner = stale_disposition(domain, r["role"], r["source_text"])
        raw.append({
            "record_id": f"STR-{n:03d}", "group_id": gid, "source_rev": r["source_rev"],
            "path": r["path"], "line": r["line"], "function": r["enclosing_function"],
            "function_line": r["function_line"], "source_text": r["source_text"],
            "s1_domain": r["domain"], "s1_role": r["role"], "reconciled_scope": sc,
            "reconciled_domain": domain, "disposition": target, "preservation": preserve,
            "test_contract": test, "responsibility": owner,
        })
    return group_rows, raw


def build_writes() -> tuple[list[dict[str, str]], list[dict[str, str]]]:
    refs = read_csv(S1 / "mutation-reference-inventory.csv")
    groups = read_csv(S1 / "mutation-function-inventory.csv")
    ids: dict[tuple[str, str, str], str] = {}
    group_rows = []
    for n, g in enumerate(groups, 1):
        gid = f"WFG-{n:03d}"
        key = (g["path"], g["function"], g["function_line"])
        ids[key] = gid
        sc = scope(g["path"], g["scope"])
        dom = domain_for(g["path"], g["function"], g["tables_written"], "test/fixture" if sc == "test/fixture" else "")
        target, preserve, test, owner = visibility_disposition(dom, g["tables_written"])
        group_rows.append({
            "group_id": gid, "source_rev": g["source_rev"], "path": g["path"],
            "function_or_schema_owner": g["function"], "owner_line": g["function_line"],
            "literal_reference_lines": g["reference_lines"], "tables_written": g["tables_written"],
            "s1_scope": g["scope"], "reconciled_scope": sc, "reconciled_domain": dom,
            "Effective_Visibility_or_exemption": target, "data_and_compatibility_preserved": preserve,
            "test_contract": test, "responsible_module_or_track": owner,
            "classification_note": "One table-writing function group; every regex-matched literal line is in WRITE-REFERENCE-LEDGER.csv.",
        })
    raw = []
    for n, r in enumerate(refs, 1):
        key = (r["path"], r["enclosing_function"], r["function_line"])
        gid = ids[key]
        sc = scope(r["path"], r["scope"])
        dom = domain_for(r["path"], r["enclosing_function"], r["table"], "test/fixture" if sc == "test/fixture" else "")
        target, preserve, test, owner = visibility_disposition(dom, r["table"], r["source_text"])
        raw.append({
            "record_id": f"WRT-{n:03d}", "group_id": gid, "source_rev": r["source_rev"],
            "path": r["path"], "line": r["line"], "function": r["enclosing_function"],
            "function_line": r["function_line"], "table": r["table"], "source_text": r["source_text"],
            "s1_scope": r["scope"], "reconciled_scope": sc, "reconciled_domain": dom,
            "disposition": target, "preservation": preserve, "test_contract": test, "responsibility": owner,
        })
    return group_rows, raw


def build_reads() -> tuple[list[dict[str, str]], list[dict[str, str]]]:
    paths = git("ls-tree", "-r", "--name-only", MASTER, SRC).splitlines()
    refs: list[dict[str, str]] = []
    rx = re.compile(r"\b(?:FROM|JOIN)\s+(?:main\.)?files\b", re.IGNORECASE)
    for path in paths:
        if not path.endswith(".rs"):
            continue
        lines = source_file(path)
        for n, line in enumerate(lines, 1):
            if not rx.search(line):
                continue
            fn, fn_line = nearest_fn(lines, n)
            test = in_test_module(path, n, lines)
            refs.append({"source_rev": MASTER, "path": path, "line": str(n), "function": fn,
                         "function_line": str(fn_line), "source_text": " ".join(line.strip().split()),
                         "scope": "test/fixture" if test else "production/schema"})
    groups: dict[tuple[str, str, str], list[dict[str, str]]] = defaultdict(list)
    for r in refs:
        groups[(r["path"], r["function"], r["function_line"])].append(r)
    ids = {k: f"RFG-{n:03d}" for n, k in enumerate(sorted(groups), 1)}
    group_rows = []
    for k in sorted(groups):
        rows = groups[k]
        path, fn, fl = k
        test = all(r["scope"] == "test/fixture" for r in rows)
        dom = domain_for(path, fn, "", "test/fixture" if test else "")
        target, preserve, tc, owner = visibility_disposition(dom, "files")
        group_rows.append({"group_id": ids[k], "source_rev": MASTER, "path": path,
                           "function_or_module": fn, "function_line": fl,
                           "query_reference_lines": ";".join(r["line"] for r in rows),
                           "query_reference_count": str(len(rows)), "reconciled_scope": "test/fixture" if test else "production/schema",
                           "authority_domain": dom, "Effective_Visibility_or_exemption": target,
                           "data_and_compatibility_preserved": preserve, "test_contract": tc,
                           "responsible_module_or_track": owner})
    raw = []
    for n, r in enumerate(sorted(refs, key=lambda x: (x["path"], int(x["line"]))), 1):
        k = (r["path"], r["function"], r["function_line"])
        test = r["scope"] == "test/fixture"
        dom = domain_for(r["path"], r["function"], "", "test/fixture" if test else "")
        target, preserve, tc, owner = visibility_disposition(dom, "files", r["source_text"])
        raw.append({"record_id": f"RFR-{n:03d}", "group_id": ids[k], "source_rev": MASTER,
                    "path": r["path"], "line": r["line"], "function": r["function"],
                    "function_line": r["function_line"], "source_text": r["source_text"],
                    "reconciled_scope": r["scope"], "authority_domain": dom,
                    "disposition": target, "preservation": preserve, "test_contract": tc,
                    "responsibility": owner})
    return group_rows, raw


def command_domain(module: str, name: str) -> tuple[str, str, str, str]:
    low = name.lower()
    if module == "db":
        if low == "init_db":
            return ("database/schema initialization command", "Opening a manifest-era database requires the #380 generation admission check before any old command or process can read/write it.", "TC-DBGEN-11; TC-SCHEMA-13", "#380 database-generation gate + joint #381 migration")
        if "saved_view" in low:
            return ("saved File Library query configuration", "Saved filters/preferences are not visibility authority; applying one always queries current Effective Visibility and cursor revision.", "TC-VIS-01; TC-FTSCURSOR-12", "#381 File Library V2")
        if low in {"insert_file", "remove_files_by_paths", "upsert_files_by_paths", "search_files", "get_paged_files"}:
            return ("legacy path-ID File Library command", "Must be rejected/removed or adapted before manifest authority; raw file/path/stale contract is a bypass.", "TC-IPC-02; TC-IDENT-03; TC-DBGEN-11", "#381 File Library + #380 client-generation gate")
        if "tag" in low or low == "mutate_file_user_tags":
            return ("tag / user metadata command", "Tag catalog is independent; batch file targets must resolve effective sources and manifest-bound selection revision.", "TC-TAG-05; TC-VIS-01", "#381 File Library tags")
        if "operation_log" in low:
            return ("historical operation read", "Keep snapshots readable; any action derived from a historical path re-resolves current source and native identity.", "TC-HISTORY-06; TC-IDENT-03", "#381 Operations/history")
        if "file_library" in low or "file_selection" in low or "get_operation_previews" in low or "permanent_delete" in low:
            return ("File Library current read/action", "Use manifest-bound Effective Visibility for detail/count/selection/preview; revalidate source and native identity before reveal/delete/restore.", "TC-VIS-01; TC-IPC-02; TC-IDENT-03", "#381 File Library / file operations")
        if "organization" in low:
            return ("Organization plan/current action", "Plan/history remains durable; current item query, preview and execute re-resolve Effective Visibility and source/manifest revisions.", "TC-WORKER-09; TC-HISTORY-06; TC-IDENT-03", "#381 Organization")
        if "rule" in low or "classification" in low or "automation" in low:
            return ("rules / classification / automation", "Admission and late completion bind current source, manifest, run and policy revisions; historical outputs stay retained.", "TC-WORKER-09; TC-HISTORY-06", "#381 Rules / AI / automation")
        if "stat" in low:
            return ("File Library statistics", "Counts must use the same Effective Visibility and manifest snapshot as rows/cursors.", "TC-VIS-01; TC-FTSCURSOR-12", "#381 File Library")
        return ("DB service command; inspect backing repository", "If it returns or mutates a managed file, bind through Effective Visibility/source CAS; durable history/config stays in its owning authority.", "TC-IPC-02; TC-DBGEN-11", "#381 DB service; #380 generation boundary")
    if module in {"global_index"}:
        if "managed_scope" in low:
            return ("Global Index managed-scope policy command", "Managed AI scope configuration remains Global Index state, not Managed File root membership. Any worker dispatched through the separate repository bridge resolves the current Managed File source and manifest.", "TC-GLOBAL-10; TC-DBGEN-11", "Global Index; #381 owns the explicit repository bridge")
        return ("independent Global Index command", "Exempt from Managed File visibility; use Global Index volume/entry state. Result-derived file operations revalidate Global Index identity.", "TC-GLOBAL-10", "Global Index")
    if module == "scanner":
        return ("Managed Scanner command", "Root/run/session/cancel revisions fence candidate preparation and atomic manifest publication.", "TC-SCAN-08; TC-DBGEN-11", "#381 Scanner / Watcher")
    if module == "content":
        return ("Content run/artifact command", "Current artifacts/search require Effective Visibility; run/history endpoints preserve durable history; mutations revalidate source/content/policy revision.", "TC-WORKER-09; TC-FTS-07; TC-HISTORY-06", "#381 Content")
    if module == "analysis":
        return ("Analysis findings/history command", "History remains readable; active candidates/actions and writes bind current source/manifest/run revisions.", "TC-WORKER-09; TC-HISTORY-06", "#381 Analysis")
    if module == "dedupe":
        return ("Dedupe run/result command", "Active candidate/group membership uses Effective Visibility; history retained; worker completion CASes source identity/revision.", "TC-WORKER-09; TC-HISTORY-06; TC-IDENT-03", "#381 Dedupe")
    if module == "rule_proposals":
        return ("Rule proposal command", "Proposal snapshots remain history; impact/preview/apply revalidate each effective source and current policy revision.", "TC-WORKER-09; TC-HISTORY-06", "#381 Rules / AI")
    if module == "file_ops":
        return ("filesystem mutation/recovery command", "A preview or retained ID is not authority: revalidate current source, manifest, native identity and boundary immediately before operation.", "TC-IDENT-03; TC-HISTORY-06; TC-DBGEN-11", "#381 File operations")
    if module == "file_workspace":
        return ("explicit user-selected workspace command", "Separate from Managed File Library visibility; preserve picker/session authority. Any later file operation rechecks the selected physical identity and permission.", "TC-IDENT-03; TC-IPC-02", "File Workspace; #380 process generation for IPC")
    if module == "storage_analyzer":
        return ("independent storage cleanup command", "Not a Managed File Library query. Cleanup/reveal/restore uses its own preview/journal and must revalidate current filesystem identity before effects.", "TC-IDENT-03; TC-HISTORY-06", "Storage Analyzer / operation journal")
    if module == "ai":
        return ("AI classification/cleanup command", "Candidate and completion must validate source, manifest, run, cancellation and policy; AI output remains advisory and retained as history.", "TC-WORKER-09; TC-HISTORY-06; TC-IDENT-03", "#381 AI / owning analysis track")
    if module in {"settings", "runtime_capabilities", "app_control"}:
        return ("non-file service command", "No current Managed File membership read/write in this command surface; excluded unless a future call chain introduces a source/path input.", "TC-STATIC-01; TC-DBGEN-11", "Owning service; #380 generation boundary")
    return ("other registered service command", "No source/path argument is inferable from registry alone; follow its command body and downstream repository. If it consumes file IDs, require source/manifest revalidation.", "TC-IPC-02; TC-DBGEN-11", "Owning service; #381 caller if managed file input")


def build_commands() -> list[dict[str, str]]:
    lines = source_file("src-tauri/src/main.rs")
    start = next(i for i, line in enumerate(lines) if "tauri::generate_handler![" in line)
    end = next(i for i in range(start + 1, len(lines)) if lines[i].strip() == "])" )
    commands = []
    rust_paths = [p for p in git("ls-tree", "-r", "--name-only", MASTER, SRC).splitlines() if p.endswith(".rs")]
    contents = {p: source_file(p) for p in rust_paths}
    definitions: dict[str, list[tuple[str, int]]] = defaultdict(list)
    definition_rx = re.compile(r"\b(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(")
    for path, src_lines in contents.items():
        for ln, text in enumerate(src_lines, 1):
            m = definition_rx.search(text)
            if m and any(re.search(r"#\[(?:tauri::)?command\b", preceding) for preceding in src_lines[max(0, ln - 8):ln - 1]):
                definitions[m.group(1)].append((path, ln))
    for n, i in enumerate(range(start + 1, end), 1):
        m = re.search(r"zen_canvas_tauri::([\w:]+)", lines[i])
        if not m:
            continue
        symbol = m.group(1)
        parts = symbol.split("::")
        module, name = parts[0], parts[-1]
        candidates = [(path, ln) for path, ln in definitions.get(name, [])]
        if module == "db":
            expected = [c for c in candidates if c[0] == "src-tauri/src/db/commands.rs"]
        else:
            module_path = "/".join(parts[:-1])
            expected = [c for c in candidates if module_path.replace("::", "/") in c[0]]
        if not expected:
            expected = candidates
        picked = expected[0] if len(expected) == 1 else (candidates[0] if len(candidates) == 1 else None)
        def_path, def_line = (picked[0], str(picked[1])) if picked else ("UNRESOLVED", "")
        cls, disposition, tests, owner = command_domain(module, name)
        commands.append({
            "command_id": f"CMD-{n:03d}", "source_rev": MASTER,
            "registered_at": f"src-tauri/src/main.rs:{i+1}", "registered_symbol": symbol,
            "definition": f"{def_path}:{def_line}" if def_line else "UNRESOLVED (definition lookup required)",
            "classification": cls, "Effective_Visibility_or_exemption": disposition,
            "preservation_and_compatibility": "Keep durable IDs/history/user state; no old Schema 37 client or Service may write the manifest-era DB. Path-derived IDs are never rebound.",
            "required_test_contract": tests, "responsible_module_or_track": owner,
        })
    return commands


def build_frontend() -> list[dict[str, str]]:
    rows = read_csv(S1 / "frontend-consumers.csv")
    out = []
    for n, r in enumerate(rows, 1):
        path = r["path"]
        if "browserMockApi" in path:
            cls = "test/demo mock; must mirror production visibility and identity contract"
            tc, owner = "TC-IPC-02; TC-VIS-01", "Frontend API/mock parity + #381"
        elif "domain.ts" in path:
            cls = "IPC/domain DTO contract; availability is derived from manifest-bound effective state"
            tc, owner = "TC-IPC-02; TC-VIS-01", "Frontend types/API + #381 IPC"
        else:
            cls = "active renderer projection or action guard; renderer is not authority"
            tc, owner = "TC-VIS-01; TC-IDENT-03; TC-IPC-02", "Frontend surface + #381 service authority"
        out.append({"consumer_id": f"FE-{n:03d}", "source_rev": r["source_rev"], "source": f"{path}:{r['line_or_symbol']}",
                    "current_use": r["current_use"], "s1_contract": r["authority_contract"],
                    "classification": cls,
                    "Effective_Visibility_or_exemption": "Renderer list/detail/preview/select/action state must derive from the same manifest-bound DTO/token; history-only display remains a separate read API.",
                    "preservation_and_compatibility": "Do not transfer stale state or actions through path equality; browser mock, IPC DTO and production Service must agree.",
                    "required_test_contract": tc, "responsible_module_or_track": owner})
    return out


def build_domain_map(stale: list[dict[str, str]], writes: list[dict[str, str]], reads: list[dict[str, str]],
                     commands: list[dict[str, str]], frontend: list[dict[str, str]]) -> list[dict[str, str]]:
    # The S7 domains are intentionally cross-cutting. IDs may appear in more
    # than one row where a command participates in a reader/writer/trigger path.
    domains = [
        ("Schema and legacy identity", lambda r: r["path"].endswith("db/schema.rs") or "legacy" in r.get("classification", "").lower(), lambda r: "schema" in r["path"] or "files" in r["tables_written"], lambda r: "scan" in r["path"] or "schema" in r["path"], lambda r: "init_db" in r["registered_symbol"] or "legacy path-ID" in r["classification"], lambda r: False, "Schema 37 files.id is path-shaped; files.path UNIQUE; preserve rowid/FK/tag/history/FTS crosswalk.", "TC-SCHEMA-13; TC-IDENT-03; TC-DBGEN-11"),
        ("File Library V2 browse/count/search", lambda r: "library/" in r["path"], lambda r: "library" in r["path"] or "file_user_tags" in r["tables_written"], lambda r: "library/" in r["path"], lambda r: "File Library current read/action" in r["classification"] or "statistics" in r["classification"] or "saved File Library" in r["classification"], lambda r: "FileLibraryList" in r["source"] or "library" in r["source"].lower(), "Rows, exact counts, FTS candidate set, filters and cursor use one manifest snapshot.", "TC-VIS-01; TC-FTSCURSOR-12; TC-FTS-07"),
        ("Legacy File Library browse/search", lambda r: "queries/files.rs" in r["path"] or "queries/mod.rs" in r["path"] or "db/types.rs" in r["path"], lambda r: "queries/files.rs" in r["path"] or "legacy" in r.get("classification", "").lower(), lambda r: "queries/files.rs" in r["path"] or "queries/mod.rs" in r["path"] or "db/types.rs" in r["path"], lambda r: "legacy path-ID" in r["classification"], lambda r: "domain.ts" in r["source"] or "browserMockApi" in r["source"], "Legacy path-shaped IDs and offset search cannot remain a parallel visibility authority.", "TC-IPC-02; TC-VIS-01; TC-IDENT-03; TC-DBGEN-11"),
        ("File Library detail / selection / path actions", lambda r: any(x in r["function_or_schema_owner"] for x in ["detail", "selection", "preview", "resolve_file_library_path", "get_indexed_file_rows_by_ids"]), lambda r: "file_ops" in r["path"] or "operation" in r["tables_written"], lambda r: "library" in r["path"] or "files.rs" in r["path"], lambda r: any(x in r["classification"] for x in ["File Library current", "filesystem mutation", "historical operation"]), lambda r: any(x in r["source"] for x in ["Inspector", "VaultView", "LibraryMode", "OrganizeSuggestionInspector"]), "Active detail/action must resolve current source; history detail is a separate read-only route; OS action repeats physical identity check.", "TC-VIS-01; TC-IDENT-03; TC-HISTORY-06; TC-IPC-02"),
        ("File Library FTS5/BM25", lambda r: "fts" in r["path"].lower() or "search" in r["function_or_schema_owner"].lower() or "query_file_library" in r["function_or_schema_owner"], lambda r: "fts" in r["tables_written"].lower() or "schema.rs" in r["path"] or "files" in r["tables_written"], lambda r: "library" in r["path"] or "search" in r["function_or_module"].lower(), lambda r: "search" in r["registered_symbol"] or "query_file_library" in r["registered_symbol"] or "get_paged_files" in r["registered_symbol"], lambda r: "FileLibraryList" in r["source"] or "presentation" in r["source"], "Use product-linked SQLite 3.51.3; BM25 corpus definition requires Owner decision; pointer changes do not fire files_au.", "TC-FTS-07; TC-FTSCURSOR-12; TC-TRIGGER-04"),
        ("Global Index and Global Search", lambda r: "/global_index/" in r["path"] or "Global Index" in r.get("reconciled_domain", r.get("authority_domain", "")), lambda r: "/global_index/" in r["path"] or any(t in r["tables_written"] for t in ["global_entries", "managed_entries", "managed_scopes", "ai_jobs"]), lambda r: "/global_index/" in r["path"], lambda r: "Global Index" in r["classification"], lambda r: False, "Independent global_entries/is_stale/FTS remain a separate authority; explicit managed bridge is dual-authority.", "TC-GLOBAL-10; TC-DBGEN-11"),
        ("Managed Scanner and root generations", lambda r: any(x in r["path"] for x in ["scan.rs", "scanner.rs"]), lambda r: any(x in r["tables_written"] for x in ["scan_roots", "scan_runs", "scan_sessions", "scan_seen"]), lambda r: "scan" in r["path"] or "scanner.rs" in r["path"], lambda r: "Managed Scanner" in r["classification"], lambda r: False, "Only complete root generation enters Effective Visibility; cancellation, Watcher and root/session revisions fence publish.", "TC-SCAN-08; TC-DBGEN-11; TC-SCHEMA-13"),
        ("Watcher and native filesystem bridge", lambda r: "watcher" in r["path"] or "apply_watcher" in r["function_or_schema_owner"], lambda r: "watcher" in r["path"] or "scan_roots" in r["tables_written"], lambda r: "watcher" in r["path"] or "scan.rs" in r["path"], lambda r: "Scanner" in r["classification"] or "filesystem mutation" in r["classification"], lambda r: False, "Per-root watcher revision; missing/error/identity uncertainty fails closed; current source is rechecked before OS effects.", "TC-SCAN-08; TC-IDENT-03; TC-DBGEN-11"),
        ("Tags and user metadata; notes inventory", lambda r: "tags" in r["path"] or "user_tag" in r.get("s1_roles", "").lower(), lambda r: "user_tags" in r["tables_written"] or "file_user_tags" in r["tables_written"], lambda r: "tags" in r["path"], lambda r: "tag / user metadata" in r["classification"], lambda r: False, "Tags/notes are user data, not membership. Batch updates bind manifest and source IDs; no standalone per-file Notes API/table was found; analysis_finding_decisions.note is finding-scoped.", "TC-TAG-05; TC-IDENT-03; TC-HISTORY-06"),
        ("Operation history and other historical reads", lambda r: any(x in r["function_or_schema_owner"] for x in ["restore", "operation", "history"]), lambda r: any(x in r["tables_written"] for x in ["classification_history", "analysis_findings"]), lambda r: "operations" in r["path"] or "analysis" in r["path"], lambda r: "historical operation" in r["classification"] or "operation_log" in r["registered_symbol"], lambda r: "history" in r["source"].lower(), "History lookup stays independent of current visibility; any derived reveal/restore/delete revalidates explicit source and physical identity.", "TC-HISTORY-06; TC-IDENT-03"),
        ("Dedupe and fingerprints", lambda r: "dedupe" in r["path"], lambda r: any(x in r["tables_written"] for x in ["file_fingerprints", "duplicate_groups", "duplicate_group_members"]), lambda r: "dedupe" in r["path"], lambda r: "Dedupe" in r["classification"], lambda r: False, "Active groups derive from visible sources; hidden results/history retained; overlap root loss cannot stale a source still visible through another root.", "TC-WORKER-09; TC-IDENT-03; TC-HISTORY-06"),
        ("Storage analysis and findings", lambda r: "analysis" in r["path"], lambda r: "analysis_findings" in r["tables_written"] or "analysis_finding_evidence" in r["tables_written"], lambda r: "analysis" in r["path"], lambda r: "Analysis findings" in r["classification"], lambda r: False, "Historical findings/evidence/decisions remain readable; active candidates and actions use effective sources and run/source CAS.", "TC-WORKER-09; TC-HISTORY-06; TC-IDENT-03"),
        ("Content analysis and content FTS", lambda r: "content" in r["path"], lambda r: "content_artifact" in r["tables_written"], lambda r: "content" in r["path"], lambda r: "Content run" in r["classification"], lambda r: "ContentUnderstandingSheet" in r["source"], "Artifact FTS is current-visible only; historical artifacts/runs retained; pointer does not fire file-update invalidation trigger.", "TC-WORKER-09; TC-FTS-07; TC-TRIGGER-04; TC-HISTORY-06"),
        ("Rules / classification / AI / automatic organization", lambda r: any(x in r["path"] for x in ["classification", "rule_proposals", "organization", "automation", "/ai/", "learning.rs"]), lambda r: any(x in r["tables_written"] for x in ["organization_plans", "rule_proposals", "classification_history", "classification_feedback", "ai_jobs"]), lambda r: any(x in r["path"] for x in ["classification", "rule_proposals", "organization", "automation", "ai/", "learning.rs"]), lambda r: any(x in r["classification"] for x in ["Organization", "rules /", "Rule proposal", "AI classification"]), lambda r: "organize" in r["source"].lower(), "All stale completions compare source/manifest/run/cancel/policy revision; AI advisory; plans/history retained and action revalidates.", "TC-WORKER-09; TC-HISTORY-06; TC-IDENT-03"),
        ("Derived views / triggers / revision side effects", lambda r: "schema.rs" in r["path"] or "schema" in r.get("reconciled_domain", ""), lambda r: "schema.rs" in r["path"] or "fts" in r["tables_written"], lambda r: "schema.rs" in r["path"], lambda r: any(x in r["registered_symbol"] for x in ["query_file_library", "query_content_artifacts", "search_global_entries"]), lambda r: False, "files_ai/ad/au, content_artifacts_file_changed, Global Index triggers and active_duplicate_membership observe table rows; none subscribes to future Manifest pointer.", "TC-TRIGGER-04; TC-FTS-07; TC-GLOBAL-10"),
        ("IPC / Service / Native Bridge / frontend state", lambda r: False, lambda r: "db/commands.rs" in r["path"], lambda r: False, lambda r: True, lambda r: True, "Every registered command is listed. Legacy path-ID routes are an active bypass risk until #380 blocks old clients/Service; renderer guards are not authority.", "TC-IPC-02; TC-DBGEN-11; TC-VIS-01"),
        ("Startup recovery / workers / pruning", lambda r: any(x in r["function_or_schema_owner"] for x in ["recover", "reconcile", "resume", "reset_running", "prune", "complete", "cancel"]), lambda r: any(x in r["function_or_schema_owner"] for x in ["recover", "reconcile", "resume", "prune"]) or "scan_sessions" in r["tables_written"] or "ai_jobs" in r["tables_written"], lambda r: any(x in r["function_or_module"] for x in ["recover", "reconcile", "resume", "prune", "complete"]), lambda r: any(x in r["registered_symbol"].lower() for x in ["retry", "cancel", "recovery", "resume", "reconcile"]), lambda r: False, "Recovery owns durable run/root/source generations; replay is idempotent, historical records retained, and GC waits for readers/jobs/cursors.", "TC-SCAN-08; TC-WORKER-09; TC-HISTORY-06; TC-DBGEN-11"),
    ]
    result = []
    for n, (name, sm, wm, rm, cm, fm, note, tests) in enumerate(domains, 1):
        result.append({
            "domain_id": f"D-{n:02d}", "s7_domain": name,
            "stale_group_ids": ";".join(r["group_id"] for r in stale if sm(r)),
            "write_function_group_ids": ";".join(r["group_id"] for r in writes if wm(r)),
            "files_read_group_ids": ";".join(r["group_id"] for r in reads if rm(r)),
            "registered_command_ids": ";".join(r["command_id"] for r in commands if cm(r)),
            "frontend_consumer_ids": ";".join(r["consumer_id"] for r in frontend if fm(r)),
            "authority_and_disposition": note, "required_test_contracts_and_gates": tests,
        })
    return result


def main() -> None:
    assert git("rev-parse", MASTER).strip() == MASTER
    # The ledgers scan the checkout for readable source, but only if its entire
    # production tree is byte-identical to the pinned master object.
    subprocess.check_call(["git", "diff", "--quiet", MASTER, "--", SRC])
    out = ROOT / "ledgers"
    stale_groups, stale_refs = build_stale()
    write_groups, write_refs = build_writes()
    read_groups, read_refs = build_reads()
    commands = build_commands()
    frontend = build_frontend()
    domain_map = build_domain_map(stale_groups, write_groups, read_groups, commands, frontend)
    expected = {
        "stale groups": (len(stale_groups), 149), "stale references": (len(stale_refs), 262),
        "write groups": (len(write_groups), 236), "write references": (len(write_refs), 433),
        "direct files read groups": (len(read_groups), 99), "direct files read references": (len(read_refs), 125),
        "registered commands": (len(commands), 209), "frontend consumers": (len(frontend), 11),
        "S7 domains": (len(domain_map), 17),
    }
    for label, (actual, required) in expected.items():
        if actual != required:
            raise SystemExit(f"{label}: expected {required}, found {actual}")
    reconciled_scope_counts = {
        "stale_groups": (sum(r["reconciled_scope"] == "production/schema" for r in stale_groups), 103),
        "write_groups": (sum(r["reconciled_scope"] == "production/schema" for r in write_groups), 121),
        "write_references": (sum(r["reconciled_scope"] == "production/schema" for r in write_refs), 210),
        "files_read_groups": (sum(r["reconciled_scope"] == "production/schema" for r in read_groups), 54),
        "files_read_references": (sum(r["reconciled_scope"] == "production/schema" for r in read_refs), 72),
    }
    for label, (actual, required) in reconciled_scope_counts.items():
        if actual != required:
            raise SystemExit(f"reconciled {label}: expected {required}, found {actual}")
    if any("UNRESOLVED" in c["definition"] for c in commands):
        raise SystemExit("one or more registered command definitions did not resolve")
    for rows, id_col, scope_col, map_col in [
        (stale_groups, "group_id", "reconciled_scope", "stale_group_ids"),
        (write_groups, "group_id", "reconciled_scope", "write_function_group_ids"),
        (read_groups, "group_id", "reconciled_scope", "files_read_group_ids"),
        (commands, "command_id", None, "registered_command_ids"),
        (frontend, "consumer_id", None, "frontend_consumer_ids"),
    ]:
        owned = {r[id_col] for r in rows if scope_col is None or r[scope_col] == "production/schema"}
        mapped = {record_id for d in domain_map for record_id in d[map_col].split(";") if record_id}
        missing = owned - mapped
        if missing:
            raise SystemExit(f"unmapped {id_col}: {','.join(sorted(missing))}")
    write_csv(out / "STALE-GROUP-LEDGER.csv", stale_groups)
    write_csv(out / "STALE-REFERENCE-LEDGER.csv", stale_refs)
    write_csv(out / "WRITE-FUNCTION-LEDGER.csv", write_groups)
    write_csv(out / "WRITE-REFERENCE-LEDGER.csv", write_refs)
    write_csv(out / "FILES-READ-GROUP-LEDGER.csv", read_groups)
    write_csv(out / "FILES-READ-REFERENCE-LEDGER.csv", read_refs)
    write_csv(out / "TAURI-COMMAND-REGISTRY.csv", commands)
    write_csv(out / "FRONTEND-CONSUMER-LEDGER.csv", frontend)
    write_csv(out / "DOMAIN-TO-ENTRY-MAP.csv", domain_map)
    print(f"stale_groups={len(stale_groups)} stale_raw_refs={len(stale_refs)}")
    print(f"write_groups={len(write_groups)} write_raw_refs={len(write_refs)}")
    print(f"direct_files_read_groups={len(read_groups)} direct_files_read_refs={len(read_refs)}")
    print(f"registered_commands={len(commands)} unresolved_command_definitions={sum('UNRESOLVED' in c['definition'] for c in commands)}")
    print(f"frontend_consumers={len(frontend)}")
    print(f"s7_domains_mapped={len(domain_map)}")
    print("reconciled_production_groups stale=103 writes=121 files_reads=54")
    print("reconciled_production_literal_refs writes=210 files_reads=72")
    print("unmapped_production_entries=0")


if __name__ == "__main__":
    main()
