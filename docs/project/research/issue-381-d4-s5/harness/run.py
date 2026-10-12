#!/usr/bin/env python3
"""D4-S5 root-scoped authority and atomic evidence research harness."""

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
S4_DIR = ROOT / "docs/project/research/issue-381-d4-s4"
S4_RUNNER = S4_DIR / "harness/run.py"
S5_SQL = HERE / "harness/scoped-authority-prototype.sql"
EVIDENCE = HERE / "evidence"
NOW = 1_800_000_500
sys.dont_write_bytecode = True


class InjectedFailure(RuntimeError):
    pass


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest_rows(rows: list[tuple[Any, ...]] | list[list[Any]]) -> str:
    ordered = sorted([list(row) for row in rows], key=lambda r: json.dumps(r, separators=(",", ":")))
    return sha256(json.dumps(ordered, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def load_s4_runner():
    spec = importlib.util.spec_from_file_location("d4s4_transitive_authority_harness", S4_RUNNER)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


S4 = load_s4_runner()
S3 = S4.S3


def register_s5_capabilities(conn: sqlite3.Connection) -> dict[str, Any]:
    state: dict[str, Any] = {"publish": False, "evidence": False, "event_id": None}
    conn.create_function("d4s5_publish_authorized", 0, lambda: int(state["publish"]))
    conn.create_function("d4s5_evidence_authorized", 0, lambda: int(state["evidence"]))
    conn.create_function("d4s5_event_id", 0, lambda: state["event_id"])
    return state


def install_s5_sql(conn: sqlite3.Connection) -> None:
    for statement in S3.D4S2.split_sql(S5_SQL.read_text(encoding="utf-8")):
        conn.execute(statement)


def fresh_db(path: str | Path = ":memory:") -> tuple[sqlite3.Connection, dict[str, bool], dict[str, Any]]:
    conn, s4_state = S4.fresh_db(s4=True, path=path)
    s5_state = register_s5_capabilities(conn)
    install_s5_sql(conn)
    return conn, s4_state, s5_state


def publish(conn: sqlite3.Connection, s4_state: dict[str, bool],
            s5_state: dict[str, Any], manifest_id: str) -> dict[str, Any]:
    s5_state["publish"] = True
    try:
        result = S4.trusted_publish(conn, s4_state, manifest_id)
    finally:
        s5_state["publish"] = False
    roots = conn.execute(
        "SELECT count(*) FROM d4s5_manifest_root_seals WHERE manifest_id=?", (manifest_id,)
    ).fetchone()[0]
    sources = conn.execute(
        "SELECT count(*) FROM d4s5_manifest_source_seals WHERE manifest_id=?", (manifest_id,)
    ).fetchone()[0]
    assert roots == result["root_count"]
    assert sources == result["source_evidence_count"]
    return result


def stage_and_publish(conn: sqlite3.Connection, s4_state: dict[str, bool],
                      s5_state: dict[str, Any], manifest_id: str,
                      roots: list[tuple[str, str]], sources: list[dict[str, str]],
                      parent: str | None = None) -> dict[str, Any]:
    S3.stage_manifest(conn, manifest_id, roots,
                      [S3.projection(source) for source in sources], parent)
    return publish(conn, s4_state, s5_state, manifest_id)


def add_named_source(conn: sqlite3.Connection, suffix: str, path: str, name: str,
                     claimed: bool = False) -> dict[str, str]:
    source = S4.add_source(conn, suffix, path, claimed=claimed)
    conn.execute("UPDATE files SET name=? WHERE id=?", (name, source["file_id"]))
    source["name"] = name
    return source


def source_ids(conn: sqlite3.Connection) -> list[str]:
    return [r[0] for r in conn.execute(
        "SELECT DISTINCT source_id FROM d4s5_effective_file_sources ORDER BY source_id"
    )]


def root_members(conn: sqlite3.Connection) -> list[tuple[str, str, str]]:
    return [tuple(r) for r in conn.execute(
        "SELECT root_id,source_id,path_entry_id FROM d4s5_effective_root_members "
        "ORDER BY root_id,source_id,path_entry_id"
    )]


def search(conn: sqlite3.Connection, expression: str) -> list[tuple[str, str, str]]:
    return [tuple(r) for r in conn.execute(
        """SELECT DISTINCT e.source_id,f.name,f.path
           FROM files_fts
           JOIN files f ON f.rowid=files_fts.rowid
           JOIN d4s5_effective_file_sources e ON e.file_id=f.id
           WHERE files_fts MATCH ? ORDER BY e.source_id""", (expression,),
    )]


def cursor_token(conn: sqlite3.Connection) -> dict[str, Any]:
    ptr = S4.pointer(conn)
    scope = [tuple(r) for r in conn.execute(
        """SELECT DISTINCT root_id,generation_id,source_id,path_entry_id,
                         root_authority_revision,source_authority_revision
           FROM d4s5_effective_root_members ORDER BY root_id,generation_id,
                         source_id,path_entry_id"""
    )]
    payload = {
        "manifest_id": ptr[0] if ptr else None,
        "pointer_revision": ptr[1] if ptr else None,
        "scoped_root_seal_sha256": digest_rows(scope),
    }
    payload["cursor_sha256"] = sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode())
    return payload


def cursor_matches(conn: sqlite3.Connection, cursor: dict[str, Any]) -> bool:
    return cursor_token(conn) == cursor


def scoped_mutation(conn: sqlite3.Connection, s4_state: dict[str, bool],
                    s5_state: dict[str, Any], event_id: str, sql: str,
                    args: tuple[Any, ...] = ()) -> None:
    """One root/source evidence event with CAS and explicit rollback behavior."""
    conn.execute("BEGIN IMMEDIATE")
    s4_state["evidence"] = True
    s5_state["evidence"] = True
    s5_state["event_id"] = event_id
    try:
        cur = conn.execute(sql, args)
        if cur.rowcount != 1:
            raise RuntimeError("scoped authority CAS matched no row")
        conn.execute("COMMIT")
    except Exception:
        if conn.in_transaction:
            conn.execute("ROLLBACK")
        raise
    finally:
        s4_state["evidence"] = False
        s5_state["evidence"] = False
        s5_state["event_id"] = None


def setup_two_roots(overlap: bool = False) -> tuple[sqlite3.Connection, dict[str, bool], dict[str, Any], list[dict[str, str]]]:
    conn, s4_state, s5_state = fresh_db()
    if overlap:
        source = add_named_source(conn, "shared", "/shared/nested/overlap-marker.txt", "overlapmarker.txt")
        sources = [source]
        S4.add_generation(conn, "root-a", "/shared", "gen-a", "run-a", 1, [source])
        S4.add_generation(conn, "root-b", "/shared/nested", "gen-b", "run-b", 1, [source])
        stage_and_publish(conn, s4_state, s5_state, "manifest-overlap",
                          [("root-a", "gen-a"), ("root-b", "gen-b")], [source])
    else:
        left = add_named_source(conn, "a", "/root-a/secret-marker.txt", "secretmarker.txt")
        right = add_named_source(conn, "b", "/root-b/visible-marker.txt", "visiblemarker.txt")
        sources = [left, right]
        S4.add_generation(conn, "root-a", "/root-a", "gen-a", "run-a", 1, [left])
        S4.add_generation(conn, "root-b", "/root-b", "gen-b", "run-b", 1, [right])
        stage_and_publish(conn, s4_state, s5_state, "manifest-two",
                          [("root-a", "gen-a"), ("root-b", "gen-b")], sources)
    return conn, s4_state, s5_state, sources


def change_root_watcher(conn: sqlite3.Connection, s4_state: dict[str, bool],
                        s5_state: dict[str, Any], root_id: str, event_id: str) -> None:
    scoped_mutation(
        conn, s4_state, s5_state, event_id,
        "UPDATE scan_roots SET watcher_revision=watcher_revision+1,revision=revision+1 WHERE id=?",
        (root_id,),
    )


def test_global_clock_false_hide_and_scoped_root_recovery() -> dict[str, Any]:
    conn, s4_state, s5_state, sources = setup_two_roots()
    before_pointer = S4.pointer(conn)
    before_global = S4.authority_revision(conn)
    before_cursor = cursor_token(conn)
    assert source_ids(conn) == ["source-a", "source-b"]
    assert S4.current_sources(conn) == ["source-a", "source-b"]

    change_root_watcher(conn, s4_state, s5_state, "root-a", "watch-root-a-1")

    legacy_global_current = S4.current_sources(conn)
    old_union_projection = S3.D4S2.visible_source_ids(conn)
    new_current = source_ids(conn)
    roots = [r[0] for r in conn.execute(
        "SELECT DISTINCT root_id FROM d4s5_effective_root_members ORDER BY root_id"
    )]
    result = {
        "global_clock_before_after": [before_global, S4.authority_revision(conn)],
        "pointer_unchanged": S4.pointer(conn) == before_pointer,
        "legacy_global_clock_view_after_root_a_watcher_change": legacy_global_current,
        "old_union_projection_after_root_a_watcher_change": old_union_projection,
        "scoped_current_sources": new_current,
        "scoped_current_roots": roots,
        "projection_rows_without_authority": [tuple(r) for r in conn.execute(
            "SELECT source_id,path_entry_id FROM d4s5_unqualified_projection_rows ORDER BY source_id"
        )],
        "search_invalid_root_a": search(conn, "secretmarker"),
        "search_unaffected_root_b": search(conn, "visiblemarker"),
        "old_cursor_rejected": not cursor_matches(conn, before_cursor),
        "fresh_cursor_matches": cursor_matches(conn, cursor_token(conn)),
        "scoped_root_revision_event": [tuple(r) for r in conn.execute(
            "SELECT event_id,domain,object_id,revision_before,revision_after "
            "FROM d4s5_authority_events ORDER BY event_id"
        )],
    }
    assert result["legacy_global_clock_view_after_root_a_watcher_change"] == []
    assert result["old_union_projection_after_root_a_watcher_change"] == ["source-a", "source-b"]
    assert result["scoped_current_sources"] == ["source-b"]
    assert result["scoped_current_roots"] == ["root-b"]
    assert result["projection_rows_without_authority"] == [("source-a", "entry-a-1")]
    assert result["search_invalid_root_a"] == []
    assert [row[0] for row in result["search_unaffected_root_b"]] == ["source-b"]
    assert result["old_cursor_rejected"] and result["pointer_unchanged"]
    conn.close()
    return result


def test_independent_source_invalidation_is_local() -> dict[str, Any]:
    conn, s4_state, s5_state, _ = setup_two_roots()
    before_cursor = cursor_token(conn)
    before_global = S4.authority_revision(conn)
    scoped_mutation(
        conn, s4_state, s5_state, "identity-source-a-1",
        "UPDATE file_sources SET status='conflicted',source_revision=source_revision+1 "
        "WHERE source_id=? AND source_revision=?", ("source-a", 2),
    )
    result = {
        "global_clock_view": S4.current_sources(conn),
        "scoped_sources": source_ids(conn),
        "search_b": search(conn, "visiblemarker"),
        "search_a": search(conn, "secretmarker"),
        "old_cursor_rejected": not cursor_matches(conn, before_cursor),
        "source_authority_revisions": [tuple(r) for r in conn.execute(
            "SELECT source_id,revision FROM d4s5_source_authority ORDER BY source_id"
        )],
        "global_revision_delta": S4.authority_revision(conn) - before_global,
        "pointer_unchanged": S4.pointer(conn) == ("manifest-two", 1),
    }
    assert result["global_clock_view"] == []
    assert result["scoped_sources"] == ["source-b"]
    assert [r[0] for r in result["search_b"]] == ["source-b"] and result["search_a"] == []
    assert result["old_cursor_rejected"] and result["pointer_unchanged"]
    assert result["source_authority_revisions"] == [("source-a", 2), ("source-b", 1)]
    conn.close()
    return result


def test_shared_source_identity_invalidation_fails_closed() -> dict[str, Any]:
    conn, s4_state, s5_state = fresh_db()
    shared = add_named_source(conn, "same", "/shared/nested/shared-marker.txt", "sharedmarker.txt")
    companion = add_named_source(conn, "companion", "/shared/nested/companion-marker.txt", "companionmarker.txt")
    S4.add_generation(conn, "root-a", "/shared", "gen-a", "run-a", 1, [shared])
    S4.add_generation(conn, "root-b", "/shared/nested", "gen-b", "run-b", 1, [shared, companion])
    stage_and_publish(conn, s4_state, s5_state, "manifest-shared-identity",
                      [("root-a", "gen-a"), ("root-b", "gen-b")], [shared, companion])
    before_members = root_members(conn)
    before_cursor = cursor_token(conn)
    assert source_ids(conn) == ["source-companion", "source-same"]

    scoped_mutation(
        conn, s4_state, s5_state, "identity-shared-source",
        "UPDATE file_sources SET status='conflicted',source_revision=source_revision+1 "
        "WHERE source_id=? AND source_revision=?", (shared["source_id"], 2),
    )
    result = {
        "shared_source_before": shared["source_id"],
        "root_members_before": before_members,
        "root_members_after_shared_source_identity_invalid": root_members(conn),
        "current_sources": source_ids(conn),
        "search_shared_file": search(conn, "sharedmarker"),
        "search_independent_root_b_file": search(conn, "companionmarker"),
        "unqualified_projection_rows": [tuple(r) for r in conn.execute(
            "SELECT source_id,path_entry_id FROM d4s5_unqualified_projection_rows ORDER BY source_id"
        )],
        "old_cursor_rejected": not cursor_matches(conn, before_cursor),
        "shared_source_revision": conn.execute(
            "SELECT revision FROM d4s5_source_authority WHERE source_id=?", (shared["source_id"],)
        ).fetchone()[0],
    }
    assert result["current_sources"] == ["source-companion"]
    assert result["root_members_after_shared_source_identity_invalid"] == [
        ("root-b", "source-companion", companion["path_entry_id"])
    ]
    assert result["search_shared_file"] == []
    assert [r[0] for r in result["search_independent_root_b_file"]] == ["source-companion"]
    assert result["old_cursor_rejected"] and result["shared_source_revision"] == 2
    conn.close()
    return result


def attach_user_metadata(conn: sqlite3.Connection, file_id: str, suffix: str) -> None:
    conn.execute("UPDATE files SET context=? WHERE id=?", (f"owner-note-{suffix}", file_id))
    conn.execute(
        "INSERT INTO user_tags(id,display_name,normalized_name,created_at,updated_at) VALUES(?,?,?,?,?)",
        (f"tag-{suffix}", f"Owner {suffix}", f"owner-{suffix}", NOW, NOW),
    )
    conn.execute(
        "INSERT INTO file_user_tags(file_id,tag_id,created_at) VALUES(?,?,?)",
        (file_id, f"tag-{suffix}", NOW),
    )


def test_overlapping_root_authority_and_last_membership() -> dict[str, Any]:
    conn, s4_state, s5_state, sources = setup_two_roots(overlap=True)
    shared = sources[0]
    attach_user_metadata(conn, shared["file_id"], "shared")
    before = root_members(conn)
    cursor_before = cursor_token(conn)
    assert [r[0] for r in before] == ["root-a", "root-b"]

    change_root_watcher(conn, s4_state, s5_state, "root-a", "watch-overlap-a")
    after_a = root_members(conn)
    legacy_after_a = S4.current_sources(conn)
    root_revision_pairs_after_a = [tuple(r) for r in conn.execute(
        "SELECT root_id,root_authority_revision,current_root_authority_revision "
        "FROM d4s5_effective_root_members ORDER BY root_id"
    )]
    search_after_a = search(conn, "overlapmarker")
    assert [r[0] for r in after_a] == ["root-b"]
    assert source_ids(conn) == [shared["source_id"]]
    assert len(search_after_a) == 1
    assert not cursor_matches(conn, cursor_before)

    # B is the last trusted membership. Its independent watcher state is part
    # of the same source/path/physical evidence; losing B must hide the source.
    change_root_watcher(conn, s4_state, s5_state, "root-b", "watch-overlap-b")
    after_b = source_ids(conn)
    assert after_b == [] and search(conn, "overlapmarker") == []

    # Publish complete replacements: A has no member; B carries a different
    # source. The removed source remains in its old Manifest and keeps metadata.
    scoped_mutation(
        conn, s4_state, s5_state, "watch-root-a-applied",
        "UPDATE scan_roots SET watcher_applied_revision=watcher_revision,revision=revision+1 "
        "WHERE id='root-a'",
    )
    scoped_mutation(
        conn, s4_state, s5_state, "watch-root-b-applied",
        "UPDATE scan_roots SET watcher_applied_revision=watcher_revision,revision=revision+1 "
        "WHERE id='root-b'",
    )
    other = add_named_source(conn, "other-b", "/shared/nested/other-marker.txt", "othermarker.txt")
    S4.add_generation(conn, "root-a", "/shared", "gen-a2", "run-a2", 2, [])
    S4.add_generation(conn, "root-b", "/shared/nested", "gen-b2", "run-b2", 2, [other])
    stage_and_publish(conn, s4_state, s5_state, "manifest-overlap-next",
                      [("root-a", "gen-a2"), ("root-b", "gen-b2")], [other], "manifest-overlap")
    result = {
        "shared_source_id": shared["source_id"],
        "members_before": before,
        "members_after_root_a_watcher_change": after_a,
        "legacy_global_clock_view_after_root_a_change": legacy_after_a,
        "root_revision_pairs_after_root_a_change": root_revision_pairs_after_a,
        "members_after_root_b_watcher_change": after_b,
        "search_after_a": search_after_a,
        "search_after_b": [],
        "successor_manifest_current_sources": source_ids(conn),
        "old_manifest_retained": conn.execute(
            "SELECT status FROM visibility_manifests WHERE manifest_id='manifest-overlap'"
        ).fetchone()[0] == "published",
        "shared_source_hidden_after_last_root_generation_drops_it": shared["source_id"] not in source_ids(conn),
        "shared_file_user_context_retained": conn.execute(
            "SELECT context FROM files WHERE id=?", (shared["file_id"],)
        ).fetchone()[0],
        "shared_file_tag_count_retained": conn.execute(
            "SELECT count(*) FROM file_user_tags WHERE file_id=?", (shared["file_id"],)
        ).fetchone()[0],
        "shared_path_and_physical_evidence_still_match": conn.execute(
            """SELECT pe.identity_status='verified'
                     AND pi.evidence_digest=se.physical_evidence_digest
                     AND pe.evidence_digest=se.path_evidence_digest
               FROM file_path_entries pe JOIN file_physical_identity_epochs pi
                 ON pi.physical_identity_id=pe.physical_identity_id
               JOIN d4s4_manifest_source_evidence se
                 ON se.physical_identity_id=pi.physical_identity_id
                AND se.path_entry_id=pe.path_entry_id
               WHERE se.manifest_id='manifest-overlap' AND pe.path_entry_id=?""",
            (shared["path_entry_id"],),
        ).fetchone()[0],
    }
    assert result["successor_manifest_current_sources"] == ["source-other-b"]
    assert result["legacy_global_clock_view_after_root_a_change"] == []
    assert result["root_revision_pairs_after_root_a_change"] == [("root-b", 1, 1)]
    assert result["old_manifest_retained"] and result["shared_source_hidden_after_last_root_generation_drops_it"]
    assert result["shared_file_user_context_retained"] == "owner-note-shared"
    assert result["shared_file_tag_count_retained"] == 1
    assert result["shared_path_and_physical_evidence_still_match"] == 1
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    conn.close()
    return result


def test_path_reuse_does_not_inherit_metadata() -> dict[str, Any]:
    conn, s4_state, s5_state = fresh_db()
    old = add_named_source(conn, "old", "/reuse/item.txt", "oldmarker.txt")
    S4.add_generation(conn, "root-a", "/reuse", "gen-old", "run-old", 1, [old])
    stage_and_publish(conn, s4_state, s5_state, "manifest-old", [("root-a", "gen-old")], [old])
    attach_user_metadata(conn, old["file_id"], "old")
    old_path = conn.execute("SELECT path FROM files WHERE id=?", (old["file_id"],)).fetchone()[0]

    new = add_named_source(conn, "new", "/reuse/item.txt", "newmarker.txt")
    S4.add_generation(conn, "root-a", "/reuse", "gen-new", "run-new", 2, [new])
    stage_and_publish(conn, s4_state, s5_state, "manifest-new", [("root-a", "gen-new")], [new], "manifest-old")

    result = {
        "old_source_file": [old["source_id"], old["file_id"]],
        "new_source_file": [new["source_id"], new["file_id"]],
        "same_path": old_path == conn.execute("SELECT path FROM files WHERE id=?", (new["file_id"],)).fetchone()[0],
        "current_sources": source_ids(conn),
        "old_name_fts_results": search(conn, "oldmarker"),
        "new_name_fts_results": search(conn, "newmarker"),
        "old_context": conn.execute("SELECT context FROM files WHERE id=?", (old["file_id"],)).fetchone()[0],
        "new_context": conn.execute("SELECT context FROM files WHERE id=?", (new["file_id"],)).fetchone()[0],
        "old_tag_count": conn.execute("SELECT count(*) FROM file_user_tags WHERE file_id=?", (old["file_id"],)).fetchone()[0],
        "new_tag_count": conn.execute("SELECT count(*) FROM file_user_tags WHERE file_id=?", (new["file_id"],)).fetchone()[0],
        "old_history_retained": conn.execute(
            "SELECT count(*) FROM source_generation_members WHERE source_id=?", (old["source_id"],)
        ).fetchone()[0],
        "old_manifest_retained": conn.execute(
            "SELECT status FROM visibility_manifests WHERE manifest_id='manifest-old'"
        ).fetchone()[0] == "published",
    }
    assert old["file_id"] != new["file_id"] and old["source_id"] != new["source_id"]
    assert result["same_path"] and result["current_sources"] == [new["source_id"]]
    assert result["old_name_fts_results"] == [] and [r[0] for r in result["new_name_fts_results"]] == [new["source_id"]]
    assert result["old_context"] == "owner-note-old" and result["new_context"] in (None, "")
    assert result["old_tag_count"] == 1 and result["new_tag_count"] == 0
    assert result["old_history_retained"] == 1 and result["old_manifest_retained"]
    conn.close()
    return result


def test_old_autocommit_revoke_failure() -> dict[str, Any]:
    conn, s4_state, claimed = S4.setup_published(s4=True, claimed=True)
    before = {
        "source_revision": conn.execute(
            "SELECT source_revision FROM file_sources WHERE source_id=?", (claimed["source_id"],)
        ).fetchone()[0],
        "global_authority_revision": S4.authority_revision(conn),
        "pointer": S4.pointer(conn),
    }
    s4_state["evidence"] = True
    try:
        conn.execute(
            "UPDATE file_sources SET source_revision=source_revision+1 WHERE source_id=?",
            (claimed["source_id"],),
        )
    finally:
        s4_state["evidence"] = False
    # D4-S4's connection is isolation_level=None. Inject a process failure
    # before the separate append-only revoke statement.
    after_update = conn.execute(
        "SELECT source_revision FROM file_sources WHERE source_id=?", (claimed["source_id"],)
    ).fetchone()[0]
    revoke_count = conn.execute(
        "SELECT count(*) FROM d4s4_claim_revocations WHERE claim_id=?", (claimed["claim_id"],)
    ).fetchone()[0]
    result = {
        "isolation_level": conn.isolation_level,
        "source_revision_before_after_failed_two_statement_event": [before["source_revision"], after_update],
        "revocation_rows_after_injected_failure": revoke_count,
        "claim_row_retained": conn.execute(
            "SELECT count(*) FROM source_identity_claims WHERE claim_id=?", (claimed["claim_id"],)
        ).fetchone()[0] == 1,
        "authority_clock_delta": S4.authority_revision(conn) - before["global_authority_revision"],
        "global_current_view_after_half_event": S4.current_sources(conn),
        "pointer_unchanged": S4.pointer(conn) == before["pointer"],
    }
    assert after_update == before["source_revision"] + 1
    assert revoke_count == 0 and result["claim_row_retained"]
    assert result["authority_clock_delta"] == 1 and result["global_current_view_after_half_event"] == []
    assert result["pointer_unchanged"]
    conn.close()
    return result


def atomic_revoke(conn: sqlite3.Connection, s4_state: dict[str, bool],
                  s5_state: dict[str, Any], source: dict[str, str], claim_id: str,
                  failure: str | None = None, before_commit=None) -> None:
    conn.execute("BEGIN IMMEDIATE")
    event_id = f"claim-revoke-{source['source_id']}"
    s4_state["evidence"] = True
    s5_state["evidence"] = True
    s5_state["event_id"] = event_id
    try:
        if failure == "after_begin":
            raise InjectedFailure(failure)
        cur = conn.execute(
            """UPDATE file_sources
               SET identity_confidence='path_only',source_revision=source_revision+1
               WHERE source_id=? AND source_revision=?""",
            (source["source_id"], 3),
        )
        if cur.rowcount != 1:
            raise RuntimeError("source revision CAS failed")
        if failure == "after_source_update":
            raise InjectedFailure(failure)
        conn.execute(
            "INSERT INTO d4s4_claim_revocations(claim_id,revoked_at,reason) VALUES(?,?,?)",
            (claim_id, NOW + 1, "owner withdrew legacy identity claim"),
        )
        if failure == "after_revoke_insert":
            raise InjectedFailure(failure)
        row = conn.execute(
            """SELECT e.revision_before,e.revision_after,e.source_revision_after,
                         sa.revision,fs.source_revision
               FROM d4s5_authority_events e JOIN d4s5_source_authority sa
                 ON sa.source_id=e.object_id
               JOIN file_sources fs ON fs.source_id=e.object_id
               WHERE e.event_id=?""", (event_id,),
        ).fetchone()
        assert row is not None and row[1] == row[0] + 1 and row[2] == row[4] and row[3] == row[1]
        if before_commit is not None:
            before_commit()
        if failure == "before_commit":
            raise InjectedFailure(failure)
        if failure == "commit_failure":
            conn.execute(
                "INSERT INTO d4s5_commit_probe(id,parent_id) VALUES('orphan','missing')"
            )
        conn.execute("COMMIT")
    except Exception:
        if conn.in_transaction:
            conn.execute("ROLLBACK")
        raise
    finally:
        s4_state["evidence"] = False
        s5_state["evidence"] = False
        s5_state["event_id"] = None


def claimed_fixture(path: str | Path = ":memory:") -> tuple[sqlite3.Connection, dict[str, bool], dict[str, Any], dict[str, str]]:
    conn, s4_state, s5_state = fresh_db(path)
    source = add_named_source(conn, "claim", "/claimed/legacy.txt", "claimedmarker.txt", claimed=True)
    S4.add_generation(conn, "root-a", "/claimed", "gen-claim", "run-claim", 1, [source])
    stage_and_publish(conn, s4_state, s5_state, "manifest-claim", [("root-a", "gen-claim")], [source])
    attach_user_metadata(conn, source["file_id"], "claim")
    conn.execute("CREATE TABLE d4s5_commit_parent(id TEXT PRIMARY KEY)")
    conn.execute(
        """CREATE TABLE d4s5_commit_probe(
             id TEXT PRIMARY KEY,parent_id TEXT REFERENCES d4s5_commit_parent(id)
             DEFERRABLE INITIALLY DEFERRED)"""
    )
    return conn, s4_state, s5_state, source


def test_revoke_failure_atomicity() -> dict[str, Any]:
    failures: list[dict[str, Any]] = []
    for failure in ("after_begin", "after_source_update", "after_revoke_insert", "before_commit", "commit_failure"):
        conn, s4_state, s5_state, source = claimed_fixture()
        before = {
            "source_revision": conn.execute(
                "SELECT source_revision FROM file_sources WHERE source_id=?", (source["source_id"],)
            ).fetchone()[0],
            "identity_confidence": conn.execute(
                "SELECT identity_confidence FROM file_sources WHERE source_id=?", (source["source_id"],)
            ).fetchone()[0],
            "authority_revision": conn.execute(
                "SELECT revision FROM d4s5_source_authority WHERE source_id=?", (source["source_id"],)
            ).fetchone()[0],
            "event_count": conn.execute("SELECT count(*) FROM d4s5_authority_events").fetchone()[0],
            "revocation_count": conn.execute("SELECT count(*) FROM d4s4_claim_revocations").fetchone()[0],
            "legacy_global_clock": S4.authority_revision(conn),
            "claim_row_count": conn.execute("SELECT count(*) FROM source_identity_claims").fetchone()[0],
            "user_tag_count": conn.execute(
                "SELECT count(*) FROM file_user_tags WHERE file_id=?", (source["file_id"],)
            ).fetchone()[0],
            "user_context": conn.execute(
                "SELECT context FROM files WHERE id=?", (source["file_id"],)
            ).fetchone()[0],
            "pointer": S4.pointer(conn),
            "visible": source_ids(conn),
        }
        error = None
        try:
            atomic_revoke(conn, s4_state, s5_state, source, source["claim_id"], failure)
        except (InjectedFailure, sqlite3.IntegrityError) as exc:
            error = str(exc)
        else:
            raise AssertionError(f"injected failure did not fire: {failure}")
        after = {
            "source_revision": conn.execute(
                "SELECT source_revision FROM file_sources WHERE source_id=?", (source["source_id"],)
            ).fetchone()[0],
            "identity_confidence": conn.execute(
                "SELECT identity_confidence FROM file_sources WHERE source_id=?", (source["source_id"],)
            ).fetchone()[0],
            "authority_revision": conn.execute(
                "SELECT revision FROM d4s5_source_authority WHERE source_id=?", (source["source_id"],)
            ).fetchone()[0],
            "event_count": conn.execute("SELECT count(*) FROM d4s5_authority_events").fetchone()[0],
            "revocation_count": conn.execute("SELECT count(*) FROM d4s4_claim_revocations").fetchone()[0],
            "legacy_global_clock": S4.authority_revision(conn),
            "claim_row_count": conn.execute("SELECT count(*) FROM source_identity_claims").fetchone()[0],
            "user_tag_count": conn.execute(
                "SELECT count(*) FROM file_user_tags WHERE file_id=?", (source["file_id"],)
            ).fetchone()[0],
            "user_context": conn.execute(
                "SELECT context FROM files WHERE id=?", (source["file_id"],)
            ).fetchone()[0],
            "pointer": S4.pointer(conn),
            "visible": source_ids(conn),
        }
        assert after == before, (failure, before, after, error)
        failures.append({"injected_at": failure, "rolled_back_completely": True, "error": error})
        conn.close()
    return {"failure_points": failures, "all_state_equal_to_pretransaction": True}


def test_revoke_reader_commit_reconnect() -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="zen-d4s5-atomic-") as temp_dir:
        path = Path(temp_dir) / "source-authority.sqlite"
        writer, s4_state, s5_state, source = claimed_fixture(path)
        writer.execute("PRAGMA journal_mode=WAL")
        reader = S3.D4S2.open_db(path)
        reader.execute("BEGIN")
        visible_before = source_ids(reader)
        writer_before = source_ids(writer)
        legacy_clock_before = S4.authority_revision(writer)
        assert visible_before == writer_before == [source["source_id"]]

        in_transaction_reader = []
        atomic_revoke(
            writer, s4_state, s5_state, source, source["claim_id"],
            before_commit=lambda: in_transaction_reader.extend(source_ids(reader)),
        )
        old_snapshot_after_commit = source_ids(reader)
        reader.execute("COMMIT")
        fresh_reader_after_commit = source_ids(reader)
        durable = {
            "source_revision": writer.execute(
                "SELECT source_revision FROM file_sources WHERE source_id=?", (source["source_id"],)
            ).fetchone()[0],
            "legacy_global_clock_delta": S4.authority_revision(writer) - legacy_clock_before,
            "authority_revision": writer.execute(
                "SELECT revision FROM d4s5_source_authority WHERE source_id=?", (source["source_id"],)
            ).fetchone()[0],
            "event_rows": [tuple(r) for r in writer.execute(
                "SELECT event_id,domain,object_id,revision_before,revision_after,source_revision_after "
                "FROM d4s5_authority_events"
            )],
            "revocation_count": writer.execute(
                "SELECT count(*) FROM d4s4_claim_revocations WHERE claim_id=?", (source["claim_id"],)
            ).fetchone()[0],
            "claim_retained": writer.execute(
                "SELECT count(*) FROM source_identity_claims WHERE claim_id=?", (source["claim_id"],)
            ).fetchone()[0],
            "user_context_retained": writer.execute(
                "SELECT context FROM files WHERE id=?", (source["file_id"],)
            ).fetchone()[0],
            "user_tag_retained": writer.execute(
                "SELECT count(*) FROM file_user_tags WHERE file_id=?", (source["file_id"],)
            ).fetchone()[0],
            "pointer_unchanged": S4.pointer(writer) == ("manifest-claim", 1),
            "manifest_seal_retained": writer.execute(
                "SELECT count(*) FROM d4s5_manifest_source_seals WHERE manifest_id='manifest-claim'"
            ).fetchone()[0] == 1,
        }
        read_check_after_close = source_ids(reader)
        reader.close()
        writer.close()

        reopened = S3.D4S2.open_db(path)
        after_reopen = source_ids(reopened)
        reopened_event = [tuple(r) for r in reopened.execute(
            "SELECT event_id,revision_before,revision_after,source_revision_after FROM d4s5_authority_events"
        )]
        result = {
            "independent_reader_before_commit": visible_before,
            "independent_reader_during_uncommitted_writer": in_transaction_reader,
            "same_reader_snapshot_after_writer_commit": old_snapshot_after_commit,
            "reader_after_snapshot_restart": fresh_reader_after_commit,
            "reader_before_close_after_restart": read_check_after_close,
            "reopened_current_sources": after_reopen,
            "durable_event_state": durable,
            "event_after_reopen": reopened_event,
            "source_authority_revision_advanced_once": durable["authority_revision"] == 2,
            "source_revision_advanced_once": durable["source_revision"] == 4,
            "foreign_key_check": reopened.execute("PRAGMA foreign_key_check").fetchall(),
            "integrity_check": reopened.execute("PRAGMA integrity_check").fetchone()[0],
        }
        assert result["same_reader_snapshot_after_writer_commit"] == [source["source_id"]]
        assert result["independent_reader_during_uncommitted_writer"] == [source["source_id"]]
        assert result["reader_after_snapshot_restart"] == []
        assert result["reopened_current_sources"] == []
        assert result["source_authority_revision_advanced_once"] and result["source_revision_advanced_once"]
        assert durable["revocation_count"] == 1 and durable["claim_retained"] == 1
        assert durable["legacy_global_clock_delta"] == 2
        assert durable["user_context_retained"] == "owner-note-claim" and durable["user_tag_retained"] == 1
        assert durable["pointer_unchanged"] and durable["manifest_seal_retained"]
        assert result["foreign_key_check"] == [] and result["integrity_check"] == "ok"
        reopened.close()
        return result


def run() -> dict[str, Any]:
    # Accepted S2/S3/S4 contracts are re-executed by the unchanged D4-S4 runner.
    preserved = S4.run()
    assert preserved["preserved_d4s3"]["d4s2_assertion_groups"] == 15
    assert preserved["preserved_d4s3"]["d4s3_failure_counterexamples"] == 7
    assert preserved["preserved_d4s3"]["d4s3_invalid_publication_cases"] == 4
    assert preserved["preserved_d4s3"]["d4s3_blocked_dml_mutations"] == 16

    schema_probe, _, _ = fresh_db()
    schema_user_version = schema_probe.execute("PRAGMA user_version").fetchone()[0]
    assert schema_user_version == 37
    schema_probe.close()

    old_autocommit = test_old_autocommit_revoke_failure()
    scoped_root = test_global_clock_false_hide_and_scoped_root_recovery()
    scoped_source = test_independent_source_invalidation_is_local()
    overlapping_identity = test_shared_source_identity_invalidation_fails_closed()
    overlapping = test_overlapping_root_authority_and_last_membership()
    path_reuse = test_path_reuse_does_not_inherit_metadata()
    atomic_failures = test_revoke_failure_atomicity()
    atomic_durability = test_revoke_reader_commit_reconnect()

    return {
        "research_scope": "D4-S5 isolated Schema 37-derived SQL/Python Harness only; no production DDL, code, IPC, or migration version changed",
        "runtime": {
            "sqlite_version": sqlite3.sqlite_version,
            "sqlite_source_id": sqlite3.connect(":memory:").execute("SELECT sqlite_source_id()").fetchone()[0],
            "schema_user_version": schema_user_version,
            "connection_isolation_level": "None / autocommit; all evidence mutation is explicitly BEGIN IMMEDIATE",
            "fts5_available": True,
        },
        "inputs": {
            "schema37_ddl_sha256": S3.sha256(S3.SCHEMA_PATH.read_bytes()),
            "d4s2_candidate_sql_sha256": S3.sha256((S3.D4S2_DIR / "harness/physical-candidate.sql").read_bytes()),
            "d4s2_runner_sha256": S3.sha256(S3.D4S2_RUNNER.read_bytes()),
            "d4s3_runner_sha256": sha256((ROOT / "docs/project/research/issue-381-d4-s3/harness/run.py").read_bytes()),
            "d4s4_runner_sha256": sha256(S4_RUNNER.read_bytes()),
            "d4s4_sql_sha256": sha256((S4_DIR / "harness/transitive-evidence-protection.sql").read_bytes()),
            "d4s4_original_log_sha256": "674d6a82e1b123e3f5976e1c922df347cd897a0d01d4005f2622550ab79a2782",
            "d4s4_archived_log_sha256": sha256((EVIDENCE / "archive/d4-s4-transitive-evidence-run.log").read_bytes()),
            "d4s5_sql_sha256": sha256(S5_SQL.read_bytes()),
            "d4s5_runner_sha256": sha256(Path(__file__).read_bytes()),
        },
        "preserved_contracts": {
            "d4s2_assertion_groups": 15,
            "d4s3_failure_counterexamples": 7,
            "d4s3_invalid_publication_cases": 4,
            "d4s3_dml_guards": 16,
            "d4s4_transitive_mutations_blocked": 11,
            "d4s4_digest_bypass_cases": 2,
            "nested_roots_rename_cursor_fts_rollback": "unchanged D4-S4 runner executed and passed",
        },
        "old_autocommit_revoke_failure": old_autocommit,
        "root_scoped_false_hide_regression": scoped_root,
        "independent_source_identity_scope": scoped_source,
        "overlapping_shared_source_identity": overlapping_identity,
        "overlapping_root_and_last_membership": overlapping,
        "same_path_new_incarnation_isolation": path_reuse,
        "claim_revoke_failure_injection": atomic_failures,
        "claim_revoke_reader_commit_reconnect": atomic_durability,
        "limits": [
            "This is a Python sqlite3 / Schema 37-derived synthetic harness. It proves relational scoping and SQLite transaction behavior, not native filesystem identity.",
            "The shared file in overlapping roots uses one source_id and one path/physical identity seal. A source identity invalidation therefore invalidates every root membership for that same source; an unrelated source in root B remains independent.",
            "A read transaction that began before a commit continues to see its internally consistent SQLite snapshot. Cross-transaction cursors must carry pointer and scoped-root seal digests and be rejected after those values change.",
            "D4-S4's global authority_clock still advances per underlying DML row change (a source update plus claim-revocation insert advances it twice). D4-S5 does not use that counter for current visibility; its unique event_id advances one source-scoped revision once per logical revoke.",
            "The connection-local SQLite UDF capability is only a test of the trusted service boundary. A process that can register arbitrary functions, drop triggers, or edit the DB file remains outside this Harness proof.",
            "SQLite 3.51.3 broad FTS/BM25, Windows/macOS native identity, GC and disk-space recovery, and #380 database-generation/client isolation remain separate gates.",
        ],
    }


def main() -> int:
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    result = run()
    encoded = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    (EVIDENCE / "scoped-authority-results.json").write_text(encoded, encoding="utf-8")
    log_lines = [
        "#381-D4-S5 root-scoped authority research Harness",
        f"sqlite_version={result['runtime']['sqlite_version']}",
        f"sqlite_source_id={result['runtime']['sqlite_source_id']}",
        "preserved_D4S2=15; D4S3_counterexamples=7; invalid_publications=4; DML_guards=16; D4S4_transitive_guards=11",
        "old_autocommit_revoke_failure=reproduced: source revision persisted, claim revocation absent, current view invalidated",
        "global_clock_root_a_watcher=reproduced: unrelated root B was hidden by the legacy global seal",
        "scoped_root_revision=root A hidden; independent root B remains visible and searchable; old cursor rejected",
        "independent_source_identity_invalidation=source A hidden; independent source B remains visible and searchable",
        "overlapping_shared_source_identity_invalidation=shared source removed from A and B; independent B companion remains visible",
        "overlapping_roots=shared source remains visible through B only while B root/source/path/physical evidence remains trusted",
        "last_valid_membership_removed=source hidden by complete successor generations; history, user context, and tag retained",
        "same_path_reuse=old and new file/source IDs isolated; old tags/context retained; new source inherits none",
        f"atomic_revoke_failure_points={len(result['claim_revoke_failure_injection']['failure_points'])}; all rolled back",
        "atomic_revoke=BEGIN IMMEDIATE; source revision + source authority revision advanced once; revoke appended in same commit",
        "reader_snapshot=precommit and same-transaction reads stay consistent; new transaction and reconnect observe committed revoke",
        "foreign_key_check=ok; integrity_check=ok",
        "production_schema_version=unassigned; product changes=none",
    ]
    log = "\n".join(log_lines) + "\n"
    (EVIDENCE / "scoped-authority-run.log").write_text(log, encoding="utf-8")
    print(log, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
