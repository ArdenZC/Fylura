#!/usr/bin/env python3
"""D4-S6 publication-fence and cursor-scope research harness."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sqlite3
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[5]
HERE = Path(__file__).resolve().parents[1]
S5_DIR = ROOT / "docs/project/research/issue-381-d4-s5"
S5_RUNNER = S5_DIR / "harness/run.py"
FENCE_SQL = HERE / "harness/publication-fence.sql"
EVIDENCE = HERE / "evidence"
sys.dont_write_bytecode = True


def load_s5_runner():
    spec = importlib.util.spec_from_file_location("d4s5_scoped_authority_harness", S5_RUNNER)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


S5 = load_s5_runner()
S4 = S5.S4
S3 = S5.S3


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def trace_publication(conn: sqlite3.Connection, call) -> tuple[Any, list[str], str | None]:
    statements: list[str] = []
    conn.set_trace_callback(statements.append)
    result: Any = None
    error: str | None = None
    try:
        result = call()
    except Exception as exc:  # retain the statements that ran before rollback
        error = str(exc)
    finally:
        conn.set_trace_callback(None)
    raw_publication = [
        statement for statement in statements
        if any(token in statement.upper() for token in (
            "BEGIN IMMEDIATE", "COMMIT", "ROLLBACK",
            "UPDATE SOURCE_GENERATIONS", "UPDATE VISIBILITY_MANIFESTS",
            "INSERT INTO ACTIVE_VISIBILITY_POINTER", "UPDATE ACTIVE_VISIBILITY_POINTER",
            "INSERT INTO D4S4_MANIFEST_AUTHORITY_SEALS",
            "INSERT INTO D4S4_MANIFEST_ROOT_EVIDENCE",
            "INSERT INTO D4S4_MANIFEST_SOURCE_EVIDENCE",
            "INSERT INTO D4S5_MANIFEST_ROOT_SEALS",
            "INSERT INTO D4S5_MANIFEST_SOURCE_SEALS",
        ))
    ]
    publication: list[str] = []
    for statement in raw_publication:
        # sqlite3's trace callback reports an outer statement again for each
        # trigger frame. Retain the ordered executed DML without that echo.
        if not publication or publication[-1] != statement:
            publication.append(statement)
    return result, publication, error


def ready_fixture(two_roots: bool = False):
    conn, s4_state, s5_state = S5.fresh_db()
    if two_roots:
        sources = [
            S5.add_named_source(conn, "a", "/root-a/a-marker.txt", "amarker.txt"),
            S5.add_named_source(conn, "b", "/root-b/b-marker.txt", "bmarker.txt"),
        ]
        S4.add_generation(conn, "root-a", "/root-a", "gen-a", "run-a", 1, [sources[0]])
        S4.add_generation(conn, "root-b", "/root-b", "gen-b", "run-b", 1, [sources[1]])
        S3.stage_manifest(conn, "manifest-a", [("root-a", "gen-a"), ("root-b", "gen-b")],
                          [S3.projection(source) for source in sources], None)
    else:
        sources = [S5.add_named_source(conn, "a", "/root/a-marker.txt", "amarker.txt")]
        S4.add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, sources)
        S3.stage_manifest(conn, "manifest-a", [("root-a", "gen-a")],
                          [S3.projection(source) for source in sources], None)
    return conn, s4_state, s5_state, sources


def status_pointer_seals_visibility(conn: sqlite3.Connection, manifest_id: str) -> dict[str, Any]:
    return {
        "manifest_status": conn.execute(
            "SELECT status FROM visibility_manifests WHERE manifest_id=?", (manifest_id,)
        ).fetchone()[0],
        "pointer": S4.pointer(conn),
        "root_seals": conn.execute(
            "SELECT count(*) FROM d4s5_manifest_root_seals WHERE manifest_id=?", (manifest_id,)
        ).fetchone()[0],
        "source_seals": conn.execute(
            "SELECT count(*) FROM d4s5_manifest_source_seals WHERE manifest_id=?", (manifest_id,)
        ).fetchone()[0],
        "projection_rows": conn.execute(
            "SELECT count(*) FROM manifest_file_paths WHERE manifest_id=?", (manifest_id,)
        ).fetchone()[0],
        "effective_sources": S5.source_ids(conn),
    }


def reproduce_old_s4_bypass() -> dict[str, Any]:
    conn, s4_state, s5_state, sources = ready_fixture()
    assert s5_state["publish"] is False
    result, sql, error = trace_publication(
        conn, lambda: S4.trusted_publish(conn, s4_state, "manifest-a")
    )
    assert error is None, error
    outcome = status_pointer_seals_visibility(conn, "manifest-a")
    outcome.update({
        "scenario": "direct D4-S4 trusted_publish with D4-S5 publish capability false",
        "d4s5_publish_capability": s5_state["publish"],
        "preflight_source_count": result["source_evidence_count"],
        "executed_publication_sql": sql,
        "bypass_confirmed": outcome["manifest_status"] == "published"
            and outcome["pointer"] == ("manifest-a", 1)
            and outcome["root_seals"] == 0 and outcome["source_seals"] == 0
            and outcome["effective_sources"] == [],
    })
    assert outcome["bypass_confirmed"]
    conn.close()
    return outcome


def reproduce_partial_seal_commit() -> dict[str, Any]:
    conn, s4_state, s5_state, sources = ready_fixture(two_roots=True)
    original_preflight = S4.publication_preflight

    def omit_one_source(connection, manifest_id):
        report, roots, evidence = original_preflight(connection, manifest_id)
        return report, roots, evidence[:-1]

    S4.publication_preflight = omit_one_source
    try:
        result, sql, error = trace_publication(
            conn, lambda: S5.publish(conn, s4_state, s5_state, "manifest-a")
        )
    finally:
        S4.publication_preflight = original_preflight
    outcome = status_pointer_seals_visibility(conn, "manifest-a")
    outcome.update({
        "scenario": "trusted helper receives one omitted source-evidence row",
        "d4s5_publish_capability_after_return": s5_state["publish"],
        "returned_source_evidence_count": result["source_evidence_count"],
        "expected_member_count": result["member_count"],
        "expected_sources": [source["source_id"] for source in sources],
        "executed_publication_sql": sql,
        "partial_commit_confirmed": outcome["manifest_status"] == "published"
            and outcome["pointer"] == ("manifest-a", 1)
            and outcome["root_seals"] == 2 and outcome["source_seals"] == 1
            and outcome["projection_rows"] == 2
            and outcome["effective_sources"] == ["source-a"],
    })
    assert error is None, error
    assert outcome["partial_commit_confirmed"]
    conn.close()
    return outcome


def register_s6_capability(conn: sqlite3.Connection) -> dict[str, bool]:
    state = {"publish": False}
    conn.create_function("d4s6_publish_authorized", 0, lambda: int(state["publish"]))
    for statement in S3.D4S2.split_sql(FENCE_SQL.read_text(encoding="utf-8")):
        conn.execute(statement)
    return state


def fenced_db():
    conn, s4_state, s5_state = S5.fresh_db()
    s6_state = register_s6_capability(conn)
    return conn, s4_state, s5_state, s6_state


def fenced_publish(conn: sqlite3.Connection, s4_state: dict[str, bool],
                   s5_state: dict[str, Any], s6_state: dict[str, bool],
                   manifest_id: str) -> dict[str, Any]:
    # This is the sole successful entry in the D6 harness. The imported S4
    # routine still owns BEGIN IMMEDIATE/COMMIT, while SQL requires all three
    # capabilities and validates seals before its pointer statement.
    s5_state["publish"] = True
    s6_state["publish"] = True
    try:
        return S4.trusted_publish(conn, s4_state, manifest_id)
    finally:
        s5_state["publish"] = False
        s6_state["publish"] = False


def fenced_ready_fixture(two_roots: bool = False):
    conn, s4_state, s5_state, s6_state = fenced_db()
    if two_roots:
        sources = [
            S5.add_named_source(conn, "a", "/root-a/a-marker.txt", "amarker.txt"),
            S5.add_named_source(conn, "b", "/root-b/b-marker.txt", "bmarker.txt"),
        ]
        S4.add_generation(conn, "root-a", "/root-a", "gen-a", "run-a", 1, [sources[0]])
        S4.add_generation(conn, "root-b", "/root-b", "gen-b", "run-b", 1, [sources[1]])
        S3.stage_manifest(conn, "manifest-a", [("root-a", "gen-a"), ("root-b", "gen-b")],
                          [S3.projection(source) for source in sources], None)
    else:
        sources = [S5.add_named_source(conn, "a", "/root/a-marker.txt", "amarker.txt")]
        S4.add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, sources)
        S3.stage_manifest(conn, "manifest-a", [("root-a", "gen-a")],
                          [S3.projection(source) for source in sources], None)
    return conn, s4_state, s5_state, s6_state, sources


def publication_state(conn: sqlite3.Connection) -> dict[str, Any]:
    tables = (
        ("visibility_manifests", "SELECT manifest_id,status FROM visibility_manifests ORDER BY manifest_id"),
        ("source_generations", "SELECT generation_id,status FROM source_generations ORDER BY generation_id"),
        ("scan_roots", "SELECT id,revision,watcher_revision,watcher_applied_revision,health_status,needs_reconciliation FROM scan_roots ORDER BY id"),
        ("scan_runs", "SELECT id,status,coverage_complete,watcher_revision_at_start FROM scan_runs ORDER BY id"),
        ("file_sources", "SELECT source_id,status,identity_confidence,source_revision,retired_at FROM file_sources ORDER BY source_id"),
        ("file_path_entries", "SELECT path_entry_id,source_id,path_text,identity_status,evidence_digest FROM file_path_entries ORDER BY path_entry_id"),
        ("file_physical_identity_epochs", "SELECT physical_identity_id,identity_epoch,evidence_digest FROM file_physical_identity_epochs ORDER BY physical_identity_id"),
        ("files", "SELECT id,path,name,context FROM files ORDER BY id"),
        ("user_tags", "SELECT id,display_name,normalized_name FROM user_tags ORDER BY id"),
        ("file_user_tags", "SELECT file_id,tag_id,created_at FROM file_user_tags ORDER BY file_id,tag_id"),
        ("source_identity_claims", "SELECT claim_id,source_id,path_entry_id,authorization_digest FROM source_identity_claims ORDER BY claim_id"),
        ("d4s4_claim_revocations", "SELECT claim_id,revoked_at,reason FROM d4s4_claim_revocations ORDER BY claim_id"),
        ("d4s4_authority_clock", "SELECT singleton_id,revision FROM d4s4_authority_clock ORDER BY singleton_id"),
        ("d4s4_manifest_authority_seals", "SELECT manifest_id,root_count,member_count,source_count FROM d4s4_manifest_authority_seals ORDER BY manifest_id"),
        ("d4s4_manifest_root_evidence", "SELECT manifest_id,root_id,generation_id FROM d4s4_manifest_root_evidence ORDER BY manifest_id,root_id"),
        ("d4s4_manifest_source_evidence", "SELECT manifest_id,source_id,path_entry_id FROM d4s4_manifest_source_evidence ORDER BY manifest_id,source_id,path_entry_id"),
        ("d4s5_root_authority", "SELECT root_id,revision FROM d4s5_root_authority ORDER BY root_id"),
        ("d4s5_source_authority", "SELECT source_id,revision FROM d4s5_source_authority ORDER BY source_id"),
        ("d4s5_manifest_root_seals", "SELECT manifest_id,root_id,generation_id,root_authority_revision FROM d4s5_manifest_root_seals ORDER BY manifest_id,root_id"),
        ("d4s5_manifest_source_seals", "SELECT manifest_id,source_id,path_entry_id,source_authority_revision FROM d4s5_manifest_source_seals ORDER BY manifest_id,source_id,path_entry_id"),
        ("d4s6_publication_trace", "SELECT manifest_id,event_kind,object_id FROM d4s6_publication_trace ORDER BY sequence_no"),
    )
    state = {name: [tuple(r) for r in conn.execute(sql)] for name, sql in tables}
    state["pointer"] = S4.pointer(conn)
    state["effective_sources"] = S5.source_ids(conn)
    return state


def publication_summary(conn: sqlite3.Connection, manifest_id: str) -> dict[str, Any]:
    return {
        "manifest_status": conn.execute(
            "SELECT status FROM visibility_manifests WHERE manifest_id=?", (manifest_id,)
        ).fetchone()[0],
        "generation_statuses": [tuple(r) for r in conn.execute(
            "SELECT sg.generation_id,sg.status FROM manifest_root_heads mr "
            "JOIN source_generations sg ON sg.generation_id=mr.generation_id "
            "WHERE mr.manifest_id=? ORDER BY sg.generation_id", (manifest_id,)
        )],
        "pointer": S4.pointer(conn),
        "d4s4_seal_count": conn.execute(
            "SELECT count(*) FROM d4s4_manifest_authority_seals WHERE manifest_id=?", (manifest_id,)
        ).fetchone()[0],
        "root_seal_count": conn.execute(
            "SELECT count(*) FROM d4s5_manifest_root_seals WHERE manifest_id=?", (manifest_id,)
        ).fetchone()[0],
        "source_seal_count": conn.execute(
            "SELECT count(*) FROM d4s5_manifest_source_seals WHERE manifest_id=?", (manifest_id,)
        ).fetchone()[0],
        "effective_sources": S5.source_ids(conn),
    }


def attempt_rejected(call) -> str:
    try:
        call()
    except (sqlite3.DatabaseError, S4.AuthorityRejected, S3.OracleRejected) as exc:
        return str(exc)
    raise AssertionError("expected publication rejection did not occur")


def test_old_entry_is_blocked_and_rolled_back() -> dict[str, Any]:
    conn, s4_state, s5_state, s6_state, _ = fenced_ready_fixture()
    before = publication_state(conn)
    trace: list[str] = []

    _, trace, error = trace_publication(
        conn, lambda: S4.trusted_publish(conn, s4_state, "manifest-a")
    )
    assert error is not None
    message = error
    after = publication_state(conn)
    assert before == after
    summary = publication_summary(conn, "manifest-a")
    result = {
        "s4_capability_managed_by_legacy_entry": s4_state["publish"],
        "s5_publish_capability": s5_state["publish"],
        "s6_publish_capability": s6_state["publish"],
        "rejection": message,
        "executed_publication_sql_before_abort": trace,
        "before_equals_after": True,
        "after": summary,
    }
    assert summary["manifest_status"] == "ready"
    assert summary["pointer"] is None and summary["root_seal_count"] == 0
    assert summary["source_seal_count"] == 0 and summary["effective_sources"] == []
    assert trace[0].upper() == "BEGIN IMMEDIATE" and trace[-1].upper() == "ROLLBACK"
    conn.close()
    return result


def test_s5_entry_alone_cannot_publish() -> dict[str, Any]:
    conn, s4_state, s5_state, s6_state, _ = fenced_ready_fixture()
    before = publication_state(conn)
    message = attempt_rejected(lambda: S5.publish(conn, s4_state, s5_state, "manifest-a"))
    after = publication_state(conn)
    assert before == after
    assert "d4s6" in message.lower()
    conn.close()
    return {"rejection": message, "before_equals_after": True,
            "manifest_status": "ready", "pointer": None,
            "s5_publish_capability_after_rollback": s5_state["publish"],
            "s6_publish_capability": s6_state["publish"]}


def test_incomplete_root_or_source_seal_rolls_back(kind: str) -> dict[str, Any]:
    conn, s4_state, s5_state, s6_state, _ = fenced_ready_fixture(two_roots=True)
    before = publication_state(conn)
    if kind == "root":
        conn.execute(
            """CREATE TEMP TRIGGER d4s6_inject_omitted_root
               BEFORE INSERT ON d4s5_manifest_root_seals
               WHEN NEW.root_id='root-b'
               BEGIN SELECT RAISE(IGNORE); END"""
        )
        expected_error = "incomplete authority seals"
    else:
        conn.execute(
            """CREATE TEMP TRIGGER d4s6_inject_omitted_source
               BEFORE INSERT ON d4s5_manifest_source_seals
               WHEN NEW.source_id='source-b'
               BEGIN SELECT RAISE(IGNORE); END"""
        )
        expected_error = "incomplete authority seals"
    message = attempt_rejected(lambda: fenced_publish(
        conn, s4_state, s5_state, s6_state, "manifest-a"
    ))
    after = publication_state(conn)
    assert expected_error.lower() in message.lower(), message
    assert before == after
    conn.close()
    return {"seal_kind": kind, "rejection": message,
            "before_equals_after": True, "pointer": None,
            "manifest_status_after": "ready",
            "root_seals_after": 0, "source_seals_after": 0}


def test_injected_sql_failure_rolls_back() -> dict[str, Any]:
    conn, s4_state, s5_state, s6_state, _ = fenced_ready_fixture(two_roots=True)
    before = publication_state(conn)
    conn.execute(
        """CREATE TEMP TRIGGER d4s6_inject_sql_abort
           BEFORE INSERT ON d4s5_manifest_source_seals
           WHEN NEW.source_id='source-b'
           BEGIN SELECT RAISE(ABORT,'injected source seal SQL failure'); END"""
    )
    message = attempt_rejected(lambda: fenced_publish(
        conn, s4_state, s5_state, s6_state, "manifest-a"
    ))
    after = publication_state(conn)
    assert "injected source seal SQL failure" in message
    assert before == after
    conn.close()
    return {"injected_at": "source seal insert", "rejection": message,
            "before_equals_after": True, "manifest_status_after": "ready",
            "pointer": None, "root_seals_after": 0, "source_seals_after": 0}


def test_wrong_parent_cas_rolls_back() -> dict[str, Any]:
    conn, s4_state, s5_state, s6_state, sources = fenced_ready_fixture()
    fenced_publish(conn, s4_state, s5_state, s6_state, "manifest-a")
    S4.add_generation(conn, "root-a", "/root", "gen-bad-parent", "run-bad-parent", 2, sources)
    conn.execute(
        "INSERT INTO visibility_manifests(manifest_id,parent_manifest_id,status,created_at,root_map_digest) "
        "VALUES('manifest-noncurrent',NULL,'abandoned',1800000000,'unpublished')"
    )
    S3.stage_manifest(conn, "manifest-bad-parent", [("root-a", "gen-bad-parent")],
                      [S3.projection(sources[0])], "manifest-noncurrent")
    before = publication_state(conn)
    message = attempt_rejected(lambda: fenced_publish(
        conn, s4_state, s5_state, s6_state, "manifest-bad-parent"
    ))
    after = publication_state(conn)
    assert before == after
    assert S4.pointer(conn) == ("manifest-a", 1)
    conn.close()
    return {"rejection": message, "before_equals_after": True,
            "active_pointer_retained": ["manifest-a", 1],
            "candidate_status": "ready", "candidate_seals": 0}


def test_pointer_cas_rowcount_miss_rolls_back() -> dict[str, Any]:
    conn, s4_state, s5_state, s6_state, sources = fenced_ready_fixture()
    fenced_publish(conn, s4_state, s5_state, s6_state, "manifest-a")
    S4.add_generation(conn, "root-a", "/root", "gen-cas-miss", "run-cas-miss", 2, sources)
    S3.stage_manifest(conn, "manifest-cas-miss", [("root-a", "gen-cas-miss")],
                      [S3.projection(sources[0])], "manifest-a")
    conn.execute(
        """CREATE TEMP TRIGGER d4s6_inject_pointer_cas_miss
           BEFORE UPDATE ON active_visibility_pointer
           WHEN NEW.manifest_id='manifest-cas-miss'
           BEGIN SELECT RAISE(IGNORE); END"""
    )
    before = publication_state(conn)
    message = attempt_rejected(lambda: fenced_publish(
        conn, s4_state, s5_state, s6_state, "manifest-cas-miss"
    ))
    after = publication_state(conn)
    assert "pointer CAS failed" in message
    assert before == after and S4.pointer(conn) == ("manifest-a", 1)
    conn.close()
    return {"injected_at": "pointer UPDATE affected zero rows",
            "rejection": message, "before_equals_after": True,
            "pointer_retained": ["manifest-a", 1],
            "candidate_status": "ready", "candidate_seals": 0}


def test_initial_successor_overlap_and_empty_generation() -> dict[str, Any]:
    # Initial publication and a successor preserve immutable old seals.
    conn, s4_state, s5_state, s6_state, sources = fenced_ready_fixture()
    S5.attach_user_metadata(conn, sources[0]["file_id"], "retained")
    first, initial_sql, initial_error = trace_publication(
        conn, lambda: fenced_publish(conn, s4_state, s5_state, s6_state, "manifest-a")
    )
    assert initial_error is None, initial_error
    assert initial_sql[0].upper() == "BEGIN IMMEDIATE" and initial_sql[-1].upper() == "COMMIT"
    first_summary = publication_summary(conn, "manifest-a")
    first_snapshot = {k: v for k, v in first_summary.items()
                      if k not in ("pointer", "effective_sources")}
    second_source = S5.add_named_source(conn, "b", "/root/b-marker.txt", "bmarker.txt")
    all_sources = [*sources, second_source]
    S4.add_generation(conn, "root-a", "/root", "gen-b", "run-b", 2, all_sources)
    S3.stage_manifest(conn, "manifest-b", [("root-a", "gen-b")],
                      [S3.projection(s) for s in all_sources], "manifest-a")
    second, successor_sql, successor_error = trace_publication(
        conn, lambda: fenced_publish(conn, s4_state, s5_state, s6_state, "manifest-b")
    )
    assert successor_error is None, successor_error
    assert successor_sql[0].upper() == "BEGIN IMMEDIATE" and successor_sql[-1].upper() == "COMMIT"
    retained_summary = publication_summary(conn, "manifest-a")
    initial_trace = [tuple(r) for r in conn.execute(
        "SELECT event_kind,object_id FROM d4s6_publication_trace "
        "WHERE manifest_id='manifest-a' ORDER BY sequence_no"
    )]
    successor = {
        "pointer": S4.pointer(conn),
        "current_sources": S5.source_ids(conn),
        "manifest_a_immutable_and_retained": {
            k: v for k, v in retained_summary.items()
            if k not in ("pointer", "effective_sources")
        } == first_snapshot,
        "manifest_a": retained_summary,
        "manifest_b": publication_summary(conn, "manifest-b"),
        "all_authority_rows": first is not None,
        "initial_publication_sql_order": initial_sql,
        "seal_and_pointer_event_order": initial_trace,
        "successor_publication_sql_order": successor_sql,
        "successor_seal_and_pointer_event_order": [tuple(r) for r in conn.execute(
            "SELECT event_kind,object_id FROM d4s6_publication_trace "
            "WHERE manifest_id='manifest-b' ORDER BY sequence_no")],
        "user_context_retained": conn.execute(
            "SELECT context FROM files WHERE id=?", (sources[0]["file_id"],)
        ).fetchone()[0],
        "user_tag_count_retained": conn.execute(
            "SELECT count(*) FROM file_user_tags WHERE file_id=?", (sources[0]["file_id"],)
        ).fetchone()[0],
    }
    assert successor["pointer"] == ("manifest-b", 2)
    assert successor["current_sources"] == ["source-a", "source-b"]
    assert successor["manifest_a_immutable_and_retained"]
    assert successor["user_context_retained"] == "owner-note-retained"
    assert successor["user_tag_count_retained"] == 1
    conn.close()

    overlap_conn, os4, os5, os6 = fenced_db()
    shared = S5.add_named_source(overlap_conn, "shared", "/shared/nested/overlap-marker.txt", "overlapmarker.txt")
    companion = S5.add_named_source(overlap_conn, "companion", "/shared/nested/companion-marker.txt", "companionmarker.txt")
    S4.add_generation(overlap_conn, "root-a", "/shared", "gen-a", "run-a", 1, [shared])
    S4.add_generation(overlap_conn, "root-b", "/shared/nested", "gen-b", "run-b", 1, [shared, companion])
    S3.stage_manifest(overlap_conn, "manifest-overlap",
                      [("root-a", "gen-a"), ("root-b", "gen-b")],
                      [S3.projection(shared), S3.projection(companion)], None)
    fenced_publish(overlap_conn, os4, os5, os6, "manifest-overlap")
    overlap = {
        "pointer": S4.pointer(overlap_conn),
        "root_members": S5.root_members(overlap_conn),
        "root_seals": [tuple(r) for r in overlap_conn.execute(
            "SELECT root_id,generation_id FROM d4s5_manifest_root_seals "
            "WHERE manifest_id='manifest-overlap' ORDER BY root_id")],
        "source_seals": [tuple(r) for r in overlap_conn.execute(
            "SELECT source_id,path_entry_id FROM d4s5_manifest_source_seals "
            "WHERE manifest_id='manifest-overlap' ORDER BY source_id")],
        "effective_sources": S5.source_ids(overlap_conn),
    }
    assert len(overlap["root_seals"]) == 2 and len(overlap["source_seals"]) == 2
    assert len(overlap["root_members"]) == 3 and overlap["effective_sources"] == ["source-companion", "source-shared"]
    overlap_conn.close()

    empty_conn, es4, es5, es6 = fenced_db()
    S4.add_generation(empty_conn, "root-empty", "/empty", "gen-empty", "run-empty", 1, [])
    S3.stage_manifest(empty_conn, "manifest-empty", [("root-empty", "gen-empty")], [], None)
    fenced_publish(empty_conn, es4, es5, es6, "manifest-empty")
    empty = publication_summary(empty_conn, "manifest-empty")
    assert empty["manifest_status"] == "published" and empty["pointer"] == ("manifest-empty", 1)
    assert empty["root_seal_count"] == 1 and empty["source_seal_count"] == 0
    assert empty["effective_sources"] == []
    empty_conn.close()
    pointer_event_index = next(i for i, row in enumerate(initial_trace)
                               if row[0] == "initial_pointer_cas")
    assert initial_trace and all(
        i < pointer_event_index for i, row in enumerate(initial_trace)
        if row[0] in ("root_seal_insert", "source_seal_insert")
    )
    successor_trace = successor["successor_seal_and_pointer_event_order"]
    successor_pointer_index = next(i for i, row in enumerate(successor_trace)
                                    if row[0] == "successor_pointer_cas")
    assert all(i < successor_pointer_index for i, row in enumerate(successor_trace)
               if row[0] in ("root_seal_insert", "source_seal_insert"))
    successor["source_seals_inserted_before_pointer"] = True
    return {"initial_and_successor": successor, "overlapping_roots": overlap,
            "empty_generation": empty}


def root_search(conn: sqlite3.Connection, root_id: str, expression: str) -> list[tuple[Any, ...]]:
    return [tuple(row) for row in conn.execute(
        """SELECT e.source_id,e.path_entry_id,f.id,f.name,bm25(files_fts)
           FROM files_fts
           JOIN files f ON f.rowid=files_fts.rowid
           JOIN d4s5_effective_root_members e ON e.file_id=f.id
           WHERE files_fts MATCH ? AND e.root_id=?
           ORDER BY bm25(files_fts),e.source_id,e.path_entry_id""",
        (expression, root_id),
    )]


def global_search(conn: sqlite3.Connection, expression: str) -> list[tuple[Any, ...]]:
    return [tuple(row) for row in conn.execute(
        """SELECT DISTINCT e.source_id,f.id,f.name,f.path,bm25(files_fts)
           FROM files_fts
           JOIN files f ON f.rowid=files_fts.rowid
           JOIN d4s5_effective_file_sources e ON e.file_id=f.id
           WHERE files_fts MATCH ?
           ORDER BY bm25(files_fts),e.source_id,f.id""", (expression,),
    )]


def global_cursor_token(conn: sqlite3.Connection, expression: str) -> dict[str, Any]:
    authority = S5.cursor_token(conn)
    ranked_results = [
        [*row[:4], float(row[4]).hex()] for row in global_search(conn, expression)
    ]
    payload = {
        "manifest_id": authority["manifest_id"],
        "pointer_revision": authority["pointer_revision"],
        "global_root_source_seal_sha256": authority["scoped_root_seal_sha256"],
        "fts_expression": expression,
        "result_filter": "all current effective File Library sources",
        "sort_order": "bm25(files_fts),source_id,file_id",
        "fts_ranked_visible_rows_sha256": S5.digest_rows(ranked_results),
    }
    payload["cursor_sha256"] = sha256(json.dumps(
        payload, sort_keys=True, separators=(",", ":")
    ).encode("utf-8"))
    return payload


def root_cursor_token(conn: sqlite3.Connection, root_id: str, expression: str) -> dict[str, Any]:
    authority_rows = [tuple(row) for row in conn.execute(
        """SELECT DISTINCT root_id,generation_id,source_id,path_entry_id,
                  root_authority_revision,current_root_authority_revision,
                  source_authority_revision,current_source_authority_revision
           FROM d4s5_effective_root_members WHERE root_id=?
           ORDER BY root_id,generation_id,source_id,path_entry_id""", (root_id,),
    )]
    ranked_results = [
        [*row[:4], float(row[4]).hex()] for row in root_search(conn, root_id, expression)
    ]
    payload = {
        "root_id": root_id,
        "fts_expression": expression,
        "result_filter": "d4s5_effective_root_members.root_id = :root_id",
        "sort_order": "bm25(files_fts),source_id,path_entry_id",
        "root_source_seal_sha256": S5.digest_rows(authority_rows),
        "fts_ranked_visible_rows_sha256": S5.digest_rows(ranked_results),
    }
    payload["cursor_sha256"] = sha256(json.dumps(
        payload, sort_keys=True, separators=(",", ":")
    ).encode("utf-8"))
    return payload


def test_global_and_root_scoped_cursors() -> dict[str, Any]:
    conn, s4_state, s5_state, s6_state = fenced_db()
    shared = S5.add_named_source(conn, "shared", "/shared/nested/overlap-marker.txt", "overlapmarker.txt")
    companion = S5.add_named_source(conn, "companion", "/shared/nested/companion-marker.txt", "companionmarker.txt")
    S4.add_generation(conn, "root-a", "/shared", "gen-a", "run-a", 1, [shared])
    S4.add_generation(conn, "root-b", "/shared/nested", "gen-b", "run-b", 1, [shared, companion])
    S3.stage_manifest(conn, "manifest-overlap",
                      [("root-a", "gen-a"), ("root-b", "gen-b")],
                      [S3.projection(shared), S3.projection(companion)], None)
    fenced_publish(conn, s4_state, s5_state, s6_state, "manifest-overlap")

    global_before = global_cursor_token(conn, "overlapmarker")
    global_results_before = global_search(conn, "overlapmarker")
    root_b_before = root_cursor_token(conn, "root-b", "overlapmarker")
    results_before = root_search(conn, "root-b", "overlapmarker")
    assert [row[0] for row in results_before] == ["source-shared"]

    S5.change_root_watcher(conn, s4_state, s5_state, "root-a", "watch-root-a-1")
    after_watcher_global = global_cursor_token(conn, "overlapmarker")
    global_results_after_watcher = global_search(conn, "overlapmarker")
    after_watcher_root_b = root_cursor_token(conn, "root-b", "overlapmarker")
    results_after_watcher = root_search(conn, "root-b", "overlapmarker")
    watcher_case = {
        "pointer_unchanged": S4.pointer(conn) == ("manifest-overlap", 1),
        "root_a_hidden_root_b_shared_source_visible": S5.root_members(conn),
        "global_cursor_sha256_before_after": [global_before["cursor_sha256"], after_watcher_global["cursor_sha256"]],
        "global_authority_digest_before_after": [global_before["global_root_source_seal_sha256"], after_watcher_global["global_root_source_seal_sha256"]],
        "root_b_cursor_sha256_before_after": [root_b_before["cursor_sha256"], after_watcher_root_b["cursor_sha256"]],
        "root_b_authority_digest_before_after": [root_b_before["root_source_seal_sha256"], after_watcher_root_b["root_source_seal_sha256"]],
        "global_cursor_rejected": after_watcher_global != global_before,
        "global_fts_rows_before_after": [
            [*row[:4], float(row[4]).hex()] for row in global_results_before
        ] + [[*row[:4], float(row[4]).hex()] for row in global_results_after_watcher],
        "global_fts_results_unchanged": global_results_after_watcher == global_results_before,
        "root_b_cursor_still_valid": after_watcher_root_b == root_b_before,
        "root_b_fts_rows_before_after": [
            [*row[:4], float(row[4]).hex()] for row in results_before
        ] + [[*row[:4], float(row[4]).hex()] for row in results_after_watcher],
        "root_b_fts_results_unchanged": results_after_watcher == results_before,
    }
    assert watcher_case["global_cursor_rejected"] and watcher_case["root_b_cursor_still_valid"]
    assert watcher_case["global_fts_results_unchanged"] and watcher_case["root_b_fts_results_unchanged"]
    assert watcher_case["root_a_hidden_root_b_shared_source_visible"] == [
        ("root-b", "source-companion", "entry-companion-1"),
        ("root-b", "source-shared", "entry-shared-1"),
    ]

    # Apply A's Watcher revision and publish an A-only replacement generation.
    # B's root generation, Source Seal, and ranked FTS rows remain unchanged.
    S5.scoped_mutation(
        conn, s4_state, s5_state, "watch-applied-root-a-1",
        "UPDATE scan_roots SET watcher_applied_revision=watcher_revision,revision=revision+1 WHERE id=?",
        ("root-a",),
    )
    S4.add_generation(conn, "root-a", "/shared", "gen-a2", "run-a2", 2, [shared])
    S3.stage_manifest(conn, "manifest-overlap-2",
                      [("root-a", "gen-a2"), ("root-b", "gen-b")],
                      [S3.projection(shared), S3.projection(companion)], "manifest-overlap")
    fenced_publish(conn, s4_state, s5_state, s6_state, "manifest-overlap-2")
    root_b_after_publish = root_cursor_token(conn, "root-b", "overlapmarker")
    results_after_publish = root_search(conn, "root-b", "overlapmarker")
    publish_case = {
        "pointer": S4.pointer(conn),
        "root_b_cursor_sha256_before_after": [root_b_before["cursor_sha256"], root_b_after_publish["cursor_sha256"]],
        "root_b_authority_digest_before_after": [root_b_before["root_source_seal_sha256"], root_b_after_publish["root_source_seal_sha256"]],
        "root_b_cursor_still_valid_across_root_a_only_publish": root_b_after_publish == root_b_before,
        "root_b_fts_results_unchanged": results_after_publish == results_before,
        "root_b_source_seal": [tuple(r) for r in conn.execute(
            "SELECT source_id,path_entry_id,source_authority_revision "
            "FROM d4s5_manifest_source_seals WHERE manifest_id='manifest-overlap-2' "
            "AND source_id='source-shared' ORDER BY source_id,path_entry_id")],
        "root_b_root_seal": [tuple(r) for r in conn.execute(
            "SELECT root_id,generation_id,root_authority_revision "
            "FROM d4s5_manifest_root_seals WHERE manifest_id='manifest-overlap-2' "
            "AND root_id='root-b'")],
    }
    assert publish_case["pointer"] == ("manifest-overlap-2", 2)
    assert publish_case["root_b_cursor_still_valid_across_root_a_only_publish"]
    assert publish_case["root_b_fts_results_unchanged"]
    assert publish_case["root_b_root_seal"] == [("root-b", "gen-b", 1)]

    # A shared-source identity change invalidates B's Source Seal too; the old
    # B cursor must not resurrect that file even though B's generation is old.
    S5.scoped_mutation(
        conn, s4_state, s5_state, "identity-shared-source",
        "UPDATE file_sources SET status='conflicted',source_revision=source_revision+1 "
        "WHERE source_id=? AND source_revision=?", (shared["source_id"], 2),
    )
    after_shared_change = root_cursor_token(conn, "root-b", "overlapmarker")
    shared_invalidated = {
        "root_b_cursor_sha256_before_after": [root_b_before["cursor_sha256"], after_shared_change["cursor_sha256"]],
        "root_b_authority_digest_before_after": [root_b_before["root_source_seal_sha256"], after_shared_change["root_source_seal_sha256"]],
        "old_root_b_cursor_rejected": after_shared_change != root_b_before,
        "root_b_search_after_source_change": root_search(conn, "root-b", "overlapmarker"),
        "visible_root_members": S5.root_members(conn),
        "source_seal_retained_as_history": conn.execute(
            "SELECT count(*) FROM d4s5_manifest_source_seals "
            "WHERE manifest_id='manifest-overlap-2' AND source_id='source-shared'"
        ).fetchone()[0] == 1,
    }
    assert shared_invalidated["old_root_b_cursor_rejected"]
    assert shared_invalidated["root_b_search_after_source_change"] == []
    assert [row[1] for row in shared_invalidated["visible_root_members"]] == ["source-companion"]
    assert shared_invalidated["source_seal_retained_as_history"]
    conn.close()
    return {"watcher_only_root_a_change": watcher_case,
            "root_a_only_manifest_publish": publish_case,
            "shared_source_change": shared_invalidated,
            "cursor_contract": {
                "global": "binds active Manifest pointer and all effective root/source seals; any effective-root change rejects it",
                "root_scoped": "binds only the requested root's current root/source seal vector and exact visible FTS ranked result rows; global pointer revision is excluded",
                "safety_condition": "root filter is applied in the query; BM25 row scores are included so changes in global FTS corpus that affect ranking reject the cursor",
            }}


def run_fence_cases() -> dict[str, Any]:
    blocked_old_entry = test_old_entry_is_blocked_and_rolled_back()
    blocked_s5_entry = test_s5_entry_alone_cannot_publish()
    root_omission = test_incomplete_root_or_source_seal_rolls_back("root")
    source_omission = test_incomplete_root_or_source_seal_rolls_back("source")
    sql_error = test_injected_sql_failure_rolls_back()
    wrong_parent = test_wrong_parent_cas_rolls_back()
    cas_miss = test_pointer_cas_rowcount_miss_rolls_back()
    successful = test_initial_successor_overlap_and_empty_generation()
    cursors = test_global_and_root_scoped_cursors()
    return {
        "blocked_legacy_s4_entry": blocked_old_entry,
        "blocked_s5_without_s6_entry": blocked_s5_entry,
        "missing_root_seal_rolls_back": root_omission,
        "missing_source_seal_rolls_back": source_omission,
        "sql_failure_rolls_back": sql_error,
        "stale_parent_cas_rolls_back": wrong_parent,
        "pointer_cas_rowcount_miss_rolls_back": cas_miss,
        "successful_publications": successful,
        "cursor_scope": cursors,
    }


def run() -> dict[str, Any]:
    baseline = {
        "direct_s4_without_s5_capability": reproduce_old_s4_bypass(),
        "partial_seals_can_still_commit": reproduce_partial_seal_commit(),
    }
    fenced = run_fence_cases()
    preserved_s5 = S5.run()
    assert preserved_s5["preserved_contracts"]["d4s2_assertion_groups"] == 15
    assert preserved_s5["preserved_contracts"]["d4s3_failure_counterexamples"] == 7
    assert preserved_s5["preserved_contracts"]["d4s3_invalid_publication_cases"] == 4
    assert preserved_s5["preserved_contracts"]["d4s3_dml_guards"] == 16
    assert preserved_s5["preserved_contracts"]["d4s4_transitive_mutations_blocked"] == 11
    schema_probe, _, _ = S5.fresh_db()
    schema_version = schema_probe.execute("PRAGMA user_version").fetchone()[0]
    schema_probe.close()
    input_hashes = {
        "schema37_ddl_sha256": sha256(S3.SCHEMA_PATH.read_bytes()),
        "d4s2_candidate_sql_sha256": sha256(S3.D4S2.CANDIDATE_PATH.read_bytes()),
        "d4s3_runner_sha256": sha256((S3.D4S2.ROOT / "docs/project/research/issue-381-d4-s3/harness/run.py").read_bytes()),
        "d4s3_oracle_sql_sha256": sha256((S3.D4S2.ROOT / "docs/project/research/issue-381-d4-s3/harness/manifest-authority-oracle.sql").read_bytes()),
        "d4s3_protection_sql_sha256": sha256((S3.D4S2.ROOT / "docs/project/research/issue-381-d4-s3/harness/manifest-authority-protection.sql").read_bytes()),
        "d4s4_runner_sha256": sha256(S5.S4_RUNNER.read_bytes()),
        "d4s4_protection_sql_sha256": sha256((S5.S4_DIR / "harness/transitive-evidence-protection.sql").read_bytes()),
        "d4s5_runner_sha256": sha256(S5_RUNNER.read_bytes()),
        "d4s5_prototype_sql_sha256": sha256((S5_DIR / "harness/scoped-authority-prototype.sql").read_bytes()),
        "d4s6_runner_sha256": sha256(Path(__file__).read_bytes()),
        "d4s6_fence_sql_sha256": sha256(FENCE_SQL.read_bytes()),
    }
    return {
        "phase": "D4-S6 baseline counterexamples and fenced publication/cursor research",
        "frozen_base": {
            "head": "59e71da8e37dc7b1f1828b71af06596d178e5611",
            "tree": "9439b7264c34130f5716e482a35fffc0b225ce01",
        },
        "input_sha256": input_hashes,
        "runtime": {
            "sqlite_version": sqlite3.sqlite_version,
            "sqlite_source_id": sqlite3.connect(":memory:").execute(
                "SELECT sqlite_source_id()"
            ).fetchone()[0],
            "schema_user_version": schema_version,
        },
        "baseline_counterexamples": baseline,
        "publication_fence_and_cursor": fenced,
        "preserved_d4s5_regressions": {
            "d4s2_assertion_groups": 15,
            "d4s3_failure_counterexamples": 7,
            "d4s3_invalid_publication_cases": 4,
            "d4s3_dml_guards": 16,
            "d4s4_transitive_mutations_blocked": 11,
            "d4s5_contracts": "unchanged D4-S5 harness executed and passed",
            "root_scoped_false_hide": preserved_s5["root_scoped_false_hide_regression"],
            "independent_source_identity": preserved_s5["independent_source_identity_scope"],
            "overlapping_shared_identity": preserved_s5["overlapping_shared_source_identity"],
            "overlapping_roots": preserved_s5["overlapping_root_and_last_membership"],
            "same_path_reuse": preserved_s5["same_path_new_incarnation_isolation"],
            "revoke_failure_injection": preserved_s5["claim_revoke_failure_injection"],
            "revoke_reader_reconnect": preserved_s5["claim_revoke_reader_commit_reconnect"],
        },
    }


def main() -> None:
    result = run()
    payload = json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    (EVIDENCE / "publication-fence-results.json").write_text(payload, encoding="utf-8")
    input_payload = {
        "frozen_base": result["frozen_base"],
        "runtime": result["runtime"],
        "sha256": result["input_sha256"],
    }
    (EVIDENCE / "INPUTS.json").write_text(
        json.dumps(input_payload, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(payload, end="")


if __name__ == "__main__":
    main()
