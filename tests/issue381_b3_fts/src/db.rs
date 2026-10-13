use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, time::Instant};

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Doc {
    pub rowid: i64,
    pub uid: String,
    pub name: String,
    pub path: String,
    pub physical: String,
    pub observation: i64,
    pub mtime: i64,
    pub roots: Vec<String>,
    pub active: bool,
}
#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Root {
    pub rid: String,
    pub enabled: bool,
    pub health: String,
}
#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Fixture {
    pub seed: i64,
    pub roots: Vec<Root>,
    pub docs: Vec<Doc>,
    pub queries: Vec<String>,
}

pub fn open(path: &Path) -> Connection {
    let c = Connection::open(path).unwrap();
    c.busy_timeout(std::time::Duration::from_millis(5000))
        .unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .unwrap();
    c
}
pub fn index_schema(c: &Connection) {
    c.execute_batch("CREATE TABLE files(uid TEXT UNIQUE NOT NULL,name TEXT NOT NULL,path TEXT NOT NULL,physical TEXT NOT NULL,observation INTEGER NOT NULL,active INTEGER NOT NULL,mtime INTEGER NOT NULL DEFAULT 0,is_stale INTEGER NOT NULL DEFAULT 0,user_note TEXT NOT NULL DEFAULT 'retained note');").unwrap();
    c.execute_batch(include_str!("product-index.sql")).unwrap();
}
pub fn insert(c: &Connection, d: &Doc) {
    c.execute("INSERT INTO files(rowid,uid,name,path,physical,observation,active,mtime,is_stale) VALUES(?,?,?,?,?,?,?,?,?)",params![d.rowid,d.uid,d.name,d.path,d.physical,d.observation,d.active,d.mtime,!d.active]).unwrap();
}
pub fn create(c: &Connection, f: &Fixture) -> Value {
    index_schema(c);
    c.execute_batch("CREATE TABLE roots(rid TEXT PRIMARY KEY,enabled INTEGER NOT NULL,health TEXT NOT NULL); CREATE TABLE claims(rid TEXT,uid TEXT,observation INTEGER,PRIMARY KEY(rid,uid)); CREATE TABLE authority(id INTEGER PRIMARY KEY,epoch INTEGER,revision INTEGER,writer INTEGER,text_revision INTEGER,root_revision INTEGER); INSERT INTO authority VALUES(1,0,0,0,0,0); CREATE TABLE generations(epoch INTEGER PRIMARY KEY,state TEXT,writer INTEGER,text_revision INTEGER,root_revision INTEGER,payload_digest TEXT,n INTEGER,tokens INTEGER); CREATE TABLE tags(uid TEXT REFERENCES files(uid),tag TEXT,PRIMARY KEY(uid,tag)); INSERT INTO roots SELECT 'placeholder',0,'disabled'; DELETE FROM roots;").unwrap();
    for r in &f.roots {
        c.execute(
            "INSERT INTO roots VALUES(?,?,?)",
            params![r.rid, r.enabled, r.health],
        )
        .unwrap();
    }
    let t = Instant::now();
    let mut batches = Vec::new();
    for batch in f.docs.chunks(1000) {
        let start = Instant::now();
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        for d in batch {
            insert(c, d);
            for root in &d.roots {
                c.execute(
                    "INSERT INTO claims VALUES(?,?,?)",
                    params![root, d.uid, d.observation],
                )
                .unwrap();
            }
        }
        c.execute_batch("COMMIT").unwrap();
        batches.push(json!({"rows":batch.len(),"ms":start.elapsed().as_secs_f64()*1000.0}));
    }
    json!({"phase":"physical_fixture_prepare","elapsed_ms":t.elapsed().as_secs_f64()*1000.0,"batches":batches,"storage":storage(c)})
}
pub fn documents(c: &Connection) -> Vec<Doc> {
    let mut s = c
        .prepare("SELECT rowid,uid,name,path,physical,observation,active,mtime FROM files ORDER BY rowid")
        .unwrap();
    s.query_map([], |r| {
        Ok(Doc {
            rowid: r.get(0)?,
            uid: r.get(1)?,
            name: r.get(2)?,
            path: r.get(3)?,
            physical: r.get(4)?,
            observation: r.get(5)?,
            active: r.get(6)?,
            mtime: r.get(7)?,
            roots: Vec::new(),
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}
pub fn visible(c: &Connection, scope: Option<&str>) -> BTreeSet<i64> {
    let mut s=c.prepare("SELECT DISTINCT f.rowid FROM files f JOIN claims m ON m.uid=f.uid AND m.observation=f.observation JOIN roots r ON r.rid=m.rid WHERE f.active=1 AND r.enabled=1 AND (? IS NULL OR r.rid=?) ORDER BY f.rowid").unwrap();
    s.query_map(params![scope, scope], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
pub fn claims(c: &Connection) -> Vec<(String, String, i64)> {
    c.prepare("SELECT rid,uid,observation FROM claims ORDER BY rid,uid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
pub fn authority(c: &Connection) -> (i64, i64, i64, i64, i64) {
    c.query_row(
        "SELECT epoch,revision,writer,text_revision,root_revision FROM authority WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .unwrap()
}
pub fn hash<T: Serialize>(value: &T) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}
pub fn docsize(c: &Connection, id: i64) -> i64 {
    let blob: Vec<u8> = c
        .query_row("SELECT sz FROM files_fts_docsize WHERE id=?", [id], |r| {
            r.get(0)
        })
        .unwrap();
    let mut n = 0;
    let mut total = 0;
    // FTS5's source-derived private docsize varint representation: source parity is
    // recorded, but exposing this internal table is an explicit production gate.
    for b in blob {
        n = (n << 7) | i64::from(b & 0x7f);
        if b & 0x80 == 0 {
            total += n;
            n = 0;
        }
    }
    total
}
pub fn stats(c: &Connection, ids: &BTreeSet<i64>) -> (i64, i64) {
    (ids.len() as i64, ids.iter().map(|id| docsize(c, *id)).sum())
}
pub fn storage(c: &Connection) -> Value {
    let page: i64 = c.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
    let pages: i64 = c.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap();
    let sizes: Vec<(String, i64)> = c
        .prepare("SELECT name,sum(pgsize) FROM dbstat GROUP BY name ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let path: String = c
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    json!({"page_size":page,"page_count":pages,"db_bytes":page*pages,"wal_bytes":std::fs::metadata(format!("{path}-wal")).map(|m|m.len()).unwrap_or(0),"btrees":sizes,"busy_timeout_ms":5000,"synchronous":"FULL","journal":"WAL"})
}
pub fn plan(c: &Connection, sql: &str, query: &str) -> Value {
    let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows: Vec<(i64, i64, String)> = s
        .query_map([query], |r| Ok((r.get(0)?, r.get(1)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    json!(rows)
}
pub fn qform(raw: &str) -> String {
    raw.split_whitespace()
        .take(12)
        .map(|p| format!("\"{}\"", p.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn user_data(c: &Connection) -> Value {
    let notes: Vec<(String, String)> = c
        .prepare("SELECT uid,user_note FROM files ORDER BY uid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let tags: Vec<(String, String)> = c
        .prepare("SELECT uid,tag FROM tags ORDER BY uid,tag")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    json!({"source_ids_and_observations":documents(c),"notes":notes,"tags":tags})
}
