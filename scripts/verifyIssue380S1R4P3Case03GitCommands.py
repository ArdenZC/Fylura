#!/usr/bin/env python3
"""Run the two Case 03 commands against real Git in a disposable repository.

This verifies Git CLI output only. It is not a PowerShell model and does not
execute the frozen PowerShell capture function.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path


MISSING_SHA1 = "0000000000000000000000000000000000000000"


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def run_raw(arguments: list[str], cwd: Path, env: dict[str, str]) -> dict[str, object]:
    start = utc_now()
    try:
        process = subprocess.run(
            arguments,
            cwd=cwd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    except OSError as error:
        return {
            "arguments": arguments,
            "started_utc": start,
            "ended_utc": utc_now(),
            "exit_code": None,
            "launch_error": f"{type(error).__name__}: {error}",
            "stdout_utf8_replace": "",
            "stdout_hex": "",
            "stdout_bytes": 0,
            "stderr_utf8_replace": "",
            "stderr_hex": "",
            "stderr_bytes": 0,
        }
    return {
        "arguments": arguments,
        "started_utc": start,
        "ended_utc": utc_now(),
        "exit_code": process.returncode,
        "stdout_utf8_replace": process.stdout.decode("utf-8", errors="replace"),
        "stdout_hex": process.stdout.hex(),
        "stdout_bytes": len(process.stdout),
        "stderr_utf8_replace": process.stderr.decode("utf-8", errors="replace"),
        "stderr_hex": process.stderr.hex(),
        "stderr_bytes": len(process.stderr),
    }


def build_observation() -> dict[str, object]:
    git_path = shutil.which("git")
    if not git_path:
        return {
            "status": "FAIL",
            "failure": "git executable was not found on PATH",
            "powershell_runtime": "NOT RUN BY THIS TOOL",
        }

    resolved_git = Path(git_path).resolve(strict=True)
    git_bytes = resolved_git.read_bytes()
    result: dict[str, object] = {
        "schema": "issue380-s1r4-p3-case03-real-git-observation-v1",
        "scope": "actual Git CLI in an isolated temporary repository; no PowerShell execution",
        "recorded_utc": utc_now(),
        "git_executable": str(resolved_git),
        "git_executable_sha256": hashlib.sha256(git_bytes).hexdigest(),
        "powershell_runtime": "NOT RUN",
        "repository": {
            "temporary": True,
            "object_format": "sha1",
            "contains_commit": False,
            "missing_object": MISSING_SHA1,
        },
        "commands": {},
        "assertions": [],
        "cleanup": {"temporary_repository_removed": False},
    }

    command_results: dict[str, dict[str, object]] = {}
    setup_error: str | None = None
    with tempfile.TemporaryDirectory(prefix="issue380-s1r4-p3-case03-") as temporary:
        root = Path(temporary)
        repo = root / "repo"
        repo.mkdir()
        empty_global = root / "empty-global.gitconfig"
        empty_global.write_bytes(b"")
        env = os.environ.copy()
        env.update(
            {
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": str(empty_global),
                "GIT_TERMINAL_PROMPT": "0",
                "GIT_OPTIONAL_LOCKS": "0",
            }
        )

        version = run_raw([str(resolved_git), "--version"], repo, env)
        init = run_raw(
            [str(resolved_git), "init", "--quiet", "--object-format=sha1"], repo, env
        )
        result["git_version"] = version["stdout_utf8_replace"].strip()
        result["git_version_command"] = version
        result["repository"]["initialization"] = init  # type: ignore[index]
        if init["exit_code"] != 0:
            setup_error = "git init did not return zero"
        else:
            original = run_raw(
                [str(resolved_git), "cat-file", "-e", MISSING_SHA1], repo, env
            )
            candidate = run_raw(
                [str(resolved_git), "cat-file", "-p", MISSING_SHA1], repo, env
            )
            command_results = {
                "original_cat_file_e_missing_object": original,
                "candidate_cat_file_p_missing_object": candidate,
            }
            result["commands"] = command_results
            result["assertions"] = [
                {
                    "name": "original_nonzero_empty_stderr_is_observed",
                    "passed": (
                        original["launch_error"] if "launch_error" in original else None
                    ) is None
                    and (
                        original["exit_code"] is not None
                        and original["exit_code"] != 0
                        and original["stdout_bytes"] == 0
                        and original["stderr_bytes"] == 0
                    ),
                    "expected": "nonzero exit; empty stdout; empty stderr",
                    "actual": {
                        "exit_code": original["exit_code"],
                        "stdout_bytes": original["stdout_bytes"],
                        "stderr_bytes": original["stderr_bytes"],
                    },
                },
                {
                    "name": "candidate_nonzero_stderr_is_observed_without_pinning_text",
                    "passed": (
                        candidate["launch_error"] if "launch_error" in candidate else None
                    ) is None
                    and (
                        candidate["exit_code"] is not None
                        and candidate["exit_code"] != 0
                        and candidate["stdout_bytes"] == 0
                        and candidate["stderr_bytes"] > 0
                    ),
                    "expected": "nonzero exit; empty stdout; non-empty stderr; no English text comparison",
                    "actual": {
                        "exit_code": candidate["exit_code"],
                        "stdout_bytes": candidate["stdout_bytes"],
                        "stderr_bytes": candidate["stderr_bytes"],
                    },
                },
            ]

    result["cleanup"] = {"temporary_repository_removed": not Path(temporary).exists()}
    all_assertions = result.get("assertions", [])
    assertions_pass = bool(all_assertions) and all(
        bool(assertion["passed"]) for assertion in all_assertions  # type: ignore[index]
    )
    result["status"] = "PASS" if setup_error is None and assertions_pass and result["cleanup"]["temporary_repository_removed"] else "FAIL"  # type: ignore[index]
    if setup_error is not None:
        result["failure"] = setup_error
    if not result["cleanup"]["temporary_repository_removed"]:  # type: ignore[index]
        result["failure"] = "temporary repository still exists after cleanup"
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(
            "docs/research/evidence/issue-380-s1r4-p3-pwsh-runtime/case03-real-git-observation.json"
        ),
        help="write actual Git output and contract assertions to this JSON file",
    )
    args = parser.parse_args()
    observation = build_observation()
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(observation, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"status": observation["status"], "output": str(output)}, ensure_ascii=False))
    return 0 if observation["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
