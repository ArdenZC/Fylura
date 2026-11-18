#!/usr/bin/env python3
"""Report which top-level lifecycle block the runner's Bash cannot parse."""

import re
import subprocess
import sys


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: verifyMacosBashCompatibility.py <bash-script>", file=sys.stderr)
        return 2

    source_path = sys.argv[1]
    with open(source_path, encoding="utf-8") as source_file:
        lines = source_file.readlines()

    function_pattern = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\(\) \{\s*$")
    function_starts = []
    for index, line in enumerate(lines):
        match = function_pattern.match(line)
        if match:
            function_starts.append((index, match.group(1)))

    if not function_starts:
        print("No top-level functions were found in " + source_path, file=sys.stderr)
        return 1

    trap_index = next(
        (index for index, line in enumerate(lines) if line.strip() == "trap cleanup EXIT"),
        len(lines),
    )
    for position, (start, name) in enumerate(function_starts):
        if name == "cleanup":
            end = trap_index
        elif position + 1 < len(function_starts):
            end = function_starts[position + 1][0]
        else:
            end = trap_index

        result = subprocess.run(
            ["bash", "-n"],
            input="".join(lines[start:end]),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            print("Bash parse failed in function " + name + ":", file=sys.stderr)
            print(result.stderr.rstrip(), file=sys.stderr)
            return result.returncode

    if trap_index < len(lines):
        result = subprocess.run(
            ["bash", "-n"],
            input="".join(lines[trap_index:]),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            print("Bash parse failed in the top-level lifecycle sequence:", file=sys.stderr)
            print(result.stderr.rstrip(), file=sys.stderr)
            return result.returncode

    print("The runner Bash parsed all lifecycle functions and the top-level sequence.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
