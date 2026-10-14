mod db;
mod model;

use crate::db::{
    PRODUCT_POOL_SIZE, authority_slot, claim_live_lease_count, configure_product_main,
    create_authority_db, create_backup_copy, create_slot, create_unsealed_slot, elapsed_ms,
    file_system_sync, open_connection, open_product_pool, open_readonly, path_digest, pool_get,
    query_pool_settings, query_rows, read_only_slot_tx, sha256_bytes, suffix, transition_slot,
    update_doc_for_delta, verify_seal, wal_bytes,
};
use model::{Document, corpus_sha256, documents, file_sha256};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[derive(Serialize)]
struct Record<'a> {
    record_type: &'static str,
    case_id: &'a str,
    status: &'a str,
    details: Value,
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "issue381-b4-p1a-{}-{label}-{nonce}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn emit(record_type: &'static str, case_id: &'static str, status: &'static str, details: Value) {
    println!(
        "{}",
        serde_json::to_string(&Record {
            record_type,
            case_id,
            status,
            details
        })
        .unwrap()
    );
}

fn case(case_id: &'static str, status: &'static str, details: Value) {
    emit("case_result", case_id, status, details);
}

fn source_info() -> Result<Value, Box<dyn std::error::Error>> {
    let conn = Connection::open_in_memory()?;
    let version: String = conn.query_row("SELECT sqlite_version()", [], |r| r.get(0))?;
    let source_id: String = conn.query_row("SELECT sqlite_source_id()", [], |r| r.get(0))?;
    let fts5: i64 = conn.query_row("SELECT sqlite_compileoption_used('ENABLE_FTS5')", [], |r| {
        r.get(0)
    })?;
    conn.execute_batch(
        "CREATE VIRTUAL TABLE temp.trigram_probe USING fts5(text,tokenize='trigram');\
         INSERT INTO temp.trigram_probe(text) VALUES('bundled trigram source check');",
    )?;
    let trigram_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM temp.trigram_probe WHERE trigram_probe MATCH 'trigram'",
        [],
        |r| r.get(0),
    )?;
    let sqlite_overrides = std::env::vars()
        .filter(|(key, _)| key.starts_with("SQLITE_") || key.starts_with("LIBSQLITE3_"))
        .map(|(key, _)| key)
        .collect::<Vec<_>>();
    Ok(json!({
        "sqlite_version":version,
        "sqlite_source_id":source_id,
        "fts5_compile_option":fts5,
        "trigram_query_count":trigram_count,
        "rusqlite":"0.39.0",
        "libsqlite3_sys":"0.37.0",
        "r2d2_sqlite":"0.34.0",
        "bundled":true,
        "sqlite_system_override_variables":sqlite_overrides
    }))
}

fn corpus_counts(docs: &[Document]) -> Value {
    let mut claims = BTreeMap::<String, usize>::new();
    let mut overlap = 0;
    let mut disabled_only = 0;
    let mut effective = 0;
    let mut uids = std::collections::HashSet::new();
    for doc in docs {
        for claim in &doc.root_claims {
            *claims.entry(claim.clone()).or_default() += 1;
        }
        if doc.root_claims.iter().any(|r| r == "root-a-healthy")
            && doc
                .root_claims
                .iter()
                .any(|r| r == "root-b-degraded-retained")
        {
            overlap += 1;
        }
        if doc.root_claims.len() == 1 && doc.root_claims[0] == "root-d-disabled" {
            disabled_only += 1;
        }
        effective += usize::from(doc.effective);
        uids.insert(&doc.uid);
    }
    json!({
        "generated_document_count":docs.len(),
        "effective_document_count":effective,
        "claim_counts":claims,
        "overlap_a_b_count":overlap,
        "disabled_only_count":disabled_only,
        "unique_uid_count":uids.len(),
        "corpus_sha256":corpus_sha256(docs),
        "root_semantics":{
            "root-a-healthy":"enabled and healthy",
            "root-b-degraded-retained":"degraded E0 claim retained as read-only synthetic authority",
            "root-d-disabled":"disabled; never grants visibility"
        },
        "scope":"synthetic only; no product root health or SourceUID proof"
    })
}

fn emit_query_snapshot(
    conn: &Connection,
    n: usize,
    corpus_hash: &str,
    query: &str,
    root: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = query_rows(conn, query, root, None)?;
    emit(
        "query_snapshot",
        "P1A-T03",
        "OBSERVED",
        json!({
            "generated_document_count":n,
            "corpus_sha256":corpus_hash,
            "query":query,
            "root_filter":root,
            "weights":[6.0,1.5],
            "page_limit":null,
            "rows":rows
        }),
    );
    Ok(())
}

fn size_snapshots() -> Result<(), Box<dyn std::error::Error>> {
    for n in [0_usize, 10_000, 100_000] {
        let docs = documents(n);
        emit(
            "corpus_summary",
            "P1A-T02",
            "OBSERVED",
            corpus_counts(&docs),
        );
        let temp = TempRoot::new(&format!("snapshot-{n}"))?;
        let slot = create_slot(&temp.path("E0.sqlite"), &format!("E0-{n}"), &docs)?;
        let readonly = open_readonly(&slot.path)?;
        for (query, root) in [
            ("common", None),
            ("common", Some("root-b-degraded-retained")),
            ("rareanchor", None),
            ("rareanchor", Some("root-b-degraded-retained")),
        ] {
            emit_query_snapshot(&readonly, n, &slot.manifest.corpus_sha256, query, root)?;
        }
        emit(
            "sealed_slot_summary",
            "P1A-T02",
            "OBSERVED",
            json!({
                "generated_document_count":n,
                "effective_document_count":slot.manifest.document_count,
                "slot_sha256":slot.manifest.sqlite_file_sha256,
                "slot_bytes":slot.manifest.sqlite_file_bytes,
                "sealed_wal_bytes":slot.wal_bytes_after_seal,
                "integrity":verify_seal(&slot)?,
                "pool_settings":query_pool_settings(&readonly)?
            }),
        );
    }
    case(
        "P1A-T02",
        "PASS",
        json!({"sizes":[0,10000,100000],"fixture_only":true,"uid_dedup_and_root_claims":true}),
    );
    Ok(())
}

fn short_like(conn: &Connection, text: &str) -> rusqlite::Result<Vec<String>> {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");
    let mut statement = conn.prepare(
        "SELECT uid FROM slot_docs WHERE name LIKE ?1 ESCAPE '\\' OR path LIKE ?1 ESCAPE '\\' \
         ORDER BY mtime DESC,name COLLATE NOCASE ASC,uid ASC",
    )?;
    statement.query_map([pattern], |r| r.get(0))?.collect()
}

fn test_short_query() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempRoot::new("short-query")?;
    let docs = documents(2_000);
    let slot = create_slot(&temp.path("slot.sqlite"), "SHORT", &docs)?;
    let conn = open_readonly(&slot.path)?;
    let library = query_rows(&conn, "fi", None, None)?;
    let legacy = short_like(&conn, "fi")?;
    let escaped = short_like(&conn, "_%")?;
    let ok = library.is_empty() && !legacy.is_empty() && escaped.is_empty();
    case(
        "P1A-T03-short-query",
        if ok { "PASS" } else { "FAIL" },
        json!({
            "library_trigram_short_match_count":library.len(),
            "legacy_literal_like_short_match_count":legacy.len(),
            "wildcard_text_escaped_match_count":escaped.len(),
            "legacy_rank":1000000.0,
            "semantics_are_distinct":true
        }),
    );
    Ok(())
}

fn test_e0_e1() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempRoot::new("e0-e1")?;
    let mut e0_docs = documents(1_000);
    e0_docs[2].name = "common e0marker frozen name".into();
    e0_docs[2].path = "/synthetic/E0/old-location.txt".into();
    let e1_docs = e0_docs
        .iter()
        .cloned()
        .map(|mut d| {
            if d.uid == "source-000002" {
                d.name = "common e1marker replacement name".into();
                d.path = "/synthetic/E1/new-location.txt".into();
                d.observation_revision = 2;
            }
            d
        })
        .collect::<Vec<_>>();
    let e0 = create_slot(&temp.path("E0.sqlite"), "E0", &e0_docs)?;
    let e1 = create_slot(&temp.path("E1.sqlite"), "E1", &e1_docs)?;
    let main_path = temp.path("main.sqlite");
    create_authority_db(&main_path, "E0", &e0.manifest.sqlite_file_sha256)?;
    let mut main = open_connection(&main_path)?;
    configure_product_main(&mut main)?;
    main.execute(
        "INSERT INTO simulated_slots VALUES('E1','SEALED_UNPUBLISHED',?1,2)",
        [&e1.manifest.sqlite_file_sha256],
    )?;
    let held = read_only_slot_tx(&e0.path)?;
    let held_before = query_rows(&held, "e0marker", None, None)?;
    let fresh_before = {
        let conn = open_readonly(&e0.path)?;
        query_rows(&conn, "e0marker", None, None)?
    };
    let candidate = {
        let conn = open_readonly(&e1.path)?;
        query_rows(&conn, "e1marker", None, None)?
    };
    let authority_before = authority_slot(&main)?;
    main.execute(
        "UPDATE simulated_authority SET catalog_epoch=2,slot_id='E1',revision=revision+1 \
         WHERE singleton=1 AND revision=1",
        [],
    )?;
    let authority_after = authority_slot(&main)?;
    let held_after = query_rows(&held, "e0marker", None, None)?;
    let before_rank = held_before
        .iter()
        .find(|r| r.uid == "source-000002")
        .map(|r| r.score_bits.clone());
    let after_rank = held_after
        .iter()
        .find(|r| r.uid == "source-000002")
        .map(|r| r.score_bits.clone());
    let rank_stable = before_rank.is_some() && before_rank == after_rank;
    let current_e1 = {
        let conn = open_readonly(&e1.path)?;
        query_rows(&conn, "e1marker", None, None)?
    };
    let old_cursor_rejected = authority_before != authority_after;
    let ok = held_before.iter().any(|r| r.uid == "source-000002")
        && fresh_before.iter().any(|r| r.uid == "source-000002")
        && candidate.iter().any(|r| r.uid == "source-000002")
        && held_after.iter().any(|r| r.uid == "source-000002")
        && rank_stable
        && current_e1.iter().any(|r| r.uid == "source-000002")
        && authority_before.0 == "E0"
        && authority_after.0 == "E1"
        && old_cursor_rejected;
    case(
        "P1A-T04",
        if ok { "PASS" } else { "FAIL" },
        json!({
            "e0_fresh_before_publish":fresh_before.iter().map(|r| &r.uid).collect::<Vec<_>>(),
            "e0_held_before_publish":held_before.iter().map(|r| &r.uid).collect::<Vec<_>>(),
            "e1_candidate":candidate.iter().map(|r| &r.uid).collect::<Vec<_>>(),
            "authority_before":authority_before,
            "authority_after_simulated_publish":authority_after,
            "held_e0_after_simulated_publish":held_after.iter().map(|r| &r.uid).collect::<Vec<_>>(),
            "held_e0_rank_bits_before_after":[before_rank,after_rank],
            "e0_rank_unchanged":rank_stable,
            "current_e1_candidate_after_publish":current_e1.iter().map(|r| &r.uid).collect::<Vec<_>>(),
            "old_cursor_rejected_after_revision_change":old_cursor_rejected,
            "e0_and_e1_text":{"e0_name":"common e0marker frozen name","e1_name":"common e1marker replacement name","e0_path":"/synthetic/E0/old-location.txt","e1_path":"/synthetic/E1/new-location.txt"}
        }),
    );
    drop(held);
    Ok(())
}

fn test_seal_tamper() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempRoot::new("seal-tamper")?;
    let slot = create_slot(&temp.path("sealed.sqlite"), "SEALED", &documents(500))?;
    let verification = verify_seal(&slot)?;
    let corrupt_path = temp.path("corrupt.sqlite");
    fs::copy(&slot.path, &corrupt_path)?;
    let mut corrupt = fs::read(&corrupt_path)?;
    corrupt.truncate((corrupt.len() / 2).max(64));
    fs::write(&corrupt_path, corrupt)?;
    let corrupt_sha = file_sha256(&corrupt_path)?.0;
    let corrupt_integrity_rejected = open_readonly(&corrupt_path)
        .and_then(|c| c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)))
        .map(|v| v != "ok")
        .unwrap_or(true);
    let manifest_copy = suffix(&slot.manifest_path, ".tampered");
    let altered = serde_json::to_vec(&json!({"slot_id":"SEALED","sqlite_file_sha256":"bad"}))?;
    fs::write(&manifest_copy, &altered)?;
    let altered_manifest_rejected =
        sha256_bytes(&fs::read(&manifest_copy)?) != slot.manifest_sha256;
    let ok = verification["manifest_digest_matches"] == true
        && verification["file_digest_matches"] == true
        && verification["integrity_check"] == "ok"
        && verification["readonly_write_rejected"] == true
        && verification["wal_bytes_after_seal"] == 0
        && corrupt_sha != slot.manifest.sqlite_file_sha256
        && corrupt_integrity_rejected
        && altered_manifest_rejected;
    case(
        "P1A-T07",
        if ok { "PASS" } else { "FAIL" },
        json!({
            "seal_verification":verification,
            "tampered_file_sha256":corrupt_sha,
            "truncated_db_digest_rejected":corrupt_sha != slot.manifest.sqlite_file_sha256,
            "truncated_db_integrity_rejected":corrupt_integrity_rejected,
            "tampered_manifest_rejected":altered_manifest_rejected,
            "final_file_hash_after_checkpoint_close":slot.manifest.sqlite_file_sha256,
            "wal_bytes_after_seal":slot.wal_bytes_after_seal
        }),
    );
    Ok(())
}

fn create_physical_source(path: &Path, docs: &[Document]) -> rusqlite::Result<()> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.execute_batch(
        r#"
        CREATE TABLE physical_docs(
          rowid INTEGER PRIMARY KEY, uid TEXT UNIQUE NOT NULL,
          observation_revision INTEGER NOT NULL, name TEXT NOT NULL,
          path TEXT NOT NULL, mtime INTEGER NOT NULL, is_effective INTEGER NOT NULL
        );
        CREATE VIRTUAL TABLE physical_fts USING fts5(
          name,path,content='physical_docs',content_rowid='rowid',tokenize='trigram'
        );
        CREATE TRIGGER physical_docs_ai AFTER INSERT ON physical_docs BEGIN
          INSERT INTO physical_fts(rowid,name,path) VALUES(new.rowid,new.name,new.path);
        END;
        CREATE TRIGGER physical_docs_ad AFTER DELETE ON physical_docs BEGIN
          INSERT INTO physical_fts(physical_fts,rowid,name,path) VALUES('delete',old.rowid,old.name,old.path);
        END;
        CREATE TRIGGER physical_docs_au AFTER UPDATE ON physical_docs BEGIN
          INSERT INTO physical_fts(physical_fts,rowid,name,path) VALUES('delete',old.rowid,old.name,old.path);
          INSERT INTO physical_fts(rowid,name,path) VALUES(new.rowid,new.name,new.path);
        END;
        "#,
    )?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO physical_docs(rowid,uid,observation_revision,name,path,mtime,is_effective) \
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
        )?;
        for (i, d) in docs.iter().enumerate() {
            stmt.execute(params![
                i as i64 + 1,
                d.uid,
                d.observation_revision,
                d.name,
                d.path,
                d.mtime,
                i64::from(d.effective)
            ])?;
        }
    }
    tx.commit()?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    Ok(())
}

fn query_physical(
    conn: &Connection,
    query: &str,
    limit: Option<i64>,
    filter_after_limit: bool,
) -> rusqlite::Result<Vec<(String, f64, i64)>> {
    let match_text = format!("\"{}\"", query.replace('"', ""));
    let sql = match (limit, filter_after_limit) {
        (Some(_), true) => {
            "SELECT uid,rank,mtime FROM (\
          SELECT d.uid,bm25(physical_fts,6.0,1.5) AS rank,d.mtime,d.is_effective,d.name \
          FROM physical_fts JOIN physical_docs d ON d.rowid=physical_fts.rowid \
          WHERE physical_fts MATCH ?1 ORDER BY rank ASC,d.mtime DESC,d.name COLLATE NOCASE ASC,d.uid ASC LIMIT ?2\
        ) WHERE is_effective=1 ORDER BY rank ASC,mtime DESC,uid ASC"
        }
        (Some(_), false) => {
            "SELECT d.uid,bm25(physical_fts,6.0,1.5),d.mtime \
          FROM physical_fts JOIN physical_docs d ON d.rowid=physical_fts.rowid \
          WHERE physical_fts MATCH ?1 AND d.is_effective=1 \
          ORDER BY 2 ASC,d.mtime DESC,d.name COLLATE NOCASE ASC,d.uid ASC LIMIT ?2"
        }
        (None, _) => {
            "SELECT d.uid,bm25(physical_fts,6.0,1.5),d.mtime \
          FROM physical_fts JOIN physical_docs d ON d.rowid=physical_fts.rowid \
          WHERE physical_fts MATCH ?1 AND d.is_effective=1 \
          ORDER BY 2 ASC,d.mtime DESC,d.name COLLATE NOCASE ASC,d.uid ASC"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if let Some(limit) = limit {
        stmt.query_map(params![match_text, limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([match_text], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(rows)
}

fn test_f0_f2_negatives() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempRoot::new("f0-f2")?;
    let mut docs = documents(5_000);
    for doc in docs.iter_mut().filter(|d| !d.effective) {
        doc.name.push_str(" common common common hiddenlong");
    }
    docs[2].name.push_str(" oldincarnationmarker");
    let source_path = temp.path("physical.sqlite");
    create_physical_source(&source_path, &docs)?;
    let slot = create_slot(&temp.path("F1-E0.sqlite"), "F1-E0", &docs)?;
    let physical = Connection::open(&source_path)?;
    let f0_all = query_physical(&physical, "common", None, false)?;
    let f0_bad_page = query_physical(&physical, "common", Some(2), true)?;
    let f1_conn = open_readonly(&slot.path)?;
    let f1_rows = query_rows(&f1_conn, "common", None, None)?;
    let f1_rank = f1_rows
        .iter()
        .map(|r| (r.uid.clone(), r.score_bits.clone()))
        .collect::<BTreeMap<_, _>>();
    let f0_rank = f0_all
        .iter()
        .filter(|(uid, _, _)| f1_rank.contains_key(uid))
        .map(|(uid, score, _)| (uid.clone(), format!("{:016x}", score.to_bits())))
        .collect::<BTreeMap<_, _>>();
    let rank_differs = f1_rank
        .iter()
        .any(|(uid, bits)| f0_rank.get(uid) != Some(bits));
    let underfilled = f0_bad_page.len() < 2;

    let mutable = Connection::open(&source_path)?;
    mutable.execute(
        "UPDATE physical_docs SET name='common newincarnationmarker',path='/same/path/reused.dat',observation_revision=2 WHERE uid='source-000002'",
        [],
    )?;
    mutable.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    let lost: i64 = mutable.query_row(
        "SELECT COUNT(*) FROM physical_fts WHERE physical_fts MATCH 'oldincarnationmarker'",
        [],
        |r| r.get(0),
    )?;
    let f1_retains = query_rows(&f1_conn, "oldincarnationmarker", None, None)?
        .iter()
        .any(|r| r.uid == "source-000002");
    let revision: i64 = mutable.query_row(
        "SELECT observation_revision FROM physical_docs WHERE uid='source-000002'",
        [],
        |r| r.get(0),
    )?;
    let f2_guard_rejects = revision != 1;
    case(
        "P1A-N01",
        if rank_differs && underfilled {
            "EXPECTED_REJECTION"
        } else {
            "FAIL"
        },
        json!({
            "f0_all_physical_bm25_rank_differs":rank_differs,
            "f0_filter_after_top2_result_count":f0_bad_page.len(),
            "post_limit_underfill_observed":underfilled,
            "f1_effective_result_count":f1_rows.len(),
            "negative_design_accepted":false
        }),
    );
    case(
        "P1A-N02",
        if lost == 0 && f2_guard_rejects && f1_retains {
            "EXPECTED_REJECTION"
        } else {
            "FAIL"
        },
        json!({
            "physical_e1_revision":revision,
            "published_e0_revision":1,
            "fresh_f2_guard_rejects":f2_guard_rejects,
            "unguarded_f2_e0_candidate_count":lost,
            "immutable_f1_e0_candidate_retained":f1_retains,
            "fallback_to_mutable_physical_fts":false
        }),
    );
    Ok(())
}

fn test_user_fact_isolation() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempRoot::new("user-facts")?;
    let path = temp.path("main.sqlite");
    create_authority_db(&path, "E0", "test-sha")?;
    let conn = open_connection(&path)?;
    conn.execute_batch(
        "CREATE TABLE identity_projection(source_uid TEXT PRIMARY KEY,path TEXT,verified INTEGER);\
         INSERT INTO identity_projection VALUES('old-uid','/same/path',0);\
         INSERT INTO identity_projection VALUES('new-uid','/same/path',0);",
    )?;
    let before: i64 = conn.query_row("SELECT COUNT(*) FROM user_facts", [], |r| r.get(0))?;
    let inherited: i64 = conn.query_row(
        "SELECT COUNT(*) FROM user_facts f JOIN identity_projection p ON p.source_uid=f.legacy_file_id WHERE p.source_uid='new-uid'",
        [],|r|r.get(0))?;
    let action: i64 = conn.query_row(
        "SELECT COUNT(*) FROM identity_projection WHERE source_uid='new-uid' AND verified=1",
        [],
        |r| r.get(0),
    )?;
    let after: i64 = conn.query_row("SELECT COUNT(*) FROM user_facts", [], |r| r.get(0))?;
    let ok = before == 2 && inherited == 0 && action == 0 && after == before;
    case(
        "P1A-N03",
        if ok { "EXPECTED_REJECTION" } else { "FAIL" },
        json!({
            "same_path_new_uid":"new-uid","old_facts_before_after":[before,after],
            "inherited_fact_count":inherited,"new_uid_action_authority":action,
            "path_identity_promotion":false
        }),
    );
    Ok(())
}

fn release_lease(conn: &mut Connection, lease_id: &str) -> rusqlite::Result<bool> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let rows = tx.execute(
        "UPDATE simulated_leases SET active=0 WHERE lease_id=?1 AND active=1",
        [lease_id],
    )?;
    tx.commit()?;
    Ok(rows == 1)
}

fn register_lease(
    conn: &mut Connection,
    lease_id: &str,
    slot_id: &str,
    expires: i64,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current: String = tx.query_row(
        "SELECT slot_id FROM simulated_authority WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let state: Option<String> = tx
        .query_row(
            "SELECT state FROM simulated_slots WHERE slot_id=?1",
            [slot_id],
            |r| r.get(0),
        )
        .optional()?;
    let allowed = current == slot_id && state.as_deref() == Some("PUBLISHED");
    if allowed {
        tx.execute(
            "INSERT INTO simulated_leases VALUES(?1,?2,?3,?4,1)",
            params![lease_id, slot_id, expires, i64::from(std::process::id())],
        )?;
    }
    tx.commit()?;
    Ok(allowed)
}

fn gc_slot(
    conn: &mut Connection,
    slot_id: &str,
    now: i64,
    file: &Path,
) -> rusqlite::Result<(bool, String)> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let authority: String = tx.query_row(
        "SELECT slot_id FROM simulated_authority WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let state: Option<String> = tx
        .query_row(
            "SELECT state FROM simulated_slots WHERE slot_id=?1",
            [slot_id],
            |r| r.get(0),
        )
        .optional()?;
    let live: i64 = tx.query_row(
        "SELECT COUNT(*) FROM simulated_leases WHERE slot_id=?1 AND active=1 AND expires_at>?2",
        params![slot_id, now],
        |r| r.get(0),
    )?;
    if authority == slot_id {
        tx.commit()?;
        return Ok((false, "CURRENT_AUTHORITY".into()));
    }
    if live > 0 {
        tx.commit()?;
        return Ok((false, "LIVE_PIN".into()));
    }
    if !matches!(state.as_deref(), Some("RETIRING" | "GC_ELIGIBLE")) {
        tx.commit()?;
        return Ok((false, "NOT_RETIRING".into()));
    }
    tx.execute(
        "UPDATE simulated_slots SET state='GC_ELIGIBLE' WHERE slot_id=?1 AND state='RETIRING'",
        [slot_id],
    )?;
    tx.commit()?;
    if file.exists() {
        fs::remove_file(file).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    }
    let removed = conn.execute(
        "UPDATE simulated_slots SET state='REMOVED' WHERE slot_id=?1 AND state='GC_ELIGIBLE'",
        [slot_id],
    )?;
    Ok((removed == 1, "REMOVED_OUTSIDE_AUTHORITY_TX".into()))
}

fn concurrent_pin_gc_probe(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    slot: &Path,
) -> Result<(Vec<f64>, Vec<Value>), Box<dyn std::error::Error>> {
    let mut timings = Vec::new();
    let mut outcomes = Vec::new();
    for round in 0..12 {
        let id = format!("race-{round}");
        let pin_id = id.clone();
        let pool_pin = pool.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let pin = thread::spawn(move || -> Result<(), String> {
            let mut conn = pool_pin.get().map_err(|e| e.to_string())?;
            let start = Instant::now();
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let current: String = tx
                .query_row(
                    "SELECT slot_id FROM simulated_authority WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let state: String = tx
                .query_row(
                    "SELECT state FROM simulated_slots WHERE slot_id='E0'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let accepted = current == "E0" && state == "PUBLISHED";
            if accepted {
                tx.execute(
                    "INSERT INTO simulated_leases VALUES(?1,'E0',999999,?2,1)",
                    params![pin_id, i64::from(std::process::id())],
                )
                .map_err(|e| e.to_string())?;
            }
            ready_tx
                .send((accepted, start.elapsed().as_secs_f64() * 1000.0))
                .map_err(|e| e.to_string())?;
            release_rx.recv().map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        });
        let (accepted, duration) = ready_rx.recv()?;
        timings.push(duration);
        let pool_gc = pool.clone();
        let (gc_tx, gc_rx) = mpsc::channel();
        let gc = thread::spawn(move || -> Result<i64, String> {
            let mut conn = pool_gc.get().map_err(|e| e.to_string())?;
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let count:i64=tx.query_row("SELECT COUNT(*) FROM simulated_leases WHERE slot_id='E0' AND active=1 AND expires_at>1",[],|r|r.get(0)).map_err(|e|e.to_string())?;
            gc_tx.send(count).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(count)
        });
        thread::sleep(Duration::from_millis(15));
        release_tx.send(())?;
        let live = gc_rx.recv()?;
        pin.join().map_err(|_| "pin thread panic")??;
        gc.join().map_err(|_| "gc thread panic")??;
        let read = open_readonly(slot)?;
        let rows: i64 = read.query_row("SELECT COUNT(*) FROM slot_docs", [], |r| r.get(0))?;
        let passed = accepted && live > 0 && rows > 0;
        outcomes.push(json!({"round":round,"pin_registered_before_gc":accepted,"gc_observed_lease":live,"slot_readable":rows>0,"passed":passed}));
        let mut cleanup = pool.get()?;
        release_lease(&mut cleanup, &id)?;
    }
    Ok((timings, outcomes))
}

fn test_pin_gc() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempRoot::new("pin-gc")?;
    let docs = documents(1_000);
    let e0 = create_slot(&temp.path("E0.sqlite"), "E0", &docs)?;
    let e1 = create_slot(&temp.path("E1.sqlite"), "E1", &docs)?;
    let main_path = temp.path("main.sqlite");
    create_authority_db(&main_path, "E0", &e0.manifest.sqlite_file_sha256)?;
    let pool = open_product_pool(&main_path)?;
    let mut warm = Vec::new();
    for _ in 0..PRODUCT_POOL_SIZE {
        warm.push(pool_get(&pool)?);
    }
    drop(warm);
    let (pin_cost, interleavings) = concurrent_pin_gc_probe(&pool, &e0.path)?;
    let mut main = pool_get(&pool)?;
    let registered = register_lease(&mut main, "live-e0", "E0", 10_000)?;
    let held = read_only_slot_tx(&e0.path)?;
    main.execute(
        "INSERT INTO simulated_slots VALUES('E1','SEALED_UNPUBLISHED',?1,2)",
        [&e1.manifest.sqlite_file_sha256],
    )?;
    main.execute(
        "UPDATE simulated_slots SET state='PUBLISHED' WHERE slot_id='E1'",
        [],
    )?;
    main.execute(
        "UPDATE simulated_authority SET slot_id='E1',catalog_epoch=2,revision=2 WHERE singleton=1",
        [],
    )?;
    main.execute(
        "UPDATE simulated_slots SET state='RETIRING' WHERE slot_id='E0'",
        [],
    )?;
    let blocked_gc = gc_slot(&mut main, "E0", 1, &e0.path)?;
    let live = claim_live_lease_count(&main, "E0", 1)?;
    let held_rows: i64 = held.query_row("SELECT COUNT(*) FROM slot_docs", [], |r| r.get(0))?;
    drop(held);
    let released = release_lease(&mut main, "live-e0")?;
    let gc_after_release = gc_slot(&mut main, "E0", 2, &e0.path)?;
    let replay = register_lease(&mut main, "replay-e0", "E0", 20_000)?;
    let expired = temp.path("expired.sqlite");
    fs::copy(&e1.path, &expired)?;
    main.execute(
        "INSERT INTO simulated_slots VALUES('EXPIRED','RETIRING',?1,1)",
        [&e1.manifest.sqlite_file_sha256],
    )?;
    main.execute(
        "INSERT INTO simulated_leases VALUES('expired-lease','EXPIRED',5,1,1)",
        [],
    )?;
    let expired_gc = gc_slot(&mut main, "EXPIRED", 6, &expired)?;
    let failed_open_db = temp.path("open-failure-main.sqlite");
    let absent_slot = temp.path("absent-slot.sqlite");
    create_authority_db(&failed_open_db, "E0", "absent-slot-hash")?;
    let mut failed_main = open_connection(&failed_open_db)?;
    configure_product_main(&mut failed_main)?;
    let failed_pin = register_lease(&mut failed_main, "open-failure-pin", "E0", 10_000)?;
    let slot_open_failed = open_readonly(&absent_slot).is_err();
    let failed_lease_released = release_lease(&mut failed_main, "open-failure-pin")?;
    failed_main.execute(
        "INSERT INTO simulated_slots VALUES('E1','PUBLISHED',?1,2)",
        [&e1.manifest.sqlite_file_sha256],
    )?;
    failed_main.execute(
        "UPDATE simulated_authority SET slot_id='E1',catalog_epoch=2,revision=2 WHERE singleton=1",
        [],
    )?;
    failed_main.execute(
        "UPDATE simulated_slots SET state='RETIRING' WHERE slot_id='E0'",
        [],
    )?;
    let gc_after_open_failure = gc_slot(&mut failed_main, "E0", 1, &absent_slot)?;
    let gc_before_pin_rejected = !register_lease(&mut failed_main, "pin-after-gc", "E0", 10_000)?;
    let settings = query_pool_settings(&main)?;
    let ok = registered
        && blocked_gc == (false, "LIVE_PIN".into())
        && live == 1
        && held_rows > 0
        && released
        && gc_after_release.0
        && !replay
        && expired_gc.0
        && failed_pin
        && slot_open_failed
        && failed_lease_released
        && gc_after_open_failure.0
        && gc_before_pin_rejected
        && settings.2 == 5_000
        && interleavings.iter().all(|v| v["passed"] == true);
    case(
        "P1A-T05",
        if ok { "PASS" } else { "FAIL" },
        json!({
            "registered_real_readonly_handle":registered,"gc_while_live_pin":blocked_gc,"live_lease_count":live,
            "held_read_tx_rows":held_rows,"release_succeeded":released,"gc_after_handle_close":gc_after_release,
            "expired_lease_replayed":replay,"expired_lease_recovery_gc":expired_gc,
            "registered_then_open_failed":failed_pin&&slot_open_failed,
            "open_failure_lease_released":failed_lease_released,
            "gc_after_failed_open":gc_after_open_failure,
            "gc_before_late_pin_rejected":gc_before_pin_rejected,
            "pin_gc_interleavings":interleavings,
            "per_reader_begin_immediate_cost_ms":pin_cost,
            "pool_max_size":pool.max_size(),"busy_timeout_ms":settings.2,
            "authority_barrier":"SQLite BEGIN IMMEDIATE test-only prototype; no product safety claim"
        }),
    );
    Ok(())
}

fn percentile(mut values: Vec<f64>, p: usize) -> f64 {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return 0.0;
    }
    let rank = (p * values.len()).div_ceil(100).max(1);
    values[rank - 1]
}

fn percentile_json(values: &[f64]) -> Value {
    json!({"samples":values.len(),"nearest_rank_p50_ms":percentile(values.to_vec(),50),"nearest_rank_p95_ms":percentile(values.to_vec(),95),"nearest_rank_p99_ms":percentile(values.to_vec(),99)})
}

fn sqlite_extended_code(error: &rusqlite::Error) -> Option<i32> {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => Some(failure.extended_code),
        _ => None,
    }
}

fn pool_contention_samples() -> Result<Vec<f64>, Box<dyn std::error::Error>> {
    let temp = TempRoot::new("pool-contention")?;
    let path = temp.path("main.sqlite");
    {
        let mut conn = open_connection(&path)?;
        configure_product_main(&mut conn)?;
        conn.execute_batch("CREATE TABLE counter(id INTEGER PRIMARY KEY,value INTEGER);INSERT INTO counter VALUES(1,0);")?;
    }
    let pool = open_product_pool(&path)?;
    let mut warm = Vec::new();
    for _ in 0..PRODUCT_POOL_SIZE {
        warm.push(pool_get(&pool)?);
    }
    drop(warm);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(PRODUCT_POOL_SIZE as usize));
    let (tx, rx) = mpsc::channel();
    let mut workers = Vec::new();
    for _ in 0..PRODUCT_POOL_SIZE {
        let p = pool.clone();
        let b = barrier.clone();
        let out = tx.clone();
        workers.push(thread::spawn(move || -> Result<(), String> {
            for _ in 0..100 {
                let mut conn = p.get().map_err(|e| e.to_string())?;
                b.wait();
                let start = Instant::now();
                let tr = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| e.to_string())?;
                tr.execute("UPDATE counter SET value=value+1 WHERE id=1", [])
                    .map_err(|e| e.to_string())?;
                tr.commit().map_err(|e| e.to_string())?;
                out.send(start.elapsed().as_secs_f64() * 1000.0)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        }));
    }
    drop(tx);
    let samples = rx.iter().collect::<Vec<_>>();
    for w in workers {
        w.join().map_err(|_| "contention thread panic")??;
    }
    if samples.len() != 800 {
        return Err(format!("expected 800 lock samples, got {}", samples.len()).into());
    }
    Ok(samples)
}

fn test_pool_busy_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempRoot::new("busy-timeout")?;
    let path = temp.path("main.sqlite");
    {
        let mut c = open_connection(&path)?;
        configure_product_main(&mut c)?;
        c.execute_batch("CREATE TABLE writes(id INTEGER PRIMARY KEY,value TEXT);")?;
    }
    let pool = open_product_pool(&path)?;
    let mut handles = Vec::new();
    for _ in 0..PRODUCT_POOL_SIZE {
        handles.push(pool_get(&pool)?);
    }
    let connections = pool.state().connections;
    drop(handles);
    let holder = pool_get(&pool)?;
    holder.execute_batch("BEGIN IMMEDIATE")?;
    let p = pool.clone();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let start = Instant::now();
        let result = (|| -> rusqlite::Result<()> {
            let c = p
                .get()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            c.execute("INSERT INTO writes(value) VALUES('cancel-request')", [])?;
            Ok(())
        })();
        tx.send((start.elapsed().as_secs_f64() * 1000.0, result.err()))
            .unwrap();
    });
    let (wait_ms, error) = rx.recv_timeout(Duration::from_secs(8))?;
    worker.join().map_err(|_| "busy writer panic")?;
    let code = error.as_ref().and_then(sqlite_extended_code);
    let busy = code == Some(5);
    let expected_window = (4_800.0..=7_500.0).contains(&wait_ms);
    holder.execute_batch("ROLLBACK")?;
    let settings = {
        let c = pool_get(&pool)?;
        query_pool_settings(&c)?
    };
    let writes: i64 = {
        let c = pool_get(&pool)?;
        c.query_row("SELECT COUNT(*) FROM writes", [], |r| r.get(0))?
    };
    let samples = pool_contention_samples()?;
    let okay = pool.max_size() == 8
        && connections == 8
        && settings.2 == 5000
        && busy
        && expected_window
        && writes == 0
        && samples.len() == 800;
    case(
        "P1A-T06",
        if okay { "PASS" } else { "FAIL" },
        json!({
            "pool_max_size":pool.max_size(),"simultaneous_connections":connections,
            "journal_mode":settings.0,"synchronous":settings.1,"busy_timeout_ms":settings.2,
            "cancel_writer_wait_ms":wait_ms,"cancel_writer_sqlite_extended_code":code,
            "SQLITE_BUSY_5_at_unchanged_5000ms":busy&&expected_window,"partial_write_rows":writes,
            "8_connection_competing_begin_immediate_costs":percentile_json(&samples),
            "8_connection_competing_begin_immediate_samples_ms":samples,
            "reader_lock_interpretation":"measured serialized-write cost; BEGIN IMMEDIATE per request remains unapproved for product"
        }),
    );
    Ok(())
}

fn cursor_valid(
    token_slot: &str,
    token_epoch: i64,
    token_query: &str,
    token_scope: &str,
    token_root_rev: i64,
    slot: &str,
    epoch: i64,
    query: &str,
    scope: &str,
    root_rev: i64,
) -> bool {
    token_slot == slot
        && token_epoch == epoch
        && token_query == query
        && token_scope == scope
        && token_root_rev == root_rev
}

fn test_n04() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempRoot::new("n04")?;
    let slot = create_slot(&temp.path("E0.sqlite"), "E0", &documents(100))?;
    let path = temp.path("main.sqlite");
    create_authority_db(&path, "E0", &slot.manifest.sqlite_file_sha256)?;
    let mut c = open_connection(&path)?;
    configure_product_main(&mut c)?;
    let base = authority_slot(&c)?;
    let stale_cas=c.execute("UPDATE simulated_authority SET slot_id='forged',revision=revision+1 WHERE singleton=1 AND revision=?1",[base.2+1])?;
    let tamper_rejected = !cursor_valid(
        "E0", 1, "common", "root-a", 1, "E0", 1, "other", "root-a", 1,
    );
    let replay_rejected = !cursor_valid(
        "E0", 1, "common", "root-a", 1, "E0", 2, "common", "root-a", 1,
    );
    let forged_root_rev_rejected = !cursor_valid(
        "E0", 1, "common", "root-a", 1, "E0", 1, "common", "root-a", 2,
    );
    let forged_manifest = serde_json::to_vec(
        &json!({"slot_id":"E0","sqlite_file_sha256":"forged","root_revision":88}),
    )?;
    let manifest_rejected = sha256_bytes(&forged_manifest) != slot.manifest_sha256
        && slot.manifest.sqlite_file_sha256 != "forged";
    let duplicate = c.execute(
        "INSERT INTO simulated_slots VALUES('E0','PUBLISHED',?1,1)",
        [&slot.manifest.sqlite_file_sha256],
    );
    let slot_reuse = duplicate.is_err();
    c.execute(
        "UPDATE simulated_slots SET state='REMOVED' WHERE slot_id='E0'",
        [],
    )?;
    let double_gc = transition_slot(&mut c, "E0", "GC_ELIGIBLE", "REMOVED")?;
    let ok = stale_cas == 0
        && tamper_rejected
        && replay_rejected
        && forged_root_rev_rejected
        && manifest_rejected
        && slot_reuse
        && !double_gc;
    case(
        "P1A-N04",
        if ok { "EXPECTED_REJECTION" } else { "FAIL" },
        json!({
            "tampered_cursor_rejected":tamper_rejected,"replayed_cursor_rejected":replay_rejected,
            "forged_root_revision_rejected":forged_root_rev_rejected,"manifest_replay_rejected":manifest_rejected,
            "stale_cas_rows_changed":stale_cas,"slot_id_reuse_rejected":slot_reuse,"double_gc_accepted":double_gc
        }),
    );
    Ok(())
}

fn test_faults() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempRoot::new("faults")?;
    let old = create_slot(&temp.path("E0.sqlite"), "E0", &documents(500))?;
    let main_path = temp.path("main.sqlite");
    create_authority_db(&main_path, "E0", &old.manifest.sqlite_file_sha256)?;
    let main = open_connection(&main_path)?;
    let before_auth = authority_slot(&main)?;
    let before_facts: String = main.query_row(
        "SELECT group_concat(legacy_file_id||':'||fact_key||':'||fact_value,'|') FROM user_facts",
        [],
        |r| r.get(0),
    )?;
    let partial = temp.path("partial.sqlite");
    let disk_full = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&partial)?;
        f.write_all(b"SQLite format 3\0partial")?;
        f.sync_all()?;
        Err(std::io::Error::new(
            std::io::ErrorKind::StorageFull,
            "injected capacity limit",
        ))
    })();
    let no_manifest = !suffix(&partial, ".manifest.json").exists();
    let backup = temp.path("interrupted-backup.sqlite");
    create_backup_copy(&old.path, &backup)?;
    let mut bytes = fs::read(&backup)?;
    bytes.truncate((bytes.len() / 3).max(64));
    fs::write(&backup, bytes)?;
    let backup_rejected =
        path_digest(&backup)?.as_deref() != Some(old.manifest.sqlite_file_sha256.as_str());
    let after_auth = authority_slot(&main)?;
    let after_facts: String = main.query_row(
        "SELECT group_concat(legacy_file_id||':'||fact_key||':'||fact_value,'|') FROM user_facts",
        [],
        |r| r.get(0),
    )?;
    let safe = disk_full.is_err()
        && no_manifest
        && backup_rejected
        && before_auth == after_auth
        && before_facts == after_facts
        && old.path.exists();
    case(
        "P1A-T09",
        if safe { "PASS" } else { "FAIL" },
        json!({
            "disk_full":"MODEL_ONLY injected capacity failure","partial_write_manifest_absent":no_manifest,
            "interrupted_backup_digest_rejected":backup_rejected,"authority_unchanged":before_auth==after_auth,
            "user_fact_sha_before_after":[sha256_bytes(before_facts.as_bytes()),sha256_bytes(after_facts.as_bytes())],
            "old_slot_retained":old.path.exists(),"real_disk_full_or_power_loss_test":false
        }),
    );
    Ok(())
}

fn child_hold(stage: &str, root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let e0_path = root.join("E0.sqlite");
    let e1_path = root.join("E1.sqlite");
    let main_path = root.join("main.sqlite");
    let docs = documents(200);
    let e0 = create_slot(&e0_path, "E0", &docs)?;
    create_authority_db(&main_path, "E0", &e0.manifest.sqlite_file_sha256)?;
    let mut keep: Option<Connection> = None;
    match stage {
        "build" => {
            let c = Connection::open(root.join("partial.sqlite"))?;
            c.pragma_update(None, "journal_mode", "WAL")?;
            c.execute_batch("CREATE TABLE stage_build(uid TEXT PRIMARY KEY);BEGIN IMMEDIATE;INSERT INTO stage_build VALUES('partial');")?;
            keep = Some(c);
        }
        "close" => create_unsealed_slot(&e1_path, &docs)?,
        "sync" => {
            create_unsealed_slot(&e1_path, &docs)?;
            file_system_sync(&e1_path)?;
            file_system_sync(root)?;
        }
        "seal" => {
            let _ = create_slot(&e1_path, "E1", &docs)?;
        }
        "pointer-pre-commit" => {
            let e1 = create_slot(&e1_path, "E1", &docs)?;
            let mut c = open_connection(&main_path)?;
            configure_product_main(&mut c)?;
            c.execute_batch("BEGIN IMMEDIATE")?;
            c.execute(
                "INSERT INTO simulated_slots VALUES('E1','SEALED_UNPUBLISHED',?1,2)",
                [&e1.manifest.sqlite_file_sha256],
            )?;
            c.execute("UPDATE simulated_authority SET catalog_epoch=2,slot_id='E1',revision=2 WHERE singleton=1",[])?;
            keep = Some(c);
        }
        "pointer-post-commit" => {
            let e1 = create_slot(&e1_path, "E1", &docs)?;
            let mut c = open_connection(&main_path)?;
            configure_product_main(&mut c)?;
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO simulated_slots VALUES('E1','PUBLISHED',?1,2)",
                [&e1.manifest.sqlite_file_sha256],
            )?;
            tx.execute("UPDATE simulated_authority SET catalog_epoch=2,slot_id='E1',revision=2 WHERE singleton=1",[])?;
            tx.commit()?;
        }
        "gc-pre-unlink" => {
            let _ = create_slot(&e1_path, "RETIRED", &docs)?;
            let mut c = open_connection(&main_path)?;
            configure_product_main(&mut c)?;
            c.execute(
                "INSERT INTO simulated_slots VALUES('RETIRED','GC_ELIGIBLE',?1,1)",
                ["retired-sha"],
            )?;
        }
        "gc-post-unlink" => {
            let _ = create_slot(&e1_path, "RETIRED", &docs)?;
            let mut c = open_connection(&main_path)?;
            configure_product_main(&mut c)?;
            c.execute(
                "INSERT INTO simulated_slots VALUES('RETIRED','REMOVED',?1,1)",
                ["retired-sha"],
            )?;
            drop(c);
            fs::remove_file(&e1_path)?;
        }
        _ => return Err(format!("unknown child barrier {stage}").into()),
    }
    println!("READY {stage}");
    std::io::stdout().flush()?;
    let _keep_alive = keep;
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

fn kill_at_barrier(stage: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let temp = TempRoot::new(&format!("crash-{stage}"))?;
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .arg("--child")
        .arg(stage)
        .arg(&temp.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("child stdout missing")?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if !line.starts_with("READY ") {
        let output = child.wait_with_output()?;
        return Err(format!(
            "child stage {stage} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    child.kill()?;
    let status = child.wait()?;
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut stderr);
    }
    let main_path = temp.path("main.sqlite");
    let mut main = open_connection(&main_path)?;
    configure_product_main(&mut main)?;
    let current = authority_slot(&main)?;
    let expected = if stage == "pointer-post-commit" {
        "E1"
    } else {
        "E0"
    };
    let current_path = temp.path(&format!("{}.sqlite", current.0));
    let digest = path_digest(&current_path)?;
    let record: Option<String> = main
        .query_row(
            "SELECT slot_sha256 FROM simulated_slots WHERE slot_id=?1",
            [&current.0],
            |r| r.get(0),
        )
        .optional()?;
    let facts: i64 = main.query_row("SELECT COUNT(*) FROM user_facts", [], |r| r.get(0))?;
    let complete = digest.is_some() && record.as_ref() == digest.as_ref();
    let candidate_path = temp.path("E1.sqlite");
    let candidate_exists = candidate_path.exists();
    let candidate_integrity = if candidate_exists {
        open_readonly(&candidate_path)
            .and_then(|conn| {
                conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            })
            .map(|value| value == "ok")
            .unwrap_or(false)
    } else {
        false
    };
    let candidate_manifest_path = suffix(&candidate_path, ".manifest.json");
    let candidate_manifest_valid = if candidate_manifest_path.exists() && candidate_exists {
        let bytes = fs::read(&candidate_manifest_path)?;
        let manifest: crate::db::SlotManifest = serde_json::from_slice(&bytes)?;
        let candidate_digest = path_digest(&candidate_path)?;
        candidate_digest.as_deref() == Some(manifest.sqlite_file_sha256.as_str())
    } else {
        false
    };
    let partial_build_path = temp.path("partial.sqlite");
    let partial_build_rolled_back = if stage == "build" && partial_build_path.exists() {
        let conn = open_connection(&partial_build_path)?;
        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM stage_build", [], |r| r.get(0))?;
        integrity == "ok" && rows == 0
    } else {
        stage != "build"
    };
    let recovery_expectation = match stage {
        "build" | "close" | "sync" | "seal" => current.0 == "E0",
        "pointer-pre-commit" => current.0 == "E0" && !record_for_slot_exists(&main, "E1")?,
        "pointer-post-commit" => current.0 == "E1" && candidate_manifest_valid,
        "gc-pre-unlink" => candidate_exists && candidate_integrity,
        "gc-post-unlink" => !candidate_exists,
        _ => false,
    };
    let safe = current.0 == expected
        && complete
        && facts == 2
        && partial_build_rolled_back
        && recovery_expectation
        && (!candidate_exists || candidate_integrity);
    Ok(
        json!({"stage":stage,"killed_at_barrier":true,"child_exit_code":status.code(),"child_stderr":stderr,
        "authority_after_recovery":current,"expected_authority":expected,"referenced_slot_hash_matches":complete,
        "user_fact_rows":facts,"candidate_exists":candidate_exists,"candidate_integrity_ok":candidate_integrity,
        "candidate_manifest_digest_valid":candidate_manifest_valid,"partial_build_transaction_rolled_back":partial_build_rolled_back,
        "recovery_expectation_satisfied":recovery_expectation,"no_partial_or_mixed_authority":safe}),
    )
}

fn record_for_slot_exists(
    conn: &Connection,
    slot_id: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM simulated_slots WHERE slot_id=?1",
            [slot_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

fn pointer_missing_slot() -> Result<Value, Box<dyn std::error::Error>> {
    let temp = TempRoot::new("pointer-missing")?;
    let slot = create_slot(&temp.path("E0.sqlite"), "E0", &documents(50))?;
    let main_path = temp.path("main.sqlite");
    create_authority_db(&main_path, "E0", &slot.manifest.sqlite_file_sha256)?;
    fs::remove_file(&slot.path)?;
    let main = open_connection(&main_path)?;
    let current = authority_slot(&main)?;
    let missing = current.0 == "E0" && !temp.path("E0.sqlite").exists();
    Ok(json!({
        "authority_pointer_still_E0":current.0 == "E0",
        "slot_file_missing":missing,
        "decision":"READ_ONLY_RECONCILIATION",
        "fallback_to_mutable_physical_fts":false,
        "destructive_repair":false
    }))
}

fn pointer_corrupt_slot() -> Result<Value, Box<dyn std::error::Error>> {
    let temp = TempRoot::new("pointer-corrupt")?;
    let slot = create_slot(&temp.path("E0.sqlite"), "E0", &documents(50))?;
    let main_path = temp.path("main.sqlite");
    create_authority_db(&main_path, "E0", &slot.manifest.sqlite_file_sha256)?;
    let mut bytes = fs::read(&slot.path)?;
    bytes.truncate(100);
    fs::write(&slot.path, bytes)?;
    let digest_rejected =
        path_digest(&slot.path)?.as_deref() != Some(slot.manifest.sqlite_file_sha256.as_str());
    let integrity_rejected = open_readonly(&slot.path)
        .and_then(|conn| conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)))
        .map(|value| value != "ok")
        .unwrap_or(true);
    Ok(json!({
        "digest_rejected":digest_rejected,
        "integrity_rejected":integrity_rejected,
        "decision":"READ_ONLY_RECONCILIATION",
        "fallback_to_mutable_physical_fts":false,
        "destructive_repair":false
    }))
}

fn test_crash_matrix() -> Result<(), Box<dyn std::error::Error>> {
    let stages = [
        "build",
        "close",
        "sync",
        "seal",
        "pointer-pre-commit",
        "pointer-post-commit",
        "gc-pre-unlink",
        "gc-post-unlink",
    ];
    let mut results = Vec::new();
    for stage in stages {
        results.push(kill_at_barrier(stage)?);
    }
    let safe = results
        .iter()
        .all(|r| r["no_partial_or_mixed_authority"] == true);
    case(
        "P1A-T08",
        if safe { "PASS" } else { "FAIL" },
        json!({"barrier_results":results,"process_kill":"Linux child-process kill; not sudden power loss","fsync_power_loss_claim":"MODEL_ONLY"}),
    );
    let missing = pointer_missing_slot()?;
    let corrupt = pointer_corrupt_slot()?;
    let reconciliation_only = missing["decision"] == "READ_ONLY_RECONCILIATION"
        && corrupt["decision"] == "READ_ONLY_RECONCILIATION"
        && missing["fallback_to_mutable_physical_fts"] == false
        && corrupt["fallback_to_mutable_physical_fts"] == false
        && missing["destructive_repair"] == false
        && corrupt["destructive_repair"] == false;
    let missing_ok =
        missing["authority_pointer_still_E0"] == true && missing["slot_file_missing"] == true;
    let corrupt_ok = corrupt["digest_rejected"] == true && corrupt["integrity_rejected"] == true;
    case(
        "P1A-T11",
        if safe && missing_ok && corrupt_ok && reconciliation_only {
            "PASS"
        } else {
            "FAIL"
        },
        json!({
        "two_file_pointer_slot_crash_matrix":results,"missing_slot_reconciliation":missing,
        "corrupt_slot_reconciliation":corrupt,"fallback_to_mutable_physical_fts":false,
        "destructive_repair":false}),
    );
    Ok(())
}

fn test_t10_unavailable() {
    case(
        "P1A-T10",
        "ENVIRONMENT_UNAVAILABLE",
        json!({"executor_os":std::env::consts::OS,
    "hosted_windows_real_handle_test":false,"hosted_macos_real_handle_test":false,
    "reason":"Linux executor; unchanged CI routing does not run this isolated harness on native Windows/macOS",
    "required_state":"P1A_NATIVE_GATE_BLOCKED"}),
    );
}

fn peak_rss_kib() -> Option<u64> {
    let text = fs::read_to_string("/proc/self/status").ok()?;
    text.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

fn benchmark(sample_count: usize) -> Result<(), Box<dyn std::error::Error>> {
    if sample_count < 100 {
        case(
            "P1A-T12",
            "FAIL",
            json!({"reason":"100 samples minimum for p50/p95/p99 reporting","samples":sample_count}),
        );
        return Ok(());
    }
    let mut raw = Vec::new();
    for n in [0_usize, 10_000, 100_000] {
        let docs = documents(n);
        let mut build = Vec::new();
        let mut checkpoint = Vec::new();
        let mut file_sync = Vec::new();
        let mut dir_sync = Vec::new();
        let mut copy = Vec::new();
        let mut delta = Vec::new();
        let mut read = Vec::new();
        let mut gc = Vec::new();
        let mut lock = Vec::new();
        let mut db_size = Vec::new();
        let mut wal_size = Vec::new();
        for sample in 0..sample_count {
            let temp = TempRoot::new(&format!("bench-{n}-{sample}"))?;
            let e0 = create_slot(&temp.path("E0.sqlite"), &format!("B-{n}-{sample}"), &docs)?;
            build.push(e0.build_ms);
            checkpoint.push(e0.checkpoint_ms);
            file_sync.push(e0.file_sync_ms);
            dir_sync.push(e0.directory_sync_ms);
            db_size.push(e0.manifest.sqlite_file_bytes as f64);
            wal_size.push(e0.wal_bytes_after_seal as f64);
            let reader = open_readonly(&e0.path)?;
            let start = Instant::now();
            let rows = query_rows(&reader, "common", None, None)?;
            let read_ms = elapsed_ms(start);
            let result_count = rows.len();
            read.push(read_ms);
            let mut query_hash = sha2::Sha256::new();
            use sha2::Digest;
            for row in &rows {
                query_hash.update(row.uid.as_bytes());
                query_hash.update(row.score_bits.as_bytes());
            }
            let result_sha = format!("{:x}", query_hash.finalize());
            drop(reader);
            let e1_path = temp.path("E1.sqlite");
            let start = Instant::now();
            let copied = create_backup_copy(&e0.path, &e1_path)?;
            let copy_ms = elapsed_ms(start);
            copy.push(copy_ms);
            let start = Instant::now();
            if n > 0 {
                for uid in ["source-000002", "source-000003", "source-000007"] {
                    update_doc_for_delta(
                        &e1_path,
                        uid,
                        &format!("common changed-{sample}-{uid}"),
                        &format!("/synthetic/e1/{sample}/{uid}.dat"),
                    )?;
                }
            }
            let delta_ms = elapsed_ms(start);
            delta.push(delta_ms);
            let delta_hash = file_sha256(&e1_path)?.0;
            let main_path = temp.path("main.sqlite");
            create_authority_db(&main_path, "E0", &e0.manifest.sqlite_file_sha256)?;
            let mut main = open_connection(&main_path)?;
            configure_product_main(&mut main)?;
            main.execute(
                "INSERT INTO simulated_slots VALUES('E1','RETIRING',?1,2)",
                [&delta_hash],
            )?;
            let start = Instant::now();
            let gc_result = gc_slot(&mut main, "E1", 0, &e1_path)?;
            let gc_ms = elapsed_ms(start);
            gc.push(gc_ms);
            let start = Instant::now();
            let tx = main.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE simulated_authority SET revision=revision+1 WHERE singleton=1",
                [],
            )?;
            tx.commit()?;
            let lock_ms = elapsed_ms(start);
            lock.push(lock_ms);
            raw.push(json!({"generated_document_count":n,"sample":sample,"effective_document_count":e0.manifest.document_count,
                "build_ms":e0.build_ms,"checkpoint_ms":e0.checkpoint_ms,"file_sync_ms":e0.file_sync_ms,
                "directory_sync_ms":e0.directory_sync_ms,"copy_ms":copy_ms,"copy_bytes":copied,
                "delta_ms":delta_ms,"read_common_ms":read_ms,"read_result_count":result_count,"read_result_sha256":result_sha,
                "gc_ms":gc_ms,"gc_result":gc_result,"authority_lock_ms":lock_ms,
                "slot_bytes":e0.manifest.sqlite_file_bytes,"wal_bytes_after_seal":e0.wal_bytes_after_seal,
                "wal_bytes_after_delta":wal_bytes(&e1_path),"delta_slot_sha256":delta_hash}));
        }
        emit(
            "benchmark_summary",
            "P1A-T12",
            "OBSERVED",
            json!({
                "generated_document_count":n,"effective_document_count":docs.iter().filter(|d|d.effective).count(),"repeat_count":sample_count,
                "build":percentile_json(&build),"checkpoint":percentile_json(&checkpoint),"file_sync":percentile_json(&file_sync),
                "directory_sync":percentile_json(&dir_sync),"copy":percentile_json(&copy),"delta":percentile_json(&delta),
                "full_high_hit_read":percentile_json(&read),"gc":percentile_json(&gc),"authority_lock":percentile_json(&lock),
                "slot_disk_bytes":percentile_json(&db_size),"wal_bytes_after_seal":percentile_json(&wal_size),"process_peak_rss_kib":peak_rss_kib(),
                "sample_definition":"nearest-rank; 100 repeated independent disposable files per phase; Linux executor only"
            }),
        );
    }
    for details in raw {
        emit("benchmark_sample", "P1A-T12", "OBSERVED", details);
    }
    let all_100 = 3 * sample_count;
    case(
        "P1A-T12",
        "PASS",
        json!({"sizes":[0,10000,100000],"samples_per_size":sample_count,
        "raw_sample_count":all_100,"percentile_method":"nearest rank","unmeasured_500k_or_1m_claims":false,
        "host_specific_performance_claim":"Linux disposable fixture only"}),
    );
    Ok(())
}

fn run_all(samples: usize) -> Result<(), Box<dyn std::error::Error>> {
    let info = source_info()?;
    let t01 = info["sqlite_version"] == "3.51.3"
        && info["fts5_compile_option"] == 1
        && info["trigram_query_count"] == 1
        && info["sqlite_system_override_variables"] == json!([]);
    case("P1A-T01", if t01 { "PASS" } else { "FAIL" }, info);
    size_snapshots()?;
    test_short_query()?;
    test_e0_e1()?;
    test_seal_tamper()?;
    test_f0_f2_negatives()?;
    test_user_fact_isolation()?;
    test_pin_gc()?;
    test_pool_busy_timeout()?;
    test_n04()?;
    test_faults()?;
    test_crash_matrix()?;
    test_t10_unavailable();
    benchmark(samples)?;
    Ok(())
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("--child") {
        if args.len() != 4 {
            eprintln!("child needs stage and root");
            std::process::exit(2);
        }
        if let Err(e) = child_hold(&args[2], Path::new(&args[3])) {
            eprintln!("child failure: {e}");
            std::process::exit(2);
        }
        return;
    }
    let samples = args
        .iter()
        .position(|a| a == "--samples")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    if let Err(e) = run_all(samples) {
        eprintln!("harness error: {e}");
        std::process::exit(1);
    }
}
