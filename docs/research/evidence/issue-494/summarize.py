"""Turn the #494 JSONL evidence into markdown tables.

Reads historical JSONL in this directory and native runner JSONL under
native/*.jsonl, then prints the source-separated tables used in
docs/research/ISSUE_494_FILENAME_AB_PROTOTYPE.md. Read-only: it never edits the
repository or the evidence.
"""

import json
import glob
import os
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))


def load():
    records = []
    paths = glob.glob(os.path.join(ROOT, "*.jsonl"))
    paths.extend(glob.glob(os.path.join(ROOT, "native", "*.jsonl")))
    for path in sorted(paths):
        with open(path, encoding="utf-8") as handle:
            for line in handle:
                line = line.strip()
                if not line or not line.startswith("{"):
                    continue
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    continue
                record["_file"] = os.path.basename(path)
                records.append(record)
    return records


def fmt(triple):
    return "/".join(f"{value:.1f}" for value in triple)


def main():
    records = load()
    queries = [r for r in records if r.get("record") == "query"]
    if not queries:
        print("no query records")
        return

    fixtures = {}
    for record in queries:
        fixtures.setdefault(
            (
                record.get("source_sha") or "historical",
                record.get("os", "unknown"),
                record.get("fixture"),
                record.get("entries"),
            ),
            [],
        ).append(record)

    for (source_sha, os_name, fixture, entries), rows in sorted(
        fixtures.items(), key=lambda kv: (kv[0][0], kv[0][1], kv[0][2], kv[0][3] or 0)
    ):
        print(f"\n### {fixture} / {entries} rows / {os_name} / source {source_sha}\n")
        header = (
            "| Query | Hits | N | FTS matches | Prod SQL page | Prod pooled page | Prod Snapshot page | "
            "A page | A window | A enumerate | B filename page | B filename window | "
            "B full page | B full window | Extension SQL |"
        )
        print(header)
        print("|" + "---|" * 15)
        for record in rows:
            print(
                "| `{q}` | {hits} | {samples} | {fts} | {p} | {pp} | {ps} | {ap} | {a} | {ae} | "
                "{bfp} | {bf} | {bfullp} | {bfull} | {ext} |".format(
                    q=record.get("query"),
                    hits=record.get("oracle_hits"),
                    samples=record.get("percentile_samples", record.get("samples", "?")),
                    fts=record.get("production_fts_matches"),
                    p=fmt(record.get("production_sql", [0, 0, 0])),
                    pp=fmt(record.get("production_pooled", [0, 0, 0])),
                    ps=fmt(record.get("production_snapshot", [0, 0, 0])),
                    ap=fmt(record.get("a_page", [0, 0, 0])),
                    a=fmt(record.get("a_full", [0, 0, 0])),
                    ae=fmt(record.get("a_enumeration_only", [0, 0, 0])),
                    bfp=fmt(record.get("b_filename_only_page", [0, 0, 0])),
                    bf=fmt(record.get("b_filename_only", [0, 0, 0])),
                    bfullp=fmt(record.get("b_full_page", [0, 0, 0])),
                    bfull=fmt(record.get("b_full", [0, 0, 0])),
                    ext=fmt(record.get("extension_channel_sql", [0, 0, 0])),
                )
            )

    builds = [r for r in records if r.get("record") == "compact_view_build"]
    if builds:
        print("\n### B cold build and resources\n")
        print("| Source SHA | OS | Fixture | Rows | Build ms | Estimated heap MiB | Name MiB | Dir MiB | Id MiB | Slot MiB | Dirs | RSS start MiB | RSS peak MiB | Peak delta MiB | RSS samples | CPU peak % | Read MiB | Written MiB |")
        print("|" + "---|" * 18)
        for record in builds:
            stats = record.get("stats", {})
            resources = record.get("resources", {})
            mb = 1024 * 1024
            print(
                "| {source} | {os} | {f} | {rows} | {ms:.0f} | {total:.1f} | {names:.1f} | {dirs_b:.1f} | {ids:.1f} | {slots:.1f} | {distinct} | {start:.1f} | {peak:.1f} | {delta:.1f} | {samples} | {cpu:.0f} | {read:.1f} | {written:.1f} |".format(
                    source=record.get("source_sha", "historical"),
                    os=record.get("os", "unknown"),
                    f=record.get("fixture"),
                    rows=record.get("rows"),
                    ms=record.get("cold_build_ms", 0),
                    total=stats.get("estimated_total_bytes", 0) / mb,
                    names=stats.get("name_bytes", 0) / mb,
                    dirs_b=stats.get("directory_bytes", 0) / mb,
                    ids=stats.get("id_bytes", 0) / mb,
                    slots=stats.get("slot_bytes", 0) / mb,
                    distinct=stats.get("distinct_directories"),
                    start=resources.get("rss_start_bytes", 0) / mb,
                    peak=resources.get("rss_peak_bytes", 0) / mb,
                    delta=(resources.get("rss_peak_bytes", 0) - resources.get("rss_start_bytes", 0)) / mb,
                    samples=resources.get("samples", 0),
                    cpu=resources.get("cpu_peak_percent", 0),
                    read=resources.get("read_bytes", 0) / mb,
                    written=resources.get("written_bytes", 0) / mb,
                )
            )

    fixtures = [r for r in records if r.get("record") == "fixture"]
    if fixtures:
        print("\n### Fixture build\n")
        print("| Source SHA | OS | Fixture | Entries | Build ms | DB MiB |")
        print("|---|---|---|---|---|---|")
        for record in fixtures:
            print(
                "| {source} | {os} | {f} | {e} | {ms:.0f} | {db:.1f} |".format(
                    source=record.get("source_sha", "historical"),
                    os=record.get("os", "unknown"),
                    f=record.get("fixture"),
                    e=record.get("entries"),
                    ms=record.get("fixture_build_ms", 0),
                    db=record.get("db_bytes", 0) / (1024 * 1024),
                )
            )

    incremental = [r for r in records if r.get("record") == "incremental"]
    if incremental:
        print("\n### Incremental write and rebuild\n")
        print("| Source SHA | OS | Fixture | Entries | Insert 512 ms | Delete 512 ms | Rebuild after insert ms | Rebuild after delete ms | Bytes/row | Narrow cache slots | Narrowed p50/95/99 | Full p50/95/99 |")
        print("|" + "---|" * 12)
        for record in incremental:
            data = record.get("incremental", {})
            print(
                "| {source} | {os} | {f} | {e} | {i:.0f} | {d:.0f} | {ri:.0f} | {rd:.0f} | {amp:.0f} | {cache} | {n} | {full} |".format(
                    source=record.get("source_sha", "historical"),
                    os=record.get("os", "unknown"),
                    f=record.get("fixture"),
                    e=record.get("entries"),
                    i=data.get("insert_512_ms", 0),
                    d=data.get("delete_512_ms", 0),
                    ri=data.get("full_rebuild_after_insert_ms", 0),
                    rd=data.get("full_rebuild_after_delete_ms", 0),
                    amp=data.get("write_amplification_bytes_per_row", 0),
                    cache=data.get("narrow_cache_slots"),
                    n=fmt(data.get("narrowed_query", [0, 0, 0])),
                    full=fmt(data.get("full_scan_query", [0, 0, 0])),
                )
            )


if __name__ == "__main__":
    main()
