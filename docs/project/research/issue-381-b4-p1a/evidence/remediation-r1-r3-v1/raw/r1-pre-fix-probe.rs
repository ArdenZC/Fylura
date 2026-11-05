#[path = "../src/db.rs"] mod db;
#[path = "../src/model.rs"] mod model;
use rusqlite::Connection;
fn main() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE slot_docs(rowid INTEGER PRIMARY KEY,uid TEXT,mtime INTEGER,name TEXT,path TEXT); CREATE TABLE slot_members(uid TEXT,root_id TEXT); CREATE VIRTUAL TABLE slot_fts USING fts5(name,path,content='slot_docs',content_rowid='rowid',tokenize='trigram'); INSERT INTO slot_docs VALUES(1,'u1',1,'common one','/one'); INSERT INTO slot_fts(rowid,name,path) VALUES(1,'common one','/one');").unwrap();
    match db::query_rows(&conn, "common", None, Some(1)) {
        Ok(rows) => { println!("UNEXPECTED_SUCCESS rows={}", rows.len()); std::process::exit(1); }
        Err(error) => { println!("EXPECTED_ORIGINAL_FAILURE query_rows(root=None, limit=Some(1)): {error}"); }
    }
}
