use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use zen_canvas_tauri::db::Database;

#[test]
fn schema_migration_adds_classification_status_and_preserves_existing_state() {
    let path = test_db_path();
    create_schema_9_database(&path);

    let db = Database::open(&path).expect("migrate schema 9 database");
    let conn = Connection::open(db.path()).expect("open migrated database");

    let version: i64 = conn
        .query_row("SELECT user_version FROM pragma_user_version", [], |row| {
            row.get(0)
        })
        .expect("schema version");
    let (status_type, status_notnull): (String, i64) = conn
        .query_row(
            "SELECT type, \"notnull\" FROM pragma_table_info('files') WHERE name = 'classification_status'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("classification_status column");
    let unclassified_status: String = conn
        .query_row(
            "SELECT classification_status FROM files WHERE id = 'file-unclassified'",
            [],
            |row| row.get(0),
        )
        .expect("unclassified status");
    let classified_status: String = conn
        .query_row(
            "SELECT classification_status FROM files WHERE id = 'file-classified'",
            [],
            |row| row.get(0),
        )
        .expect("classified status");

    let cleanup_table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('cleanup_trash_batches', 'cleanup_trash_items')",
            [],
            |row| row.get(0),
        )
        .expect("cleanup trash tables");

    assert_eq!(version, 44);
    assert_eq!(cleanup_table_count, 2);
    assert_eq!(status_type, "TEXT");
    assert_eq!(status_notnull, 1);
    assert_eq!(unclassified_status, "unclassified");
    assert_eq!(classified_status, "classified");
}

fn create_schema_9_database(path: &Path) {
    let conn = Connection::open(path).expect("open schema 9 database");
    conn.execute_batch(
        r#"
        CREATE TABLE files (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            extension TEXT NOT NULL DEFAULT '',
            size INTEGER NOT NULL DEFAULT 0,
            mtime INTEGER NOT NULL DEFAULT 0,
            is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1)),
            state_code INTEGER NOT NULL DEFAULT 0,
            file_type TEXT NOT NULL DEFAULT 'Other',
            purpose TEXT NOT NULL DEFAULT 'Unknown',
            lifecycle TEXT NOT NULL DEFAULT 'Inbox',
            context TEXT NOT NULL DEFAULT '',
            risk_level TEXT NOT NULL DEFAULT 'Normal',
            suggested_action TEXT NOT NULL DEFAULT 'Keep',
            suggested_target_path TEXT NOT NULL DEFAULT '',
            suggested_name TEXT NOT NULL DEFAULT '',
            confidence REAL NOT NULL DEFAULT 0.5,
            classification_reason TEXT NOT NULL DEFAULT 'Indexed by Zen Canvas Tauri backend.',
            matched_rules TEXT NOT NULL DEFAULT '[]',
            requires_confirmation INTEGER NOT NULL DEFAULT 0,
            ctime INTEGER NOT NULL DEFAULT 0,
            is_stale INTEGER NOT NULL DEFAULT 0,
            last_seen_at INTEGER NOT NULL DEFAULT 0,
            last_classified_at INTEGER NOT NULL DEFAULT 0,
            classified_rule_version TEXT NOT NULL DEFAULT '',
            last_classified_mtime INTEGER NOT NULL DEFAULT 0,
            last_classified_size INTEGER NOT NULL DEFAULT 0,
            content_hash TEXT NOT NULL DEFAULT ''
        );
        INSERT INTO files (
            id, path, name, extension, size, purpose, lifecycle, matched_rules,
            last_classified_at, classified_rule_version, last_classified_mtime, last_classified_size
        )
        VALUES
            ('file-unclassified', '/legacy/unclassified.txt', 'unclassified.txt', 'txt', 128, 'Unknown', 'Inbox', '[]', 0, '', 0, 0),
            ('file-classified', '/legacy/resume.pdf', 'resume.pdf', 'pdf', 2048, 'Career', 'Reference', '["Career"]', 1900000000, 'legacy-rule-version', 1900000000, 2048);
        PRAGMA user_version = 9;
        "#,
    )
    .expect("create schema 9 database");
}

fn test_db_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "zen-canvas-classification-status-test-{nonce}.sqlite3"
    ))
}
