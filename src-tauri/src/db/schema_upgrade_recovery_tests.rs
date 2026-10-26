use super::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "zen-upgrade-{label}-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
fn historical_fixture(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    migrate(&conn).unwrap();
    // Historical fixture construction only. Production never lowers a version.
    conn.execute_batch("DROP INDEX idx_global_entries_active_name_order;
        DROP INDEX idx_global_entries_active_source_facts;
        CREATE INDEX idx_global_entries_active_name_order ON global_entries(name_normalized,modified_at_fs DESC,id ASC) WHERE is_stale=0;
        PRAGMA user_version=42;").unwrap();
    conn.execute_batch("BEGIN;").unwrap();
    {
        let mut stmt=conn.prepare("INSERT INTO global_entries(id,volume_id,platform_file_id,parent_platform_file_id,name,name_normalized,path,path_normalized,extension,is_directory,size,source_provider,last_seen_at) VALUES(?1,?2,?1,'parent',?3,?3,?3,?3,'txt',0,12,'recursive_fallback',42)").unwrap();
        let volume = format!(
            "gv_{}",
            blake3::hash(b"Volume{A0C934B1-073A-4A24-B72C-99AAB23D8771}").to_hex()
        );
        for i in 0..3000 {
            let id = format!("ge_{}", blake3::hash(format!("{i}").as_bytes()).to_hex());
            stmt.execute(rusqlite::params![
                id,
                volume,
                format!("quarterly report {i:06}.txt")
            ])
            .unwrap();
        }
    }
    conn.execute_batch("COMMIT; PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    conn
}
fn assert_original(conn: &Connection) {
    assert_eq!(schema_version(conn).unwrap(), 42);
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='idx_global_entries_active_name_order'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!sql.contains("volume_id"));
    assert_eq!(conn.query_row::<i64,_,_>("SELECT COUNT(*) FROM global_entries INDEXED BY idx_global_entries_active_name_order WHERE is_stale=0 AND name_normalized GLOB 'quarterly*'",[],|r|r.get(0)).unwrap(),3000);
    assert_eq!(
        conn.query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap(),
        "ok"
    );
}
fn cleanup(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn schema_upgrade_full_rolls_back_and_retry_rebuilds_indexes() {
    let path = path("full");
    let conn = historical_fixture(&path);
    let pages: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    conn.pragma_update(None, "max_page_count", pages).unwrap();
    let error = migrate(&conn).unwrap_err();
    assert!(
        matches!(error,DbError::Sqlite(rusqlite::Error::SqliteFailure(ref e,_)) if e.code==rusqlite::ErrorCode::DiskFull),
        "{error:?}"
    );
    assert!(conn.is_autocommit());
    assert_original(&conn);
    drop(conn);
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "max_page_count", 2_147_483_646_i64)
        .unwrap();
    migrate(&conn).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), 43);
    assert_eq!(conn.query_row::<i64,_,_>("SELECT COUNT(*) FROM global_entries INDEXED BY idx_global_entries_active_name_order WHERE is_stale=0",[],|r|r.get(0)).unwrap(),3000);
    drop(conn);
    cleanup(&path);
}

#[test]
fn schema_upgrade_cancel_after_drop_rolls_back_and_restores_settings() {
    let path = path("cancel");
    let conn = historical_fixture(&path);
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let guard = super::super::upgrade::UpgradeGuard::new(&conn, cancel.clone()).unwrap();
        let error = with_migration_transaction(&conn, |conn| {
            conn.execute_batch("DROP INDEX idx_global_entries_active_name_order")?;
            cancel.store(true, Ordering::Release);
            migrate_global_search_covering_indexes(conn)?;
            guard.check_cancelled()
        })
        .unwrap_err();
        assert!(
            matches!(error,DbError::Sqlite(rusqlite::Error::SqliteFailure(ref e,_)) if e.code==rusqlite::ErrorCode::OperationInterrupted),
            "{error:?}"
        );
    }
    assert!(conn.is_autocommit());
    assert_original(&conn);
    migrate(&conn).unwrap();
    drop(conn);
    cleanup(&path);
}

#[test]
fn schema_upgrade_killed_child() {
    let Some(path) = std::env::var_os("ZC_SCHEMA_UPGRADE_CHILD") else {
        return;
    };
    let conn = Connection::open(&path).unwrap();
    with_migration_transaction(&conn, |conn| {
        migrate_global_search_covering_indexes(conn)?;
        std::fs::write(
            std::env::var_os("ZC_SCHEMA_UPGRADE_MARKER").unwrap(),
            b"uncommitted43",
        )
        .unwrap();
        // Parent kills the process here, after both new indexes and user_version
        // are dirty but before COMMIT. No unwind or graceful rollback runs.
        loop {
            std::thread::park_timeout(Duration::from_secs(1));
        }
    })
    .unwrap();
}

#[test]
fn schema_upgrade_process_death_recovers_on_restart() {
    let path = path("kill");
    drop(historical_fixture(&path));
    let marker = path.with_extension("marker");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "db::schema::upgrade_recovery_tests::schema_upgrade_killed_child",
            "--nocapture",
        ])
        .env("ZC_SCHEMA_UPGRADE_CHILD", &path)
        .env("ZC_SCHEMA_UPGRADE_MARKER", &marker)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !marker.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            panic!("migration child exited before precommit marker");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let reached = marker.exists();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(reached, "child did not reach precommit checkpoint");
    let conn = Connection::open(&path).unwrap();
    assert_original(&conn);
    migrate(&conn).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), 43);
    migrate(&conn).unwrap();
    drop(conn);
    cleanup(&path);
    let _ = std::fs::remove_file(marker);
}

#[test]
fn schema_upgrade_resource_preflight_uses_real_vfs_and_keys() {
    let path = path("resources");
    let conn = historical_fixture(&path);
    let resources = super::super::upgrade::inspect_resources(&conn).unwrap();
    assert_eq!(resources.active_rows, 3000);
    assert!(resources.key_bytes > 3000 * 200);
    assert!(resources.temporary_directory.is_dir());
    assert_original(&conn);
    drop(conn);
    cleanup(&path);
}
