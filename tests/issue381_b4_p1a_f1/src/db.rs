use crate::model::{Document, file_sha256};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const PRODUCT_BUSY_TIMEOUT_MS: u64 = 5_000;
pub const PRODUCT_POOL_SIZE: u32 = 8;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SlotManifest {
    pub format_version: u32,
    pub slot_id: String,
    pub document_count: usize,
    pub corpus_sha256: String,
    pub sqlite_file_sha256: String,
    pub sqlite_file_bytes: u64,
    pub source_id: String,
    pub root_revision: i64,
    pub source_revision: i64,
    pub sealed_stage: String,
}

pub struct ManifestExpectation<'a> {
    pub format_version: u32,
    pub slot_id: &'a str,
    pub document_count: usize,
    pub corpus_sha256: &'a str,
    pub sqlite_file_sha256: &'a str,
    pub sqlite_file_bytes: u64,
    pub source_id: &'a str,
    pub root_revision: i64,
    pub source_revision: i64,
    pub sealed_stage: &'a str,
    pub manifest_sha256: &'a str,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct QueryRow {
    pub uid: String,
    pub score_bits: String,
    pub mtime: i64,
    pub name: String,
    pub path: String,
}

pub struct BuiltSlot {
    pub path: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest: SlotManifest,
    pub manifest_sha256: String,
    pub build_ms: f64,
    pub checkpoint_ms: f64,
    pub file_sync_ms: f64,
    pub directory_sync_ms: f64,
    pub wal_bytes_after_seal: u64,
}

pub fn open_connection(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open(path)
}

pub fn configure_product_main(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 3_000_000_000_i64)?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    Ok(())
}

pub fn open_product_pool(
    path: &Path,
) -> Result<Pool<SqliteConnectionManager>, Box<dyn std::error::Error>> {
    let manager =
        SqliteConnectionManager::file(path).with_init(|conn| configure_product_main(conn));
    Ok(Pool::builder().max_size(PRODUCT_POOL_SIZE).build(manager)?)
}

pub fn query_pool_settings(conn: &Connection) -> rusqlite::Result<(String, i64, i64, i64, i64)> {
    Ok((
        conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?,
        conn.query_row("PRAGMA synchronous", [], |r| r.get(0))?,
        conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?,
        conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?,
        conn.query_row("PRAGMA mmap_size", [], |r| r.get(0))?,
    ))
}

pub fn create_slot(
    path: &Path,
    slot_id: &str,
    docs: &[Document],
) -> Result<BuiltSlot, Box<dyn std::error::Error>> {
    let start = Instant::now();
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "cache_size", -131_072_i64)?;
    conn.execute_batch(
        r#"
        CREATE TABLE slot_docs(
          rowid INTEGER PRIMARY KEY,
          uid TEXT NOT NULL UNIQUE,
          observation_revision INTEGER NOT NULL,
          name TEXT NOT NULL,
          path TEXT NOT NULL,
          mtime INTEGER NOT NULL,
          root_claims TEXT NOT NULL
        );
        CREATE TABLE slot_members(
          uid TEXT NOT NULL,
          root_id TEXT NOT NULL,
          PRIMARY KEY(uid, root_id)
        ) WITHOUT ROWID;
        CREATE INDEX idx_slot_members_root_uid ON slot_members(root_id,uid);
        CREATE VIRTUAL TABLE slot_fts USING fts5(
          name,
          path,
          content='slot_docs',
          content_rowid='rowid',
          tokenize='trigram'
        );
        CREATE TRIGGER slot_docs_ai AFTER INSERT ON slot_docs BEGIN
          INSERT INTO slot_fts(rowid,name,path) VALUES(new.rowid,new.name,new.path);
        END;
        CREATE TRIGGER slot_docs_ad AFTER DELETE ON slot_docs BEGIN
          INSERT INTO slot_fts(slot_fts,rowid,name,path)
          VALUES('delete',old.rowid,old.name,old.path);
        END;
        CREATE TRIGGER slot_docs_au AFTER UPDATE ON slot_docs BEGIN
          INSERT INTO slot_fts(slot_fts,rowid,name,path)
          VALUES('delete',old.rowid,old.name,old.path);
          INSERT INTO slot_fts(rowid,name,path) VALUES(new.rowid,new.name,new.path);
        END;
        "#,
    )?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    {
        let mut insert_doc = transaction.prepare_cached(
            "INSERT INTO slot_docs(rowid,uid,observation_revision,name,path,mtime,root_claims) \
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
        )?;
        let mut insert_member =
            transaction.prepare_cached("INSERT INTO slot_members(uid,root_id) VALUES(?1,?2)")?;
        let mut rowid = 0_i64;
        for doc in docs.iter().filter(|doc| doc.effective) {
            rowid += 1;
            let claims = doc.root_claims.join(",");
            insert_doc.execute(params![
                rowid,
                doc.uid,
                doc.observation_revision,
                doc.name,
                doc.path,
                doc.mtime,
                claims
            ])?;
            for root in doc.root_claims.iter().filter(|root| {
                root.as_str() == "root-a-healthy" || root.as_str() == "root-b-degraded-retained"
            }) {
                insert_member.execute(params![doc.uid, root])?;
            }
        }
    }
    transaction.commit()?;
    conn.execute(
        "INSERT INTO slot_fts(slot_fts,rank) VALUES('integrity-check',1)",
        [],
    )?;
    let build_ms = start.elapsed().as_secs_f64() * 1_000.0;
    let checkpoint = Instant::now();
    let checkpoint_state: (i64, i64, i64) =
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    let checkpoint_ms = checkpoint.elapsed().as_secs_f64() * 1_000.0;
    if checkpoint_state.0 != 0 || checkpoint_state.1 != checkpoint_state.2 {
        return Err(format!("slot checkpoint was incomplete: {checkpoint_state:?}").into());
    }
    drop(conn);
    let wal_path = suffix(path, "-wal");
    let wal_bytes_after_seal = std::fs::metadata(wal_path).map(|m| m.len()).unwrap_or(0);
    let file_sync = Instant::now();
    std::fs::File::open(path)?.sync_all()?;
    let file_sync_ms = file_sync.elapsed().as_secs_f64() * 1_000.0;
    let parent = path.parent().ok_or("slot parent missing")?;
    let dir_sync = Instant::now();
    std::fs::File::open(parent)?.sync_all()?;
    let directory_sync_ms = dir_sync.elapsed().as_secs_f64() * 1_000.0;
    let (sqlite_file_sha256, sqlite_file_bytes) = file_sha256(path)?;
    let source_id = {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.query_row("SELECT sqlite_source_id()", [], |row| row.get(0))?
    };
    let manifest = SlotManifest {
        format_version: 2,
        slot_id: slot_id.to_string(),
        document_count: docs.iter().filter(|doc| doc.effective).count(),
        corpus_sha256: crate::model::corpus_sha256(docs),
        sqlite_file_sha256,
        sqlite_file_bytes,
        source_id,
        root_revision: 1,
        source_revision: docs
            .iter()
            .map(|doc| i64::from(doc.observation_revision))
            .max()
            .unwrap_or(0),
        sealed_stage: "SEALED_UNPUBLISHED".to_string(),
    };
    let manifest_path = suffix(path, ".manifest.json");
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let manifest_tmp = suffix(path, ".manifest.tmp");
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&manifest_tmp)?;
        file.write_all(&manifest_bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&manifest_tmp, &manifest_path)?;
    std::fs::File::open(parent)?.sync_all()?;
    let manifest_sha256 = sha256_bytes(&manifest_bytes);
    Ok(BuiltSlot {
        path: path.to_path_buf(),
        manifest_path,
        manifest,
        manifest_sha256,
        build_ms,
        checkpoint_ms,
        file_sync_ms,
        directory_sync_ms,
        wal_bytes_after_seal,
    })
}

pub fn create_unsealed_slot(
    path: &Path,
    docs: &[Document],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.execute_batch(
        r#"
        CREATE TABLE slot_docs(rowid INTEGER PRIMARY KEY,uid TEXT UNIQUE NOT NULL,
          observation_revision INTEGER NOT NULL,name TEXT NOT NULL,path TEXT NOT NULL,
          mtime INTEGER NOT NULL,root_claims TEXT NOT NULL);
        CREATE TABLE slot_members(uid TEXT NOT NULL,root_id TEXT NOT NULL,
          PRIMARY KEY(uid,root_id)) WITHOUT ROWID;
        CREATE VIRTUAL TABLE slot_fts USING fts5(name,path,content='slot_docs',
          content_rowid='rowid',tokenize='trigram');
        CREATE TRIGGER slot_docs_ai AFTER INSERT ON slot_docs BEGIN
          INSERT INTO slot_fts(rowid,name,path) VALUES(new.rowid,new.name,new.path); END;
        CREATE TRIGGER slot_docs_ad AFTER DELETE ON slot_docs BEGIN
          INSERT INTO slot_fts(slot_fts,rowid,name,path) VALUES('delete',old.rowid,old.name,old.path); END;
        CREATE TRIGGER slot_docs_au AFTER UPDATE ON slot_docs BEGIN
          INSERT INTO slot_fts(slot_fts,rowid,name,path) VALUES('delete',old.rowid,old.name,old.path);
          INSERT INTO slot_fts(rowid,name,path) VALUES(new.rowid,new.name,new.path); END;
        "#,
    )?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    {
        let mut doc = tx.prepare_cached("INSERT INTO slot_docs(rowid,uid,observation_revision,name,path,mtime,root_claims) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
        let mut member =
            tx.prepare_cached("INSERT INTO slot_members(uid,root_id) VALUES(?1,?2)")?;
        let mut rowid = 0_i64;
        for value in docs.iter().filter(|d| d.effective) {
            rowid += 1;
            doc.execute(params![
                rowid,
                value.uid,
                value.observation_revision,
                value.name,
                value.path,
                value.mtime,
                value.root_claims.join(",")
            ])?;
            for root in value.root_claims.iter().filter(|r| {
                r.as_str() == "root-a-healthy" || r.as_str() == "root-b-degraded-retained"
            }) {
                member.execute(params![value.uid, root])?;
            }
        }
    }
    tx.commit()?;
    conn.execute(
        "INSERT INTO slot_fts(slot_fts,rank) VALUES('integrity-check',1)",
        [],
    )?;
    let _: (i64, i64, i64) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    drop(conn);
    Ok(())
}

pub fn open_readonly(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.pragma_update(None, "query_only", "ON")?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    Ok(conn)
}

pub fn query_rows(
    conn: &Connection,
    query: &str,
    root: Option<&str>,
    limit: Option<i64>,
) -> rusqlite::Result<Vec<QueryRow>> {
    query_rows_window(conn, query, root, limit.map(|limit| (limit, 0)))
}

pub fn query_rows_page(
    conn: &Connection,
    query: &str,
    root: Option<&str>,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<QueryRow>> {
    query_rows_window(conn, query, root, Some((limit, offset)))
}

fn query_rows_window(
    conn: &Connection,
    query: &str,
    root: Option<&str>,
    window: Option<(i64, i64)>,
) -> rusqlite::Result<Vec<QueryRow>> {
    let mut sql = String::from(
        "SELECT d.uid,bm25(slot_fts,6.0,1.5),d.mtime,d.name,d.path \
         FROM slot_fts JOIN slot_docs d ON d.rowid=slot_fts.rowid \
         WHERE slot_fts MATCH ?1",
    );
    if root.is_some() {
        sql.push_str(
            " AND EXISTS(SELECT 1 FROM slot_members m WHERE m.uid=d.uid AND m.root_id=?2)",
        );
    }
    sql.push_str(" ORDER BY 2 ASC,d.mtime DESC,d.name COLLATE NOCASE ASC,d.uid ASC");
    if let Some((_, _)) = window {
        if root.is_some() {
            sql.push_str(" LIMIT ?3 OFFSET ?4");
        } else {
            sql.push_str(" LIMIT ?2 OFFSET ?3");
        }
    }
    let match_text = format!("\"{}\"", query.replace('"', ""));
    let mut statement = conn.prepare(&sql)?;
    let mut rows = match (root, window) {
        (Some(root), Some((limit, offset))) => {
            statement.query(params![match_text, root, limit, offset])?
        }
        (Some(root), None) => statement.query(params![match_text, root])?,
        (None, Some((limit, offset))) => statement.query(params![match_text, limit, offset])?,
        (None, None) => statement.query(params![match_text])?,
    };
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let score: f64 = row.get(1)?;
        result.push(QueryRow {
            uid: row.get(0)?,
            score_bits: format!("{:016x}", score.to_bits()),
            mtime: row.get(2)?,
            name: row.get(3)?,
            path: row.get(4)?,
        });
    }
    Ok(result)
}

pub fn verify_manifest_file(
    slot_path: &Path,
    manifest_path: &Path,
    expected: &ManifestExpectation<'_>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(manifest_path)?;
    let manifest: SlotManifest = serde_json::from_slice(&bytes)?;
    let manifest_sha256 = sha256_bytes(&bytes);
    if manifest.format_version != expected.format_version {
        return Err(format!(
            "manifest format version mismatch: {}",
            manifest.format_version
        )
        .into());
    }
    if manifest.slot_id != expected.slot_id {
        return Err(format!("manifest slot ID mismatch: {}", manifest.slot_id).into());
    }
    if manifest.document_count != expected.document_count {
        return Err(format!(
            "manifest document count mismatch: {}",
            manifest.document_count
        )
        .into());
    }
    if manifest.corpus_sha256 != expected.corpus_sha256 {
        return Err("manifest corpus digest mismatch".into());
    }
    if manifest.root_revision != expected.root_revision {
        return Err(format!(
            "manifest root revision mismatch: {}",
            manifest.root_revision
        )
        .into());
    }
    if manifest.source_revision != expected.source_revision {
        return Err(format!(
            "manifest source revision mismatch: {}",
            manifest.source_revision
        )
        .into());
    }
    if manifest.sealed_stage != expected.sealed_stage {
        return Err(format!("manifest stage mismatch: {}", manifest.sealed_stage).into());
    }
    let (actual_sha, actual_bytes) = file_sha256(slot_path)?;
    if manifest.sqlite_file_sha256 != actual_sha || expected.sqlite_file_sha256 != actual_sha {
        return Err("manifest SQLite file hash mismatch".into());
    }
    if manifest.sqlite_file_bytes != actual_bytes || expected.sqlite_file_bytes != actual_bytes {
        return Err(format!("manifest SQLite file length mismatch: {actual_bytes}").into());
    }
    let conn = open_readonly(slot_path)?;
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM slot_docs", [], |row| row.get(0))?;
    let source_id: String = conn.query_row("SELECT sqlite_source_id()", [], |row| row.get(0))?;
    if count != expected.document_count as i64
        || manifest.source_id != source_id
        || expected.source_id != source_id
    {
        return Err("manifest SQLite content count or source ID mismatch".into());
    }
    if integrity != "ok" {
        return Err(format!("SQLite integrity check failed: {integrity}").into());
    }
    if manifest_sha256 != expected.manifest_sha256 {
        return Err("manifest SHA-256 mismatch".into());
    }
    Ok(serde_json::json!({
        "manifest_digest_matches": true,
        "file_digest_matches": true,
        "file_bytes": actual_bytes,
        "integrity_check": integrity,
        "reopened_readonly": true,
        "document_count": count,
        "manifest": manifest,
        "manifest_sha256": manifest_sha256,
        "file_sha256": actual_sha,
        "root_revision_bound": expected.root_revision,
        "source_revision_bound": expected.source_revision
    }))
}

pub fn verify_seal(slot: &BuiltSlot) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let expectation = ManifestExpectation {
        format_version: slot.manifest.format_version,
        slot_id: &slot.manifest.slot_id,
        document_count: slot.manifest.document_count,
        corpus_sha256: &slot.manifest.corpus_sha256,
        sqlite_file_sha256: &slot.manifest.sqlite_file_sha256,
        sqlite_file_bytes: slot.manifest.sqlite_file_bytes,
        source_id: &slot.manifest.source_id,
        root_revision: slot.manifest.root_revision,
        source_revision: slot.manifest.source_revision,
        sealed_stage: &slot.manifest.sealed_stage,
        manifest_sha256: &slot.manifest_sha256,
    };
    let mut details = verify_manifest_file(&slot.path, &slot.manifest_path, &expectation)?;
    let conn = open_readonly(&slot.path)?;
    let readonly_write_rejected = conn
        .execute(
            "INSERT INTO slot_docs(uid,observation_revision,name,path,mtime,root_claims) \
             VALUES('should-not-write',1,'x','/x',0,'')",
            [],
        )
        .is_err();
    let object = details
        .as_object_mut()
        .ok_or("manifest verification result is not an object")?;
    object.insert(
        "wal_bytes_after_seal".into(),
        serde_json::json!(slot.wal_bytes_after_seal),
    );
    object.insert(
        "readonly_write_rejected".into(),
        serde_json::json!(readonly_write_rejected),
    );
    object.insert(
        "checkpoint_ms".into(),
        serde_json::json!(slot.checkpoint_ms),
    );
    object.insert("file_sync_ms".into(), serde_json::json!(slot.file_sync_ms));
    object.insert(
        "directory_sync_ms".into(),
        serde_json::json!(slot.directory_sync_ms),
    );
    Ok(details)
}

pub fn create_authority_db(path: &Path, slot_id: &str, slot_sha256: &str) -> rusqlite::Result<()> {
    let mut conn = open_connection(path)?;
    configure_product_main(&mut conn)?;
    conn.execute_batch(
        r#"
        CREATE TABLE simulated_authority(
          singleton INTEGER PRIMARY KEY CHECK(singleton=1),
          catalog_epoch INTEGER NOT NULL,
          slot_id TEXT NOT NULL,
          revision INTEGER NOT NULL
        );
        CREATE TABLE simulated_slots(
          slot_id TEXT PRIMARY KEY,
          state TEXT NOT NULL,
          slot_sha256 TEXT NOT NULL,
          root_revision INTEGER NOT NULL
        );
        CREATE TABLE simulated_leases(
          lease_id TEXT PRIMARY KEY,
          slot_id TEXT NOT NULL,
          expires_at INTEGER NOT NULL,
          process_id INTEGER NOT NULL,
          active INTEGER NOT NULL CHECK(active IN (0,1))
        );
        CREATE TABLE user_facts(
          legacy_file_id TEXT NOT NULL,
          fact_key TEXT NOT NULL,
          fact_value TEXT NOT NULL,
          PRIMARY KEY(legacy_file_id,fact_key)
        );
        "#,
    )?;
    conn.execute(
        "INSERT INTO simulated_authority(singleton,catalog_epoch,slot_id,revision) VALUES(1,1,?1,1)",
        [slot_id],
    )?;
    conn.execute(
        "INSERT INTO simulated_slots(slot_id,state,slot_sha256,root_revision) VALUES(?1,'PUBLISHED',?2,1)",
        params![slot_id,slot_sha256],
    )?;
    conn.execute_batch(
        "INSERT INTO user_facts VALUES('legacy-file-001','tag','keep-me');\
         INSERT INTO user_facts VALUES('legacy-file-001','note','retain historical fact');",
    )?;
    Ok(())
}

pub fn transition_slot(
    conn: &mut Connection,
    slot_id: &str,
    expected_state: &str,
    next_state: &str,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "UPDATE simulated_slots SET state=?1 WHERE slot_id=?2 AND state=?3",
        params![next_state, slot_id, expected_state],
    )?;
    tx.commit()?;
    Ok(changed == 1)
}

pub fn authority_slot(conn: &Connection) -> rusqlite::Result<(String, i64, i64)> {
    conn.query_row(
        "SELECT slot_id,catalog_epoch,revision FROM simulated_authority WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
}

pub fn pool_get(
    pool: &Pool<SqliteConnectionManager>,
) -> Result<PooledConnection<SqliteConnectionManager>, Box<dyn std::error::Error>> {
    Ok(pool.get()?)
}

pub fn suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut text = path.as_os_str().to_os_string();
    text.push(suffix);
    PathBuf::from(text)
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn path_digest(path: &Path) -> rusqlite::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    file_sha256(path)
        .map(|(sha, _)| Some(sha))
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

pub fn claim_live_lease_count(conn: &Connection, slot_id: &str, now: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM simulated_leases WHERE slot_id=?1 AND active=1 AND expires_at>?2",
        params![slot_id, now],
        |row| row.get(0),
    )
}

pub fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1_000.0
}

pub fn wal_bytes(path: &Path) -> u64 {
    std::fs::metadata(suffix(path, "-wal"))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

pub fn file_system_sync(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

pub fn update_doc_for_delta(
    path: &Path,
    uid: &str,
    new_name: &str,
    new_path: &str,
) -> rusqlite::Result<()> {
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = tx.execute(
        "UPDATE slot_docs SET name=?1,path=?2,observation_revision=observation_revision+1 \
         WHERE uid=?3",
        params![new_name, new_path, uid],
    )?;
    if updated != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    tx.commit()?;
    conn.execute(
        "INSERT INTO slot_fts(slot_fts,rank) VALUES('integrity-check',1)",
        [],
    )?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    drop(conn);
    Ok(())
}

pub fn read_only_slot_tx(path: &Path) -> rusqlite::Result<Connection> {
    let conn = open_readonly(path)?;
    conn.execute_batch("BEGIN")?;
    let _: i64 = conn.query_row("SELECT COUNT(*) FROM slot_docs", [], |row| row.get(0))?;
    Ok(conn)
}

pub fn create_backup_copy(source: &Path, dest: &Path) -> std::io::Result<u64> {
    std::fs::copy(source, dest)
}
