use super::*;
use crate::db::{schema, Database};

struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn fixture() -> (Fixture, Database) {
    let path = Fixture(
        std::env::temp_dir().join(format!("zc380-migration-{}.sqlite", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&path.0).unwrap();
    (path, db)
}

fn legacy_snapshot(conn: &Connection) -> Vec<Vec<String>> {
    [
        "global_volumes",
        "global_entries",
        "managed_scopes",
        "managed_entries",
        "ai_jobs",
        "ai_job_items",
        "ai_analysis_state",
        "files",
        "user_tags",
        "file_user_tags",
    ]
    .iter()
    .map(|table| {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |r| {
            Ok((0..columns)
                .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    })
    .collect()
}

fn populate41(conn: &Connection) {
    drop_for_test(conn);
    // Only the additive 42 namespace was removed: this is the actual previous
    // product schema, not a miniature model of migration tables.
    conn.execute_batch("PRAGMA user_version=41;
        INSERT INTO global_volumes(id,platform,stable_volume_id,display_name,mount_path,filesystem_type,
            drive_kind,provider,index_status,last_error,journal_id,journal_cursor,last_full_index_at,
            entry_count,created_at,updated_at,operation_revision,operation_owner_id,operation_owner_pid,
            operation_owner_started_at,operation_owner_route,operation_kind,operation_state,
            rebuild_intent_revision,rebuild_completed_revision)
        VALUES('v','windows','stable','V','C:/','ntfs','fixed','windows_mft_usn','ready',
            'preserved-real-error','journal','cursor',99,0,1,2,7,'unknown-service',123,456,
            'service','rebuild','unknown',3,2);
        INSERT INTO global_entries(id,volume_id,platform_file_id,name,name_normalized,path,path_normalized,
            extension,source_provider,last_seen_at)
        VALUES('entry','v','native','legacytoken.txt','legacytoken.txt','C:/legacytoken.txt',
            'c:/legacytoken.txt','txt','windows_mft_usn',3);
        INSERT INTO managed_scopes(id,path,created_at,updated_at) VALUES('scope','C:/',1,1);
        INSERT INTO managed_entries(id,global_entry_id,managed_scope_id,created_at,updated_at)
            VALUES('me','entry','scope',1,1);
        INSERT INTO ai_analysis_state(global_entry_id,status,classification_json,user_corrected,updated_at)
            VALUES('entry','completed','{\"user\":true}',1,1);
        INSERT INTO ai_jobs(id,global_entry_id,managed_scope_id,input_fingerprint,provider,status,created_at)
            VALUES('job','entry','scope','fingerprint','local','canceled',1);
        INSERT INTO ai_job_items(id,job_id,global_entry_id,status,created_at,updated_at)
            VALUES('item','job','entry','canceled',1,1);
        INSERT INTO files(id,path,name,extension,size,mtime,is_dir,state_code)
            VALUES('f','C:/tagged.txt','tagged.txt','txt',8,1,0,0);
        INSERT INTO user_tags(id,display_name,normalized_name,color_token,created_at,updated_at,revision)
            VALUES('tag','Tag','tag','blue',1,1,1);
        INSERT INTO file_user_tags(file_id,tag_id,created_at) VALUES('f','tag',1);") .unwrap();
}

#[test]
fn schema42_populated41_upgrade_preserves_all_legacy_facts_and_marks_no_completeness() {
    let (path, db) = fixture();
    let conn = db.conn().unwrap();
    populate41(&conn);
    let before = legacy_snapshot(&conn);
    schema::migrate(&conn).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        42
    );
    assert_eq!(legacy_snapshot(&conn), before);
    let head: (String, Option<String>, Option<String>) = conn.query_row(
        "SELECT legacy_state,published_generation_id,candidate_generation_id FROM global_index_generation_heads WHERE volume_id='v'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        head,
        ("LEGACY_UNVERIFIED".into(), None, None),
        "even ready+prior timestamp is not proof"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM global_index_generations", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM global_entries_fts WHERE global_entries_fts MATCH 'legacytoken'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    validate(&conn).unwrap();
    drop(conn);
    drop(db);
    let db = Database::open(&path.0).unwrap();
    assert_eq!(legacy_snapshot(&db.conn().unwrap()), before);
}

#[test]
fn schema42_mid_ddl_failure_rolls_back_without_changing41_or_user_data() {
    let (_path, db) = fixture();
    let conn = db.conn().unwrap();
    populate41(&conn);
    let before = legacy_snapshot(&conn);
    // Conflict at the third DDL statement, after both earlier CREATE TABLEs.
    conn.execute_batch("CREATE TABLE global_index_generation_entries(injected TEXT)")
        .unwrap();
    assert!(schema::migrate(&conn).is_err());
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        41
    );
    assert_eq!(legacy_snapshot(&conn), before);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name IN ('global_index_generations','global_index_generation_heads')", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    conn.execute_batch("DROP TABLE global_index_generation_entries")
        .unwrap();
    schema::migrate(&conn).unwrap();
    assert_eq!(legacy_snapshot(&conn), before);
}

#[test]
fn schema42_fresh_reopen_signature_drift_and_future_version_fail_closed() {
    let (path, db) = fixture();
    validate(&db.conn().unwrap()).unwrap();
    assert_eq!(
        db.conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM global_index_generation_entries",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(db);
    let db = Database::open(&path.0).unwrap();
    let conn = db.conn().unwrap();
    conn.execute_batch("DROP TRIGGER global_index_generation_entries_no_update")
        .unwrap();
    assert!(validate(&conn)
        .unwrap_err()
        .to_string()
        .contains("schema42_generation_signature_conflict"));
    drop(conn);
    drop(db);
    assert!(
        Database::open(&path.0).is_err(),
        "do not repair away signature drift"
    );
    let conn = Connection::open(&path.0).unwrap();
    conn.execute_batch("PRAGMA user_version=43").unwrap();
    drop(conn);
    let error = Database::open(&path.0).err().unwrap();
    assert!(error.to_string().contains("newer than this app supports"));
    let conn = Connection::open(&path.0).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        43
    );
}
