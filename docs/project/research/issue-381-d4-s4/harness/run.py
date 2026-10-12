#!/usr/bin/env python3
"""D4-S4 counterexample-first transitive authority harness; research only."""

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
S3_DIR = ROOT / "docs/project/research/issue-381-d4-s3"
S3_RUNNER = S3_DIR / "harness/run.py"
S3_SQL = HERE / "harness/transitive-evidence-protection.sql"
EVIDENCE = HERE / "evidence"
NOW = 1_800_000_000
sys.dont_write_bytecode = True


class AuthorityRejected(RuntimeError):
    pass


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest_rows(rows: list[tuple[Any, ...]] | list[list[Any]]) -> str:
    canonical = lambda row: json.dumps(row, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    ordered = sorted([list(row) for row in rows], key=canonical)
    payload = json.dumps(ordered, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    return sha256(payload)


def load_s3_runner():
    spec = importlib.util.spec_from_file_location("d4s3_manifest_authority_harness", S3_RUNNER)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


S3 = load_s3_runner()


def register_capabilities(conn: sqlite3.Connection) -> dict[str, bool]:
    state = {"publish": False, "evidence": False}
    conn.create_function("d4s4_publish_authorized", 0, lambda: int(state["publish"]))
    conn.create_function("d4s4_evidence_change_authorized", 0, lambda: int(state["evidence"]))
    return state


def install_s4_sql(conn: sqlite3.Connection) -> None:
    for statement in S3.D4S2.split_sql(S3_SQL.read_text(encoding="utf-8")):
        conn.execute(statement)


def fresh_db(s4: bool = True, path: str | Path = ":memory:") -> tuple[sqlite3.Connection, dict[str, bool]]:
    conn, _ = S3.fresh_db(path, protected=True)
    state = register_capabilities(conn)
    if s4:
        install_s4_sql(conn)
    return conn, state


def add_source(conn: sqlite3.Connection, suffix: str, path: str,
               claimed: bool = False) -> dict[str, str]:
    source = S3.add_source(conn, suffix, path)
    if claimed:
        conn.execute(
            "UPDATE file_sources SET identity_confidence='user_claimed',source_revision=source_revision+1 WHERE source_id=?",
            (source["source_id"],),
        )
        conn.execute(
            """INSERT INTO source_identity_claims
               (claim_id,source_id,path_entry_id,actor_id,claim_kind,authorization_digest,claimed_at)
               VALUES(?,?,?,?,?,?,?)""",
            (f"claim-{suffix}", source["source_id"], source["path_entry_id"],
             "owner-test", "explicit_legacy_claim", f"approval-{suffix}", NOW),
        )
        source["claim_id"] = f"claim-{suffix}"
    return source


def add_generation(conn: sqlite3.Connection, root_id: str, root_path: str,
                   generation_id: str, run_id: str, generation_no: int,
                   members: list[dict[str, str]]) -> None:
    S3.add_generation(conn, root_id, root_path, generation_id, run_id, generation_no, members)
    # Schema 37's `revision` is a broad operational CAS that also changes for
    # lease lifecycle. The candidate generation snapshots its exact value for
    # publication CAS; watcher coverage has a separate revision field.
    conn.execute(
        """UPDATE scan_roots SET health_status='healthy',needs_reconciliation=0
           WHERE id=?""", (root_id,),
    )
    root = conn.execute(
        "SELECT revision,watcher_revision FROM scan_roots WHERE id=?", (root_id,)
    ).fetchone()
    conn.execute(
        """UPDATE source_generations SET root_revision=?,watcher_revision=?
           WHERE generation_id=?""", (root[0], root[1], generation_id),
    )
    conn.execute(
        "UPDATE scan_runs SET watcher_revision_at_start=? WHERE id=?",
        (root[1], run_id),
    )


def root_path_contains(root: str, path: str) -> bool:
    """POSIX boundary probe for the synthetic fixture; not native path proof."""
    root_norm = root.replace("\\", "/").rstrip("/") or "/"
    path_norm = path.replace("\\", "/")
    if root_norm == "/":
        return path_norm.startswith("/")
    return path_norm == root_norm or path_norm.startswith(root_norm + "/")


def publication_preflight(conn: sqlite3.Connection, manifest_id: str) -> tuple[dict[str, Any], list[dict[str, Any]], list[dict[str, Any]]]:
    report = S3.oracle_report(conn, manifest_id)
    problems: list[str] = []
    if not report["valid"]:
        problems.append("D4-S3 Oracle rejected root/member/path/identity digest relation")

    roots: list[dict[str, Any]] = []
    for root_id, generation_id in S3.expected_roots(conn, manifest_id):
        row = conn.execute(
            """SELECT sg.root_revision,sg.watcher_revision,sg.status,
                      r.revision,r.watcher_revision,r.watcher_applied_revision,
                      r.normalized_path,r.source_kind,r.enabled,r.health_status,
                      r.needs_reconciliation,sr.status,sr.coverage_complete,
                      sr.watcher_revision_at_start
               FROM source_generations sg
               JOIN scan_roots r ON r.id=sg.root_id
               JOIN scan_runs sr ON sr.id=sg.run_id
               WHERE sg.generation_id=? AND sg.root_id=?""",
            (generation_id, root_id),
        ).fetchone()
        if row is None:
            problems.append(f"missing root/run evidence {root_id}:{generation_id}")
            continue
        item = {
            "root_id": root_id,
            "generation_id": generation_id,
            "generation_root_revision": row[0],
            "generation_watcher_revision": row[1],
            "generation_status": row[2],
            "current_root_revision": row[3],
            "current_watcher_revision": row[4],
            "watcher_applied_revision": row[5],
            "normalized_path": row[6],
            "source_kind": row[7],
            "enabled": row[8],
            "health_status": row[9],
            "needs_reconciliation": row[10],
            "run_status": row[11],
            "coverage_complete": row[12],
            "watcher_revision_at_start": row[13],
        }
        roots.append(item)
        if row[0] != row[3]:
            problems.append(f"root revision CAS mismatch {root_id}: generation={row[0]} current={row[3]}")
        if row[1] != row[4] or row[1] != row[13] or row[4] != row[5]:
            problems.append(f"watcher revision/coverage mismatch {root_id}")
        if row[2] not in ("complete", "published") or row[11] != "completed" or row[12] != 1:
            problems.append(f"run or generation is incomplete {root_id}:{generation_id}")
        if row[7] != "file_library" or row[8] != 1 or row[9] != "healthy" or row[10] != 0:
            problems.append(f"root is disabled or not fully reconciled {root_id}")

    source_evidence: list[dict[str, Any]] = []
    expected_rows = [tuple(r) for r in conn.execute(
        """SELECT DISTINCT mr.manifest_id,gm.source_id,gm.path_entry_id,
                  gm.identity_digest,pe.path_text,pe.path_key,pe.evidence_digest,
                  pe.physical_identity_id,pe.identity_status,
                  fs.file_id,fs.status,fs.identity_confidence,fs.source_revision,
                  pi.platform_kind,pi.volume_identity,pi.platform_file_identity,
                  pi.identity_epoch,pi.evidence_digest
           FROM manifest_root_heads mr
           JOIN source_generation_members gm ON gm.generation_id=mr.generation_id
           JOIN file_path_entries pe ON pe.path_entry_id=gm.path_entry_id AND pe.source_id=gm.source_id
           JOIN file_sources fs ON fs.source_id=gm.source_id
           JOIN file_physical_identity_epochs pi ON pi.physical_identity_id=pe.physical_identity_id
           WHERE mr.manifest_id=? ORDER BY gm.source_id,gm.path_entry_id""",
        (manifest_id,),
    )]
    for row in expected_rows:
        claims = [tuple(c) for c in conn.execute(
            """SELECT claim_id,actor_id,claim_kind,authorization_digest,claimed_at
               FROM source_identity_claims
               WHERE source_id=? AND path_entry_id=?
                 AND claim_kind='explicit_legacy_claim'
               ORDER BY claim_id""", (row[1], row[2]),
        )]
        active_claims = [claim for claim in claims if conn.execute(
            "SELECT 1 FROM d4s4_claim_revocations WHERE claim_id=?", (claim[0],)
        ).fetchone() is None]
        if row[11] == "user_claimed" and not active_claims:
            problems.append(f"missing or revoked explicit identity claim {row[1]}:{row[2]}")
        if not row[17] or len(row[17]) != 64:
            problems.append(f"physical evidence digest absent or malformed {row[7]}")
        selected_root_rows = conn.execute(
            """SELECT r.normalized_path FROM manifest_root_heads mr
               JOIN source_generations sg ON sg.generation_id=mr.generation_id
               JOIN scan_roots r ON r.id=mr.root_id
               JOIN source_generation_members gm ON gm.generation_id=sg.generation_id
               WHERE mr.manifest_id=? AND gm.source_id=? AND gm.path_entry_id=?""",
            (manifest_id, row[1], row[2]),
        ).fetchall()
        if any(not root_path_contains(root[0], row[4]) for root in selected_root_rows):
            problems.append(f"member path outside selected root boundary {row[1]}:{row[2]}")
        commitment = digest_rows([list(row) + [claims]])
        source_evidence.append({
            "source_id": row[1],
            "path_entry_id": row[2],
            "source_revision": row[12],
            "source_status": row[10],
            "identity_confidence": row[11],
            "physical_identity_id": row[7],
            "identity_epoch": row[16],
            "platform_file_identity": row[15],
            "physical_evidence_digest": row[17],
            "path_evidence_digest": row[6],
            "claim_ids": [claim[0] for claim in active_claims],
            "claim_ids_json": json.dumps([claim[0] for claim in active_claims], separators=(",", ":")),
            "evidence_commitment": commitment,
            "member_identity_digest": row[3],
            "path_text": row[4],
            "path_key": row[5],
            "identity_status": row[8],
        })
    if problems:
        raise AuthorityRejected("; ".join(problems))
    return report, roots, source_evidence


def trusted_publish(conn: sqlite3.Connection, state: dict[str, bool], manifest_id: str) -> dict[str, Any]:
    conn.execute("BEGIN IMMEDIATE")
    try:
        report, roots, sources = publication_preflight(conn, manifest_id)
        authority_revision = conn.execute(
            "SELECT revision FROM d4s4_authority_clock WHERE singleton_id=1"
        ).fetchone()[0]
        root_rows = [(item["root_id"], item["generation_id"]) for item in roots]
        root_digest = digest_rows(sorted(root_rows))
        source_digest_rows = [(item["source_id"], item["path_entry_id"], item["evidence_commitment"])
                              for item in sources]
        source_digest = digest_rows(source_digest_rows)
        state["publish"] = True
        try:
            conn.execute(
                """INSERT INTO d4s4_manifest_authority_seals
                   (manifest_id,authority_revision,root_count,root_map_digest,member_count,
                    member_digest,source_count,source_evidence_digest,sealed_at)
                   VALUES(?,?,?,?,?,?,?,?,?)""",
                (manifest_id, authority_revision, len(roots), root_digest,
                 report["expected_count"], report["expected_sha256"], len(sources),
                 source_digest, NOW),
            )
            for item in roots:
                conn.execute(
                    """INSERT INTO d4s4_manifest_root_evidence
                       (manifest_id,root_id,generation_id,root_revision,watcher_revision,
                        normalized_path,source_kind,enabled,health_status,needs_reconciliation)
                       VALUES(?,?,?,?,?,?,?,?,?,?)""",
                    (manifest_id, item["root_id"], item["generation_id"],
                     item["current_root_revision"], item["current_watcher_revision"],
                     item["normalized_path"], item["source_kind"], item["enabled"],
                     item["health_status"], item["needs_reconciliation"]),
                )
            for item in sources:
                conn.execute(
                    """INSERT INTO d4s4_manifest_source_evidence
                       (manifest_id,source_id,path_entry_id,source_revision,source_status,
                        identity_confidence,physical_identity_id,identity_epoch,
                        platform_file_identity,physical_evidence_digest,path_evidence_digest,
                        claim_ids_json,evidence_commitment)
                       VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)""",
                    (manifest_id, item["source_id"], item["path_entry_id"],
                     item["source_revision"], item["source_status"], item["identity_confidence"],
                     item["physical_identity_id"], item["identity_epoch"],
                     item["platform_file_identity"], item["physical_evidence_digest"],
                     item["path_evidence_digest"], item["claim_ids_json"],
                     item["evidence_commitment"]),
                )
            s3_result = S3.finalize_current_transaction(conn, manifest_id)
            conn.commit()
        finally:
            state["publish"] = False
        return {
            "d4s3_oracle": s3_result,
            "authority_revision": authority_revision,
            "root_count": len(roots),
            "root_map_digest": root_digest,
            "member_count": report["expected_count"],
            "member_digest": report["expected_sha256"],
            "source_evidence_count": len(sources),
            "source_evidence_digest": source_digest,
            "source_evidence": sources,
        }
    except Exception:
        if conn.in_transaction:
            conn.rollback()
        raise
    finally:
        state["publish"] = False


def current_sources(conn: sqlite3.Connection) -> list[str]:
    return [r[0] for r in conn.execute(
        "SELECT source_id FROM d4s4_effective_file_sources ORDER BY source_id"
    )]


def pointer(conn: sqlite3.Connection) -> tuple[str, int] | None:
    row = conn.execute(
        "SELECT manifest_id,revision FROM active_visibility_pointer WHERE singleton_id=1"
    ).fetchone()
    return tuple(row) if row else None


def authority_revision(conn: sqlite3.Connection) -> int:
    return int(conn.execute(
        "SELECT revision FROM d4s4_authority_clock WHERE singleton_id=1"
    ).fetchone()[0])


def setup_published(s4: bool = True, claimed: bool = False,
                    path: str = "/root/a.txt") -> tuple[sqlite3.Connection, dict[str, bool], dict[str, str]]:
    conn, state = fresh_db(s4=s4)
    source = add_source(conn, "a", path, claimed=claimed)
    add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, [source])
    S3.stage_manifest(conn, "manifest-a", [("root-a", "gen-a")], [S3.projection(source)], None)
    if s4:
        trusted_publish(conn, state, "manifest-a")
    else:
        S3.publish(conn, "manifest-a")
    return conn, state, source


def old_model_counterexample(action: str) -> dict[str, Any]:
    claimed = action == "claim-delete"
    conn, _, source = setup_published(s4=False, claimed=claimed)
    before_pointer = pointer(conn)
    before_visible = S3.D4S2.visible_source_ids(conn)
    original_physical_digest = conn.execute(
        "SELECT evidence_digest FROM file_physical_identity_epochs WHERE physical_identity_id=?",
        (source["physical_id"],),
    ).fetchone()[0]
    if action == "run-coverage":
        conn.execute("UPDATE scan_runs SET coverage_complete=0 WHERE id='run-a'")
    elif action == "run-status":
        conn.execute("UPDATE scan_runs SET status='failed' WHERE id='run-a'")
    elif action == "source-status":
        conn.execute("UPDATE file_sources SET status='conflicted' WHERE source_id=?", (source["source_id"],))
    elif action == "source-confidence":
        conn.execute("UPDATE file_sources SET identity_confidence='path_only' WHERE source_id=?", (source["source_id"],))
    elif action == "physical-digest":
        conn.execute("UPDATE file_physical_identity_epochs SET evidence_digest='tampered-after-publish' WHERE physical_identity_id=?",
                     (source["physical_id"],))
    elif action == "claim-delete":
        conn.execute("DELETE FROM source_identity_claims WHERE claim_id=?", (source["claim_id"],))
    elif action == "root-path":
        conn.execute("UPDATE scan_roots SET normalized_path='/attacker-root' WHERE id='root-a'")
    else:
        raise ValueError(action)
    report = S3.oracle_report(conn, "manifest-a")
    after_visible = S3.D4S2.visible_source_ids(conn)
    member_digest_matches = all(
        item["declared_member_digest"] == item["actual_member_digest"]
        and item["declared_member_count"] == item["actual_member_count"]
        for item in report["generations"]
    )
    source_path = conn.execute(
        "SELECT pe.path_text,pi.evidence_digest,r.normalized_path "
        "FROM file_path_entries pe JOIN file_physical_identity_epochs pi "
        "ON pi.physical_identity_id=pe.physical_identity_id "
        "JOIN source_generations sg ON sg.generation_id='gen-a' "
        "JOIN scan_roots r ON r.id=sg.root_id WHERE pe.path_entry_id=?",
        (source["path_entry_id"],),
    ).fetchone()
    assert pointer(conn) == before_pointer
    assert before_visible == after_visible == [source["source_id"]]
    result = {
        "case": action,
        "old_sql_mutation_succeeded": True,
        "pointer_before_after": [list(before_pointer), list(pointer(conn))],
        "raw_s3_effective_source_visible_after": after_visible,
        "s3_oracle_ready_predicate_after": report["valid"],
        "s3_transitive_evidence_checks_after": {
            "run_generation_errors": report["generation_errors"],
            "invalid_source_identity_rows": report["invalid_identity_count"],
            "member_set_differences": [report["missing_count"], report["extra_count"]],
            "root_map_digest_matches": report["root_map_digest_matches"],
            "generation_member_digest_and_count_match": member_digest_matches,
            "physical_evidence_digest_unchanged": source_path[1] == original_physical_digest,
            "member_still_within_current_root": root_path_contains(source_path[2], source_path[0]),
        },
        "s3_invalid_identity_count_after": report["invalid_identity_count"],
        "s3_generation_errors_after": report["generation_errors"],
        "s3_oracle_checks_physical_epoch_evidence_digest": False,
    }
    conn.close()
    return result


def old_digest_bypass(kind: str) -> dict[str, Any]:
    conn, _ = S3.fresh_db(protected=True)
    source = S3.add_source(conn, "digest", "/root/digest.txt")
    S3.add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, [source])
    S3.stage_manifest(conn, f"legacy-bad-{kind}", [("root-a", "gen-a")], [S3.projection(source)], None)
    if kind == "member_digest":
        conn.execute("UPDATE source_generations SET member_digest='wrong-member-sha256' WHERE generation_id='gen-a'")
    elif kind == "root_map_digest":
        conn.execute("UPDATE visibility_manifests SET root_map_digest='wrong-root-map-sha256' WHERE manifest_id=?",
                     (f"legacy-bad-{kind}",))
    else:
        raise ValueError(kind)
    oracle = S3.oracle_report(conn, f"legacy-bad-{kind}")
    conn.execute("BEGIN IMMEDIATE")
    conn.execute("UPDATE source_generations SET status='published' WHERE generation_id='gen-a'")
    conn.execute("UPDATE visibility_manifests SET status='published' WHERE manifest_id=?", (f"legacy-bad-{kind}",))
    conn.execute(
        "INSERT INTO active_visibility_pointer(singleton_id,manifest_id,revision,updated_at) VALUES(1,?,1,?)",
        (f"legacy-bad-{kind}", NOW),
    )
    conn.commit()
    result = {
        "case": kind,
        "application_oracle_valid": oracle["valid"],
        "stored_digest": oracle["generations"][0]["declared_member_digest"] if kind == "member_digest" else oracle["declared_root_map_digest"],
        "actual_digest": oracle["generations"][0]["actual_member_digest"] if kind == "member_digest" else oracle["actual_root_map_digest"],
        "raw_sql_published_and_advanced_pointer": pointer(conn) == (f"legacy-bad-{kind}", 1),
        "d3_trigger_error": None,
    }
    assert result["application_oracle_valid"] is False and result["raw_sql_published_and_advanced_pointer"]
    conn.close()
    return result


def assert_rejects(conn: sqlite3.Connection, label: str, sql: str,
                   args: tuple[Any, ...] = ()) -> dict[str, str]:
    conn.execute("SAVEPOINT d4s4_attempt")
    try:
        conn.execute(sql, args)
        conn.execute("RELEASE d4s4_attempt")
        raise AssertionError(f"{label}: mutation unexpectedly succeeded")
    except (sqlite3.IntegrityError, sqlite3.OperationalError) as exc:
        message = str(exc)
        conn.execute("ROLLBACK TO d4s4_attempt")
        conn.execute("RELEASE d4s4_attempt")
        return {"operation": label, "rejected": message}


def authority_mutation(conn: sqlite3.Connection, state: dict[str, bool], sql: str,
                       args: tuple[Any, ...] = ()) -> int:
    before = authority_revision(conn)
    state["evidence"] = True
    try:
        conn.execute(sql, args)
    finally:
        state["evidence"] = False
    return authority_revision(conn) - before


def test_transitive_mutation_guards() -> dict[str, Any]:
    conn, state, source = setup_published(s4=True)
    before_pointer = pointer(conn)
    before_clock = authority_revision(conn)
    before_raw = S3.D4S2.visible_source_ids(conn)
    before_current = current_sources(conn)

    rejected = [
        assert_rejects(conn, "scan_runs.coverage_complete",
                       "UPDATE scan_runs SET coverage_complete=0 WHERE id='run-a'"),
        assert_rejects(conn, "scan_runs.status",
                       "UPDATE scan_runs SET status='failed' WHERE id='run-a'"),
        assert_rejects(conn, "file_sources.status_without_revision",
                       "UPDATE file_sources SET status='conflicted' WHERE source_id=?", (source["source_id"],)),
        assert_rejects(conn, "file_sources.identity_confidence_without_revision",
                       "UPDATE file_sources SET identity_confidence='path_only' WHERE source_id=?", (source["source_id"],)),
        assert_rejects(conn, "file_sources.status_with_revision_without_capability",
                       "UPDATE file_sources SET status='conflicted',source_revision=source_revision+1 WHERE source_id=?",
                       (source["source_id"],)),
        assert_rejects(conn, "file_sources.confidence_with_revision_without_capability",
                       "UPDATE file_sources SET identity_confidence='path_only',source_revision=source_revision+1 WHERE source_id=?",
                       (source["source_id"],)),
        assert_rejects(conn, "physical_identity.evidence_digest",
                       "UPDATE file_physical_identity_epochs SET evidence_digest='tampered' WHERE physical_identity_id=?",
                       (source["physical_id"],)),
        assert_rejects(conn, "scan_roots.normalized_path",
                       "UPDATE scan_roots SET normalized_path='/attacker-root' WHERE id='root-a'"),
        assert_rejects(conn, "scan_roots.watcher_revision",
                       "UPDATE scan_roots SET watcher_revision=watcher_revision+1 WHERE id='root-a'"),
        assert_rejects(conn, "files.current_path_without_publisher",
                       "UPDATE files SET path='/root/other.txt' WHERE id=?", (source["file_id"],)),
    ]
    assert conn.execute("SELECT coverage_complete FROM scan_runs WHERE id='run-a'").fetchone()[0] == 1
    assert conn.execute("SELECT status FROM scan_runs WHERE id='run-a'").fetchone()[0] == "completed"
    assert conn.execute("SELECT evidence_digest FROM file_physical_identity_epochs WHERE physical_identity_id=?",
                         (source["physical_id"],)).fetchone()[0] != "tampered"
    assert pointer(conn) == before_pointer
    assert authority_revision(conn) == before_clock
    assert current_sources(conn) == before_current == [source["source_id"]]

    # Harmless display metadata and user-owned metadata/tag writes are outside
    # the authority seal and remain writable without pointer or clock changes.
    conn.execute("UPDATE scan_roots SET display_name='Friendly name' WHERE id='root-a'")
    conn.execute("UPDATE files SET context='owner context survives' WHERE id=?", (source["file_id"],))
    conn.execute(
        "INSERT INTO user_tags(id,display_name,normalized_name,created_at,updated_at) VALUES('tag-s4','Research','research',?,?)",
        (NOW, NOW),
    )
    conn.execute("INSERT INTO file_user_tags(file_id,tag_id,created_at) VALUES(?,?,?)",
                 (source["file_id"], "tag-s4", NOW))
    assert authority_revision(conn) == before_clock
    assert current_sources(conn) == [source["source_id"]]
    assert conn.execute("SELECT context FROM files WHERE id=?", (source["file_id"],)).fetchone()[0] == "owner context survives"
    assert conn.execute("SELECT count(*) FROM file_user_tags WHERE file_id=?", (source["file_id"],)).fetchone()[0] == 1

    # An authorized source-state mutation advances its own revision and the
    # authority clock. Old SQL membership remains, but current visibility and
    # cross-transaction Cursors fail closed until a successor is published.
    source_epoch_delta = authority_mutation(
        conn, state,
        "UPDATE file_sources SET status='conflicted',source_revision=source_revision+1 WHERE source_id=?",
        (source["source_id"],),
    )
    source_after = {
        "raw_s3_view_sources": S3.D4S2.visible_source_ids(conn),
        "d4s4_current_sources": current_sources(conn),
        "authority_revision_before_after": [before_clock, authority_revision(conn)],
        "pointer_before_after": [list(before_pointer), list(pointer(conn))],
        "cursor_from_before_rejected": authority_revision(conn) != before_clock,
        "retained_source_seal_count": conn.execute("SELECT count(*) FROM d4s4_manifest_source_evidence WHERE manifest_id='manifest-a'").fetchone()[0],
        "source_revision": conn.execute("SELECT source_revision FROM file_sources WHERE source_id=?", (source["source_id"],)).fetchone()[0],
        "clock_increments": source_epoch_delta,
    }
    assert source_after["raw_s3_view_sources"] == [source["source_id"]]
    assert source_after["d4s4_current_sources"] == []
    assert source_after["cursor_from_before_rejected"]
    assert source_after["pointer_before_after"][0] == source_after["pointer_before_after"][1]
    conn.close()

    # The published explicit claim itself cannot be deleted; an append-only
    # revocation event invalidates the authority while preserving the claim row.
    claim_conn, claim_state, claimed = setup_published(s4=True, claimed=True)
    claim_pointer = pointer(claim_conn)
    claim_clock = authority_revision(claim_conn)
    claim_delete = assert_rejects(
        claim_conn, "source_identity_claims.delete_published",
        "DELETE FROM source_identity_claims WHERE claim_id=?", (claimed["claim_id"],),
    )
    claim_state["evidence"] = True
    try:
        claim_conn.execute(
            "UPDATE file_sources SET source_revision=source_revision+1 WHERE source_id=?",
            (claimed["source_id"],),
        )
        claim_conn.execute(
            "INSERT INTO d4s4_claim_revocations(claim_id,revoked_at,reason) VALUES(?,?,?)",
            (claimed["claim_id"], NOW + 1, "identity ownership withdrawn"),
        )
    finally:
        claim_state["evidence"] = False
    revoke_delta = authority_revision(claim_conn) - claim_clock
    assert claim_conn.execute("SELECT count(*) FROM source_identity_claims WHERE claim_id=?", (claimed["claim_id"],)).fetchone()[0] == 1
    assert current_sources(claim_conn) == []
    assert pointer(claim_conn) == claim_pointer
    claim_result = {
        "direct_delete": claim_delete,
        "claim_row_retained": True,
        "revocation_event_count": claim_conn.execute("SELECT count(*) FROM d4s4_claim_revocations WHERE claim_id=?", (claimed["claim_id"],)).fetchone()[0],
        "source_revision_after_revocation": claim_conn.execute("SELECT source_revision FROM file_sources WHERE source_id=?", (claimed["source_id"],)).fetchone()[0],
        "current_view_after_revocation": current_sources(claim_conn),
        "authority_revision_before_after": [claim_clock, authority_revision(claim_conn)],
        "pointer_unchanged": pointer(claim_conn) == claim_pointer,
        "clock_increments": revoke_delta,
    }
    claim_conn.close()

    # A root path/coverage or watcher change uses the explicit evidence
    # mutation path, increments the authority clock, and leaves the old
    # published Manifest and pointer intact for history.
    root_conn, root_state, root_source = setup_published(s4=True)
    root_pointer = pointer(root_conn)
    root_clock = authority_revision(root_conn)
    root_old = tuple(root_conn.execute(
        "SELECT normalized_path,root_revision,watcher_revision FROM d4s4_manifest_root_evidence WHERE manifest_id='manifest-a' AND root_id='root-a'"
    ).fetchone())
    root_delta = authority_mutation(
        root_conn, root_state,
        "UPDATE scan_roots SET normalized_path='/renamed-root',revision=revision+1 WHERE id='root-a'",
    )
    root_after = {
        "sealed_old_root_evidence": list(root_old),
        "current_root": root_conn.execute("SELECT normalized_path FROM scan_roots WHERE id='root-a'").fetchone()[0],
        "current_view_sources": current_sources(root_conn),
        "raw_s3_view_sources": S3.D4S2.visible_source_ids(root_conn),
        "authority_revision_before_after": [root_clock, authority_revision(root_conn)],
        "pointer_before_after": [list(root_pointer), list(pointer(root_conn))],
        "clock_increments": root_delta,
    }
    assert root_after["sealed_old_root_evidence"][0] == "/root"
    assert root_after["current_root"] == "/renamed-root"
    assert root_after["current_view_sources"] == []
    assert root_after["raw_s3_view_sources"] == [root_source["source_id"]]
    assert pointer(root_conn) == root_pointer
    root_conn.close()

    watch_conn, watch_state, watch_source = setup_published(s4=True)
    watch_pointer = pointer(watch_conn)
    watch_clock = authority_revision(watch_conn)
    watch_delta = authority_mutation(
        watch_conn, watch_state,
        "UPDATE scan_roots SET watcher_revision=watcher_revision+1,revision=revision+1 WHERE id='root-a'",
    )
    watcher_after = {
        "current_view_sources": current_sources(watch_conn),
        "raw_s3_view_sources": S3.D4S2.visible_file_sources if hasattr(S3.D4S2, "visible_file_sources") else S3.D4S2.visible_source_ids(watch_conn),
        "authority_revision_before_after": [watch_clock, authority_revision(watch_conn)],
        "pointer_before_after": [list(watch_pointer), list(pointer(watch_conn))],
        "clock_increments": watch_delta,
        "cursor_bound_to_old_authority_revision_rejected": authority_revision(watch_conn) != watch_clock,
    }
    assert watcher_after["current_view_sources"] == []
    assert pointer(watch_conn) == watch_pointer
    assert watcher_after["cursor_bound_to_old_authority_revision_rejected"]
    watch_conn.close()

    return {
        "blocked_direct_transitive_dml": rejected,
        "blocked_count": len(rejected) + 1,
        "legacy_raw_view_before": before_raw,
        "source_version_invalidation": source_after,
        "claim_revocation": claim_result,
        "root_reconfiguration_invalidation": root_after,
        "watcher_revision_invalidation": watcher_after,
        "independent_user_metadata_and_tag_writes_remain_allowed": True,
        "display_name_change_does_not_invalidate": True,
    }


def test_digest_bypass_closed() -> dict[str, Any]:
    cases = []
    for kind in ("member_digest", "root_map_digest"):
        conn, state = fresh_db(s4=True)
        source = add_source(conn, "bad-" + kind, "/root/bad.txt")
        add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, [source])
        S3.stage_manifest(conn, "manifest-bad", [("root-a", "gen-a")], [S3.projection(source)], None)
        if kind == "member_digest":
            conn.execute("UPDATE source_generations SET member_digest='wrong-member-sha256' WHERE generation_id='gen-a'")
        else:
            conn.execute("UPDATE visibility_manifests SET root_map_digest='wrong-root-map-sha256' WHERE manifest_id='manifest-bad'")
        preflight_error = None
        try:
            trusted_publish(conn, state, "manifest-bad")
            raise AssertionError(f"trusted publisher accepted wrong {kind}")
        except AuthorityRejected as exc:
            preflight_error = str(exc)
        direct_error = assert_rejects(
            conn, "direct_status_publish_with_wrong_" + kind,
            "UPDATE visibility_manifests SET status='published' WHERE manifest_id='manifest-bad'",
        )
        assert conn.execute("SELECT status FROM visibility_manifests WHERE manifest_id='manifest-bad'").fetchone()[0] == "ready"
        assert conn.execute("SELECT status FROM source_generations WHERE generation_id='gen-a'").fetchone()[0] == "complete"
        assert pointer(conn) is None
        cases.append({
            "case": kind,
            "trusted_oracle_rejected": True,
            "trusted_oracle_error": preflight_error,
            "raw_sql_transition_rejected": direct_error,
            "generation_after_rollback": conn.execute("SELECT status FROM source_generations WHERE generation_id='gen-a'").fetchone()[0],
            "manifest_after_rollback": conn.execute("SELECT status FROM visibility_manifests WHERE manifest_id='manifest-bad'").fetchone()[0],
            "pointer_after_rollback": pointer(conn),
        })
        conn.close()

    # The S4 trigger references an application capability function. A normal
    # sqlite3 connection that was not opened through the trusted DB service has
    # no function and therefore cannot execute a direct status transition.
    with tempfile.TemporaryDirectory(prefix="zen-d4s4-raw-connection-") as temp_dir:
        db_path = Path(temp_dir) / "raw.sqlite"
        conn, state = fresh_db(s4=True, path=db_path)
        source = add_source(conn, "raw", "/root/raw.txt")
        add_generation(conn, "root-a", "/root", "gen-a", "run-a", 1, [source])
        S3.stage_manifest(conn, "manifest-raw", [("root-a", "gen-a")], [S3.projection(source)], None)
        conn.close()
        raw = sqlite3.connect(db_path)
        raw.execute("PRAGMA foreign_keys=ON")
        try:
            raw.execute("UPDATE visibility_manifests SET status='published' WHERE manifest_id='manifest-raw'")
            raw.commit()
            raise AssertionError("raw connection without the authority capability published a Manifest")
        except sqlite3.OperationalError as exc:
            raw.rollback()
            raw_error = str(exc)
        raw.close()
        check = S3.D4S2.open_db(db_path)
        register_capabilities(check)
        assert check.execute("SELECT status FROM visibility_manifests WHERE manifest_id='manifest-raw'").fetchone()[0] == "ready"
        assert check.execute("SELECT count(*) FROM active_visibility_pointer").fetchone()[0] == 0
        check.close()

    return {
        "bad_digest_cases": cases,
        "unwrapped_sqlite_connection": {
            "publication_rejected": True,
            "sqlite_error": raw_error,
            "manifest_remained_ready": True,
            "pointer_absent": True,
        },
        "sqlite_sha256_udf_used": False,
    }


def test_preflight_revision_and_old_pointer_retention() -> dict[str, Any]:
    conn, state, old_source = setup_published(s4=True)
    old_pointer = pointer(conn)
    old_view_before = current_sources(conn)
    new_source = add_source(conn, "next", "/root/next.txt")
    add_generation(conn, "root-a", "/root", "gen-next", "run-next", 2, [new_source])
    S3.stage_manifest(conn, "manifest-next", [("root-a", "gen-next")], [S3.projection(new_source)], "manifest-a")
    before_clock = authority_revision(conn)
    # Simulate a Watcher event after candidate preparation but before the
    # trusted publication call. The candidate still carries watcher revision 0.
    delta = authority_mutation(
        conn, state,
        "UPDATE scan_roots SET watcher_revision=watcher_revision+1,revision=revision+1 WHERE id='root-a'",
    )
    try:
        trusted_publish(conn, state, "manifest-next")
        raise AssertionError("stale watcher candidate unexpectedly published")
    except AuthorityRejected as exc:
        failure = str(exc)
    result = {
        "old_pointer_before_after": [list(old_pointer), list(pointer(conn))],
        "old_manifest_status": conn.execute("SELECT status FROM visibility_manifests WHERE manifest_id='manifest-a'").fetchone()[0],
        "new_manifest_status": conn.execute("SELECT status FROM visibility_manifests WHERE manifest_id='manifest-next'").fetchone()[0],
        "new_generation_status": conn.execute("SELECT status FROM source_generations WHERE generation_id='gen-next'").fetchone()[0],
        "old_manifest_retained": conn.execute("SELECT count(*) FROM d4s4_manifest_authority_seals WHERE manifest_id='manifest-a'").fetchone()[0] == 1,
        "old_view_before_event": old_view_before,
        "current_view_after_event": current_sources(conn),
        "preflight_error": failure,
        "authority_revision_before_event_after": [before_clock, authority_revision(conn)],
        "event_clock_increments": delta,
        "pointer_revision_unchanged": pointer(conn) == old_pointer,
        "no_half_published_successor": conn.execute("SELECT count(*) FROM d4s4_manifest_authority_seals WHERE manifest_id='manifest-next'").fetchone()[0] == 0,
    }
    assert result["old_pointer_before_after"][0] == result["old_pointer_before_after"][1]
    assert result["old_manifest_status"] == "published"
    assert result["new_manifest_status"] == "ready" and result["new_generation_status"] == "complete"
    assert result["old_manifest_retained"] and result["current_view_after_event"] == []
    assert result["pointer_revision_unchanged"] and result["no_half_published_successor"]
    assert old_source["source_id"] != new_source["source_id"]
    conn.close()
    return result


def test_independent_metadata_and_valid_publish() -> dict[str, Any]:
    conn, state = fresh_db(s4=True)
    source = add_source(conn, "valid", "/root/valid.txt", claimed=True)
    add_generation(conn, "root-a", "/root", "gen-valid", "run-valid", 1, [source])
    S3.stage_manifest(conn, "manifest-valid", [("root-a", "gen-valid")], [S3.projection(source)], None)
    published = trusted_publish(conn, state, "manifest-valid")
    assert current_sources(conn) == [source["source_id"]]
    assert conn.execute("SELECT count(*) FROM d4s4_manifest_source_evidence WHERE manifest_id='manifest-valid'").fetchone()[0] == 1
    assert conn.execute("SELECT count(*) FROM d4s4_manifest_root_evidence WHERE manifest_id='manifest-valid'").fetchone()[0] == 1
    assert published["root_map_digest"] == S3.oracle_report(conn, "manifest-valid")["actual_root_map_digest"]
    assert published["member_digest"] == S3.oracle_report(conn, "manifest-valid")["expected_sha256"]
    assert published["source_evidence_count"] == 1
    conn.close()
    return {
        "published": {k: published[k] for k in ("authority_revision", "root_count", "root_map_digest", "member_count", "member_digest", "source_evidence_count", "source_evidence_digest")},
        "current_sources": [source["source_id"]],
        "explicit_claim_bound": published["source_evidence"][0]["claim_ids"] == [source["claim_id"]],
        "all_three_digest_domains_recorded": True,
    }


def run() -> dict[str, Any]:
    preserved_s3 = S3.run()
    assert preserved_s3["preserved_d4s2_baseline"]["assertion_groups"] == 15
    assert len(preserved_s3["legacy_counterexamples"]) == 7
    assert len(preserved_s3["protected_invalid_publications"]) == 4
    assert preserved_s3["immutability"]["blocked_mutations"] == 16

    old_cases = [
        old_model_counterexample("run-coverage"),
        old_model_counterexample("run-status"),
        old_model_counterexample("source-status"),
        old_model_counterexample("source-confidence"),
        old_model_counterexample("physical-digest"),
        old_model_counterexample("claim-delete"),
        old_model_counterexample("root-path"),
    ]
    old_digest_bypasses = [old_digest_bypass("member_digest"), old_digest_bypass("root_map_digest")]
    safe_publication = test_independent_metadata_and_valid_publish()
    mutation_protection = test_transitive_mutation_guards()
    digest_protection = test_digest_bypass_closed()
    preflight_race = test_preflight_revision_and_old_pointer_retention()

    conn, _ = fresh_db(s4=True)
    integrity = {
        "foreign_key_check": "ok" if not conn.execute("PRAGMA foreign_key_check").fetchall() else "failed",
        "sqlite_integrity_check": conn.execute("PRAGMA integrity_check").fetchone()[0],
        "fts5_external_content_integrity_check": None,
    }
    # The trigger/index contract itself is checked in the preserved S3 runner;
    # this disposable connection has the real Schema 37 files_fts virtual table.
    try:
        conn.execute("INSERT INTO files_fts(files_fts,rank) VALUES('integrity-check',1)")
        integrity["fts5_external_content_integrity_check"] = "ok"
    except sqlite3.DatabaseError as exc:
        integrity["fts5_external_content_integrity_check"] = str(exc)
    conn.close()
    assert integrity == {
        "foreign_key_check": "ok",
        "sqlite_integrity_check": "ok",
        "fts5_external_content_integrity_check": "ok",
    }

    return {
        "research_scope": "D4-S4 isolated Schema 37 SQL/Python harness only; no production code or version changed",
        "runtime": {
            "sqlite_version": sqlite3.sqlite_version,
            "sqlite_source_id": sqlite3.connect(":memory:").execute("SELECT sqlite_source_id()").fetchone()[0],
            "sha256_sql_function": False,
            "publication_capability": "connection-local application UDF, set only by trusted publisher wrapper",
        },
        "inputs": {
            "schema37_ddl_sha256": S3.sha256(S3.SCHEMA_PATH.read_bytes()),
            "d4s2_candidate_sql_sha256": S3.sha256((S3.D4S2_DIR / "harness/physical-candidate.sql").read_bytes()),
            "d4s2_runner_sha256": S3.sha256(S3.D4S2_RUNNER.read_bytes()),
            "d4s3_runner_sha256": sha256(S3_RUNNER.read_bytes()),
            "d4s3_oracle_sql_sha256": sha256((S3_DIR / "harness/manifest-authority-oracle.sql").read_bytes()),
            "d4s3_protection_sql_sha256": sha256((S3_DIR / "harness/manifest-authority-protection.sql").read_bytes()),
            "d4s4_sql_sha256": sha256(S3_SQL.read_bytes()),
            "d4s4_runner_sha256": sha256(Path(__file__).read_bytes()),
            "d4s3_evidence_results_sha256": sha256((S3_DIR / "evidence/manifest-authority-results.json").read_bytes()),
            "d4s3_original_log_sha256": "8f61c7c87fc73a0fa1a38440d9987d954eb070fe1f29df9b6329e6c4abfee266",
        },
        "preserved_d4s3": {
            "d4s2_assertion_groups": 15,
            "d4s3_failure_counterexamples": 7,
            "d4s3_invalid_publication_cases": 4,
            "d4s3_blocked_dml_mutations": 16,
            "nested_roots_rename_cursor_fts_rollback_contracts": "all rerun unchanged by the committed D4-S3 runner",
        },
        "old_model_transitive_counterexamples": old_cases,
        "old_model_digest_bypasses": old_digest_bypasses,
        "valid_trusted_publication": safe_publication,
        "transitive_protection": mutation_protection,
        "digest_bypass_protection": digest_protection,
        "preflight_revision_race": preflight_race,
        "integrity": integrity,
        "limits": [
            "Schema 37 filesystem identities are synthetic. The harness seals bytes from candidate evidence rows but cannot prove the OS identity claim is true.",
            "The POSIX root boundary helper is only a synthetic path test; Windows reparse points, junctions, volume identity, case folding, and native file IDs remain untested.",
            "SQLite has no native SHA-256 function here. Exact digests are recomputed by Python inside BEGIN IMMEDIATE; SQL triggers enforce DML relations and the connection-local capability only.",
            "A process with raw write access that can register a forged capability UDF, drop triggers, or replace the DB remains outside this service trust boundary; #380 must establish compatible single-writer/database-generation isolation.",
            "All transitive invalidation increments one global authority clock, so one root/source evidence change temporarily hides every root until a successor Manifest publishes. A per-root epoch could reduce that scope but is not proven here.",
            "No production query was changed. Existing production consumers still read files.is_stale directly; every relevant reader would need to consume the future authoritative visibility relation before production GO.",
            "No SQLite 3.51.3 product FTS/BM25, Windows/macOS native identity, GC, WAL/performance, or full migration validation was run.",
        ],
    }


def main() -> int:
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    result = run()
    encoded = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    (EVIDENCE / "transitive-evidence-results.json").write_text(encoded, encoding="utf-8")
    lines = [
        "#381-D4-S4 transitive evidence authority harness",
        f"sqlite_version={result['runtime']['sqlite_version']}",
        f"sqlite_source_id={result['runtime']['sqlite_source_id']}",
        "preserved_D4S2_assertion_groups=15; preserved_D4S3_counterexamples=7; invalid_publication_cases=4; DML_guards=16",
        f"old_transitive_mutations_accepted={len(result['old_model_transitive_counterexamples'])}",
        f"old_digest_bypass_cases={len(result['old_model_digest_bypasses'])} direct SQL publication succeeded",
        f"new_direct_mutations_blocked={result['transitive_protection']['blocked_count']}",
        "authorized source/root/watcher changes invalidate authority revision and fail current reads closed",
        "claim deletion blocked; append-only revocation retained claim evidence and invalidated current visibility",
        "member_digest and root_map_digest wrong values rejected by trusted Oracle and raw status transition capability",
        "ordinary file context and user tag writes remain allowed without authority revision change",
        "stale preflight after Watcher revision change rejected; old pointer and old Manifest retained",
        "foreign_key_check=ok; integrity_check=ok; FTS5 integrity-check=ok",
        "production_schema_version=unassigned (synthetic PRAGMA user_version remains 37)",
    ]
    log = "\n".join(lines) + "\n"
    (EVIDENCE / "transitive-evidence-run.log").write_text(log, encoding="utf-8")
    print(log, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
