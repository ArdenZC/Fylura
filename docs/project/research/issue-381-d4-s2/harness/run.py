#!/usr/bin/env python3
"""One-shot, test-only Schema 37 physical identity migration harness."""

from __future__ import annotations

import hashlib
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[5]
HERE = Path(__file__).resolve().parents[1]
SCHEMA_PATH = ROOT / "docs/project/research/issue-381-d4-s0/evidence/schema37-sqlite-schema.sql"
CANDIDATE_PATH = HERE / "harness/physical-candidate.sql"
EVIDENCE = HERE / "evidence"
MIGRATION_KEY = "test-only-381-d4s2-source-incarnation-v1"
NOW = 1_800_000_000


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def split_sql(sql: str) -> list[str]:
    statements: list[str] = []
    buffer = ""
    for line in sql.splitlines(True):
        buffer += line
        if sqlite3.complete_statement(buffer):
            statement = "\n".join(
                line for line in buffer.splitlines() if not line.lstrip().startswith("--")
            ).strip()
            buffer = ""
            if statement:
                statements.append(statement)
    if buffer.strip():
        raise AssertionError(f"incomplete SQL tail: {buffer[-120:]!r}")
    return statements


def schema_category(statement: str) -> int:
    upper = statement.upper()
    if upper.startswith("CREATE VIRTUAL TABLE"):
        return 1
    if upper.startswith("CREATE TABLE"):
        match = re.match(r"CREATE TABLE\s+['\"]?([^ '\"]+)", statement, re.I)
        if match and "_fts_" in match.group(1):
            return -1  # FTS creates its own shadow tables.
        return 0
    if upper.startswith(("CREATE INDEX", "CREATE UNIQUE INDEX")):
        return 2
    if upper.startswith("CREATE TRIGGER"):
        return 3
    return 4


def execute_schema37(conn: sqlite3.Connection) -> None:
    conn.execute("PRAGMA foreign_keys=ON")
    statements = split_sql(SCHEMA_PATH.read_text(encoding="utf-8"))
    for category in (0, 1, 2, 3, 4):
        for statement in statements:
            if schema_category(statement) == category:
                conn.execute(statement)
    conn.execute("PRAGMA user_version=37")
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []


def insert_legacy_fixture(conn: sqlite3.Connection) -> dict[str, str]:
    path = "/managed/reused/legacy marker.txt"
    file_id = path  # Frozen Scanner behavior: id == normalized path in Schema 37.
    conn.execute(
        """INSERT INTO files
           (id,path,name,extension,size,mtime,is_dir,state_code,file_type,purpose,
            lifecycle,context,risk_level,suggested_action,classification_reason,
            is_stale,last_seen_at,content_hash,classification_status)
           VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
        (file_id, path, "legacyfilemarker.txt", "txt", 23, 10, 0, 0,
         "Document", "Reference", "Archive", "owner-note-legacy", "Normal",
         "Keep", "legacy user classification", 0, NOW, "legacy-content-hash", "classified"),
    )
    conn.execute(
        "INSERT INTO user_tags(id,display_name,normalized_name,created_at,updated_at) VALUES(?,?,?,?,?)",
        ("tag-owner", "Owner tag", "owner tag", NOW, NOW),
    )
    conn.execute(
        "INSERT INTO file_user_tags(file_id,tag_id,created_at) VALUES(?,?,?)",
        (file_id, "tag-owner", NOW),
    )
    conn.execute(
        """INSERT INTO file_fingerprints
           (file_id,path_snapshot,identity_status,platform_kind,platform_volume_id,
            platform_file_id,physical_key,size,modified_ns,full_hash,fingerprint_status,
            captured_at,last_verified_at)
           VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)""",
        (file_id, path, "verified", "unix", "vol-old", "ino-old", "unix:v1:vol-old:ino-old",
         23, 10, "hash-old", "complete", NOW, NOW),
    )
    conn.execute(
        """INSERT INTO dedupe_runs
           (id,request_key,scope_json,scope_hash,status,phase,created_at,updated_at)
           VALUES(?,?,?,?,?,?,?,?)""",
        ("dedupe-old", "request-old", "{}", "scope-old", "completed", "completed", NOW, NOW),
    )
    conn.execute(
        """INSERT INTO duplicate_groups
           (id,size_each,full_hash,member_count,physical_copy_count,
            potential_reclaimable_bytes,reclaimable_confidence,status,last_built_run_id,
            created_at,updated_at,last_verified_at)
           VALUES(?,?,?,?,?,?,?,?,?,?,?,?)""",
        ("dupe-old", 23, "hash-old", 1, 1, 23, "exact", "active", "dedupe-old",
         NOW, NOW, NOW),
    )
    conn.execute(
        """INSERT INTO duplicate_group_members
           (group_id,file_id,path_snapshot,physical_key,identity_status,size,verified_at)
           VALUES(?,?,?,?,?,?,?)""",
        ("dupe-old", file_id, path, "unix:v1:vol-old:ino-old", "verified", 23, NOW),
    )
    conn.execute("INSERT INTO content_catalog(id,updated_at) VALUES(1,?)", (NOW,))
    conn.execute(
        """INSERT INTO content_runs
           (id,scope_json,scope_fingerprint,mode,provider_mode,status,
            expected_library_revision,policy_fingerprint,created_at,updated_at)
           VALUES(?,?,'scope-content-old','local','none','completed',1,'policy-old',?,?)""",
        ("content-run-old", "{}", NOW, NOW),
    )
    conn.execute(
        """INSERT INTO content_run_items
           (id,run_id,file_id,ordinal,status,source_size,source_mtime,created_at,updated_at)
           VALUES(?,?,?,0,'completed',23,10,?,?)""",
        ("content-item-old", "content-run-old", file_id, NOW, NOW),
    )
    conn.execute(
        """INSERT INTO content_artifacts
           (id,file_id,source_size,source_mtime,source_is_dir,extractor_family,
            extractor_version,policy_revision,content_fingerprint,status,summary,
            keywords_json,language,text_retained,raw_text,created_at,updated_at)
           VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
        ("artifact-old", file_id, 23, 10, 0, "text", "test-1", 1, "fp-old", "current",
         "legacy content summary token", '["legacycontenttoken"]', "en", 1,
         "legacycontenttoken retained body", NOW, NOW),
    )
    conn.execute(
        """INSERT INTO content_artifact_fts
           (artifact_id,summary,keywords,language,raw_text) VALUES(?,?,?,?,?)""",
        ("artifact-old", "legacy content summary token", "legacycontenttoken", "en",
         "legacycontenttoken retained body"),
    )
    conn.execute(
        """INSERT INTO classification_history
           (id,file_id,file_name,file_path,source,file_type,purpose,lifecycle,risk_level,
            suggested_action,suggested_target_path,created_at)
           VALUES(?,?,?,?,?,?,?,?,?,?,?,?)""",
        ("classification-old", file_id, "legacyfilemarker.txt", path, "user", "Document",
         "Reference", "Archive", "Normal", "Keep", "", NOW),
    )
    conn.execute(
        """INSERT INTO classification_feedback
           (id,file_id,file_name,original_json,corrected_json,created_at)
           VALUES(?,?,?,?,?,?)""",
        ("feedback-old", file_id, "legacyfilemarker.txt", '{"purpose":"Unknown"}',
         '{"purpose":"Reference"}', NOW),
    )
    conn.execute(
        """INSERT INTO analysis_runs
           (id,request_key,scope_json,scope_hash,source_snapshot_json,source_snapshot_hash,
            detector_set_json,detector_set_hash,status,phase,created_at,updated_at)
           VALUES(?,?,?,?,?,?,?,?,?,?,?,?)""",
        ("analysis-old", "analysis-request-old", "{}", "scope-old", "{}", "snapshot-old",
         "[]", "detectors-old", "completed", "completed", NOW, NOW),
    )
    conn.execute(
        """INSERT INTO analysis_findings
           (id,finding_key,run_id,detector_id,detector_version,scope_hash,status,tier,
            category,action_kind,title,reason,confidence,primary_subject_kind,
            primary_subject_id,path_snapshot,identity_snapshot_json,created_at,updated_at)
           VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
        ("finding-old", "finding-key-old", "analysis-old", "detector-old", 1, "scope-old",
         "active", "review", "retained", "none", "Retained finding", "legacy source evidence",
         "unknown", "path", path, path, '{"legacy":true}', NOW, NOW),
    )
    conn.execute(
        """INSERT INTO analysis_finding_evidence
           (id,finding_id,evidence_kind,subject_kind,subject_id,path_snapshot,value_json,created_at)
           VALUES(?,?,?,?,?,?,?,?)""",
        ("evidence-old", "finding-old", "legacy_snapshot", "path", file_id, path,
         '{"ownerData":"retain"}', NOW),
    )
    conn.execute(
        "INSERT INTO operation_batches(id,created_at,status) VALUES(?,?,?)",
        ("op-batch-old", NOW, "completed"),
    )
    conn.execute(
        """INSERT INTO operation_logs
           (id,batch_id,operation_type,source_path,target_path,old_name,new_name,status,
            created_at,path_before,path_after,name_before,name_after)
           VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)""",
        ("op-old", "op-batch-old", "rename", path, path, "legacyfilemarker.txt",
         "legacyfilemarker.txt", "success", NOW, path, path, "legacyfilemarker.txt",
         "legacyfilemarker.txt"),
    )
    conn.execute(
        """INSERT INTO global_volumes
           (id,platform,stable_volume_id,display_name,mount_path,filesystem_type,drive_kind,
            provider,created_at,updated_at)
           VALUES(?,?,?,?,?,?,?,?,?,?)""",
        ("volume-global", "unix", "stable-vol-global", "volume", "/managed", "testfs",
         "fixed", "test", NOW, NOW),
    )
    conn.execute(
        """INSERT INTO global_entries
           (id,volume_id,platform_file_id,name,name_normalized,path,path_normalized,extension,
            size,source_provider,last_seen_at)
           VALUES(?,?,?,?,?,?,?,?,?,?,?)""",
        ("global-old", "volume-global", "global-file-old", "legacyfilemarker.txt",
         "legacyfilemarker.txt", path, path, "txt", 23, "test", NOW),
    )
    conn.execute(
        """INSERT INTO ai_analysis_state
           (global_entry_id,status,input_fingerprint,provider,model,content_summary,
            classification_json,user_corrected,updated_at)
           VALUES(?,?,?,?,?,?,?,?,?)""",
        ("global-old", "completed", "ai-input-old", "local", "fixture-model",
         "legacy AI result", '{"kind":"legacy"}', 1, NOW),
    )
    return {"legacy_file_id": file_id, "legacy_path": path}


def capture_file_objects(conn: sqlite3.Connection) -> tuple[list[str], list[str]]:
    indexes = [
        row[0]
        for row in conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='index' AND tbl_name='files' AND sql IS NOT NULL ORDER BY name"
        )
    ]
    triggers = [
        row[0]
        for row in conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND tbl_name='files' ORDER BY name"
        )
    ]
    return indexes, triggers


def rebuild_files_and_apply_candidate(conn: sqlite3.Connection, force_constraint_failure: bool = False) -> None:
    original_sql = conn.execute(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='files'"
    ).fetchone()[0]
    assert re.search(r"path\s+TEXT\s+NOT NULL\s+UNIQUE", original_sql, re.I)
    rebuilt_sql = re.sub(
        r"CREATE TABLE\s+files\s*\(", "CREATE TABLE files__d4s2_rebuild (", original_sql,
        count=1, flags=re.I,
    )
    rebuilt_sql = re.sub(r"path\s+TEXT\s+NOT NULL\s+UNIQUE", "path TEXT NOT NULL", rebuilt_sql,
                         count=1, flags=re.I)
    rebuilt_sql = re.sub(r"\bid\s+TEXT\s+PRIMARY KEY\s*,",
                         "id TEXT PRIMARY KEY, source_id TEXT NOT NULL UNIQUE,",
                         rebuilt_sql, count=1, flags=re.I)
    indexes, triggers = capture_file_objects(conn)
    conn.execute(rebuilt_sql)
    legacy_columns = [row[1] for row in conn.execute("PRAGMA table_info(files)")]
    quote = lambda column: '"' + column.replace('"', '""') + '"'
    target_columns = [legacy_columns[0], "source_id", *legacy_columns[1:]]
    target_sql = ",".join(quote(column) for column in target_columns)
    select_sql = ",".join([
        quote(legacy_columns[0]),
        "'legacy-source:'||lower(hex(randomblob(16)))",
        *(quote(column) for column in legacy_columns[1:]),
    ])
    conn.execute(
        f"INSERT INTO files__d4s2_rebuild(rowid,{target_sql}) SELECT rowid,{select_sql} FROM files"
    )
    conn.execute("DROP TABLE files")
    conn.execute("ALTER TABLE files__d4s2_rebuild RENAME TO files")
    for statement in indexes + triggers:
        conn.execute(statement)
    for statement in split_sql(CANDIDATE_PATH.read_text(encoding="utf-8")):
        conn.execute(statement)

    legacy_rows = conn.execute("SELECT id,path,source_id FROM files ORDER BY id").fetchall()
    for file_id, path, source_id in legacy_rows:
        path_entry_id = "legacy-entry:" + source_id.split(":", 1)[1]
        conn.execute(
            """INSERT INTO file_sources
               (source_id,file_id,status,identity_confidence,source_revision,created_at)
               VALUES(?,?,'legacy_unverified','none',1,?)""",
            (source_id, file_id, NOW),
        )
        conn.execute(
            """INSERT INTO file_path_entries
               (path_entry_id,source_id,path_text,path_key,path_key_version,identity_status,observed_at)
               VALUES(?,?,?,?,1,'legacy_unverified',?)""",
            (path_entry_id, source_id, path, path, NOW),
        )
    if force_constraint_failure:
        conn.execute(
            """INSERT INTO file_sources
               (source_id,file_id,status,identity_confidence,created_at)
               VALUES('bad-status','missing-file','not-a-status','none',?)""",
            (NOW,),
        )


def migrate_candidate(conn: sqlite3.Connection, fail: bool = False) -> str:
    existing = conn.execute(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='d4s2_candidate_migration_journal'"
    ).fetchone()
    if existing and conn.execute(
        "SELECT 1 FROM d4s2_candidate_migration_journal WHERE migration_key=?", (MIGRATION_KEY,)
    ).fetchone():
        assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
        return "already_applied"
    conn.execute("PRAGMA foreign_keys=OFF")
    conn.execute("BEGIN IMMEDIATE")
    try:
        rebuild_files_and_apply_candidate(conn, force_constraint_failure=fail)
        conn.execute(
            """INSERT INTO d4s2_candidate_migration_journal
               (migration_key,input_schema_sha256,candidate_ddl_sha256,applied_at)
               VALUES(?,?,?,?)""",
            (MIGRATION_KEY, sha256_bytes(SCHEMA_PATH.read_bytes()),
             sha256_bytes(CANDIDATE_PATH.read_bytes()), NOW),
        )
        foreign_key_errors = conn.execute("PRAGMA foreign_key_check").fetchall()
        if foreign_key_errors:
            raise AssertionError(f"foreign key check failed before commit: {foreign_key_errors}")
        if conn.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
            raise AssertionError("integrity check failed before commit")
        conn.execute("INSERT INTO files_fts(files_fts,rank) VALUES('integrity-check',1)")
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.execute("PRAGMA foreign_keys=ON")
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    return "applied"


def open_db(path: Path | str = ":memory:") -> sqlite3.Connection:
    conn = sqlite3.connect(path, isolation_level=None, timeout=5.0)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def file_row(conn: sqlite3.Connection, file_id: str, source_id: str, path: str, name: str,
             size: int, content_hash: str = "") -> None:
    conn.execute(
        """INSERT INTO files
           (id,source_id,path,name,extension,size,mtime,is_dir,state_code,file_type,purpose,lifecycle,
            context,risk_level,suggested_action,classification_reason,is_stale,last_seen_at,
            content_hash,classification_status)
           VALUES(?,?,?,?,?,?,1,0,0,'Document','Reference','Inbox','', 'Normal','Keep',
                  'scanner candidate',0,?,?,'classified')""",
        (file_id, source_id, path, name, "txt", size, NOW, content_hash),
    )


def insert_physical_identity(conn: sqlite3.Connection, identity_id: str, file_identity: str) -> None:
    conn.execute(
        """INSERT INTO file_physical_identity_epochs
           (physical_identity_id,platform_kind,volume_identity,platform_file_identity,
            identity_epoch,first_verified_at,last_verified_at,evidence_digest)
           VALUES(?,'unix','vol-test',?,?,?, ?,?)""",
        (identity_id, file_identity, "observed-epoch-1", NOW, NOW,
         sha256_bytes(identity_id.encode())),
    )


def insert_path_entry(conn: sqlite3.Connection, entry_id: str, source_id: str,
                      path: str, identity_id: str, identity_status: str = "verified") -> None:
    conn.execute(
        """INSERT INTO file_path_entries
           (path_entry_id,source_id,path_text,path_key,path_key_version,identity_status,
            physical_identity_id,evidence_digest,observed_at)
           VALUES(?,?,?,?,1,?,?,?,?)""",
        (entry_id, source_id, path, path, identity_status, identity_id,
         sha256_bytes((entry_id + identity_id).encode()), NOW),
    )


def insert_root_generation(conn: sqlite3.Connection, root_id: str, root_path: str,
                           generation_id: str, run_id: str, generation_no: int,
                           source_path_pairs: list[tuple[str, str]]) -> None:
    conn.execute(
        """INSERT OR IGNORE INTO scan_roots
           (id,normalized_path,display_name,created_at,updated_at)
           VALUES(?,?,?,?,?)""",
        (root_id, root_path, root_id, NOW, NOW),
    )
    conn.execute(
        """INSERT INTO scan_runs
           (id,scan_root_id,generation,lease_token,status,phase,created_at,updated_at,
            coverage_complete,watcher_revision_at_start)
           VALUES(?,?,?,?, 'completed','completed',?,?,1,0)""",
        (run_id, root_id, generation_no, "lease:" + run_id, NOW, NOW),
    )
    conn.execute(
        """INSERT INTO source_generations
           (generation_id,root_id,run_id,status,root_revision,watcher_revision,
            member_count,member_digest,completed_at)
           VALUES(?,?,?,'complete',1,0,?,?,?)""",
        (generation_id, root_id, run_id, len(source_path_pairs),
         sha256_bytes(json.dumps(source_path_pairs, sort_keys=True).encode()), NOW),
    )
    for source_id, path_entry_id in source_path_pairs:
        conn.execute(
            """INSERT INTO source_generation_members
               (generation_id,source_id,path_entry_id,identity_digest)
               VALUES(?,?,?,?)""",
            (generation_id, source_id, path_entry_id,
             sha256_bytes((source_id + path_entry_id).encode())),
        )


def create_manifest(conn: sqlite3.Connection, manifest_id: str,
                    root_generations: list[tuple[str, str]],
                    projections: list[tuple[str, str, str, str]],
                    expected_parent: str | None,
                    manage_transaction: bool = True) -> None:
    owns_transaction = manage_transaction and not conn.in_transaction
    if owns_transaction:
        conn.execute("BEGIN IMMEDIATE")
    try:
        conn.execute(
            "INSERT INTO visibility_manifests(manifest_id,parent_manifest_id,status,created_at,root_map_digest) VALUES(?,?,'ready',?,?)",
            (manifest_id, expected_parent, NOW,
             sha256_bytes(json.dumps(root_generations, sort_keys=True).encode())),
        )
        for root_id, generation_id in root_generations:
            conn.execute(
                "INSERT INTO manifest_root_heads(manifest_id,root_id,generation_id) VALUES(?,?,?)",
                (manifest_id, root_id, generation_id),
            )
        for source_id, path_entry_id, path_key, path_text in projections:
            conn.execute(
                """INSERT INTO manifest_file_paths
                   (manifest_id,source_id,path_entry_id,path_key,projected_path)
                   VALUES(?,?,?,?,?)""",
                (manifest_id, source_id, path_entry_id, path_key, path_text),
            )
        if expected_parent is None:
            conn.execute(
                "INSERT INTO active_visibility_pointer(singleton_id,manifest_id,revision,updated_at) VALUES(1,?,1,?)",
                (manifest_id, NOW),
            )
        else:
            changed = conn.execute(
                """UPDATE active_visibility_pointer
                   SET manifest_id=?,revision=revision+1,updated_at=?
                   WHERE singleton_id=1 AND manifest_id=?""",
                (manifest_id, NOW, expected_parent),
            ).rowcount
            assert changed == 1, "manifest pointer CAS failed"
        conn.execute(
            "UPDATE visibility_manifests SET status='published' WHERE manifest_id=?",
            (manifest_id,),
        )
        if owns_transaction:
            conn.commit()
    except Exception:
        if owns_transaction and conn.in_transaction:
            conn.rollback()
        raise


def visible_source_ids(conn: sqlite3.Connection) -> list[str]:
    return [row[0] for row in conn.execute(
        "SELECT source_id FROM effective_file_sources GROUP BY source_id ORDER BY source_id"
    )]


def raw_file_fts_ids(conn: sqlite3.Connection, term: str) -> list[str]:
    return [row[0] for row in conn.execute(
        """SELECT f.id FROM files_fts JOIN files AS f ON f.rowid=files_fts.rowid
           WHERE files_fts MATCH ? ORDER BY f.id""", (term,)
    )]


def visible_file_fts_ids(conn: sqlite3.Connection, term: str) -> list[str]:
    return [row[0] for row in conn.execute(
        """SELECT DISTINCT e.source_id
           FROM files_fts
           JOIN files AS f ON f.rowid=files_fts.rowid
           JOIN effective_file_sources AS e ON e.file_id=f.id
           WHERE files_fts MATCH ? ORDER BY e.source_id""", (term,)
    )]


def assert_fk_and_integrity(conn: sqlite3.Connection) -> None:
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"


def run_crash_child(db_path: str) -> None:
    conn = open_db(db_path)
    conn.execute("PRAGMA foreign_keys=OFF")
    conn.execute("BEGIN IMMEDIATE")
    rebuild_files_and_apply_candidate(conn)
    os._exit(73)  # Deliberate process interruption before SQLite COMMIT.


def test_rollback_and_crash_recovery() -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="zen-d4s2-failure-") as tmp:
        db_path = Path(tmp) / "rollback.sqlite"
        conn = open_db(db_path)
        execute_schema37(conn)
        fixture = insert_legacy_fixture(conn)
        conn.commit()
        before_rows = conn.execute("SELECT count(*) FROM files").fetchone()[0]
        before_tags = conn.execute("SELECT count(*) FROM file_user_tags").fetchone()[0]
        try:
            migrate_candidate(conn, fail=True)
            raise AssertionError("forced constraint failure did not fail")
        except sqlite3.IntegrityError:
            pass
        assert "UNIQUE" in conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='files'"
        ).fetchone()[0]
        assert conn.execute("SELECT count(*) FROM sqlite_master WHERE name='file_sources'").fetchone()[0] == 0
        assert conn.execute("SELECT count(*) FROM files").fetchone()[0] == before_rows
        assert conn.execute("SELECT count(*) FROM file_user_tags").fetchone()[0] == before_tags
        assert raw_file_fts_ids(conn, "legacyfilemarker") == [fixture["legacy_file_id"]]
        assert_fk_and_integrity(conn)
        conn.close()

        crash_path = Path(tmp) / "crash.sqlite"
        crash_conn = open_db(crash_path)
        execute_schema37(crash_conn)
        crash_fixture = insert_legacy_fixture(crash_conn)
        crash_conn.commit()
        crash_conn.close()
        child = subprocess.run([sys.executable, str(Path(__file__).resolve()), "--crash-child", str(crash_path)],
                               check=False, capture_output=True, text=True)
        assert child.returncode == 73, child.returncode
        recovered = open_db(crash_path)
        assert "UNIQUE" in recovered.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='files'"
        ).fetchone()[0]
        assert recovered.execute("SELECT count(*) FROM sqlite_master WHERE name='file_sources'").fetchone()[0] == 0
        assert recovered.execute("SELECT count(*) FROM files").fetchone()[0] == 1
        assert recovered.execute("SELECT count(*) FROM file_user_tags").fetchone()[0] == 1
        assert raw_file_fts_ids(recovered, "legacyfilemarker") == [crash_fixture["legacy_file_id"]]
        assert_fk_and_integrity(recovered)
        rerun = migrate_candidate(recovered)
        assert rerun == "applied"
        assert conn_count(recovered, "file_sources") == 1
        assert_fk_and_integrity(recovered)
        recovered.close()
    return {
        "injected_constraint_failure": "rolled back full DDL; original files.path UNIQUE and child data intact",
        "process_interruption_exit_code": 73,
        "startup_recovery": "SQLite recovered Schema 37; migration rerun committed once; FK/integrity checks clean",
    }


def conn_count(conn: sqlite3.Connection, table: str) -> int:
    return int(conn.execute(f'SELECT count(*) FROM "{table}"').fetchone()[0])


def table_digest(conn: sqlite3.Connection, table: str, include_rowid: bool = False) -> str:
    columns = [row[1] for row in conn.execute(f'PRAGMA table_info("{table}")')
               if row[1] != "source_id"]
    projection = ("rowid," if include_rowid else "") + ",".join('"' + c.replace('"', '""') + '"' for c in columns)
    rows = conn.execute(f'SELECT {projection} FROM "{table}" ORDER BY rowid').fetchall()
    values = [list(row) for row in rows]
    return sha256_bytes(json.dumps(values, ensure_ascii=False, separators=(",", ":"),
                                   default=lambda value: {"blob_hex": value.hex()} if isinstance(value, bytes) else str(value)).encode())


def run_main_experiment() -> dict[str, Any]:
    conn = open_db()
    execute_schema37(conn)
    fixture = insert_legacy_fixture(conn)
    conn.commit()
    original_fts_ids = raw_file_fts_ids(conn, "legacyfilemarker")
    original_user_data = {
        table: conn_count(conn, table)
        for table in (
            "file_user_tags", "classification_history", "classification_feedback",
            "file_fingerprints", "duplicate_groups", "duplicate_group_members",
            "content_artifacts", "content_run_items", "analysis_findings", "analysis_finding_evidence",
            "operation_logs", "ai_analysis_state",
        )
    }
    original_user_data_digests = {
        table: table_digest(conn, table, include_rowid=(table == "files"))
        for table in ("files", *original_user_data.keys())
    }
    assert original_fts_ids == [fixture["legacy_file_id"]]

    first_apply = migrate_candidate(conn)
    after_first = {
        table: conn_count(conn, table)
        for table in original_user_data
    }
    assert after_first == original_user_data
    after_first_digests = {
        table: table_digest(conn, table, include_rowid=(table == "files"))
        for table in original_user_data_digests
    }
    assert after_first_digests == original_user_data_digests
    second_apply = migrate_candidate(conn)
    assert second_apply == "already_applied"
    assert {table: conn_count(conn, table) for table in original_user_data} == original_user_data
    assert raw_file_fts_ids(conn, "legacyfilemarker") == original_fts_ids
    assert conn.execute("PRAGMA user_version").fetchone()[0] == 37
    assert not any(
        row[2] and [col[2] for col in conn.execute(f'PRAGMA index_info("{row[1]}")')] == ["path"]
        for row in conn.execute("PRAGMA index_list(files)")
    )
    assert conn.execute("SELECT count(*) FROM file_sources").fetchone()[0] == conn.execute(
        "SELECT count(*) FROM files"
    ).fetchone()[0]
    assert conn.execute(
        """SELECT count(*) FROM files AS f
           LEFT JOIN file_sources AS fs ON fs.file_id=f.id
           WHERE fs.source_id IS NULL OR fs.source_id<>f.source_id"""
    ).fetchone()[0] == 0

    old = conn.execute(
        "SELECT source_id,file_id,status,identity_confidence FROM file_sources"
    ).fetchone()
    assert old["file_id"] == fixture["legacy_file_id"]
    assert old["status"] == "legacy_unverified"
    legacy_source_id = old["source_id"]
    assert conn.execute(
        "SELECT count(*) FROM legacy_unverified_compatibility WHERE source_id=?",
        (legacy_source_id,),
    ).fetchone()[0] == 1
    assert visible_source_ids(conn) == []  # Raw files.is_stale=0 is not authority.

    # Explicit owner claim: selected legacy row + authorized native observation.
    insert_physical_identity(conn, "phys-old", "os-file-id-old")
    old_entry_id = "entry-old-claimed"
    insert_path_entry(conn, old_entry_id, legacy_source_id, fixture["legacy_path"],
                      "phys-old", "verified")
    conn.execute(
        "UPDATE file_sources SET status='observed',identity_confidence='user_claimed',source_revision=source_revision+1 WHERE source_id=?",
        (legacy_source_id,),
    )
    conn.execute(
        """INSERT INTO source_identity_claims
           (claim_id,source_id,path_entry_id,actor_id,claim_kind,authorization_digest,claimed_at)
           VALUES(?,?,?,'owner-fixture','explicit_legacy_claim',?,?)""",
        ("claim-old", legacy_source_id, old_entry_id, sha256_bytes(b"explicit-owner-authorization"), NOW),
    )

    insert_root_generation(conn, "root-a", "/managed", "gen-a-old", "run-a-old", 1,
                           [(legacy_source_id, old_entry_id)])
    insert_root_generation(conn, "root-b", "/managed/reused", "gen-b-old", "run-b-old", 1,
                           [(legacy_source_id, old_entry_id)])
    create_manifest(conn, "manifest-old", [("root-a", "gen-a-old"), ("root-b", "gen-b-old")],
                    [(legacy_source_id, old_entry_id, fixture["legacy_path"], fixture["legacy_path"])], None)
    assert visible_source_ids(conn) == [legacy_source_id]
    assert visible_file_fts_ids(conn, "legacyfilemarker") == [legacy_source_id]

    # One root drops its membership: the other root still owns the same source.
    create_manifest(conn, "manifest-root-b-only", [("root-b", "gen-b-old")],
                    [(legacy_source_id, old_entry_id, fixture["legacy_path"], fixture["legacy_path"])],
                    "manifest-old")
    assert visible_source_ids(conn) == [legacy_source_id]

    # Both complete root generations omit the old source. No side rows are deleted.
    insert_root_generation(conn, "root-a", "/managed", "gen-a-empty", "run-a-empty", 2, [])
    insert_root_generation(conn, "root-b", "/managed/reused", "gen-b-empty", "run-b-empty", 2, [])
    create_manifest(conn, "manifest-no-old", [("root-a", "gen-a-empty"), ("root-b", "gen-b-empty")],
                    [], "manifest-root-b-only")
    assert visible_source_ids(conn) == []
    assert visible_file_fts_ids(conn, "legacyfilemarker") == []
    assert raw_file_fts_ids(conn, "legacyfilemarker") == [fixture["legacy_file_id"]]
    assert conn.execute(
        "SELECT count(*) FROM content_artifact_fts WHERE content_artifact_fts MATCH 'legacycontenttoken'"
    ).fetchone()[0] == 1
    assert conn.execute(
        """SELECT count(*) FROM content_artifacts AS ca
           JOIN file_sources AS fs ON fs.file_id=ca.file_id
           JOIN effective_file_sources AS e ON e.source_id=fs.source_id
           WHERE ca.id='artifact-old'"""
    ).fetchone()[0] == 0

    # A different physical object now occupies exactly the old textual path.
    new_source_id = "source-new-at-reused-path"
    new_file_id = "file-row-new-at-reused-path"
    file_row(conn, new_file_id, new_source_id, fixture["legacy_path"], "newfilemarker.txt", 31, "hash-new")
    assert conn.execute(
        "SELECT file_id FROM file_sources WHERE source_id=?", (new_source_id,)
    ).fetchone()[0] == new_file_id
    conn.execute(
        "UPDATE file_sources SET identity_confidence='native_verified',source_revision=source_revision+1 WHERE source_id=?",
        (new_source_id,),
    )
    insert_physical_identity(conn, "phys-new", "os-file-id-new")
    new_entry_id = "entry-new-at-reused-path"
    insert_path_entry(conn, new_entry_id, new_source_id, fixture["legacy_path"],
                      "phys-new", "verified")
    insert_root_generation(conn, "root-a", "/managed", "gen-a-new", "run-a-new", 3,
                           [(new_source_id, new_entry_id)])
    insert_root_generation(conn, "root-b", "/managed/reused", "gen-b-new", "run-b-new", 3,
                           [(new_source_id, new_entry_id)])
    create_manifest(conn, "manifest-new", [("root-a", "gen-a-new"), ("root-b", "gen-b-new")],
                    [(new_source_id, new_entry_id, fixture["legacy_path"], fixture["legacy_path"])],
                    "manifest-no-old")
    assert visible_source_ids(conn) == [new_source_id]
    assert visible_file_fts_ids(conn, "newfilemarker") == [new_source_id]
    assert visible_file_fts_ids(conn, "legacyfilemarker") == []
    assert conn.execute("SELECT count(*) FROM files WHERE path=?", (fixture["legacy_path"],)).fetchone()[0] == 2
    assert conn.execute(
        "SELECT count(*) FROM file_user_tags WHERE file_id=?", (fixture["legacy_file_id"],)
    ).fetchone()[0] == 1
    assert conn.execute(
        "SELECT count(*) FROM file_user_tags WHERE file_id=?", (new_file_id,)
    ).fetchone()[0] == 0
    assert conn.execute(
        "SELECT count(*) FROM file_fingerprints WHERE file_id=?", (fixture["legacy_file_id"],)
    ).fetchone()[0] == 1
    assert conn.execute(
        "SELECT count(*) FROM content_artifacts WHERE file_id=?", (fixture["legacy_file_id"],)
    ).fetchone()[0] == 1
    assert conn.execute(
        "SELECT count(*) FROM duplicate_group_members WHERE file_id=?", (fixture["legacy_file_id"],)
    ).fetchone()[0] == 1
    assert conn.execute(
        "SELECT count(*) FROM analysis_findings WHERE primary_subject_id=?", (fixture["legacy_path"],)
    ).fetchone()[0] == 1
    assert conn.execute(
        "SELECT content_summary FROM ai_analysis_state WHERE global_entry_id='global-old'"
    ).fetchone()[0] == "legacy AI result"
    try:
        conn.execute("DELETE FROM files WHERE id=?", (fixture["legacy_file_id"],))
        raise AssertionError("unapproved source delete unexpectedly succeeded")
    except sqlite3.IntegrityError as exc:
        assert "retention-aware GC" in str(exc)
    conn.execute("BEGIN IMMEDIATE")
    conn.execute(
        "INSERT INTO source_gc_authorizations(source_id,approved_by,reason,approved_at) VALUES(?,?,?,?)",
        (legacy_source_id, "owner-fixture", "exercise guard only", NOW + 1),
    )
    try:
        conn.execute("DELETE FROM file_sources WHERE source_id=?", (legacy_source_id,))
        raise AssertionError("source mapping deletion left its files row orphaned")
    except sqlite3.IntegrityError as exc:
        assert "delete the authorized file row" in str(exc)
        conn.rollback()
    assert conn.execute(
        "SELECT count(*) FROM file_sources WHERE source_id=?", (legacy_source_id,)
    ).fetchone()[0] == 1

    # Verified same-object rename: source_id/file_id remain; path history is append-only.
    move_source = "source-renamed"
    move_file_id = "file-row-renamed"
    path_before = "/managed/move-before.txt"
    path_after = "/managed/move/subdir/move-after.txt"
    file_row(conn, move_file_id, move_source, path_before, "move-before.txt", 12, "move-hash")
    conn.execute(
        "UPDATE file_sources SET identity_confidence='native_verified',source_revision=source_revision+1 WHERE source_id=?",
        (move_source,),
    )
    insert_physical_identity(conn, "phys-move", "os-file-id-move")
    move_entry_before = "entry-move-before"
    move_entry_after = "entry-move-after"
    insert_path_entry(conn, move_entry_before, move_source, path_before, "phys-move")
    insert_root_generation(conn, "root-c", "/managed/move", "gen-c-before", "run-c-before", 1,
                           [(move_source, move_entry_before)])
    create_manifest(conn, "manifest-before-move",
                    [("root-a", "gen-a-new"), ("root-b", "gen-b-new"), ("root-c", "gen-c-before")],
                    [(new_source_id,new_entry_id,fixture["legacy_path"],fixture["legacy_path"]),
                     (move_source,move_entry_before,path_before,path_before)], "manifest-new")

    # A failed path collision must roll back payload, path entry, FTS, and pointer together.
    pointer_before_move_failure = conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()
    conn.execute("BEGIN IMMEDIATE")
    try:
        insert_path_entry(conn, "entry-move-conflict", move_source,
                          fixture["legacy_path"], "phys-move")
        conn.execute("UPDATE files SET path=?,name=? WHERE id=?",
                     (fixture["legacy_path"], "conflicting-name.txt", move_file_id))
        insert_root_generation(conn, "root-c", "/managed/move", "gen-c-conflict", "run-c-conflict", 2,
                               [(move_source, "entry-move-conflict")])
        create_manifest(conn, "manifest-move-conflict",
                        [("root-a", "gen-a-new"), ("root-b", "gen-b-new"),
                         ("root-c", "gen-c-conflict")],
                        [(new_source_id,new_entry_id,fixture["legacy_path"],fixture["legacy_path"]),
                         (move_source,"entry-move-conflict",fixture["legacy_path"],fixture["legacy_path"])],
                        "manifest-before-move", manage_transaction=False)
        conn.commit()
        raise AssertionError("conflicting path publication unexpectedly committed")
    except sqlite3.IntegrityError:
        conn.rollback()
    assert tuple(conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()) == tuple(pointer_before_move_failure)
    assert conn.execute("SELECT path FROM files WHERE id=?", (move_file_id,)).fetchone()[0] == path_before
    assert conn.execute("SELECT count(*) FROM file_path_entries WHERE path_entry_id='entry-move-conflict'").fetchone()[0] == 0
    assert raw_file_fts_ids(conn, "before") == [move_file_id]
    assert raw_file_fts_ids(conn, "conflicting") == []

    conn.execute("BEGIN IMMEDIATE")
    try:
        insert_path_entry(conn, move_entry_after, move_source, path_after, "phys-move")
        conn.execute("UPDATE files SET path=?,name=? WHERE id=?", (path_after, "move-after.txt", move_file_id))
        insert_root_generation(conn, "root-c", "/managed/move", "gen-c-after", "run-c-after", 2,
                               [(move_source, move_entry_after)])
        create_manifest(conn, "manifest-after-move",
                        [("root-a", "gen-a-new"), ("root-b", "gen-b-new"), ("root-c", "gen-c-after")],
                        [(new_source_id,new_entry_id,fixture["legacy_path"],fixture["legacy_path"]),
                         (move_source,move_entry_after,path_after,path_after)],
                        "manifest-before-move", manage_transaction=False)
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    assert conn.execute("SELECT file_id FROM file_sources WHERE source_id=?", (move_source,)).fetchone()[0] == move_file_id
    assert conn.execute("SELECT count(*) FROM file_path_entries WHERE source_id=?", (move_source,)).fetchone()[0] == 2
    assert [row[0] for row in conn.execute(
        "SELECT projected_path FROM manifest_file_paths WHERE manifest_id='manifest-after-move' AND source_id=?",
        (move_source,),
    )] == [path_after]
    assert raw_file_fts_ids(conn, "before") == []
    assert raw_file_fts_ids(conn, "after") == [move_file_id]
    assert visible_file_fts_ids(conn, "after") == [move_source]

    # Hardlink aliases have distinct source/path-entry IDs but one verified physical epoch.
    file_row(conn, "file-row-hard-a", "source-hard-a", "/managed/hard-a.txt", "hard-a.txt", 7, "hard-hash")
    file_row(conn, "file-row-hard-b", "source-hard-b", "/managed/hard-b.txt", "hard-b.txt", 7, "hard-hash")
    conn.execute(
        "UPDATE file_sources SET identity_confidence='native_verified',source_revision=source_revision+1 WHERE source_id IN ('source-hard-a','source-hard-b')"
    )
    insert_physical_identity(conn, "phys-hardlink", "os-file-id-hardlink")
    insert_path_entry(conn, "entry-hard-a", "source-hard-a", "/managed/hard-a.txt", "phys-hardlink")
    insert_path_entry(conn, "entry-hard-b", "source-hard-b", "/managed/hard-b.txt", "phys-hardlink")
    insert_root_generation(conn, "root-d", "/managed/hard", "gen-d-hardlinks", "run-d-hardlinks", 1,
                           [("source-hard-a", "entry-hard-a"), ("source-hard-b", "entry-hard-b")])
    create_manifest(conn, "manifest-hardlinks",
                    [("root-a", "gen-a-new"), ("root-b", "gen-b-new"),
                     ("root-c", "gen-c-after"), ("root-d", "gen-d-hardlinks")],
                    [(new_source_id,new_entry_id,fixture["legacy_path"],fixture["legacy_path"]),
                     (move_source,move_entry_after,path_after,path_after),
                     ("source-hard-a","entry-hard-a","/managed/hard-a.txt","/managed/hard-a.txt"),
                     ("source-hard-b","entry-hard-b","/managed/hard-b.txt","/managed/hard-b.txt")],
                    "manifest-after-move")
    assert conn.execute(
        "SELECT physical_identity_id FROM file_path_entries WHERE path_entry_id='entry-hard-a'"
    ).fetchone()[0] == conn.execute(
        "SELECT physical_identity_id FROM file_path_entries WHERE path_entry_id='entry-hard-b'"
    ).fetchone()[0]
    assert conn.execute(
        "SELECT count(DISTINCT source_id) FROM file_path_entries WHERE physical_identity_id='phys-hardlink'"
    ).fetchone()[0] == 2
    assert conn.execute(
        "SELECT count(*) FROM file_user_tags WHERE file_id IN ('file-row-hard-a','file-row-hard-b')"
    ).fetchone()[0] == 0

    # An identity/path conflict violates the manifest projection uniqueness before CAS.
    pointer_before_conflict = conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()
    conn.execute("BEGIN IMMEDIATE")
    conn.execute(
        "INSERT INTO visibility_manifests(manifest_id,parent_manifest_id,status,created_at,root_map_digest) VALUES('manifest-conflict',?,'ready',?,?)",
        (pointer_before_conflict[0], NOW, "conflict"),
    )
    conn.execute(
        "INSERT INTO manifest_file_paths(manifest_id,source_id,path_entry_id,path_key,projected_path) VALUES('manifest-conflict',?,?,?,?)",
        (new_source_id, new_entry_id, fixture["legacy_path"], fixture["legacy_path"]),
    )
    try:
        conn.execute(
            "INSERT INTO manifest_file_paths(manifest_id,source_id,path_entry_id,path_key,projected_path) VALUES('manifest-conflict',?,?,?,?)",
            ("source-hard-a", "entry-hard-a", fixture["legacy_path"], fixture["legacy_path"]),
        )
        raise AssertionError("duplicate path projection unexpectedly succeeded")
    except sqlite3.IntegrityError:
        conn.rollback()
    after_conflict = conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()
    assert tuple(after_conflict) == tuple(pointer_before_conflict)
    assert conn.execute("SELECT count(*) FROM visibility_manifests WHERE manifest_id='manifest-conflict'").fetchone()[0] == 0
    assert_fk_and_integrity(conn)

    # Check the external-content FTS index against the rebuilt table and preserved rowids.
    conn.execute("INSERT INTO files_fts(files_fts,rank) VALUES('integrity-check',1)")
    assert_fk_and_integrity(conn)
    result = {
        "runtime": {
            "sqlite_version": sqlite3.sqlite_version,
            "sqlite_source_id": conn.execute("SELECT sqlite_source_id()").fetchone()[0],
            "fts5_available": True,
            "user_version_after_candidate": conn.execute("PRAGMA user_version").fetchone()[0],
        },
        "input": {
            "schema37_ddl_sha256": sha256_bytes(SCHEMA_PATH.read_bytes()),
            "candidate_sql_sha256": sha256_bytes(CANDIDATE_PATH.read_bytes()),
            "runner_script_sha256": sha256_bytes(Path(__file__).read_bytes()),
            "legacy_file_id_equals_path": fixture["legacy_file_id"] == fixture["legacy_path"],
            "schema37_path_unique_before_migration": True,
            "files_path_unique_after_candidate": False,
        },
        "migration": {
            "first_run": first_apply,
            "second_run": second_apply,
            "user_rows_before_after": {"before": original_user_data, "after": after_first},
            "table_sha256_before_after": {
                "before": original_user_data_digests,
                "after": after_first_digests,
            },
            "direct_files_id_fk_tables_after": sorted({
                table
                for (table,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")
                if any(fk[2] == "files" for fk in conn.execute(f'PRAGMA foreign_key_list("{table}")'))
            }),
            "old_rowid_and_fts_terms_preserved": True,
            "existing_file_fks_clean": conn.execute("PRAGMA foreign_key_check").fetchall() == [],
            "file_fts_integrity_check": "ok",
        },
        "assertions": {
            "legacy_default_is_read_only_unverified": True,
            "source_id_crosswalk_is_complete_and_insert_triggered": True,
            "explicit_legacy_claim_audited": True,
            "one_root_removal_keeps_other_root_visible": True,
            "all_roots_remove_old_source_without_deleting_history": True,
            "same_path_new_identity_gets_new_source_and_no_inheritance": True,
            "raw_fts_retains_old_term_but_effective_search_hides_it": True,
            "content_fts_and_artifact_are_retained_but_hidden": True,
            "metadata_tag_history_fingerprint_duplicate_analysis_ai_retained": True,
            "same_identity_cross_directory_move_keeps_source_and_path_history": True,
            "failed_path_conflict_rolls_back_path_fts_generation_and_pointer": True,
            "hardlinks_share_physical_identity_but_not_source_or_tags": True,
            "conflicting_manifest_path_rolls_back_without_pointer_change": True,
            "unapproved_source_delete_rejected": True,
            "final_integrity_and_foreign_key_check": "ok",
        },
        "failure_recovery": test_rollback_and_crash_recovery(),
        "scope_limits": [
            "Synthetic rows over the complete captured Schema 37 DDL; no production/user database was opened.",
            "SQLite runtime is the workspace Python library, not Windows/macOS or the product-linked SQLite 3.51.3.",
            "No performance benchmark, Schema version allocation, production code, IPC, installer, PR, or CI change.",
            "Filesystem identity is represented by deterministic fixture IDs; this does not prove platform handle, inode reuse, reparse, ACL, or mount behavior.",
        ],
    }
    conn.close()
    return result


def main() -> int:
    if len(sys.argv) == 3 and sys.argv[1] == "--crash-child":
        run_crash_child(sys.argv[2])
        return 0
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    result = run_main_experiment()
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    (EVIDENCE / "schema37-physical-harness-results.json").write_text(encoded, encoding="utf-8")
    output = "\n".join([
        "#381-D4-S2 Schema 37 physical source harness",
        f"sqlite_version={result['runtime']['sqlite_version']}",
        f"sqlite_source_id={result['runtime']['sqlite_source_id']}",
        f"schema37_ddl_sha256={result['input']['schema37_ddl_sha256']}",
        f"candidate_sql_sha256={result['input']['candidate_sql_sha256']}",
        f"runner_script_sha256={result['input']['runner_script_sha256']}",
        f"migration={result['migration']['first_run']}; idempotent_retry={result['migration']['second_run']}",
        f"assertion_groups={len(result['assertions'])} passed; rollback, process interruption and startup rerun passed",
        "foreign_key_check=ok; integrity_check=ok; FTS5 external-content integrity-check=ok",
        "production_schema_version=unassigned (synthetic PRAGMA user_version remains 37)",
    ]) + "\n"
    (EVIDENCE / "schema37-physical-harness-run.log").write_text(output, encoding="utf-8")
    print(output, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
