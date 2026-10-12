#!/usr/bin/env python3
"""Build a source-linked stale-reference inventory from a frozen Git tree.

Usage from the repository root:
  python3 docs/project/research/issue-381-d4-s1/source-audit/build_inventory.py \
    3275ac7c72df8ae4139a30f16c7f649971324e97 \
    docs/project/research/issue-381-d4-s1/source-audit

This is a text-level inventory, not a Rust/SQL parser. The ADR names the
manually reviewed indirect callers, trigger effects, and background tasks.
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


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], text=True)


def is_test_context(path: str, line_no: int, source_lines: list[str]) -> bool:
    if "/tests/" in path or "/db/tests/" in path:
        return True
    for i, line in enumerate(source_lines[:line_no]):
        if re.search(r"#\[cfg\(test\)\]", line):
            following = "\n".join(source_lines[i : i + 4])
            if re.search(r"\bmod\s+\w+\s*\{", following):
                return True
    return False


def enclosing_function(source_lines: list[str], line_no: int) -> tuple[str, int]:
    for i in range(line_no - 1, -1, -1):
        match = re.search(r"\b(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(", source_lines[i])
        if match:
            return match.group(1), i + 1
    return "<module-or-type-field>", 0


def classify(path: str, line: str, context: str, test: bool, fn: str) -> tuple[str, str]:
    low = (line + " " + context).lower()
    if test:
        return "test/fixture", "test or diagnostic fixture"
    if "create trigger" in low or "create view" in low or "drop trigger" in low:
        return "schema/derived-object", "trigger or view definition"
    if path.endswith("/db/schema.rs"):
        if fn.startswith("ensure_global_index"):
            return "global-index-schema", "independent global_entries schema"
        if fn == "migrate":
            return "schema/migration", "legacy schema migration and compatibility"
        if fn == "ensure_content_schema":
            return "managed-file-trigger", "managed file derived-state trigger"
        return "managed-file-schema", "managed files column, index, or migration"
    if "/global_index/" in path:
        if "from files" in low or "update files" in low or "files where" in low:
            return "managed-file/global-index-bridge", "managed File Library bridge"
        return "global-index", "independent global_entries visibility"
    if "update files" in low or re.search(r"\bset\s+is_stale\s*=", low):
        return "managed-file-writer", "managed files stale-state mutation"
    if "is_stale:" in line or "is_stale," in line or "is_stale)" in line:
        return "managed-file-projection", "managed file DTO or row projection"
    if "is_stale" in low and any(word in low for word in ("where", "and", "filter", "select", "!file.")):
        return "managed-file-reader", "managed file availability read or guard"
    return "managed-file-reference", f"managed file reference in {fn}"


OUT.mkdir(parents=True, exist_ok=True)
paths = [p for p in git("ls-tree", "-r", "--name-only", REV, ROOT).splitlines() if p.endswith(".rs")]
rows: list[dict[str, str]] = []
source_cache: dict[str, list[str]] = {}
for path in paths:
    try:
        source_lines = git("show", f"{REV}:{path}").splitlines()
    except subprocess.CalledProcessError:
        continue
    source_cache[path] = source_lines
    for line_no, line in enumerate(source_lines, 1):
        if "is_stale" not in line:
            continue
        test = is_test_context(path, line_no, source_lines)
        fn, fn_line = enclosing_function(source_lines, line_no)
        context = " ".join(source_lines[max(0, line_no - 8) : min(len(source_lines), line_no + 6)])
        domain, role = classify(path, line, context, test, fn)
        rows.append(
            {
                "source_rev": REV,
                "path": path,
                "line": str(line_no),
                "enclosing_function": fn,
                "function_line": str(fn_line),
                "domain": domain,
                "role": role,
                "source_text": " ".join(line.strip().split()),
            }
        )

with (OUT / "stale-reference-inventory.csv").open("w", newline="") as f:
    writer = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)

grouped: dict[tuple[str, str, str], list[dict[str, str]]] = defaultdict(list)
for row in rows:
    grouped[(row["path"], row["enclosing_function"], row["function_line"])].append(row)

function_rows = []
for (path, fn, fn_line), items in sorted(grouped.items()):
    function_rows.append(
        {
            "source_rev": REV,
            "domains": ";".join(sorted({item["domain"] for item in items})),
            "path": path,
            "function": fn,
            "function_line": fn_line,
            "reference_lines": ";".join(item["line"] for item in items),
            "roles": ";".join(sorted({item["role"] for item in items})),
            "reference_count": str(len(items)),
        }
    )
with (OUT / "function-inventory.csv").open("w", newline="") as f:
    writer = csv.DictWriter(f, fieldnames=list(function_rows[0]), lineterminator="\n")
    writer.writeheader()
    writer.writerows(function_rows)

production = [row for row in rows if not row["domain"].startswith("test/")]
print(f"revision={REV}")
print(f"rust_files={len(paths)}")
print(f"stale_reference_lines={len(rows)}")
print(f"production_reference_lines={len(production)}")
print(f"function_groups={len(function_rows)}")
print(f"output={OUT}")
