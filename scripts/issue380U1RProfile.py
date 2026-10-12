"""Test-only fixture and read-only snapshot helper for disposable U1R SQLite profiles."""

from __future__ import annotations

import json
import sqlite3
import sys
import time
from pathlib import Path


def _fts_count(conn: sqlite3.Connection, marker: str) -> int:
    query = '"' + marker.replace('"', '""') + '"'
    return int(
        conn.execute(
            "SELECT COUNT(*) FROM global_entries_fts WHERE global_entries_fts MATCH ?",
            (query,),
        ).fetchone()[0]
    )


def _snapshot(conn: sqlite3.Connection, source_id: str, marker: str) -> dict[str, object]:
    source = conn.execute(
        """SELECT id, enabled, provider, index_status, last_error, journal_id,
                  journal_cursor, last_full_index_at, last_incremental_sync_at,
                  entry_count, updated_at
           FROM global_volumes WHERE id = ?""",
        (source_id,),
    ).fetchone()
    if source is None:
        raise RuntimeError("isolated source row missing")
    entry = conn.execute(
        """SELECT id, name, path, is_stale, source_provider
           FROM global_entries WHERE volume_id = ? AND name = ? ORDER BY id LIMIT 1""",
        (source_id, marker),
    ).fetchone()
    scopes = conn.execute(
        """SELECT id, path, global_entry_id, enabled, allow_local_ai, allow_cloud_ai
           FROM managed_scopes WHERE global_entry_id = ? ORDER BY id""",
        (entry["id"],) if entry else ("__missing__",),
    ).fetchall()
    managed = conn.execute(
        """SELECT id, global_entry_id, managed_scope_id, enabled
           FROM managed_entries WHERE global_entry_id = ? ORDER BY id""",
        (entry["id"],) if entry else ("__missing__",),
    ).fetchall()
    analysis = conn.execute(
        """SELECT global_entry_id, status, input_fingerprint, provider, model,
                  content_summary, classification_json, user_corrected, last_error
           FROM ai_analysis_state WHERE global_entry_id = ?""",
        (entry["id"],) if entry else ("__missing__",),
    ).fetchall()
    jobs = conn.execute(
        """SELECT id, global_entry_id, managed_scope_id, status, attempt_count,
                  last_error, created_at, started_at, completed_at
           FROM ai_jobs WHERE global_entry_id = ? ORDER BY id""",
        (entry["id"],) if entry else ("__missing__",),
    ).fetchall()
    job_items = conn.execute(
        """SELECT id, job_id, global_entry_id, status, last_error
           FROM ai_job_items WHERE global_entry_id = ? ORDER BY id""",
        (entry["id"],) if entry else ("__missing__",),
    ).fetchall()
    active = int(
        conn.execute(
            "SELECT COUNT(*) FROM global_entries WHERE volume_id = ? AND is_stale = 0",
            (source_id,),
        ).fetchone()[0]
    )
    stale = int(
        conn.execute(
            "SELECT COUNT(*) FROM global_entries WHERE volume_id = ? AND is_stale != 0",
            (source_id,),
        ).fetchone()[0]
    )
    return {
        "source": dict(source),
        "entry": dict(entry) if entry else None,
        "activeEntries": active,
        "staleEntries": stale,
        "markerFtsHits": _fts_count(conn, marker),
        "managedScopes": [dict(row) for row in scopes],
        "managedEntries": [dict(row) for row in managed],
        "aiAnalysisState": [dict(row) for row in analysis],
        "aiJobs": [dict(row) for row in jobs],
        "aiJobItems": [dict(row) for row in job_items],
        "counts": {
            "managedScopes": int(conn.execute("SELECT COUNT(*) FROM managed_scopes").fetchone()[0]),
            "managedEntries": int(conn.execute("SELECT COUNT(*) FROM managed_entries").fetchone()[0]),
            "aiAnalysisState": int(conn.execute("SELECT COUNT(*) FROM ai_analysis_state").fetchone()[0]),
            "aiJobs": int(conn.execute("SELECT COUNT(*) FROM ai_jobs").fetchone()[0]),
            "aiJobItems": int(conn.execute("SELECT COUNT(*) FROM ai_job_items").fetchone()[0]),
        },
    }


def _checkpoint(conn: sqlite3.Connection) -> None:
    result = conn.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
    if result and int(result[0]) != 0:
        raise RuntimeError(f"WAL checkpoint was busy: {result}")


def main() -> int:
    if len(sys.argv) != 5 or sys.argv[1] not in {"seed-rich", "set-ready", "snapshot"}:
        raise SystemExit("usage: issue380U1RProfile.py <seed-rich|set-ready|snapshot> <db> <source-id> <marker>")
    mode, raw_db, source_id, marker = sys.argv[1:]
    db = Path(raw_db).resolve(strict=True)
    if mode == "snapshot":
        conn = sqlite3.connect(f"{db.as_uri()}?mode=ro", uri=True, timeout=30)
    else:
        conn = sqlite3.connect(db, timeout=30)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA busy_timeout=30000")
    if mode != "snapshot":
        conn.execute("PRAGMA foreign_keys=ON")
    try:
        schema = int(conn.execute("PRAGMA user_version").fetchone()[0])
        if schema != 37:
            raise RuntimeError(f"fixture schema is {schema}; expected unchanged production Schema 37")
        before = _snapshot(conn, source_id, marker)
        entry = before["entry"]
        if mode == "seed-rich":
            if not entry or int(entry["is_stale"]) != 0 or before["markerFtsHits"] < 1:
                raise RuntimeError("fixture marker is not an active, searchable Global Entry")
            source = before["source"]
            now = int(source["updated_at"]) + 1
            wall_clock = int(time.time())
            # A disposable legacy-indexed profile may not have checkpoint or
            # incremental metadata yet. Seed only missing values using the same
            # sentinel values used by the old master's recovery QA path. Preserve
            # any metadata observed from the actual old binary and report the
            # provenance of every field so synthetic values are never mistaken
            # for real volume state.
            checkpoint = {
                "journal_id": source["journal_id"] or "18446744073709551615",
                "journal_cursor": source["journal_cursor"] or "9223372036854775807",
                "last_full_index_at": source["last_full_index_at"] if source["last_full_index_at"] is not None else max(1, wall_clock - 1),
                "last_incremental_sync_at": source["last_incremental_sync_at"] if source["last_incremental_sync_at"] is not None else max(1, wall_clock),
            }
            checkpoint_provenance = {
                key: "observed_from_old_master_index" if source[key] not in (None, "") else "synthetic_u1r_legacy_recovery_qa_fixture"
                for key in checkpoint
            }
            if int(checkpoint["last_full_index_at"]) > int(checkpoint["last_incremental_sync_at"]):
                raise RuntimeError("effective U1R timestamps are not chronologically valid")
            scope_id = "u1r-synthetic-managed-scope"
            job_id = "u1r-synthetic-ai-job"
            conn.execute("BEGIN IMMEDIATE")
            cursor = conn.execute(
                """UPDATE global_volumes SET journal_id=?, journal_cursor=?,
                          last_full_index_at=?, last_incremental_sync_at=?
                   WHERE id=? AND enabled=1""",
                (
                    checkpoint["journal_id"],
                    checkpoint["journal_cursor"],
                    checkpoint["last_full_index_at"],
                    checkpoint["last_incremental_sync_at"],
                    source_id,
                ),
            )
            if cursor.rowcount != 1:
                raise RuntimeError("could not seed missing metadata on the isolated enabled source")
            conn.execute(
                """INSERT INTO managed_scopes
                   (id, path, global_entry_id, enabled, allow_local_ai, allow_cloud_ai, created_at, updated_at)
                   VALUES (?, ?, ?, 1, 1, 0, ?, ?)""",
                (scope_id, f"{entry['path']}\\..u1r-managed-root", entry["id"], now, now),
            )
            conn.execute(
                """INSERT INTO managed_entries
                   (id, global_entry_id, managed_scope_id, enabled, created_at, updated_at)
                   VALUES (?, ?, ?, 1, ?, ?)""",
                ("u1r-synthetic-managed-entry", entry["id"], scope_id, now, now),
            )
            conn.execute(
                """INSERT INTO ai_analysis_state
                   (global_entry_id, status, input_fingerprint, provider, model,
                    content_summary, classification_json, user_corrected, updated_at)
                   VALUES (?, 'completed', 'u1r-fixture-fingerprint', 'local', 'fixture',
                    'synthetic U1R association', '{"fixture":"u1r","user_note":"preserve for test"}', 1, ?)""",
                (entry["id"], now),
            )
            conn.execute(
                """INSERT INTO ai_jobs
                   (id, global_entry_id, managed_scope_id, input_fingerprint, provider,
                    model, processing_mode, status, attempt_count, created_at)
                   VALUES (?, ?, ?, 'u1r-fixture-fingerprint', 'local', 'fixture',
                    'metadata', 'pending', 0, ?)""",
                (job_id, entry["id"], scope_id, now),
            )
            conn.execute(
                """INSERT INTO ai_job_items
                   (id, job_id, global_entry_id, status, created_at, updated_at)
                   VALUES ('u1r-synthetic-ai-job-item', ?, ?, 'pending', ?, ?)""",
                (job_id, entry["id"], now, now),
            )
            cursor = conn.execute(
                """UPDATE global_volumes SET index_status='rebuild_required',
                          last_error=NULL, updated_at=updated_at+1
                   WHERE id=? AND enabled=1 AND journal_id IS NOT NULL
                     AND journal_cursor IS NOT NULL AND last_full_index_at IS NOT NULL
                     AND last_incremental_sync_at IS NOT NULL""",
                (source_id,),
            )
            if cursor.rowcount != 1:
                raise RuntimeError("could not arm rebuild while preserving nonempty baseline checkpoint")
            conn.commit()
            after = _snapshot(conn, source_id, marker)
            if (
                after["entry"] != before["entry"]
                or after["markerFtsHits"] != before["markerFtsHits"]
                or after["source"]["journal_id"] != checkpoint["journal_id"]
                or after["source"]["journal_cursor"] != checkpoint["journal_cursor"]
                or after["source"]["last_full_index_at"] != checkpoint["last_full_index_at"]
                or after["source"]["last_incremental_sync_at"] != checkpoint["last_incremental_sync_at"]
                or len(after["managedEntries"]) != 1
                or len(after["aiAnalysisState"]) != 1
                or len(after["aiJobs"]) != 1
                or len(after["aiJobItems"]) != 1
            ):
                raise RuntimeError("rich pre-request fixture did not preserve its baseline index metadata")
            _checkpoint(conn)
            result = {
                "mode": mode,
                "schemaVersion": schema,
                "sourceId": source_id,
                "marker": marker,
                "checkpointProvenance": checkpoint_provenance,
                "effectiveCheckpoint": checkpoint,
                "before": before,
                "after": _snapshot(conn, source_id, marker),
            }
        elif mode == "set-ready":
            source = before["source"]
            if not entry or not source["journal_id"] or not source["journal_cursor"]:
                raise RuntimeError("ready clone must retain a real checkpoint and active marker")
            if source["last_full_index_at"] is None or source["last_incremental_sync_at"] is None:
                raise RuntimeError("ready clone must retain both real index timestamps")
            conn.execute("BEGIN IMMEDIATE")
            cursor = conn.execute(
                """UPDATE global_volumes SET index_status='ready', last_error=NULL,
                          updated_at=updated_at+1 WHERE id=? AND enabled=1""",
                (source_id,),
            )
            if cursor.rowcount != 1:
                raise RuntimeError("could not set isolated installer clone to READY")
            after = _snapshot(conn, source_id, marker)
            if (
                after["entry"] != before["entry"]
                or after["markerFtsHits"] != before["markerFtsHits"]
                or after["managedScopes"] != before["managedScopes"]
                or after["managedEntries"] != before["managedEntries"]
                or after["aiAnalysisState"] != before["aiAnalysisState"]
                or after["aiJobs"] != before["aiJobs"]
                or after["aiJobItems"] != before["aiJobItems"]
                or after["source"]["journal_id"] != source["journal_id"]
                or after["source"]["journal_cursor"] != source["journal_cursor"]
                or after["source"]["last_full_index_at"] != source["last_full_index_at"]
                or after["source"]["last_incremental_sync_at"] != source["last_incremental_sync_at"]
            ):
                raise RuntimeError("setting READY changed the rich index or association fixture")
            conn.commit()
            _checkpoint(conn)
            result = {"mode": mode, "schemaVersion": schema, "sourceId": source_id, "marker": marker, "before": before, "after": after}
        else:
            result = {"mode": mode, "schemaVersion": schema, "sourceId": source_id, "marker": marker, **before}
        raw = json.dumps(result, sort_keys=True, separators=(",", ":"))
        print(raw)
    except Exception:
        if mode in {"seed-rich", "set-ready"}:
            conn.rollback()
        raise
    finally:
        conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
