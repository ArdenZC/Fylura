#!/usr/bin/env python3
"""Fail-closed gates for the single Issue #494 1M native qualification."""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path


LIMIT_MS = 100.0
PRIVATE_LIMIT_BYTES = 128 * 1024 * 1024
WRITE_OVERHEAD_LIMIT_PERCENT = 10.0


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"FAIL: {message}")


def number(value: object, label: str) -> float:
    require(not isinstance(value, bool), f"{label} is missing or non-numeric")
    if isinstance(value, (int, float)):
        parsed = float(value)
    elif isinstance(value, str):
        try:
            parsed = float(value.strip())
        except ValueError as error:
            raise SystemExit(f"FAIL: {label} is missing or non-numeric") from error
    else:
        raise SystemExit(f"FAIL: {label} is missing or non-numeric")
    require(math.isfinite(parsed), f"{label} is not finite")
    return parsed


def read_jsonl(path: Path) -> list[dict]:
    records = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError as error:
            raise SystemExit(f"FAIL: malformed JSONL line {line_number}: {error}") from error
        require(isinstance(record, dict), f"JSONL line {line_number} is not an object")
        records.append(record)
    return records


def route_p95(record: dict, route: str, label: str) -> float:
    routes = record.get("routes")
    require(isinstance(routes, dict), f"{label} has no route distributions")
    distribution = routes.get(route)
    require(isinstance(distribution, dict), f"{label} has no {route} distribution")
    return number(distribution.get("p95"), f"{label}.{route}.p95")


def main() -> None:
    require(len(sys.argv) == 3, "usage: validate_issue494_1m_evidence.py JSONL SOURCE")
    jsonl_path, source_path = map(Path, sys.argv[1:])
    require(jsonl_path.is_file(), f"missing raw JSONL: {jsonl_path}")
    require(source_path.is_file(), f"missing native source record: {source_path}")
    records = read_jsonl(jsonl_path)

    def of_type(record_type: str) -> list[dict]:
        return [record for record in records if record.get("record_type") == record_type]

    started = of_type("issue494_1m_native_qualification_started")
    searches = of_type("issue494_1m_search_pair")
    snapshots = of_type("issue494_1m_snapshot_pair")
    windows = of_type("issue494_1m_full_candidate_window")
    revocations = of_type("issue494_1m_source_revocation_fallback")
    summaries = of_type("issue494_filename_first_1m_native_qualification")
    require(len(started) == 1, "expected exactly one 1M qualification start record")
    require(len(summaries) == 1, "expected exactly one complete 1M qualification summary")
    require(len(windows) == 1, "expected the complete 4,096-result candidate window record")
    require(len(revocations) == 1, "expected one source-revocation fallback record")

    context = started[0].get("context")
    require(isinstance(context, dict), "start record has no build context")
    require(context.get("entries") == 1_000_000, "fixture is not exactly 1M entries")
    require(context.get("debug_assertions") is False, "native run is not a Release build")
    require(context.get("desktop_runtime_feature") is True, "desktop-runtime feature was not enabled")
    working_tree = context.get("working_tree")
    require(
        working_tree in ("", "?? ../ci-evidence/"),
        "qualification source tree was dirty beyond the generated evidence directory",
    )
    source_text = source_path.read_text(encoding="utf-8")
    source_fields = dict(
        line.split("=", 1) for line in source_text.splitlines() if "=" in line
    )
    require(context.get("source_sha") == source_fields.get("candidate_sha"), "JSONL source SHA differs from native source record")
    require(context.get("actual_head") == source_fields.get("candidate_sha"), "test harness HEAD differs from native source record")
    require(context.get("actual_tree") == source_fields.get("candidate_tree"), "test harness tree differs from native source record")
    require(context.get("runner_os", "").lower() == source_fields.get("runner_os", "").lower(), "runner OS differs between test and source record")
    require(source_fields.get("hardware_cpu"), "native CPU model is missing")
    require(number(source_fields.get("physical_memory_bytes"), "native physical memory") > 0, "native physical memory is missing")
    require(number(source_fields.get("workspace_volume_free_bytes"), "native workspace free disk") > 0, "native free disk is missing")
    require(source_fields.get("os_version"), "native OS version is missing")
    require(context.get("runner_os", "").lower() in ("windows", "macos"), "unsupported runner OS")
    require(
        (context.get("runner_os") == "windows" and context.get("runner_arch") == "x86_64")
        or (context.get("runner_os") == "macos" and context.get("runner_arch") == "aarch64"),
        "native OS/architecture does not match Windows x86_64 or Apple Silicon macOS",
    )

    summary = summaries[0]
    require(summary.get("context") == context, "final summary context differs from the start record")
    fixture = summary.get("fixture", {})
    construction = summary.get("construction", {})
    require(fixture.get("source_rows_verified") == 1_000_000, "SQLite source row count is not 1M")
    required_triggers = {
        "global_entries_ai",
        "global_entries_ad",
        "global_entries_au",
        "global_entries_count_ai",
        "global_entries_count_ad",
        "global_entries_count_au",
    }
    require(required_triggers.issubset(set(fixture.get("production_triggers_preserved", []))), "production entry triggers were not all present")
    require(construction.get("index_entries") == 1_000_000, "candidate index does not contain 1M entries")

    high_search = {
        record.get("query_class"): record
        for record in searches
        if record.get("query_class") in {"high_filename_substring", "high_filename_substring_deep_window"}
    }
    high_snapshot = {
        record.get("query_class"): record
        for record in snapshots
        if record.get("query_class") in {"high_filename_substring", "high_filename_substring_deep_window"}
    }
    required_classes = {"high_filename_substring", "high_filename_substring_deep_window"}
    expected_query_classes = {
        "single_name_prefix",
        "one_filename_substring",
        "high_filename_substring",
        "high_filename_substring_deep_window",
        "zero_filename_match",
        "sparse_filename_substring",
    }
    require({record.get("query_class") for record in searches} == expected_query_classes, "Search query matrix is incomplete")
    require({record.get("query_class") for record in snapshots} == expected_query_classes, "Snapshot query matrix is incomplete")
    for record in searches:
        require(record.get("independent_oracle_page_ids") == record.get("rowid_only_page_ids"), f"{record.get('query_class')} Search page differs from its oracle")
    for record in snapshots:
        require(record.get("independent_oracle_page_ids") == record.get("rowid_only_ids"), f"{record.get('query_class')} Snapshot page differs from its oracle")
    require(set(high_search) == required_classes, "missing first-page or offset-4016 Search record")
    require(set(high_snapshot) == required_classes, "missing first-page or offset-4016 Snapshot record")
    search_p95 = {}
    snapshot_p95 = {}
    expected_offsets = {"high_filename_substring": 0, "high_filename_substring_deep_window": 4_016}
    for query_class in sorted(required_classes):
        search = high_search[query_class]
        snapshot = high_snapshot[query_class]
        require(search.get("offset") == expected_offsets[query_class], f"Search {query_class} has the wrong offset")
        require(snapshot.get("offset") == expected_offsets[query_class], f"Snapshot {query_class} has the wrong offset")
        for record, label in ((search, "Search"), (snapshot, "Snapshot")):
            raw = record.get("route_raw_ms", {}).get("rowid_only_candidate")
            require(isinstance(raw, list) and len(raw) == 30, f"{label} {query_class} does not have 30 measured samples")
        require(search.get("expected_total_matches") == 50_000, f"{query_class} did not exercise exactly 50,000 hits")
        require(search.get("both_candidate_routes_match_independent_oracle_ids_and_order") is True, f"{query_class} Search failed its ordering oracle")
        require(snapshot.get("both_candidate_routes_match_independent_oracle_ids_and_order") is True, f"{query_class} Snapshot failed its ordering oracle")
        require(snapshot.get("all_source_health_revision_and_index_status_equal") is True, f"{query_class} Snapshot source facts differ by route")
        require(search.get("independent_oracle_page_ids") == search.get("rowid_only_page_ids"), f"{query_class} Search page differs from its independent oracle")
        require(snapshot.get("independent_oracle_page_ids") == snapshot.get("rowid_only_ids"), f"{query_class} Snapshot page differs from its independent oracle")
        search_p95[query_class] = route_p95(search, "rowid_only_candidate", f"Search {query_class}")
        snapshot_p95[query_class] = route_p95(snapshot, "rowid_only_candidate", f"Snapshot {query_class}")
        require(search_p95[query_class] <= LIMIT_MS, f"Search {query_class} p95 exceeds {LIMIT_MS} ms")
        require(snapshot_p95[query_class] <= LIMIT_MS, f"Snapshot {query_class} p95 exceeds {LIMIT_MS} ms")

    window = windows[0]
    actual_window = window.get("candidate_ids")
    oracle_window = window.get("independent_oracle_ids")
    require(window.get("candidate_window_size") == 4_096, "candidate window is not 4,096")
    require(window.get("full_candidate_window_and_order_equal") is True, "complete candidate window ordering was not verified")
    require(isinstance(actual_window, list) and len(actual_window) == 4_096, "candidate window did not contain 4,096 IDs")
    require(actual_window == oracle_window, "complete candidate window differs from independent oracle")

    revocation = revocations[0]
    require(revocation.get("source_enabled_after_revocation") == 0, "revoked source remains enabled")
    require(revocation.get("scope_authorization_after_revocation") == "not_authorized", "source authorization was not revoked")
    require(revocation.get("source_health_reports_disabled") is True, "revoked source health was not disabled in Snapshot")
    require(revocation.get("source_revision_changed") is True, "source revision did not change after revocation")
    require(revocation.get("candidate_route_safely_fell_back_on_source_change") is True, "candidate did not report safe authorization fallback")
    require(revocation.get("revoked_candidate_snapshot_equals_sqlite_snapshot") is True, "revoked snapshot differs from SQLite fallback")
    require(revocation.get("no_revoked_source_results_returned") is True, "revoked source returned results")
    require(revocation.get("candidate_rebuild_worker_settled") is True, "candidate rebuild worker did not settle")
    require(
        number(revocation.get("source_fallback_counter_after"), "source fallback counter after")
        > number(revocation.get("source_fallback_counter_before"), "source fallback counter before"),
        "source fallback counter did not advance",
    )

    resources = summary.get("resource_attribution", {})
    sampled_peak = construction.get("sampled_process_index_build_peak", {})
    require(sampled_peak.get("measured") is True and sampled_peak.get("sample_count", 0) > 0, "process peak during index build was not sampled")
    private_deltas = [
        sampled_peak.get("peak_private_delta_from_prebuild_bytes"),
        resources.get("private_bytes_delta_index_build"),
        resources.get("private_bytes_delta_before_index_to_after_queries_and_writes"),
    ]
    require(all(isinstance(value, (int, float)) for value in private_deltas), "native process private-memory deltas are incomplete")
    peak_private_delta = max(float(value) for value in private_deltas)
    require(peak_private_delta <= PRIVATE_LIMIT_BYTES, "additional process private memory exceeds the 128 MiB ceiling")
    heap_peak = number(construction.get("index_build_peak_bytes"), "index build peak heap bytes")
    require(heap_peak <= PRIVATE_LIMIT_BYTES, "candidate index heap peak exceeds the 128 MiB ceiling")
    require(construction.get("persistent_search_index_bytes") == 0, "unexpected persistent sidecar index was created")
    idle = resources.get("idle_30m", {})
    require(idle.get("measured") is True and idle.get("requested_seconds") == 1_800, "30-minute idle capture is missing")
    idle_cpu = number(idle.get("normalized_one_core_cpu_percent"), "30-minute idle process CPU")
    require(idle_cpu <= 0.5, "30-minute average idle CPU exceeds 0.5%")

    writes = summary.get("paired_batch_writes", [])
    require("forced-FULL-checkpoint stress" in summary.get("write_cost_semantics", ""), "1M write cost policy is not explicitly identified")
    by_batch = {entry.get("batch_size"): entry for entry in writes}
    require(set(by_batch) == {512, 5_000}, "1M paired write evidence is incomplete")
    write_overhead = {}
    for batch_size in (512, 5_000):
        entry = by_batch[batch_size]
        value = number(entry.get("candidate_p95_over_baseline_percent"), f"{batch_size}-row write overhead")
        write_overhead[batch_size] = value
        require(value <= WRITE_OVERHEAD_LIMIT_PERCENT, f"{batch_size}-row write stress overhead exceeds 10%")

    print(
        json.dumps(
            {
                "status": "PASS",
                "candidate_sha": source_fields["candidate_sha"],
                "candidate_tree": source_fields["candidate_tree"],
                "runner_os": context["runner_os"],
                "runner_arch": context["runner_arch"],
                "search_p95_ms": search_p95,
                "snapshot_p95_ms": snapshot_p95,
                "all_search_candidate_p95_ms": {
                    record["query_class"]: route_p95(record, "rowid_only_candidate", f"Search {record['query_class']}")
                    for record in searches
                },
                "all_snapshot_candidate_p95_ms": {
                    record["query_class"]: route_p95(record, "rowid_only_candidate", f"Snapshot {record['query_class']}")
                    for record in snapshots
                },
                "candidate_window_size": len(actual_window),
                "peak_private_delta_bytes": peak_private_delta,
                "idle_cpu_percent": idle_cpu,
                "forced_full_checkpoint_write_stress_overhead_percent": write_overhead,
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
