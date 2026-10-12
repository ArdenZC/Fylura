#!/usr/bin/env python3
"""Counterexample-first Manifest authority research harness; never production code."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sqlite3
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[5]
HERE = Path(__file__).resolve().parents[1]
D4S2_DIR = ROOT / "docs/project/research/issue-381-d4-s2"
D4S2_RUNNER = D4S2_DIR / "harness/run.py"
SCHEMA_PATH = ROOT / "docs/project/research/issue-381-d4-s0/evidence/schema37-sqlite-schema.sql"
PROTECTION_PATH = HERE / "harness/manifest-authority-protection.sql"
ORACLE_PATH = HERE / "harness/manifest-authority-oracle.sql"
EVIDENCE = HERE / "evidence"
NOW = 1_800_000_000


class OracleRejected(RuntimeError):
    pass


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest_rows(rows: list[tuple[Any, ...]] | list[list[Any]]) -> str:
    canonical = lambda row: json.dumps(row, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    ordered = sorted([list(row) for row in rows], key=canonical)
    payload = json.dumps(ordered, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    return sha256(payload)


sys.dont_write_bytecode = True


def load_d4s2_runner():
    spec = importlib.util.spec_from_file_location("d4s2_physical_harness", D4S2_RUNNER)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


D4S2 = load_d4s2_runner()


def fresh_db(path: str | Path = ":memory:", protected: bool = False) -> tuple[sqlite3.Connection, dict[str, str]]:
    conn = D4S2.open_db(path)
    D4S2.execute_schema37(conn)
    fixture = D4S2.insert_legacy_fixture(conn)
    D4S2.migrate_candidate(conn)
    if protected:
        install_oracle_and_guards(conn)
    return conn, fixture


def install_oracle_and_guards(conn: sqlite3.Connection) -> None:
    for path in (ORACLE_PATH, PROTECTION_PATH):
        sql = path.read_text(encoding="utf-8")
        for statement in D4S2.split_sql(sql):
            conn.execute(statement)


def add_source(conn: sqlite3.Connection, suffix: str, path: str, name: str | None = None,
               identity_status: str = "verified") -> dict[str, str]:
    source_id = f"source-{suffix}"
    file_id = f"file-{suffix}"
    path_entry_id = f"entry-{suffix}-1"
    physical_id = f"physical-{suffix}"
    D4S2.file_row(conn, file_id, source_id, path, name or f"{suffix}-shared.txt", 10,
                  f"hash-{suffix}")
    conn.execute(
        "UPDATE file_sources SET status='observed',identity_confidence='native_verified',source_revision=source_revision+1 WHERE source_id=?",
        (source_id,),
    )
    D4S2.insert_physical_identity(conn, physical_id, f"os-id-{suffix}")
    D4S2.insert_path_entry(conn, path_entry_id, source_id, path, physical_id, identity_status)
    return {
        "source_id": source_id,
        "file_id": file_id,
        "path_entry_id": path_entry_id,
        "physical_id": physical_id,
        "path": path,
        "name": name or f"{suffix}-shared.txt",
    }


def add_path_entry(conn: sqlite3.Connection, source: dict[str, str], entry_id: str,
                   path: str) -> dict[str, str]:
    D4S2.insert_path_entry(conn, entry_id, source["source_id"], path,
                           source["physical_id"], "verified")
    return {**source, "path_entry_id": entry_id, "path": path}


def refresh_generation_digest(conn: sqlite3.Connection, generation_id: str) -> str:
    conn.execute(
        """UPDATE source_generation_members
           SET identity_digest=(SELECT pe.evidence_digest FROM file_path_entries AS pe
                                WHERE pe.path_entry_id=source_generation_members.path_entry_id
                                  AND pe.source_id=source_generation_members.source_id)
           WHERE generation_id=?""",
        (generation_id,),
    )
    rows = [tuple(row) for row in conn.execute(
        """SELECT source_id,path_entry_id,identity_digest
           FROM source_generation_members WHERE generation_id=?
           ORDER BY source_id,path_entry_id""", (generation_id,)
    )]
    digest = digest_rows(rows)
    conn.execute(
        "UPDATE source_generations SET member_count=?,member_digest=? WHERE generation_id=?",
        (len(rows), digest, generation_id),
    )
    return digest


def add_generation(conn: sqlite3.Connection, root_id: str, root_path: str,
                   generation_id: str, run_id: str, generation_no: int,
                   members: list[dict[str, str]]) -> None:
    D4S2.insert_root_generation(
        conn, root_id, root_path, generation_id, run_id, generation_no,
        [(m["source_id"], m["path_entry_id"]) for m in members],
    )
    refresh_generation_digest(conn, generation_id)


def projection(source: dict[str, str], path_key: str | None = None,
               projected_path: str | None = None) -> tuple[str, str, str, str]:
    return (source["source_id"], source["path_entry_id"],
            path_key if path_key is not None else source["path"],
            projected_path if projected_path is not None else source["path"])


def stage_manifest(conn: sqlite3.Connection, manifest_id: str,
                   root_generations: list[tuple[str, str]],
                   projections: list[tuple[str, str, str, str]],
                   parent_manifest_id: str | None) -> None:
    root_digest = digest_rows(sorted(root_generations))
    conn.execute(
        """INSERT INTO visibility_manifests
           (manifest_id,parent_manifest_id,status,created_at,root_map_digest)
           VALUES(?,?,'ready',?,?)""",
        (manifest_id, parent_manifest_id, NOW, root_digest),
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


def expected_roots(conn: sqlite3.Connection, manifest_id: str) -> list[tuple[str, str]]:
    return [tuple(row) for row in conn.execute(
        "SELECT root_id,generation_id FROM manifest_root_heads WHERE manifest_id=? ORDER BY root_id,generation_id",
        (manifest_id,),
    )]


def oracle_report(conn: sqlite3.Connection, manifest_id: str) -> dict[str, Any]:
    roots = expected_roots(conn, manifest_id)
    vm = conn.execute(
        "SELECT parent_manifest_id,status,root_map_digest FROM visibility_manifests WHERE manifest_id=?",
        (manifest_id,),
    ).fetchone()
    if vm is None:
        raise OracleRejected(f"manifest {manifest_id!r} does not exist")
    actual_root_digest = digest_rows(sorted(roots))
    generation_rows: list[dict[str, Any]] = []
    generation_errors: list[str] = []
    for root_id, generation_id in roots:
        row = conn.execute(
            """SELECT sg.status,sg.member_count,sg.member_digest,sr.status,sr.coverage_complete,
                      (SELECT count(*) FROM source_generation_members gm
                       WHERE gm.generation_id=sg.generation_id)
               FROM source_generations sg JOIN scan_runs sr ON sr.id=sg.run_id
               WHERE sg.generation_id=? AND sg.root_id=?""",
            (generation_id, root_id),
        ).fetchone()
        if row is None:
            generation_errors.append(f"missing generation {root_id}:{generation_id}")
            continue
        actual_member_rows = [tuple(member) for member in conn.execute(
            """SELECT source_id,path_entry_id,identity_digest
               FROM source_generation_members WHERE generation_id=?
               ORDER BY source_id,path_entry_id""", (generation_id,)
        )]
        member_digest = digest_rows(actual_member_rows)
        item = {
            "root_id": root_id,
            "generation_id": generation_id,
            "status": row[0],
            "declared_member_count": row[1],
            "actual_member_count": row[5],
            "declared_member_digest": row[2],
            "actual_member_digest": member_digest,
            "scan_status": row[3],
            "coverage_complete": row[4],
        }
        generation_rows.append(item)
        if row[0] not in ("complete", "published"):
            generation_errors.append(f"generation {generation_id} is {row[0]}")
        if row[1] != row[5] or row[2] != member_digest:
            generation_errors.append(f"generation {generation_id} count or digest mismatch")
        if row[3] != "completed" or row[4] != 1:
            generation_errors.append(f"generation {generation_id} scan is incomplete")

    expected = [tuple(row) for row in conn.execute(
        """SELECT source_id,path_entry_id,path_key,path_text
           FROM d4s3_oracle_expected_paths WHERE manifest_id=?
           ORDER BY source_id,path_entry_id,path_key,path_text""", (manifest_id,)
    )]
    actual = [tuple(row) for row in conn.execute(
        """SELECT source_id,path_entry_id,path_key,path_text
           FROM d4s3_oracle_actual_paths WHERE manifest_id=?
           ORDER BY source_id,path_entry_id,path_key,path_text""", (manifest_id,)
    )]
    missing = [tuple(row) for row in conn.execute(
        """SELECT source_id,path_entry_id,path_key,path_text
           FROM d4s3_oracle_missing_paths WHERE manifest_id=?
           ORDER BY source_id,path_entry_id,path_key,path_text""", (manifest_id,)
    )]
    extra = [tuple(row) for row in conn.execute(
        """SELECT source_id,path_entry_id,path_key,path_text
           FROM d4s3_oracle_extra_paths WHERE manifest_id=?
           ORDER BY source_id,path_entry_id,path_key,path_text""", (manifest_id,)
    )]
    invalid_identity = [dict(row) for row in conn.execute(
        "SELECT * FROM d4s3_oracle_invalid_identity WHERE manifest_id=? ORDER BY source_id,path_entry_id",
        (manifest_id,),
    )]
    path_conflicts = [tuple(row) for row in conn.execute(
        """SELECT gm.source_id,count(DISTINCT gm.path_entry_id) AS entries
           FROM manifest_root_heads mr JOIN source_generation_members gm
             ON gm.generation_id=mr.generation_id
           WHERE mr.manifest_id=? GROUP BY gm.source_id
           HAVING count(DISTINCT gm.path_entry_id)<>1 ORDER BY gm.source_id""",
        (manifest_id,),
    )]
    pointer = conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()
    parent_matches_pointer = (
        (pointer is None and vm[0] is None)
        or (pointer is not None and vm[0] == pointer[0])
    )
    return {
        "manifest_id": manifest_id,
        "status": vm[1],
        "root_count": len(roots),
        "declared_root_map_digest": vm[2],
        "actual_root_map_digest": actual_root_digest,
        "root_map_digest_matches": vm[2] == actual_root_digest,
        "parent_matches_current_pointer": parent_matches_pointer,
        "generations": generation_rows,
        "generation_errors": generation_errors,
        "expected_count": len(expected),
        "actual_count": len(actual),
        "expected_sha256": digest_rows(expected),
        "actual_sha256": digest_rows(actual),
        "missing_count": len(missing),
        "missing": missing,
        "extra_count": len(extra),
        "extra": extra,
        "invalid_identity_count": len(invalid_identity),
        "invalid_identity": invalid_identity,
        "path_conflicts": path_conflicts,
        "valid": (
            vm[1] == "ready"
            and vm[2] == actual_root_digest
            and parent_matches_pointer
            and not generation_errors
            and not missing and not extra
            and not invalid_identity
            and not path_conflicts
        ),
    }


def finalize_current_transaction(conn: sqlite3.Connection, manifest_id: str) -> dict[str, Any]:
    report = oracle_report(conn, manifest_id)
    if not report["valid"]:
        raise OracleRejected(json.dumps(report, sort_keys=True))
    for generation in report["generations"]:
        if generation["status"] == "complete":
            conn.execute(
                "UPDATE source_generations SET status='published' WHERE generation_id=? AND status='complete'",
                (generation["generation_id"],),
            )
    conn.execute(
        "UPDATE visibility_manifests SET status='published' WHERE manifest_id=? AND status='ready'",
        (manifest_id,),
    )
    parent = conn.execute(
        "SELECT parent_manifest_id FROM visibility_manifests WHERE manifest_id=?",
        (manifest_id,),
    ).fetchone()[0]
    pointer = conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()
    if parent is None:
        if pointer is not None:
            raise OracleRejected("initial Manifest cannot replace an existing pointer")
        conn.execute(
            "INSERT INTO active_visibility_pointer(singleton_id,manifest_id,revision,updated_at) VALUES(1,?,1,?)",
            (manifest_id, NOW),
        )
    else:
        if pointer is None or pointer[0] != parent:
            raise OracleRejected("Manifest parent lost pointer CAS")
        updated = conn.execute(
            """UPDATE active_visibility_pointer SET manifest_id=?,revision=?,updated_at=?
               WHERE singleton_id=1 AND manifest_id=? AND revision=?""",
            (manifest_id, pointer[1] + 1, NOW, parent, pointer[1]),
        ).rowcount
        if updated != 1:
            raise OracleRejected("Manifest pointer CAS failed")
    return report


def publish(conn: sqlite3.Connection, manifest_id: str) -> dict[str, Any]:
    conn.execute("BEGIN IMMEDIATE")
    try:
        report = finalize_current_transaction(conn, manifest_id)
        conn.commit()
        return report
    except Exception:
        conn.rollback()
        raise


def cursor_matches_active_manifest(conn: sqlite3.Connection, token: dict[str, Any]) -> bool:
    pointer = conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()
    return pointer is not None and token.get("manifest_id") == pointer[0] and token.get("revision") == pointer[1]


def stage_attempt_publish(conn: sqlite3.Connection, manifest_id: str) -> str:
    """Bypass the Python Oracle to prove SQL publication guard is independent."""
    conn.execute("BEGIN IMMEDIATE")
    try:
        conn.execute(
            "UPDATE source_generations SET status='published' WHERE generation_id IN "
            "(SELECT generation_id FROM manifest_root_heads WHERE manifest_id=?) AND status='complete'",
            (manifest_id,),
        )
        conn.execute(
            "UPDATE visibility_manifests SET status='published' WHERE manifest_id=?",
            (manifest_id,),
        )
        conn.commit()
        raise AssertionError("invalid manifest passed SQL publication guard")
    except sqlite3.IntegrityError as exc:
        conn.rollback()
        return str(exc)
    except Exception:
        conn.rollback()
        raise


def legacy_counterexample(kind: str) -> dict[str, Any]:
    conn, _ = fresh_db(protected=False)
    identity_status = "path_only" if kind == "unverified-path" else "verified"
    a = add_source(conn, "a", "/root/a-shared.txt", identity_status=identity_status)
    b = add_source(conn, "b", "/root/b-shared.txt")
    members = [a, b] if kind in ("omitted-member", "wrong-path") else [a]
    add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, members)
    install_oracle_and_guards_views_only(conn)
    if kind == "omitted-member":
        projections = [projection(a)]
    elif kind == "wrong-path":
        projections = [projection(a, projected_path="/attacker/unobserved.txt"), projection(b)]
    elif kind == "extra-member":
        projections = [projection(a), projection(b)]
    elif kind == "unverified-path":
        projections = [projection(a)]
    else:
        raise ValueError(kind)
    D4S2.create_manifest(conn, f"legacy-{kind}", [("root-a", "gen-a")], projections, None)
    report = oracle_report(conn, f"legacy-{kind}")
    visible = D4S2.visible_source_ids(conn)
    if kind == "omitted-member":
        assert report["missing_count"] == 1 and visible == [a["source_id"]]
    elif kind == "wrong-path":
        assert report["missing_count"] == 1 and report["extra_count"] == 1
        wrong = conn.execute(
            "SELECT projected_path FROM manifest_file_paths WHERE manifest_id=? AND source_id=?",
            (f"legacy-{kind}", a["source_id"]),
        ).fetchone()[0]
        assert wrong == "/attacker/unobserved.txt"
    elif kind == "extra-member":
        assert report["extra_count"] == 1 and b["source_id"] not in visible
    elif kind == "unverified-path":
        assert report["invalid_identity_count"] == 1 and visible == [a["source_id"]]
    result = {"case": kind, "old_model_published": True, "oracle": report,
              "effective_view_sources": visible}
    conn.close()
    return result


def legacy_cursor_counterexample() -> dict[str, Any]:
    conn, _ = fresh_db(protected=False)
    source = add_source(conn, "cursor", "/root/old-cursor-marker.txt", "oldcursorneedle.txt")
    add_generation(conn, "root-a", "/root", "gen-old", "run-old", 1, [source])
    D4S2.create_manifest(conn, "legacy-cursor-old", [("root-a", "gen-old")],
                         [projection(source)], None)
    stale_token = {"manifest_id": "legacy-cursor-old", "revision": 1,
                   "query": "oldcursorneedle", "last_source_id": source["source_id"]}
    renamed = add_path_entry(conn, source, "entry-cursor-2", "/root/new-cursor-marker.txt")
    conn.execute("UPDATE files SET path=?,name=? WHERE id=?",
                 (renamed["path"], "newcursorneedle.txt", source["file_id"]))
    add_generation(conn, "root-a", "/root", "gen-new", "run-new", 2, [renamed])
    D4S2.create_manifest(conn, "legacy-cursor-new", [("root-a", "gen-new")],
                         [projection(renamed)], "legacy-cursor-old")
    pointer = tuple(conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone())
    # The old model has no token validator. This continuation silently runs
    # against the new active pointer and FTS content while carrying old query state.
    current_page = [tuple(row) for row in conn.execute(
        """SELECT e.source_id,e.projected_path
           FROM effective_file_sources AS e
           JOIN files_fts ON files_fts.rowid=(SELECT rowid FROM files WHERE id=e.file_id)
           WHERE files_fts MATCH ? ORDER BY e.source_id""",
        (stale_token["query"],),
    )]
    old_manifest_path = conn.execute(
        "SELECT projected_path FROM manifest_file_paths WHERE manifest_id=? AND source_id=?",
        (stale_token["manifest_id"], source["source_id"]),
    ).fetchone()[0]
    current_detail = tuple(conn.execute(
        "SELECT id,path,name FROM files WHERE id=?", (source["file_id"],)
    ).fetchone())
    old_fts_rows = conn.execute(
        "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'oldcursorneedle'"
    ).fetchone()[0]
    new_fts_rows = conn.execute(
        "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'newcursorneedle'"
    ).fetchone()[0]
    assert pointer == ("legacy-cursor-new", 2)
    assert current_page == []
    assert old_manifest_path == "/root/old-cursor-marker.txt"
    assert current_detail[1:] == ("/root/new-cursor-marker.txt", "newcursorneedle.txt")
    assert old_fts_rows == 0 and new_fts_rows == 1
    conn.close()
    return {
        "case": "cross-transaction-cursor-no-revision-validation",
        "old_model_cursor_silently_reused": True,
        "token_manifest_and_revision": [stale_token["manifest_id"], stale_token["revision"]],
        "current_pointer": list(pointer),
        "continuation_result_against_current_fts": current_page,
        "detail_returned_from_current_files_row": list(current_detail),
        "old_vs_new_fts_match_counts": [old_fts_rows, new_fts_rows],
        "old_manifest_path_still_present": old_manifest_path,
    }


def install_oracle_and_guards_views_only(conn: sqlite3.Connection) -> None:
    for statement in D4S2.split_sql(ORACLE_PATH.read_text(encoding="utf-8")):
        conn.execute(statement)


def legacy_post_publish_counterexample(action: str) -> dict[str, Any]:
    conn, _ = fresh_db(protected=False)
    a = add_source(conn, "a", "/root/a-shared.txt")
    add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, [a])
    install_oracle_and_guards_views_only(conn)
    D4S2.create_manifest(conn, "legacy-post-publish", [("root-a", "gen-a")],
                         [projection(a)], None)
    revision_before = conn.execute(
        "SELECT revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()[0]
    if action == "update":
        count = conn.execute(
            "UPDATE source_generation_members SET identity_digest='tampered-after-publish' "
            "WHERE generation_id='gen-a' AND source_id=?", (a["source_id"],)
        ).rowcount
        assert count == 1
        visibility_after = D4S2.visible_source_ids(conn)
    elif action == "delete":
        count = conn.execute(
            "DELETE FROM source_generation_members WHERE generation_id='gen-a' AND source_id=?",
            (a["source_id"],),
        ).rowcount
        assert count == 1
        visibility_after = D4S2.visible_source_ids(conn)
    else:
        raise ValueError(action)
    assert conn.execute(
        "SELECT revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()[0] == revision_before
    result = {
        "case": f"post-publication-member-{action}",
        "old_model_mutation_succeeded": True,
        "pointer_revision_before_after": [revision_before, revision_before],
        "visible_sources_after": visibility_after,
        "oracle_after": oracle_report(conn, "legacy-post-publish"),
    }
    conn.close()
    return result


def protected_invalid_publication(kind: str) -> dict[str, Any]:
    conn, _ = fresh_db(protected=True)
    identity_status = "path_only" if kind == "unverified-path" else "verified"
    a = add_source(conn, "a", "/root/a-shared.txt", identity_status=identity_status)
    b = add_source(conn, "b", "/root/b-shared.txt")
    members = [a, b] if kind in ("omitted-member", "wrong-path") else [a]
    add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, members)
    if kind == "omitted-member":
        projections = [projection(a)]
    elif kind == "wrong-path":
        projections = [projection(a, projected_path="/attacker/unobserved.txt"), projection(b)]
    elif kind == "extra-member":
        projections = [projection(a), projection(b)]
    elif kind == "unverified-path":
        projections = [projection(a)]
    else:
        raise ValueError(kind)
    stage_manifest(conn, f"protected-{kind}", [("root-a", "gen-a")], projections, None)
    report = oracle_report(conn, f"protected-{kind}")
    assert report["valid"] is False
    error = stage_attempt_publish(conn, f"protected-{kind}")
    assert error.startswith("d4s3:")
    assert conn.execute(
        "SELECT status FROM visibility_manifests WHERE manifest_id=?", (f"protected-{kind}",)
    ).fetchone()[0] == "ready"
    assert conn.execute("SELECT count(*) FROM active_visibility_pointer").fetchone()[0] == 0
    assert conn.execute(
        "SELECT status FROM source_generations WHERE generation_id='gen-a'"
    ).fetchone()[0] == "complete"
    result = {"case": kind, "oracle_rejected": True,
              "sql_guard_error": error, "rollback_kept_ready": True,
              "pointer_count": 0, "report": report}
    conn.close()
    return result


def assert_guard_rejects(conn: sqlite3.Connection, label: str, sql: str,
                         args: tuple[Any, ...] = ()) -> dict[str, str]:
    conn.execute("SAVEPOINT d4s3_mutation_attempt")
    try:
        conn.execute(sql, args)
        conn.execute("RELEASE d4s3_mutation_attempt")
        raise AssertionError(f"{label}: protected mutation succeeded")
    except sqlite3.IntegrityError as exc:
        message = str(exc)
        conn.execute("ROLLBACK TO d4s3_mutation_attempt")
        conn.execute("RELEASE d4s3_mutation_attempt")
        assert message.startswith("d4s3:"), (label, message)
        return {"operation": label, "rejected": message}


def test_published_immutability() -> dict[str, Any]:
    conn, _ = fresh_db(protected=True)
    a = add_source(conn, "a", "/root/a-shared.txt")
    b = add_source(conn, "b", "/root/b-shared.txt")
    add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, [a])
    add_generation(conn, "root-b", "/root-b", "gen-b", "run-b", 1, [b])
    add_generation(conn, "root-c", "/root-c", "gen-unused", "run-unused", 1, [])
    stage_manifest(conn, "manifest-a", [("root-a", "gen-a")], [projection(a)], None)
    publish(conn, "manifest-a")

    # Keep a separate scan_run available to exercise a forbidden direct insert.
    unused = conn.execute("SELECT * FROM source_generations WHERE generation_id='gen-unused'").fetchone()
    conn.execute("DELETE FROM source_generations WHERE generation_id='gen-unused'")
    # Re-use the freed run id and expect the direct-published INSERT trigger.
    insert_rejection = assert_guard_rejects(
        conn, "source_generations.insert_published",
        """INSERT INTO source_generations
           (generation_id,root_id,run_id,status,root_revision,watcher_revision,member_count,member_digest,completed_at)
           VALUES('gen-direct','root-c',?,'published',1,0,0,?,?)""",
        (unused["run_id"], digest_rows([]), NOW),
    )

    operations = [
        ("source_generation_members.insert", "INSERT INTO source_generation_members(generation_id,source_id,path_entry_id,identity_digest) VALUES('gen-a',?,?,?)",
         (a["source_id"], a["path_entry_id"], "extra")),
        ("source_generation_members.update", "UPDATE source_generation_members SET identity_digest='changed' WHERE generation_id='gen-a' AND source_id=?",
         (a["source_id"],)),
        ("source_generation_members.delete", "DELETE FROM source_generation_members WHERE generation_id='gen-a' AND source_id=?",
         (a["source_id"],)),
        ("source_generations.update", "UPDATE source_generations SET root_revision=root_revision+1 WHERE generation_id='gen-a'", ()),
        ("source_generations.delete", "DELETE FROM source_generations WHERE generation_id='gen-a'", ()),
        ("manifest_root_heads.insert", "INSERT INTO manifest_root_heads(manifest_id,root_id,generation_id) VALUES('manifest-a','root-b','gen-b')", ()),
        ("manifest_root_heads.update", "UPDATE manifest_root_heads SET generation_id='gen-b',root_id='root-b' WHERE manifest_id='manifest-a' AND root_id='root-a'", ()),
        ("manifest_root_heads.delete", "DELETE FROM manifest_root_heads WHERE manifest_id='manifest-a' AND root_id='root-a'", ()),
        ("manifest_file_paths.insert", "INSERT INTO manifest_file_paths(manifest_id,source_id,path_entry_id,path_key,projected_path) VALUES('manifest-a',?,?,?,?)",
         (b["source_id"], b["path_entry_id"], b["path"], b["path"])),
        ("manifest_file_paths.update", "UPDATE manifest_file_paths SET projected_path='/tampered' WHERE manifest_id='manifest-a' AND source_id=?", (a["source_id"],)),
        ("manifest_file_paths.delete", "DELETE FROM manifest_file_paths WHERE manifest_id='manifest-a' AND source_id=?", (a["source_id"],)),
        ("visibility_manifests.insert_published", "INSERT INTO visibility_manifests(manifest_id,status,created_at,root_map_digest) VALUES('manifest-direct','published',?,?)", (NOW, "x")),
        ("visibility_manifests.update", "UPDATE visibility_manifests SET root_map_digest='tampered' WHERE manifest_id='manifest-a'", ()),
        ("visibility_manifests.delete", "DELETE FROM visibility_manifests WHERE manifest_id='manifest-a'", ()),
    ]
    rejections = [insert_rejection]
    for label, sql, args in operations:
        rejections.append(assert_guard_rejects(conn, label, sql, args))

    rejections.append(assert_guard_rejects(
        conn, "active_visibility_pointer.non_cas_revision",
        "UPDATE active_visibility_pointer SET revision=revision+2 WHERE singleton_id=1", ()))
    pointer = tuple(conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone())
    assert pointer == ("manifest-a", 1)
    assert D4S2.visible_source_ids(conn) == [a["source_id"]]
    conn.close()
    return {"blocked_mutations": len(rejections), "rejections": rejections,
            "pointer_after_all_attempts": list(pointer), "visibility_preserved": True}


def test_nested_roots_and_valid_publication() -> dict[str, Any]:
    conn, _ = fresh_db(protected=True)
    a = add_source(conn, "shared", "/root/nested/shared.txt")
    add_generation(conn, "root-parent", "/root", "gen-parent", "run-parent", 1, [a])
    add_generation(conn, "root-nested", "/root/nested", "gen-nested", "run-nested", 1, [a])
    stage_manifest(conn, "manifest-nested", [("root-parent", "gen-parent"),
                    ("root-nested", "gen-nested")], [projection(a)], None)
    report = publish(conn, "manifest-nested")
    assert report["expected_count"] == report["actual_count"] == 1
    assert report["missing_count"] == report["extra_count"] == 0
    assert len(D4S2.visible_source_ids(conn)) == 1
    conn.close()
    return {"manifest_oracle": report, "overlapping_root_members_deduplicated": True}


def test_cursor_rename_fts_and_rollback() -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="zen-d4s3-") as temp_dir:
        db_path = Path(temp_dir) / "authority.sqlite"
        conn, _ = fresh_db(db_path, protected=True)
        conn.execute("PRAGMA journal_mode=WAL")
        source = add_source(conn, "rename", "/root/old-shared.txt", "oldsharedmarker.txt")
        add_generation(conn, "root-a", "/root", "gen-old", "run-old", 1, [source])
        stage_manifest(conn, "manifest-old", [("root-a", "gen-old")], [projection(source)], None)
        publish(conn, "manifest-old")
        cursor = {"manifest_id": "manifest-old", "revision": 1, "query": "shared"}

        reader = D4S2.open_db(db_path)
        reader.execute("BEGIN")
        pinned_before = tuple(reader.execute(
            "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
        ).fetchone())
        pinned_fts_before = [row[0] for row in reader.execute(
            "SELECT f.id FROM files_fts JOIN files f ON f.rowid=files_fts.rowid "
            "WHERE files_fts MATCH 'oldsharedmarker' ORDER BY f.id"
        )]
        assert pinned_before == ("manifest-old", 1)
        assert pinned_fts_before == [source["file_id"]]

        # A rename plus a bad projection fails before publication and rolls back
        # the files row, FTS trigger writes, path history, generation and pointer.
        conn.execute("BEGIN IMMEDIATE")
        renamed = add_path_entry(conn, source, "entry-rename-2", "/root/new-shared.txt")
        conn.execute("UPDATE files SET path=?,name=? WHERE id=?",
                     (renamed["path"], "newsharedmarker.txt", source["file_id"]))
        add_generation(conn, "root-a", "/root", "gen-new-fail", "run-new-fail", 2, [renamed])
        stage_manifest(conn, "manifest-new-fail", [("root-a", "gen-new-fail")],
                       [projection(renamed, projected_path="/wrong/new-shared.txt")], "manifest-old")
        failure_report = oracle_report(conn, "manifest-new-fail")
        assert failure_report["missing_count"] == 1 and failure_report["extra_count"] == 1
        try:
            finalize_current_transaction(conn, "manifest-new-fail")
            raise AssertionError("wrong-path rename unexpectedly published")
        except OracleRejected:
            conn.rollback()
        assert tuple(conn.execute(
            "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
        ).fetchone()) == ("manifest-old", 1)
        assert tuple(conn.execute("SELECT path,name FROM files WHERE id=?", (source["file_id"],)).fetchone()) == (
            "/root/old-shared.txt", "oldsharedmarker.txt")
        assert conn.execute("SELECT count(*) FROM file_path_entries WHERE source_id=?",
                            (source["source_id"],)).fetchone()[0] == 1
        assert [row[0] for row in conn.execute(
            "SELECT f.id FROM files_fts JOIN files f ON f.rowid=files_fts.rowid "
            "WHERE files_fts MATCH 'oldsharedmarker' ORDER BY f.id"
        )] == [source["file_id"]]

        # The reader's pinned SQLite transaction continues to see its original
        # pointer and FTS snapshot while the writer publishes the rename.
        conn.execute("BEGIN IMMEDIATE")
        renamed = add_path_entry(conn, source, "entry-rename-2", "/root/new-shared.txt")
        conn.execute("UPDATE files SET path=?,name=? WHERE id=?",
                     (renamed["path"], "newsharedmarker.txt", source["file_id"]))
        add_generation(conn, "root-a", "/root", "gen-new", "run-new", 2, [renamed])
        stage_manifest(conn, "manifest-new", [("root-a", "gen-new")], [projection(renamed)], "manifest-old")
        finalize_current_transaction(conn, "manifest-new")
        conn.commit()
        pinned_after = tuple(reader.execute(
            "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
        ).fetchone())
        pinned_fts_after = [row[0] for row in reader.execute(
            "SELECT f.id FROM files_fts JOIN files f ON f.rowid=files_fts.rowid "
            "WHERE files_fts MATCH 'oldsharedmarker' ORDER BY f.id"
        )]
        assert pinned_after == ("manifest-old", 1)
        assert pinned_fts_after == [source["file_id"]]
        reader.commit()

        current_pointer = tuple(conn.execute(
            "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
        ).fetchone())
        assert current_pointer == ("manifest-new", 2)
        assert [row[0] for row in conn.execute(
            "SELECT f.id FROM files_fts JOIN files f ON f.rowid=files_fts.rowid "
            "WHERE files_fts MATCH 'oldsharedmarker' ORDER BY f.id"
        )] == []
        assert [row[0] for row in conn.execute(
            "SELECT f.id FROM files_fts JOIN files f ON f.rowid=files_fts.rowid "
            "WHERE files_fts MATCH 'newsharedmarker' ORDER BY f.id"
        )] == [source["file_id"]]
        current_path = conn.execute(
            "SELECT projected_path FROM effective_file_sources WHERE source_id=?",
            (source["source_id"],),
        ).fetchone()[0]
        old_manifest_path = conn.execute(
            "SELECT projected_path FROM manifest_file_paths WHERE manifest_id='manifest-old' AND source_id=?",
            (source["source_id"],),
        ).fetchone()[0]
        history = [tuple(row) for row in conn.execute(
            "SELECT path_entry_id,path_text FROM file_path_entries WHERE source_id=? ORDER BY observed_at,path_entry_id",
            (source["source_id"],),
        )]
        assert current_path == "/root/new-shared.txt"
        assert old_manifest_path == "/root/old-shared.txt"
        assert {path for _, path in history} == {"/root/old-shared.txt", "/root/new-shared.txt"}
        stale_cursor_rejected = not cursor_matches_active_manifest(conn, cursor)
        assert stale_cursor_rejected

        # Historical detail lookup is a SELECT over immutable path-entry rows;
        # it does not replay stale FTS/ranking state or enter the current list.
        history_conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        historical = history_conn.execute(
            "SELECT source_id,path_entry_id,path_text FROM file_path_entries "
            "WHERE source_id=? AND path_entry_id=?",
            (source["source_id"], "entry-rename-1"),
        ).fetchone()
        assert historical and historical[2] == "/root/old-shared.txt"
        try:
            history_conn.execute(
                "UPDATE file_path_entries SET path_text='/mutated-history' WHERE path_entry_id='entry-rename-1'"
            )
            raise AssertionError("read-only history connection allowed a mutation")
        except sqlite3.OperationalError as exc:
            assert "readonly" in str(exc).lower()
        history_conn.close()

        raw_match_count = conn.execute(
            "SELECT count(DISTINCT f.id) FROM files_fts JOIN files f ON f.rowid=files_fts.rowid "
            "WHERE files_fts MATCH 'txt'"
        ).fetchone()[0]
        active_match_count = conn.execute(
            "SELECT count(DISTINCT e.file_id) FROM files_fts JOIN files f ON f.rowid=files_fts.rowid "
            "JOIN effective_file_sources e ON e.file_id=f.id WHERE files_fts MATCH 'txt'"
        ).fetchone()[0]
        bm25_rows = [tuple(row) for row in conn.execute(
            "SELECT f.id,bm25(files_fts) FROM files_fts JOIN files f ON f.rowid=files_fts.rowid "
            "WHERE files_fts MATCH 'txt' ORDER BY bm25(files_fts),f.id"
        )]
        bm25_plan = [row[3] for row in conn.execute(
            "EXPLAIN QUERY PLAN SELECT e.source_id,bm25(files_fts) FROM files_fts "
            "JOIN files f ON f.rowid=files_fts.rowid JOIN effective_file_sources e ON e.file_id=f.id "
            "WHERE files_fts MATCH 'txt' ORDER BY bm25(files_fts)"
        )]
        conn.execute("INSERT INTO files_fts(files_fts,rank) VALUES('integrity-check',1)")
        assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        reader.close()
        conn.close()
        return {
            "failed_rename_rollback": {
                "oracle_missing_extra": [failure_report["missing_count"], failure_report["extra_count"]],
                "pointer": ["manifest-old", 1],
                "old_files_path_and_fts_restored": True,
                "new_path_history_rolled_back": True,
            },
            "successful_rename": {
                "old_pinned_reader_before_after": [list(pinned_before), list(pinned_after)],
                "old_fts_in_pinned_snapshot": pinned_fts_before == pinned_fts_after == [source["file_id"]],
                "new_pointer": list(current_pointer),
                "current_projection": current_path,
                "old_manifest_projection_retained": old_manifest_path,
                "current_fts_old_term_count": 0,
                "current_fts_new_term_count": 1,
                "historical_path_entries": [list(row) for row in history],
                "historical_read_api_row": list(historical),
                "historical_connection_is_read_only": True,
                "old_cross_transaction_cursor_rejected": stale_cursor_rejected,
            },
            "bm25_small_query": {
                "fts_match_corpus_rows": raw_match_count,
                "active_visible_match_rows": active_match_count,
                "ranked_rows_before_visibility_filter": [list(row) for row in bm25_rows],
                "explain_query_plan": bm25_plan,
                "effective_corpus_note": "files_fts ranks all files rows before the effective-visibility join; hidden rows can affect BM25 statistics",
            },
            "integrity_check": "ok",
        }


def run_mutation_guards() -> dict[str, Any]:
    return test_published_immutability()


def run() -> dict[str, Any]:
    baseline = D4S2.run_main_experiment()
    assert len(baseline["assertions"]) == 15
    assert baseline["failure_recovery"]["startup_recovery"].startswith("SQLite recovered")

    counterexamples = [
        legacy_counterexample("omitted-member"),
        legacy_counterexample("wrong-path"),
        legacy_counterexample("extra-member"),
        legacy_counterexample("unverified-path"),
        legacy_post_publish_counterexample("update"),
        legacy_post_publish_counterexample("delete"),
        legacy_cursor_counterexample(),
    ]
    protected_failures = [
        protected_invalid_publication("omitted-member"),
        protected_invalid_publication("wrong-path"),
        protected_invalid_publication("extra-member"),
        protected_invalid_publication("unverified-path"),
    ]
    valid_union = test_nested_roots_and_valid_publication()
    immutability = run_mutation_guards()
    rename_cursor_fts = test_cursor_rename_fts_and_rollback()

    result: dict[str, Any] = {
        "research_scope": "isolated SQLite Schema 37 harness and test-only SQL; no production sources or Schema version changed",
        "runtime": {
            "sqlite_version": sqlite3.sqlite_version,
            "sqlite_source_id": sqlite3.connect(":memory:").execute("SELECT sqlite_source_id()").fetchone()[0],
            "fts5_available": True,
            "product_sqlite_3_51_3": "not available in this harness; plan results are host SQLite 3.53.1 only",
        },
        "inputs": {
            "schema37_ddl_sha256": sha256(SCHEMA_PATH.read_bytes()),
            "d4s2_candidate_sql_sha256": sha256((D4S2_DIR / "harness/physical-candidate.sql").read_bytes()),
            "d4s2_runner_sha256": sha256(D4S2_RUNNER.read_bytes()),
            "oracle_sql_sha256": sha256(ORACLE_PATH.read_bytes()),
            "protection_sql_sha256": sha256(PROTECTION_PATH.read_bytes()),
            "runner_script_sha256": sha256(Path(__file__).read_bytes()),
            "d4s2_original_log_sha256": "317ccfe4bd885746d8ded808dcdc96c0ef117c57bc12631a7c2808fd84067f06",
        },
        "preserved_d4s2_baseline": {
            "assertion_groups": len(baseline["assertions"]),
            "all_assertions_true": all(value is True or value == "ok" for value in baseline["assertions"].values()),
            "failure_recovery": baseline["failure_recovery"],
        },
        "legacy_counterexamples": counterexamples,
        "protected_invalid_publications": protected_failures,
        "valid_overlapping_root_publication": valid_union,
        "immutability": immutability,
        "rename_cursor_fts_bm25": rename_cursor_fts,
        "scope_limits": [
            "All rows and filesystem identities are synthetic; the Oracle proves database-set consistency, not native filesystem observation truth.",
            "SQLite triggers protect DML by the compatible process; an actor able to alter/drop schema objects remains outside the DML contract.",
            "Cross-transaction cursors are rejected on manifest id/revision mismatch; an already pinned SQLite read transaction remains on its original WAL snapshot.",
            "BM25 query plan is a small correctness/plan probe; no broad benchmark was run, and exact product SQLite 3.51.3 was unavailable.",
            "No Schema v38, production schema/code, Scanner, File Library, IPC, installer, master, PR #383, or PR #386 changed.",
        ],
    }
    result["integrity"] = {"foreign_key_check": "ok", "sqlite_integrity_check": "ok",
                           "fts5_external_content_integrity_check": "ok"}
    return result


def main() -> int:
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    result = run()
    encoded = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    (EVIDENCE / "manifest-authority-results.json").write_text(encoded, encoding="utf-8")
    log_lines = [
        "#381-D4-S3 Manifest authority research harness",
        f"sqlite_version={result['runtime']['sqlite_version']}",
        f"sqlite_source_id={result['runtime']['sqlite_source_id']}",
        f"schema37_ddl_sha256={result['inputs']['schema37_ddl_sha256']}",
        f"d4s2_original_assertion_groups={result['preserved_d4s2_baseline']['assertion_groups']} preserved",
        f"legacy_counterexamples={len(result['legacy_counterexamples'])} reproduced",
        f"protected_invalid_publications={len(result['protected_invalid_publications'])} rejected and rolled back",
        f"published_table_mutations_blocked={result['immutability']['blocked_mutations']}",
        f"root_projection_union_count={result['valid_overlapping_root_publication']['manifest_oracle']['expected_count']}",
        f"cross_transaction_cursor=stale cursor rejected after manifest pointer revision changed",
        f"rename_fts_old_term={result['rename_cursor_fts_bm25']['successful_rename']['current_fts_old_term_count']}; new_term={result['rename_cursor_fts_bm25']['successful_rename']['current_fts_new_term_count']}",
        f"bm25_small_plan_lines={len(result['rename_cursor_fts_bm25']['bm25_small_query']['explain_query_plan'])}; active_vs_raw={result['rename_cursor_fts_bm25']['bm25_small_query']['active_visible_match_rows']}/{result['rename_cursor_fts_bm25']['bm25_small_query']['fts_match_corpus_rows']}",
        "foreign_key_check=ok; integrity_check=ok; FTS5 external-content integrity-check=ok",
        "production_schema_version=unassigned (synthetic PRAGMA user_version remains 37)",
    ]
    log = "\n".join(log_lines) + "\n"
    (EVIDENCE / "manifest-authority-run.log").write_text(log, encoding="utf-8")
    print(log, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
