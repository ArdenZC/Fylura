use rusqlite::{params, Connection};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use zen_canvas_tauri::db::{Database, InsertFileRequest};

static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_db_path(label: &str) -> PathBuf {
    let sequence = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after unix epoch")
        .as_nanos();
    let root = std::env::var_os("ZEN_CANVAS_TEST_TEMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("manifest parent")
                .join(".tmp-tests")
                .join("migrations")
        });
    fs::create_dir_all(&root).expect("create migration test temp root");
    root.join(format!(
        "zen-canvas-migration-{label}-{}-{timestamp}-{sequence}.sqlite3",
        std::process::id(),
    ))
}

fn column_names(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare table info")
        .query_map([], |row| row.get(1))
        .expect("query table info")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect columns")
}

fn downgrade_current_fixture_to_schema_16(path: &PathBuf) {
    let db = Database::open(path).expect("create current database");
    drop(db);
    let conn = Connection::open(path).expect("open downgrade fixture");
    conn.execute(
        "UPDATE app_settings SET value = ?1, revision = 41 WHERE key = 'app_settings_v1'",
        [r#"{"closeBehavior":"minimize","folderNamingLanguage":"en","defaultScanFolders":[],"restoreRetentionDays":30,"launchAtLogin":false}"#],
    )
    .expect("seed legacy settings");
    conn.execute(
        "INSERT INTO operation_batches (id, created_at, status) VALUES ('legacy-batch', 1700000000000, 'completed')",
        [],
    )
    .expect("seed operation batch");
    conn.execute(
        r#"
        INSERT INTO operation_logs (
          id, batch_id, operation_type, source_path, target_path, old_name, new_name,
          status, created_at, can_undo, path_before, path_after, name_before, name_after,
          can_restore, restore_status
        ) VALUES (
          'legacy-operation', 'legacy-batch', 'move', 'C:/fixture/source.txt',
          'C:/fixture/target.txt', 'source.txt', 'target.txt', 'success', 1700000000000,
          1, 'C:/fixture/source.txt', 'C:/fixture/target.txt', 'source.txt', 'target.txt',
          1, 'not_restored'
        )
        "#,
        [],
    )
    .expect("seed legacy operation");
    conn.execute(
        "INSERT INTO cleanup_trash_batches (id, created_at, root, total_items, total_size, status) VALUES ('legacy-trash-batch', '2026-07-17T00:00:00Z', 'C:/fixture', 1, 7, 'completed')",
        [],
    )
    .expect("seed cleanup batch");
    conn.execute(
        r#"
        INSERT INTO cleanup_trash_items (
          id, batch_id, original_path, trash_path, name, size, moved_at, status
        ) VALUES (
          'legacy-trash-item', 'legacy-trash-batch', 'C:/fixture/trash.txt',
          'C:/fixture/.zen-canvas-trash/trash.txt', 'trash.txt', 7,
          '2026-07-17T00:00:00Z', 'moved'
        )
        "#,
        [],
    )
    .expect("seed legacy trash item");

    conn.execute_batch(
        r#"
        ALTER TABLE app_settings DROP COLUMN revision;
        ALTER TABLE operation_logs DROP COLUMN source_size;
        ALTER TABLE operation_logs DROP COLUMN source_modified_ns;
        ALTER TABLE operation_logs DROP COLUMN source_platform_file_id;
        ALTER TABLE operation_logs DROP COLUMN source_platform_volume_id;
        ALTER TABLE operation_logs DROP COLUMN source_quick_hash;
        ALTER TABLE operation_logs DROP COLUMN target_platform_file_id;
        ALTER TABLE operation_logs DROP COLUMN target_platform_volume_id;
        ALTER TABLE cleanup_trash_items DROP COLUMN source_modified_ns;
        ALTER TABLE cleanup_trash_items DROP COLUMN source_platform_file_id;
        ALTER TABLE cleanup_trash_items DROP COLUMN source_quick_hash;
        ALTER TABLE cleanup_trash_items DROP COLUMN trash_modified_ns;
        ALTER TABLE cleanup_trash_items DROP COLUMN trash_platform_volume_id;
        ALTER TABLE cleanup_trash_items DROP COLUMN trash_platform_file_id;
        ALTER TABLE cleanup_trash_items DROP COLUMN trash_quick_hash;
        ALTER TABLE cleanup_trash_items DROP COLUMN identity_status;
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 16;
        "#,
    )
    .expect("downgrade to schema 16");
}

fn downgrade_current_fixture_to_schema_20_or_21(path: &PathBuf, version: i32) {
    assert!(matches!(version, 20 | 21));
    let db = Database::open(path).expect("create current database");
    drop(db);
    let conn = Connection::open(path).expect("open journal downgrade fixture");
    conn.execute(
        "INSERT INTO operation_batches (id, created_at, status) VALUES ('legacy-pending-batch', 1, 'pending')",
        [],
    )
    .expect("seed legacy pending batch");
    conn.execute(
        r#"
        INSERT INTO operation_logs (
            id, batch_id, operation_type, source_path, target_path, old_name, new_name,
            status, created_at, can_restore, restore_status,
            path_before, path_after, name_before, name_after
        ) VALUES ('legacy-pending-restore', 'legacy-pending-batch', 'move', 'C:/source', 'C:/target',
            'source', 'target', 'success', 1, 0, 'pending', 'C:/source', 'C:/target', 'source', 'target')
        "#,
        [],
    )
    .expect("seed legacy pending restore");
    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS operation_logs_phase_guard_insert;
        DROP TRIGGER IF EXISTS operation_logs_phase_guard_update;
        DROP TRIGGER IF EXISTS operation_logs_restore_phase_guard_insert;
        DROP TRIGGER IF EXISTS operation_logs_restore_phase_guard_update;
        DROP INDEX IF EXISTS idx_operation_logs_restore_phase;
        DROP TRIGGER IF EXISTS cleanup_items_phase_guard_insert;
        DROP TRIGGER IF EXISTS cleanup_items_phase_guard_update;
        ALTER TABLE operation_logs DROP COLUMN source_platform_volume_id;
        ALTER TABLE operation_logs DROP COLUMN target_platform_volume_id;
        ALTER TABLE operation_logs DROP COLUMN source_claim_path;
        ALTER TABLE operation_logs DROP COLUMN operation_phase;
        ALTER TABLE operation_logs DROP COLUMN claim_created_at;
        ALTER TABLE operation_logs DROP COLUMN claim_platform_file_id;
        ALTER TABLE operation_logs DROP COLUMN claim_platform_volume_id;
        ALTER TABLE operation_logs DROP COLUMN claim_full_hash;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_path;
        ALTER TABLE operation_logs DROP COLUMN restore_phase;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_created_at;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_platform_file_id;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_platform_volume_id;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_full_hash;
        ALTER TABLE cleanup_trash_items DROP COLUMN source_claim_path;
        ALTER TABLE cleanup_trash_items DROP COLUMN operation_phase;
        ALTER TABLE cleanup_trash_items DROP COLUMN claim_created_at;
        ALTER TABLE cleanup_trash_items DROP COLUMN claim_platform_file_id;
        ALTER TABLE cleanup_trash_items DROP COLUMN claim_full_hash;
        "#,
    )
    .expect("remove schema 22 journal columns");

    if version == 20 {
        conn.execute_batch(
            r#"
            ALTER TABLE operation_logs DROP COLUMN source_full_hash;
            ALTER TABLE operation_logs DROP COLUMN target_full_hash;
            ALTER TABLE cleanup_trash_items DROP COLUMN source_full_hash;
            ALTER TABLE cleanup_trash_items DROP COLUMN trash_full_hash;
            "#,
        )
        .expect("remove schema 21 full hash columns");
    }

    conn.execute_batch("DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; ").expect("remove future Automation schema from legacy fixture");
    conn.execute(&format!("PRAGMA user_version = {version}"), [])
        .expect("downgrade journal fixture");
}

fn assert_schema_23_journal_columns(conn: &Connection) {
    let operation_columns = column_names(conn, "operation_logs");
    for column in [
        "source_full_hash",
        "target_full_hash",
        "source_claim_path",
        "operation_phase",
        "claim_created_at",
        "claim_platform_file_id",
        "claim_platform_volume_id",
        "claim_full_hash",
        "restore_claim_path",
        "restore_phase",
        "restore_claim_created_at",
        "restore_claim_platform_file_id",
        "restore_claim_platform_volume_id",
        "restore_claim_full_hash",
        "source_platform_volume_id",
        "target_platform_volume_id",
    ] {
        assert!(
            operation_columns.contains(&column.to_string()),
            "missing {column}"
        );
    }
    let cleanup_columns = column_names(conn, "cleanup_trash_items");
    for column in [
        "source_platform_volume_id",
        "source_full_hash",
        "trash_full_hash",
        "source_claim_path",
        "operation_phase",
        "claim_created_at",
        "claim_platform_file_id",
        "claim_full_hash",
    ] {
        assert!(
            cleanup_columns.contains(&column.to_string()),
            "missing {column}"
        );
    }
}

fn insert_schema_34_cleanup_item(
    conn: &Connection,
    id: &str,
    source_file_id: Option<&str>,
    trash_volume_id: Option<&str>,
    trash_file_id: Option<&str>,
    claim_file_id: Option<&str>,
) {
    conn.execute(
        r#"
        INSERT INTO cleanup_trash_items (
            id, batch_id, original_path, trash_path, name, size, moved_at,
            status, message, source_platform_file_id, trash_platform_volume_id,
            trash_platform_file_id, identity_status, claim_platform_file_id
        ) VALUES (?1, 'td014-batch', ?2, ?3, ?4, 7, '1', 'moved', NULL, ?5, ?6, ?7, 'verified', ?8)
        "#,
        params![
            id,
            format!("C:/td014/{id}.txt"),
            format!("C:/td014/.zen-canvas-trash/{id}.txt"),
            id,
            source_file_id,
            trash_volume_id,
            trash_file_id,
            claim_file_id,
        ],
    )
    .expect("insert schema 34 cleanup item");
}

fn downgrade_current_fixture_to_schema_34(path: &PathBuf) {
    let db = Database::open(path).expect("create schema 35 database");
    drop(db);
    let conn = Connection::open(path).expect("open schema 34 cleanup fixture");
    conn.execute(
        "INSERT INTO cleanup_trash_batches (id, created_at, root, total_items, total_size, status) VALUES ('td014-batch', '1', 'C:/td014', 10, 70, 'success')",
        [],
    )
    .expect("insert schema 34 cleanup batch");
    insert_schema_34_cleanup_item(
        &conn,
        "source-normalize",
        Some("macos-dev-ino:source-volume:source-file"),
        None,
        None,
        None,
    );
    insert_schema_34_cleanup_item(
        &conn,
        "trash-normalize",
        None,
        Some("trash-volume"),
        Some("macos-dev-ino:trash-volume:trash-file"),
        None,
    );
    insert_schema_34_cleanup_item(
        &conn,
        "claim-normalize",
        Some("macos-dev-ino:claim-volume:source-file"),
        None,
        None,
        Some("macos-dev-ino:claim-volume:claim-file"),
    );
    insert_schema_34_cleanup_item(
        &conn,
        "source-claim-conflict",
        Some("macos-dev-ino:source-volume-1:source-file"),
        None,
        None,
        Some("macos-dev-ino:source-volume-2:claim-file"),
    );
    insert_schema_34_cleanup_item(
        &conn,
        "trash-conflict",
        None,
        Some("trash-volume-1"),
        Some("macos-dev-ino:trash-volume-2:trash-file"),
        None,
    );
    insert_schema_34_cleanup_item(
        &conn,
        "legacy-untagged",
        Some("legacy-source-file"),
        Some("legacy-volume"),
        Some("legacy-trash-file"),
        None,
    );
    insert_schema_34_cleanup_item(
        &conn,
        "source-tagged-raw-trash",
        Some("macos-dev-ino:source-volume:source-file"),
        Some("trash-volume"),
        Some("legacy-trash-file"),
        None,
    );
    insert_schema_34_cleanup_item(
        &conn,
        "source-tagged-raw-claim",
        Some("macos-dev-ino:source-volume:source-file"),
        None,
        None,
        Some("legacy-claim-file"),
    );
    insert_schema_34_cleanup_item(
        &conn,
        "coherent-fully-tagged",
        Some("macos-dev-ino:source-volume:source-file"),
        Some("trash-volume"),
        Some("macos-dev-ino:trash-volume:trash-file"),
        Some("macos-dev-ino:source-volume:claim-file"),
    );
    insert_schema_34_cleanup_item(
        &conn,
        "source-untagged-tagged-trash",
        Some("legacy-source-file-with-tagged-trash"),
        Some("trash-volume-2"),
        Some("macos-dev-ino:trash-volume-2:trash-file"),
        None,
    );
    conn.execute_batch(
        r#"
        ALTER TABLE cleanup_trash_items DROP COLUMN source_platform_volume_id;
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 34;
        "#,
    )
    .expect("downgrade schema 35 cleanup fixture to schema 34");
}

type CleanupIdentityRecord = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

type LegacyCleanupIdentityRecord = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn cleanup_identity_snapshot(conn: &Connection) -> Vec<CleanupIdentityRecord> {
    conn.prepare(
        "SELECT id, source_platform_volume_id, source_platform_file_id, trash_platform_volume_id, trash_platform_file_id, claim_platform_file_id FROM cleanup_trash_items ORDER BY id",
    )
    .expect("prepare cleanup identity snapshot")
    .query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    })
    .expect("query cleanup identity snapshot")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect cleanup identity snapshot")
}

#[test]
fn schema_34_normalizes_cleanup_identity_components_and_fails_closed_on_conflicts() {
    let path = test_db_path("td014-normalize");
    downgrade_current_fixture_to_schema_34(&path);

    let db = Database::open(&path).expect("migrate schema 34 cleanup fixture");
    drop(db);
    let conn = Connection::open(&path).expect("inspect schema 35 cleanup fixture");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read schema 35 version");
    assert_eq!(version, 38);
    assert!(column_names(&conn, "cleanup_trash_items")
        .contains(&"source_platform_volume_id".to_string()));

    let source: (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT source_platform_volume_id, source_platform_file_id FROM cleanup_trash_items WHERE id = 'source-normalize'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read normalized source identity");
    assert_eq!(
        source,
        (
            Some("source-volume".to_string()),
            Some("source-file".to_string())
        )
    );

    let trash: (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT trash_platform_volume_id, trash_platform_file_id FROM cleanup_trash_items WHERE id = 'trash-normalize'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read normalized trash identity");
    assert_eq!(
        trash,
        (
            Some("trash-volume".to_string()),
            Some("trash-file".to_string())
        )
    );

    let claim: (Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT source_platform_volume_id, source_platform_file_id, claim_platform_file_id FROM cleanup_trash_items WHERE id = 'claim-normalize'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read normalized claim identity");
    assert_eq!(
        claim,
        (
            Some("claim-volume".to_string()),
            Some("source-file".to_string()),
            Some("claim-file".to_string())
        )
    );

    for id in ["source-claim-conflict", "trash-conflict"] {
        let (identity_status, source_volume, source_file, trash_volume, trash_file, claim_file):
            CleanupIdentityRecord = conn
            .query_row(
                "SELECT identity_status, source_platform_volume_id, source_platform_file_id, trash_platform_volume_id, trash_platform_file_id, claim_platform_file_id FROM cleanup_trash_items WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .expect("read conflicting cleanup identity");
        assert_ne!(
            identity_status, "verified",
            "conflict {id} must fail closed"
        );
        if id == "source-claim-conflict" {
            assert_eq!(source_volume, None);
            assert_eq!(
                source_file.as_deref(),
                Some("macos-dev-ino:source-volume-1:source-file")
            );
            assert_eq!(
                claim_file.as_deref(),
                Some("macos-dev-ino:source-volume-2:claim-file")
            );
        } else {
            assert_eq!(trash_volume.as_deref(), Some("trash-volume-1"));
            assert_eq!(
                trash_file.as_deref(),
                Some("macos-dev-ino:trash-volume-2:trash-file")
            );
        }
    }

    let legacy: LegacyCleanupIdentityRecord = conn
        .query_row(
            "SELECT source_platform_volume_id, source_platform_file_id, trash_platform_volume_id, trash_platform_file_id FROM cleanup_trash_items WHERE id = 'legacy-untagged'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read legacy untagged identity");
    assert_eq!(legacy.0, None);
    assert_eq!(legacy.1.as_deref(), Some("legacy-source-file"));
    assert_eq!(legacy.2.as_deref(), Some("legacy-volume"));
    assert_eq!(legacy.3.as_deref(), Some("legacy-trash-file"));

    for (id, raw_field, expected_raw) in [
        (
            "source-tagged-raw-trash",
            "trash_platform_file_id",
            "legacy-trash-file",
        ),
        (
            "source-tagged-raw-claim",
            "claim_platform_file_id",
            "legacy-claim-file",
        ),
    ] {
        let (identity_status, source_volume, source_file, trash_file, claim_file): (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT identity_status, source_platform_volume_id, source_platform_file_id, trash_platform_file_id, claim_platform_file_id FROM cleanup_trash_items WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .expect("read mixed legacy cleanup identity");
        assert_eq!(
            identity_status, "legacy_unverified",
            "mixed row {id} must be blocked"
        );
        assert_eq!(
            source_volume, None,
            "mixed row {id} must not promote source volume"
        );
        assert_eq!(
            source_file.as_deref(),
            Some("macos-dev-ino:source-volume:source-file")
        );
        if raw_field == "trash_platform_file_id" {
            assert_eq!(trash_file.as_deref(), Some(expected_raw));
        } else {
            assert_eq!(claim_file.as_deref(), Some(expected_raw));
        }
    }

    let coherent: CleanupIdentityRecord = conn
        .query_row(
            "SELECT identity_status, source_platform_volume_id, source_platform_file_id, trash_platform_volume_id, trash_platform_file_id, claim_platform_file_id FROM cleanup_trash_items WHERE id = 'coherent-fully-tagged'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .expect("read coherent fully tagged cleanup identity");
    assert_eq!(coherent.0, "verified");
    assert_eq!(coherent.1.as_deref(), Some("source-volume"));
    assert_eq!(coherent.2.as_deref(), Some("source-file"));
    assert_eq!(coherent.3.as_deref(), Some("trash-volume"));
    assert_eq!(coherent.4.as_deref(), Some("trash-file"));
    assert_eq!(coherent.5.as_deref(), Some("claim-file"));

    let source_without_volume: CleanupIdentityRecord = conn
        .query_row(
            "SELECT identity_status, source_platform_volume_id, source_platform_file_id, trash_platform_volume_id, trash_platform_file_id, claim_platform_file_id FROM cleanup_trash_items WHERE id = 'source-untagged-tagged-trash'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .expect("read untagged source cleanup identity");
    assert_ne!(source_without_volume.0, "verified");
    assert_eq!(source_without_volume.1, None);
    assert_eq!(
        source_without_volume.2.as_deref(),
        Some("legacy-source-file-with-tagged-trash")
    );
    assert_eq!(
        source_without_volume.4.as_deref(),
        Some("macos-dev-ino:trash-volume-2:trash-file")
    );
}

#[test]
fn schema_34_cleanup_identity_migration_rolls_back_column_and_rows_together() {
    let fixture = test_db_path("td014-rollback-fixture");
    downgrade_current_fixture_to_schema_34(&fixture);
    let conn = Connection::open(&fixture).expect("open rollback migration fixture");
    conn.execute_batch(
        r#"
        CREATE TRIGGER td014_reject_cleanup_identity_update
        BEFORE UPDATE ON cleanup_trash_items
        BEGIN
            SELECT RAISE(ABORT, 'injected TD-014 migration failure');
        END;
        "#,
    )
    .expect("install rollback trigger");
    drop(conn);

    let error = match Database::open(&fixture) {
        Ok(_) => panic!("migration must roll back on induced failure"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("TD-014 migration failure"));
    let conn = Connection::open(&fixture).expect("inspect rolled-back migration fixture");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read rolled-back schema version");
    assert_eq!(version, 34);
    assert!(!column_names(&conn, "cleanup_trash_items")
        .contains(&"source_platform_volume_id".to_string()));
    let source_file: String = conn
        .query_row(
            "SELECT source_platform_file_id FROM cleanup_trash_items WHERE id = 'source-normalize'",
            [],
            |row| row.get(0),
        )
        .expect("read rolled-back source identity");
    assert_eq!(source_file, "macos-dev-ino:source-volume:source-file");
    drop(conn);

    let _ = fs::remove_file(fixture);
}

#[test]
fn schema_38_reopen_is_idempotent_and_future_schema_39_is_rejected() {
    let path = test_db_path("td014-idempotent");
    downgrade_current_fixture_to_schema_34(&path);
    let db = Database::open(&path).expect("migrate schema 34 fixture to schema 35");
    drop(db);
    let first = Connection::open(&path).expect("open migrated schema 35 fixture");
    let before = cleanup_identity_snapshot(&first);
    drop(first);

    Database::open(&path).expect("schema 35 reopen is idempotent");
    let second = Connection::open(&path).expect("reopen migrated schema 35 fixture");
    assert_eq!(cleanup_identity_snapshot(&second), before);
    second
        .execute_batch("PRAGMA user_version = 39;")
        .expect("set future schema version");
    drop(second);
    let error = match Database::open(&path) {
        Ok(_) => panic!("future schema must remain rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("newer than this app supports"));
}

#[test]
fn schema_37_migrates_the_unconditional_fts_trigger_once_and_preserves_indexed_data() {
    let path = test_db_path("schema37-fts-trigger");
    let db = Database::open(&path).expect("create full project schema fixture");
    let file_path = path.with_file_name("issue381-migration-oldmarker.txt");
    let file_path_text = file_path.to_string_lossy().into_owned();
    db.insert_file(InsertFileRequest {
        id: "issue381-migration-file".to_string(),
        path: file_path_text.clone(),
        name: "issue381-migration-oldmarker.txt".to_string(),
        extension: "txt".to_string(),
        size: 17,
        mtime: 1,
        ctime: 1,
        is_dir: false,
        state_code: 0,
    })
    .expect("insert indexed row before simulating schema 37");
    drop(db);

    let conn = Connection::open(&path).expect("open schema-37 fixture");
    conn.execute_batch(
        r#"
        INSERT INTO user_tags (id, display_name, normalized_name, color_token, created_at, updated_at)
        VALUES ('issue381-tag', 'Owner Tag', 'owner tag', 'blue', 1, 1);
        INSERT INTO file_user_tags (file_id, tag_id, created_at)
        VALUES ('issue381-migration-file', 'issue381-tag', 1);
        DROP TRIGGER files_au;
        CREATE TRIGGER files_au AFTER UPDATE ON files BEGIN
            INSERT INTO files_fts(files_fts, rowid, name, path)
            VALUES('delete', old.rowid, old.name, old.path);
            INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
        END;
        PRAGMA user_version = 37;
        "#,
    )
    .expect("restore old unconditional schema-37 trigger");
    drop(conn);

    let migrated =
        Database::open(&path).expect("migrate schema 37 to the provisional v38 candidate");
    assert_eq!(
        migrated
            .search_files("oldmarker", Some(10))
            .expect("search before stale-only update")
            .len(),
        1
    );
    drop(migrated);

    let conn = Connection::open(&path).expect("inspect migrated trigger");
    let trigger_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = 'files_au'",
            [],
            |row| row.get(0),
        )
        .expect("read migrated files_au SQL");
    assert!(trigger_sql.contains("old.name IS NOT new.name"));
    assert!(trigger_sql.contains("old.path IS NOT new.path"));
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read migrated schema version");
    assert_eq!(version, 38);

    let before_stale_only_update = conn.total_changes();
    conn.execute(
        "UPDATE files SET is_stale = 1 WHERE id = 'issue381-migration-file'",
        [],
    )
    .expect("publish stale-only state");
    let stale_update_changes = conn.total_changes() - before_stale_only_update;
    assert_eq!(
        stale_update_changes, 1,
        "a stale-only update must not write FTS rows"
    );
    let retained_tag_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_user_tags WHERE file_id = 'issue381-migration-file' AND tag_id = 'issue381-tag'",
            [],
            |row| row.get(0),
        )
        .expect("read retained user tag");
    assert_eq!(retained_tag_count, 1);
    drop(conn);

    let reopened = Database::open(&path).expect("reopen migrated schema");
    assert_eq!(
        reopened
            .search_files("oldmarker", Some(10))
            .expect("old FTS term remains searchable")
            .len(),
        1
    );
    drop(reopened);

    let conn = Connection::open(&path).expect("open to test rename and rowid changes");
    conn.execute(
        "UPDATE files SET name = 'issue381-migration-newmarker.txt', path = ?1 WHERE id = 'issue381-migration-file'",
        [format!("{file_path_text}.renamed")],
    )
    .expect("rename indexed path and name");
    let old_term_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files_fts WHERE files_fts MATCH 'oldmarker'",
            [],
            |row| row.get(0),
        )
        .expect("search removed old term");
    let new_term_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files_fts WHERE files_fts MATCH 'newmarker'",
            [],
            |row| row.get(0),
        )
        .expect("search renamed term");
    assert_eq!(old_term_count, 0);
    assert_eq!(new_term_count, 1);
    conn.execute(
        "UPDATE files SET rowid = rowid + 100000 WHERE id = 'issue381-migration-file'",
        [],
    )
    .expect("change content-rowid key");
    let indexed_rowid: i64 = conn
        .query_row(
            "SELECT rowid FROM files_fts WHERE files_fts MATCH 'newmarker'",
            [],
            |row| row.get(0),
        )
        .expect("read FTS rowid after content-rowid change");
    let file_rowid: i64 = conn
        .query_row(
            "SELECT rowid FROM files WHERE id = 'issue381-migration-file'",
            [],
            |row| row.get(0),
        )
        .expect("read current files rowid");
    assert_eq!(indexed_rowid, file_rowid);
    let schema_cookie_before_reopen: i64 = conn
        .query_row("PRAGMA schema_version", [], |row| row.get(0))
        .expect("read schema cookie");
    drop(conn);

    Database::open(&path).expect("schema-38 reopen must not rebuild the trigger");
    let conn = Connection::open(&path).expect("inspect idempotent reopen");
    let schema_cookie_after_reopen: i64 = conn
        .query_row("PRAGMA schema_version", [], |row| row.get(0))
        .expect("read schema cookie after reopen");
    assert_eq!(schema_cookie_after_reopen, schema_cookie_before_reopen);
    conn.execute(
        "INSERT INTO files (id, path, name, extension) VALUES ('issue381-insert-trigger', '/tmp/issue381-insert-trigger.txt', 'issue381insertmarker.txt', 'txt')",
        [],
    )
    .expect("insert trigger keeps FTS synchronized");
    let inserted_term_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files_fts WHERE files_fts MATCH 'insertmarker'",
            [],
            |row| row.get(0),
        )
        .expect("search inserted term");
    assert_eq!(inserted_term_count, 1);
    conn.execute("DELETE FROM files WHERE id = 'issue381-insert-trigger'", [])
        .expect("delete trigger keeps FTS synchronized");
    let deleted_term_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files_fts WHERE files_fts MATCH 'insertmarker'",
            [],
            |row| row.get(0),
        )
        .expect("search deleted term");
    assert_eq!(deleted_term_count, 0);
}

#[test]
fn schema_16_migrates_settings_and_recovery_identity_without_trusting_legacy_rows() {
    let path = test_db_path("v16");
    downgrade_current_fixture_to_schema_16(&path);

    let db = Database::open(&path).expect("migrate schema 16 to current");
    drop(db);
    let conn = Connection::open(&path).expect("inspect migrated database");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read schema version");
    let (settings_json, revision): (String, i64) = conn
        .query_row(
            "SELECT value, revision FROM app_settings WHERE key = 'app_settings_v1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read migrated settings");
    let (can_restore, restore_status, restore_error): (i64, String, String) = conn
        .query_row(
            "SELECT can_restore, restore_status, restore_error FROM operation_logs WHERE id = 'legacy-operation'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read legacy operation state");
    let identity_status: String = conn
        .query_row(
            "SELECT identity_status FROM cleanup_trash_items WHERE id = 'legacy-trash-item'",
            [],
            |row| row.get(0),
        )
        .expect("read legacy trash identity state");

    assert_eq!(version, 38);
    assert!(settings_json.contains("minimize"));
    assert_eq!(revision, 0);
    assert_eq!(can_restore, 0);
    assert_eq!(restore_status, "manual_review");
    assert!(restore_error.contains("legacy identity"));
    assert_eq!(identity_status, "legacy_unverified");
    assert!(column_names(&conn, "operation_logs").contains(&"source_quick_hash".to_string()));
    assert!(column_names(&conn, "operation_logs").contains(&"source_full_hash".to_string()));
    assert!(column_names(&conn, "cleanup_trash_items").contains(&"trash_quick_hash".to_string()));
    assert!(column_names(&conn, "cleanup_trash_items").contains(&"trash_full_hash".to_string()));
    assert_eq!(
        conn.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM operation_batches WHERE id = 'legacy-batch'",
            [],
            |row| row.get(0)
        )
        .unwrap(),
        1
    );

    drop(conn);
    Database::open(&path).expect("migration is idempotent");
}

#[test]
fn migration_failure_rolls_back_prior_steps_and_preserves_schema_16_data() {
    let path = test_db_path("rollback");
    downgrade_current_fixture_to_schema_16(&path);
    let conn = Connection::open(&path).expect("open rollback fixture");
    conn.execute_batch("DROP TABLE operation_logs;")
        .expect("break later identity migration");
    drop(conn);

    let error = match Database::open(&path) {
        Ok(_) => panic!("migration should fail when the operation log table is missing"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("operation_logs"));

    let conn = Connection::open(&path).expect("inspect rolled back fixture");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read rolled back version");
    assert_eq!(version, 16);
    assert!(!column_names(&conn, "app_settings").contains(&"revision".to_string()));
    let settings_json: String = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'app_settings_v1'",
            [],
            |row| row.get(0),
        )
        .expect("legacy settings survive rollback");
    assert!(settings_json.contains("minimize"));
}

#[test]
fn schema_20_and_21_migrate_to_schema_23_with_independent_restore_claim_columns() {
    for version in [20, 21] {
        let path = test_db_path(&format!("v{version}-journal"));
        downgrade_current_fixture_to_schema_20_or_21(&path, version);

        let db = Database::open(&path).expect("migrate journal fixture to schema 23");
        drop(db);
        let conn = Connection::open(&path).expect("inspect journal migration");
        let migrated_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("read migrated journal version");
        assert_eq!(migrated_version, 38);
        assert_schema_23_journal_columns(&conn);
        let restore_phase: String = conn
            .query_row(
                "SELECT restore_phase FROM operation_logs WHERE id = 'legacy-pending-restore'",
                [],
                |row| row.get(0),
            )
            .expect("read restore phase default");
        assert_eq!(restore_phase, "prepared");
        let restore_status: String = conn
            .query_row(
                "SELECT restore_status FROM operation_logs WHERE id = 'legacy-pending-restore'",
                [],
                |row| row.get(0),
            )
            .expect("read legacy restore status");
        assert_eq!(restore_status, "pending");

        drop(conn);
        Database::open(&path).expect("journal migration is idempotent");
    }
}

#[test]
fn schema_20_normalizes_invalid_historical_rule_domains_in_transaction() {
    let path = test_db_path("enums");
    let db = Database::open(&path).expect("create current database");
    drop(db);
    let conn = Connection::open(&path).expect("open enum fixture");
    conn.execute(
        r#"
        INSERT INTO rules (
          id, name, source, enabled, priority, weight, root_operator,
          groups_json, action_json, created_at, updated_at
        ) VALUES (?1, 'Legacy', 'invalid-source', 1, 1, 1, 'XOR', ?2, ?3, '', '')
        "#,
        params![
            "legacy-enums",
            r#"[{"id":"g","operator":"XOR","conditions":[{"id":"c","field":"bad-field","operator":"bad-operator","value":"x"}]}]"#,
            r#"{"purpose":"bad-purpose","lifecycle":"bad-lifecycle","risk_level":"bad-risk","suggested_action":"bad-action"}"#
        ],
    )
    .expect("seed invalid rule domains");
    conn.execute_batch("DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 19;")
        .expect("downgrade enum fixture");
    drop(conn);

    Database::open(&path).expect("run enum migration");
    let conn = Connection::open(&path).expect("inspect enum migration");
    let (source, root, groups, action): (String, String, String, String) = conn
        .query_row(
            "SELECT source, root_operator, groups_json, action_json FROM rules WHERE id = 'legacy-enums'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read normalized rule");

    assert_eq!(source, "unknown");
    assert_eq!(root, "UNKNOWN");
    assert!(groups.contains("unknown"));
    assert!(!groups.contains("XOR"));
    assert!(!groups.contains("bad-field"));
    assert_eq!(action.matches("Unknown").count(), 4);
    assert!(!action.contains("bad-purpose"));
}

#[test]
#[ignore = "Task 02 migration/WAL benchmark; invoked by npm run test:performance"]
fn performance_100k_files_schema_28_to_29_and_wal_reader() {
    const FILE_COUNT: usize = 100_000;
    let path = test_db_path("performance-100k-schema-29");
    let db = Database::open(&path).expect("create schema 29 fixture");
    drop(db);

    let conn = Connection::open(&path).expect("open schema 29 fixture");
    conn.execute(
        "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, current_generation, needs_reconciliation, created_at, updated_at) VALUES ('migration-performance-root', '/tmp/migration-performance-root', 'migration-performance-root', 'file_library', 1, 'healthy', 0, 0, 1, 1)",
        [],
    )
    .expect("seed migration root");
    let tx = conn
        .unchecked_transaction()
        .expect("start file fixture transaction");
    for index in 0..FILE_COUNT {
        let path_text = format!("/tmp/migration-performance-root/file-{index:06}.bin");
        tx.execute(
            "INSERT INTO files (id, path, name, extension, size, mtime, is_dir, state_code) VALUES (?1, ?1, ?2, 'bin', 4096, 1, 0, 0)",
            params![path_text, format!("file-{index:06}.bin")],
        )
        .expect("seed migration file");
    }
    tx.commit().expect("commit 100k file fixture");
    conn.execute_batch(
        r#"
        DROP VIEW active_duplicate_membership;
        DROP TABLE duplicate_group_members;
        DROP TABLE duplicate_groups;
        DROP TABLE dedupe_run_errors;
        DROP TABLE file_fingerprints;
        DROP TABLE dedupe_runs;
        ALTER TABLE scan_roots DROP COLUMN watcher_rule_recovery_required;
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 28;
        "#,
    )
    .expect("downgrade fixture to schema 28");
    drop(conn);

    let migration_started = Instant::now();
    let migrated = Database::open(&path).expect("migrate 100k-file schema 28 fixture");
    let migration_elapsed = migration_started.elapsed();
    drop(migrated);

    let reader = Connection::open(&path).expect("open WAL reader");
    reader
        .execute_batch("PRAGMA journal_mode = WAL;")
        .expect("enable WAL reader mode");
    let reader_started = Instant::now();
    let count: i64 = reader
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .expect("read migrated 100k files");
    let reader_elapsed = reader_started.elapsed();
    assert_eq!(count, FILE_COUNT as i64);
    println!(
        "Task 02 migration performance: files={FILE_COUNT}, schema_28_to_29_ms={:.3}, wal_reader_count_ms={:.3}",
        migration_elapsed.as_secs_f64() * 1000.0,
        reader_elapsed.as_secs_f64() * 1000.0,
    );
    drop(reader);
    let _ = fs::remove_file(path);
}

#[test]
#[ignore = "Task 03 schema 29->30 migration/WAL benchmark; invoked by npm run test:performance"]
fn performance_100k_files_schema_29_to_30_analysis_and_wal_reader() {
    const FILE_COUNT: usize = 100_000;
    let path = test_db_path("performance-100k-schema-30");
    let db = Database::open(&path).expect("create schema 30 fixture");
    drop(db);

    let conn = Connection::open(&path).expect("open schema 30 fixture");
    conn.execute(
        "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, current_generation, needs_reconciliation, created_at, updated_at) VALUES ('analysis-migration-root', '/tmp/analysis-migration-root', 'analysis-migration-root', 'file_library', 1, 'healthy', 0, 0, 1, 1)",
        [],
    )
    .expect("seed analysis migration root");
    let tx = conn
        .unchecked_transaction()
        .expect("start analysis migration file fixture transaction");
    for index in 0..FILE_COUNT {
        let path_text = format!("/tmp/analysis-migration-root/file-{index:06}.bin");
        tx.execute(
            "INSERT INTO files (id, path, name, extension, size, mtime, is_dir, state_code) VALUES (?1, ?1, ?2, 'bin', 4096, 1, 0, 0)",
            params![path_text, format!("file-{index:06}.bin")],
        )
        .expect("seed analysis migration file");
    }
    tx.commit().expect("commit 100k analysis migration files");
    conn.execute_batch(
        r#"
        DROP TABLE analysis_finding_evidence;
        DROP TABLE analysis_finding_decisions;
        DROP TABLE analysis_findings;
        DROP TABLE analysis_run_detectors;
        DROP TABLE analysis_runs;
        DROP TABLE dedupe_authority_state;
        ALTER TABLE dedupe_runs DROP COLUMN publication_mode;
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 29;
        "#,
    )
    .expect("downgrade fixture to schema 29");
    drop(conn);

    let migration_started = Instant::now();
    let migrated = Database::open(&path).expect("migrate 100k-file schema 29 fixture");
    let migration_elapsed = migration_started.elapsed();
    drop(migrated);

    let reader = Connection::open(&path).expect("open schema 30 WAL reader");
    reader
        .execute_batch("PRAGMA journal_mode = WAL;")
        .expect("enable schema 30 WAL reader mode");
    let reader_started = Instant::now();
    let count: i64 = reader
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .expect("read migrated 100k files");
    let analysis_tables: i64 = reader
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('analysis_runs', 'analysis_findings', 'analysis_finding_decisions')",
            [],
            |row| row.get(0),
        )
        .expect("read schema 30 analysis tables");
    let reader_elapsed = reader_started.elapsed();
    assert_eq!(count, FILE_COUNT as i64);
    assert_eq!(analysis_tables, 3);
    println!(
        "Task 03 migration performance: files={FILE_COUNT}, schema_29_to_30_ms={:.3}, wal_reader_count_ms={:.3}",
        migration_elapsed.as_secs_f64() * 1000.0,
        reader_elapsed.as_secs_f64() * 1000.0,
    );
    drop(reader);
    let _ = fs::remove_file(path);
}

fn downgrade_current_fixture_to_schema_22(path: &PathBuf) {
    let db = Database::open(path).expect("create current schema 23 database");
    drop(db);
    let conn = Connection::open(path).expect("open schema 22 fixture");
    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS operation_logs_restore_phase_guard_insert;
        DROP TRIGGER IF EXISTS operation_logs_restore_phase_guard_update;
        DROP INDEX IF EXISTS idx_operation_logs_restore_phase;
        ALTER TABLE operation_logs DROP COLUMN source_platform_volume_id;
        ALTER TABLE operation_logs DROP COLUMN target_platform_volume_id;
        ALTER TABLE operation_logs DROP COLUMN claim_platform_volume_id;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_path;
        ALTER TABLE operation_logs DROP COLUMN restore_phase;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_created_at;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_platform_file_id;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_platform_volume_id;
        ALTER TABLE operation_logs DROP COLUMN restore_claim_full_hash;
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 22;
        "#,
    )
    .expect("downgrade to schema 22");
}

#[test]
fn schema_22_to_23_adds_restore_claim_defaults_and_repairs_all_journal_triggers() {
    let path = test_db_path("v22-to-v23");
    downgrade_current_fixture_to_schema_22(&path);

    let db = Database::open(&path).expect("migrate schema 22 to schema 23");
    drop(db);
    let conn = Connection::open(&path).expect("inspect current-schema fixture");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
            .expect("read schema version"),
        37
    );
    assert_schema_23_journal_columns(&conn);

    conn.execute(
        "INSERT INTO operation_batches (id, created_at, status) VALUES ('v23-batch', 1, 'success')",
        [],
    )
    .expect("insert v23 batch");
    conn.execute(
        r#"
        INSERT INTO operation_logs (
            id, batch_id, operation_type, source_path, target_path, old_name, new_name,
            status, created_at, path_before, path_after, name_before, name_after
        ) VALUES ('v23-log', 'v23-batch', 'move', 'C:/source', 'C:/target', 'source', 'target',
            'success', 1, 'C:/source', 'C:/target', 'source', 'target')
        "#,
        [],
    )
    .expect("insert v23 log with defaults");
    let defaults: (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT restore_phase, restore_claim_path, restore_claim_full_hash FROM operation_logs WHERE id = 'v23-log'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read restore claim defaults");
    assert_eq!(defaults.0, "idle");
    assert!(defaults.1.is_none());
    assert!(defaults.2.is_none());

    assert!(conn
        .execute(
            "UPDATE operation_logs SET restore_phase = 'invalid' WHERE id = 'v23-log'",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "UPDATE operation_logs SET operation_phase = 'invalid' WHERE id = 'v23-log'",
            [],
        )
        .is_err());

    conn.execute_batch(
        r#"
        DROP TRIGGER operation_logs_restore_phase_guard_update;
        DROP TRIGGER operation_logs_phase_guard_update;
        "#,
    )
    .expect("remove current schema guards");
    drop(conn);
    Database::open(&path).expect("repair current schema guards idempotently");
    let conn = Connection::open(&path).expect("reopen repaired schema");
    assert!(conn
        .execute(
            "UPDATE operation_logs SET restore_phase = 'invalid' WHERE id = 'v23-log'",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "UPDATE operation_logs SET operation_phase = 'invalid' WHERE id = 'v23-log'",
            [],
        )
        .is_err());
    drop(conn);
    Database::open(&path).expect("schema 23 repeat open");
}
