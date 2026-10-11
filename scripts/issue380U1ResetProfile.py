"""Test-only reset/snapshot helper for a disposable U1 SQLite profile."""

import json
import sqlite3
import sys
from pathlib import Path


def _fts_hits(connection: sqlite3.Connection, marker: str) -> int:
    query = '"' + marker.replace('"', '""') + '"'
    return int(
        connection.execute(
            "SELECT COUNT(*) FROM global_entries_fts "
            "WHERE global_entries_fts MATCH ?",
            (query,),
        ).fetchone()[0]
    )


def _snapshot(
    connection: sqlite3.Connection, source_id: str, old_marker: str, next_marker: str
) -> dict[str, object]:
    source = connection.execute(
        """SELECT id, enabled, provider, index_status, last_error, journal_id,
                  journal_cursor, last_full_index_at, last_incremental_sync_at,
                  entry_count, updated_at
           FROM global_volumes WHERE id = ?""",
        (source_id,),
    ).fetchone()
    if source is None:
        raise RuntimeError("isolated source is absent")
    active_count = int(
        connection.execute(
            "SELECT COUNT(*) FROM global_entries WHERE volume_id = ? AND is_stale = 0",
            (source_id,),
        ).fetchone()[0]
    )
    stale_count = int(
        connection.execute(
            "SELECT COUNT(*) FROM global_entries WHERE volume_id = ? AND is_stale != 0",
            (source_id,),
        ).fetchone()[0]
    )
    old_active = int(
        connection.execute(
            """SELECT COUNT(*) FROM global_entries
               WHERE volume_id = ? AND name = ? AND is_stale = 0""",
            (source_id, old_marker),
        ).fetchone()[0]
    )
    next_active = int(
        connection.execute(
            """SELECT COUNT(*) FROM global_entries
               WHERE volume_id = ? AND name = ? AND is_stale = 0""",
            (source_id, next_marker),
        ).fetchone()[0]
    )
    return {
        "source": dict(source),
        "activeEntries": active_count,
        "staleEntries": stale_count,
        "oldMarkerActiveRows": old_active,
        "nextMarkerActiveRows": next_active,
        "oldMarkerFtsHits": _fts_hits(connection, old_marker),
        "nextMarkerFtsHits": _fts_hits(connection, next_marker),
    }


def main() -> int:
    if len(sys.argv) != 6 or sys.argv[1] not in {"reset", "snapshot"}:
        raise SystemExit(
            "usage: issue380U1ResetProfile.py <reset|snapshot> "
            "<db> <source-id> <old-marker> <next-marker>"
        )
    mode, database_path, source_id, old_marker, next_marker = sys.argv[1:]
    database = Path(database_path).resolve(strict=True)
    if mode == "snapshot":
        connection = sqlite3.connect(f"file:{database.as_posix()}?mode=ro", uri=True)
    else:
        connection = sqlite3.connect(database, timeout=30)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA busy_timeout=30000")
    try:
        before = _snapshot(connection, source_id, old_marker, next_marker)
        if mode == "reset":
            source = before["source"]
            if not source["enabled"]:
                raise RuntimeError("isolated source is disabled")
            if before["activeEntries"] < 1 or before["oldMarkerActiveRows"] != 1:
                raise RuntimeError("baseline did not leave one live old marker")
            if before["oldMarkerFtsHits"] < 1:
                raise RuntimeError("old marker is not searchable in Global Index FTS")
            connection.execute("BEGIN IMMEDIATE")
            cursor = connection.execute(
                """UPDATE global_volumes
                   SET index_status = 'discovered', last_error = NULL,
                       journal_id = NULL, journal_cursor = NULL,
                       last_full_index_at = NULL, last_incremental_sync_at = NULL,
                       updated_at = updated_at + 1
                   WHERE id = ? AND enabled = 1""",
                (source_id,),
            )
            if cursor.rowcount != 1:
                raise RuntimeError("test-only reset lost the enabled source row")
            connection.commit()
            after = _snapshot(connection, source_id, old_marker, next_marker)
            if (
                after["activeEntries"] != before["activeEntries"]
                or after["staleEntries"] != before["staleEntries"]
                or after["oldMarkerActiveRows"] != 1
                or after["oldMarkerFtsHits"] < 1
                or after["nextMarkerActiveRows"] != 0
                or after["nextMarkerFtsHits"] != 0
            ):
                raise RuntimeError("startup re-arm changed the active index or FTS")
            result = {
                "mode": mode,
                "reset": True,
                "sourceId": source_id,
                "oldMarker": old_marker,
                "nextMarker": next_marker,
                "before": before,
                "after": after,
                "statement": "only source scan metadata reset; active entries and FTS preserved",
            }
        else:
            result = {"mode": mode, "sourceId": source_id, **before}
        print(json.dumps(result, sort_keys=True))
    except Exception:
        if mode == "reset":
            connection.rollback()
        raise
    finally:
        connection.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
