#!/usr/bin/env python3
"""Run the isolated B4-P1A matrix and preserve every command's raw output."""

from __future__ import annotations

import datetime as dt
import gzip
import hashlib
import json
import os
import platform
import subprocess
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARNESS = ROOT / "tests/issue381_b4_p1a_f1"
EVIDENCE = ROOT / "docs/project/research/issue-381-b4-p1a/evidence/final"
RAW = EVIDENCE / "raw"
RUNS: list[dict[str, object]] = []


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def run_command(name: str, args: list[str], env: dict[str, str]) -> subprocess.CompletedProcess[bytes]:
    completed = subprocess.run(args, cwd=ROOT, env=env, capture_output=True, check=False)
    stdout_path = RAW / f"{name}.stdout"
    stderr_path = RAW / f"{name}.stderr"
    stdout_path.write_bytes(completed.stdout)
    stderr_path.write_bytes(completed.stderr)
    RUNS.append(
        {
            "name": name,
            "argv": args,
            "cwd": str(ROOT),
            "exit_code": completed.returncode,
            "stdout_path": str(stdout_path.relative_to(ROOT)),
            "stdout_sha256": sha256(stdout_path),
            "stdout_bytes": stdout_path.stat().st_size,
            "stderr_path": str(stderr_path.relative_to(ROOT)),
            "stderr_sha256": sha256(stderr_path),
            "stderr_bytes": stderr_path.stat().st_size,
        }
    )
    return completed


def registry_packages(lock_path: Path) -> dict[tuple[str, str], str]:
    with lock_path.open("rb") as stream:
        lock = tomllib.load(stream)
    return {
        (entry["name"], entry["version"]): entry["checksum"]
        for entry in lock["package"]
        if entry.get("source", "").startswith("registry+")
    }


def check_lock_parity() -> dict[str, object]:
    harness_packages = registry_packages(HARNESS / "Cargo.lock")
    product_packages = registry_packages(ROOT / "src-tauri/Cargo.lock")
    mismatches = []
    for package, checksum in sorted(harness_packages.items()):
        if product_packages.get(package) != checksum:
            mismatches.append(
                {
                    "name": package[0],
                    "version": package[1],
                    "harness_checksum": checksum,
                    "product_checksum": product_packages.get(package),
                }
            )
    return {
        "harness_registry_package_count": len(harness_packages),
        "product_registry_package_count": len(product_packages),
        "all_harness_package_versions_and_checksums_in_product_lock": not mismatches,
        "mismatches": mismatches,
    }


def parse_jsonl(path: Path) -> list[dict[str, object]]:
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rt", encoding="utf-8") as stream:
        return [json.loads(line) for line in stream if line.strip()]


def validate_matrix(native_path: Path, oracle_path: Path) -> dict[str, object]:
    records = parse_jsonl(native_path)
    oracle = parse_jsonl(oracle_path)
    cases = {
        str(record["case_id"]): record
        for record in records
        if record.get("record_type") == "case_result"
    }
    oracle_ok = len(oracle) == 12 and all(row.get("status") == "PASS" for row in oracle)
    oracle_summary = {
        "query_snapshots": len(oracle),
        "passed": sum(row.get("status") == "PASS" for row in oracle),
        "failed": sum(row.get("status") != "PASS" for row in oracle),
        "exact_float_bits_and_full_sorted_rows": oracle_ok
        and all(row.get("exact_float_bits_and_order") is True for row in oracle),
    }
    expected = {
        "P1A-T01": "PASS",
        "P1A-T02": "PASS",
        "P1A-T03-short-query": "PASS",
        "P1A-T04": "PASS",
        "P1A-T05": "PASS",
        "P1A-T06": "PASS",
        "P1A-T07": "PASS",
        "P1A-T08": "PASS",
        "P1A-T09": "PASS",
        "P1A-T10": "ENVIRONMENT_UNAVAILABLE",
        "P1A-T11": "PASS",
        "P1A-T12": "PASS",
        "P1A-N01": "EXPECTED_REJECTION",
        "P1A-N02": "EXPECTED_REJECTION",
        "P1A-N03": "EXPECTED_REJECTION",
        "P1A-N04": "EXPECTED_REJECTION",
    }
    matrix = []
    for case_id, expected_status in expected.items():
        actual = str(cases.get(case_id, {}).get("status", "MISSING"))
        passed = actual == expected_status
        if case_id == "P1A-T03-short-query":
            passed = passed and oracle_ok
        matrix.append(
            {
                "case_id": case_id,
                "expected_status": expected_status,
                "actual_status": actual,
                "gate_satisfied": passed,
                "details": cases.get(case_id, {}).get("details", {}),
            }
        )
    all_satisfied = all(row["gate_satisfied"] for row in matrix)
    native_exit = next(
        (entry["exit_code"] for entry in RUNS if entry["name"] == "matrix-native"),
        None,
    )
    outcome = "P1A_BLOCKED_ENVIRONMENT" if all_satisfied and cases.get("P1A-T10", {}).get("status") == "ENVIRONMENT_UNAVAILABLE" else "P1A_FAIL"
    return {
        "matrix_version": "B4-P1A / 16 issue gates; T03 includes 12 independent query snapshots",
        "gates": matrix,
        "gate_count": len(matrix),
        "satisfied_gate_count": sum(row["gate_satisfied"] for row in matrix),
        "independent_oracle": oracle_summary,
        "native_exit_code": native_exit,
        "all_declared_expectations_satisfied": all_satisfied and oracle_ok and native_exit == 0,
        "overall_decision": outcome,
        "native_gate": "BLOCKED: native Windows/macOS handle evidence unavailable on Linux executor",
        "power_loss_gate": "NOT_ESTABLISHED: process kill and fsync model evidence are not sudden power-loss evidence",
    }


def main() -> int:
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    RAW.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(
        {
            "PATH": f"/workspace/.cache/cargo/bin:{env.get('PATH', '')}",
            "CARGO_HOME": "/workspace/.cache/cargo",
            "RUSTUP_HOME": "/workspace/.cache/rustup",
            "CARGO_TARGET_DIR": "/tmp/issue381-b4-p1a-target",
        }
    )
    manifest = str(HARNESS / "Cargo.toml")
    for name, args in [
        ("rustc-version", ["rustc", "--version", "--verbose"]),
        ("cargo-version", ["cargo", "--version"]),
        ("cargo-tree-features", ["cargo", "tree", "-e", "features", "--locked", "--manifest-path", manifest]),
        ("cargo-fmt-check", ["cargo", "fmt", "--check", "--manifest-path", manifest]),
        ("cargo-build-release", ["cargo", "build", "--release", "--locked", "--manifest-path", manifest]),
    ]:
        result = run_command(name, args, env)
        if result.returncode != 0:
            break

    lock_parity = check_lock_parity()
    parity_path = EVIDENCE / "cargo-lock-parity.json"
    parity_path.write_text(json.dumps(lock_parity, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    build_ok = all(entry["exit_code"] == 0 for entry in RUNS)
    if build_ok and lock_parity["all_harness_package_versions_and_checksums_in_product_lock"]:
        binary = Path(env["CARGO_TARGET_DIR"]) / "release/issue381-b4-p1a-f1"
        matrix_native = RAW / "matrix-native.stdout.jsonl.gz"
        native_result = subprocess.run(
            [str(binary), "--samples", "100"], cwd=ROOT, env=env, capture_output=True, check=False
        )
        matrix_native.write_bytes(gzip.compress(native_result.stdout, compresslevel=9, mtime=0))
        native_stderr = RAW / "matrix-native.stderr.txt"
        native_stderr.write_bytes(native_result.stderr)
        RUNS.append(
            {
                "name": "matrix-native",
                "argv": [str(binary), "--samples", "100"],
                "cwd": str(ROOT),
                "exit_code": native_result.returncode,
                "stdout_path": str(matrix_native.relative_to(ROOT)),
                "stdout_sha256": sha256(matrix_native),
                "stdout_bytes": matrix_native.stat().st_size,
                "stdout_uncompressed_sha256": hashlib.sha256(native_result.stdout).hexdigest(),
                "stdout_uncompressed_bytes": len(native_result.stdout),
                "stderr_path": str(native_stderr.relative_to(ROOT)),
                "stderr_sha256": sha256(native_stderr),
                "stderr_bytes": native_stderr.stat().st_size,
            }
        )
        oracle_path = RAW / "matrix-oracle-checks.jsonl"
        oracle_stdout = RAW / "matrix-oracle.stdout"
        oracle_err = RAW / "matrix-oracle.stderr"
        oracle_result = subprocess.run(
            [
                sys.executable,
                str(HARNESS / "oracle.py"),
                str(matrix_native),
                str(oracle_path),
            ],
            cwd=ROOT,
            env=env,
            capture_output=True,
            check=False,
        )
        oracle_stdout.write_bytes(oracle_result.stdout)
        oracle_err.write_bytes(oracle_result.stderr)
        RUNS.append(
            {
                "name": "matrix-oracle",
                "argv": [sys.executable, str(HARNESS / "oracle.py"), str(matrix_native), str(oracle_path)],
                "cwd": str(ROOT),
                "exit_code": oracle_result.returncode,
                "stdout_path": str(oracle_stdout.relative_to(ROOT)),
                "stdout_sha256": sha256(oracle_stdout),
                "stdout_bytes": oracle_stdout.stat().st_size,
                "stderr_path": str(oracle_err.relative_to(ROOT)),
                "stderr_sha256": sha256(oracle_err),
                "stderr_bytes": oracle_err.stat().st_size,
                "checks_path": str(oracle_path.relative_to(ROOT)),
                "checks_sha256": sha256(oracle_path),
                "checks_bytes": oracle_path.stat().st_size,
            }
        )
        matrix = validate_matrix(matrix_native, oracle_path)
        # The oracle writes per-snapshot checks separately from its one-line summary.
        # Keep that stream as a second first-class input to the matrix validator.
        matrix["independent_oracle"]["summary_stdout"] = oracle_result.stdout.decode("utf-8", "replace").strip()
        matrix["independent_oracle"]["oracle_exit_code"] = oracle_result.returncode
        matrix_path = EVIDENCE / "test-matrix.json"
        matrix_path.write_text(json.dumps(matrix, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    else:
        matrix = {
            "overall_decision": "P1A_FAIL",
            "all_declared_expectations_satisfied": False,
            "reason": "Build or product-lock parity failed; formal matrix was not run",
            "lock_parity": lock_parity,
        }
        matrix_path = EVIDENCE / "test-matrix.json"
        matrix_path.write_text(json.dumps(matrix, sort_keys=True, indent=2) + "\n", encoding="utf-8")

    source = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, check=True).stdout.decode().strip()
    tree = subprocess.run(["git", "rev-parse", "HEAD^{tree}"], cwd=ROOT, capture_output=True, check=True).stdout.decode().strip()
    branch = subprocess.run(["git", "branch", "--show-current"], cwd=ROOT, capture_output=True, check=True).stdout.decode().strip()
    base = subprocess.run(["git", "rev-parse", "HEAD^"], cwd=ROOT, capture_output=True, check=True).stdout.decode().strip()
    fs_type = subprocess.run(["stat", "-f", "-c", "%T", str(ROOT)], cwd=ROOT, capture_output=True, check=False).stdout.decode().strip()
    environment = {
        "recorded_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "repository": "ArdenZC/Zen-Canvas",
        "branch": branch,
        "source_head": source,
        "source_tree": tree,
        "base_commit": base,
        "executor_os": platform.platform(),
        "executor_kernel": platform.release(),
        "executor_architecture": platform.machine(),
        "workspace_filesystem_type": fs_type,
        "lock_parity": lock_parity,
        "commands": RUNS,
        "native_gate": "ENVIRONMENT_UNAVAILABLE (requires hosted native Windows and macOS runs)",
        "power_loss_gate": "not established by process kill or fsync",
    }
    (EVIDENCE / "environment-and-commands.json").write_text(
        json.dumps(environment, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    (EVIDENCE / "run-receipt.json").write_text(
        json.dumps(
            {
                "source_head": source,
                "source_tree": tree,
                "all_commands_succeeded": all(entry["exit_code"] == 0 for entry in RUNS),
                "matrix_decision": matrix.get("overall_decision"),
                "matrix_gates_satisfied": matrix.get("satisfied_gate_count"),
                "matrix_gate_count": matrix.get("gate_count"),
            },
            sort_keys=True,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return 0 if matrix.get("all_declared_expectations_satisfied") is True else 1


if __name__ == "__main__":
    raise SystemExit(main())
