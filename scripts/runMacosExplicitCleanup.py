#!/usr/bin/env python3
"""Run Fylura's explicit macOS cleanup command with a bounded timeout."""

import os
import subprocess
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: runMacosExplicitCleanup.py <app-binary> <task-temp>", file=sys.stderr)
        return 2

    environment = os.environ.copy()
    environment["TMPDIR"] = sys.argv[2]
    environment["FYLURA_CLEANUP_DIAGNOSTICS"] = "1"
    try:
        result = subprocess.run(
            [sys.argv[1], "--fylura-uninstall-clean-user-data"],
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=180,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        output = error.stdout or ""
        if isinstance(output, bytes):
            output = output.decode("utf-8", errors="replace")
        print(output, end="")
        print("Fylura explicit user-data cleanup timed out after 180 seconds.", file=sys.stderr)
        return 124

    print(result.stdout, end="")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
