#!/usr/bin/env python3
"""Inventory literal SQL mutation entrypoints in a frozen Rust source tree.

Usage:
  python3 docs/project/research/issue-381-d4-s1/source-audit/build_write_inventory.py \
    3275ac7c72df8ae4139a30f16c7f649971324e97 \
    docs/project/research/issue-381-d4-s1/source-audit

The output is deliberately a source audit, not a SQL parser. Dynamically
assembled SQL and mutations hidden in schema triggers are reviewed in ADR-001.
"""

from __future__ import annotations

import csv
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path


REV = sys.argv[1]
OUT = Path(sys.argv[2])
ROOT = "src-tauri/src"
MUTATION = re.compile(
    r"\b(?:insert(?:\s+or\s+(?:ignore|replace|abort|fail|rollback))?\s+into|"
    r"update(?:\s+or\s+\w+)?|delete\s+from|replace\s+into)\s+[`\"']?([a-z_]\w*)",
    re.IGNORECASE,
)


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], text=True)


def is_test(path: str, line_no: int, lines: list[str]) -> bool:
    if "/tests/" in path or "/db/tests/" in path:
        return True
    for i, line in enumerate(lines[:line_no]):
        if re.search(r"#\[cfg\(test\)\]", line) and re.search(
            r"\bmod\s+\w+\s*\{", "\n".join(lines[i : i + 4])
        ):
            return True
    return False


def enclosing_function(lines: list[str], line_no: int) -> tuple[str, int]:
    for i in range(line_no - 1, -1, -1):
        match = re.search(r"\b(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(", lines[i])
        if match:
            return match.group(1), i + 1
    return "<module-or-schema-trigger>", 0


OUT.mkdir(parents=True, exist_ok=True)
files = [p for p in git("ls-tree", "-r", "--name-only", REV, ROOT).splitlines() if p.endswith(".rs")]
rows: list[dict[str, str]] = []
for path in files:
    lines = git("show", f"{REV}:{path}").splitlines()
    for line_no, line in enumerate(lines, 1):
        for match in MUTATION.finditer(line):
            table = match.group(1).lower()
            if table not in {
                "files", "file_user_tags", "user_tags", "file_fingerprints",
                "duplicate_groups", "duplicate_group_members", "content_artifacts",
                "analysis_findings", "analysis_finding_evidence", "scan_roots",
                "scan_runs", "scan_sessions", "scan_session_roots", "scan_seen",
                "global_entries", "managed_entries", "managed_scopes", "ai_jobs",
                "classification_history", "classification_feedback", "organization_plans",
                "rule_proposals", "content_artifact_fts", "files_fts",
            }:
                continue
            fn, fn_line = enclosing_function(lines, line_no)
            test = is_test(path, line_no, lines)
            rows.append({
                "source_rev": REV,
                "path": path,
                "line": str(line_no),
                "enclosing_function": fn,
                "function_line": str(fn_line),
                "table": table,
                "scope": "test/fixture" if test else "production/schema",
                "source_text": " ".join(line.strip().split()),
            })

with (OUT / "mutation-reference-inventory.csv").open("w", newline="") as f:
    writer = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)

grouped: dict[tuple[str, str, str], list[dict[str, str]]] = defaultdict(list)
for row in rows:
    grouped[(row["path"], row["enclosing_function"], row["function_line"])].append(row)

functions = []
for (path, fn, fn_line), items in sorted(grouped.items()):
    functions.append({
        "source_rev": REV,
        "path": path,
        "function": fn,
        "function_line": fn_line,
        "tables_written": ";".join(sorted({item["table"] for item in items})),
        "reference_lines": ";".join(item["line"] for item in items),
        "scope": "test/fixture" if all(item["scope"] == "test/fixture" for item in items) else "production/schema",
    })
with (OUT / "mutation-function-inventory.csv").open("w", newline="") as f:
    writer = csv.DictWriter(f, fieldnames=list(functions[0]), lineterminator="\n")
    writer.writeheader()
    writer.writerows(functions)

print(f"revision={REV}")
print(f"rust_files={len(files)}")
print(f"literal_mutation_lines={len(rows)}")
print(f"mutation_function_groups={len(functions)}")
print(f"output={OUT}")
