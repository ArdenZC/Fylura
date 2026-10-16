use rusqlite::{params, types::ValueRef, Connection};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use zen_canvas_tauri::db::{
    CreateUserTagRequest, Database, FileLibraryScopeV2, FileQueryFiltersV2, FileQueryRequestV2,
    FileQuerySpecV2, InsertFileRequest, LibrarySelectionV1, MutateFileUserTagsRequest,
    UserTagMutationOperation,
};
use zen_canvas_tauri::file_ops::OperationLogDto;
use zen_canvas_tauri::global_index::{GlobalEntryInput, GlobalVolume, PROVIDER_WINDOWS_MFT_USN};

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

fn identity_foundation_table_names() -> [&'static str; 4] {
    [
        "library_source_incarnations",
        "library_source_observations",
        "library_legacy_attributions",
        "library_generation_authority",
    ]
}

fn identity_foundation_object_names() -> [&'static str; 8] {
    [
        "library_source_incarnations",
        "library_source_observations",
        "idx_library_observations_legacy_id",
        "idx_library_observations_path",
        "library_legacy_attributions",
        "idx_library_attributions_legacy",
        "idx_library_attributions_source",
        "library_generation_authority",
    ]
}

fn source_writer_object_names() -> [&'static str; 7] {
    [
        "library_source_root_claims",
        "library_root_epoch_state",
        "idx_library_source_physical_verified",
        "idx_library_source_physical_lookup",
        "idx_library_source_root_claims_path_state",
        "idx_library_source_root_claims_root_path",
        "idx_library_source_root_claims_source_state",
    ]
}

fn drop_source_writer_schema(conn: &Connection) {
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_library_source_root_claims_source_state;
         DROP INDEX IF EXISTS idx_library_source_root_claims_root_path;
         DROP INDEX IF EXISTS idx_library_source_root_claims_path_state;
         DROP INDEX IF EXISTS idx_library_source_physical_verified;
         DROP INDEX IF EXISTS idx_library_source_physical_lookup;
         DROP TABLE IF EXISTS library_source_root_claims;
         DROP TABLE IF EXISTS library_root_epoch_state;",
    )
    .expect("remove schema 41 source writer objects from historical fixture");
}

fn drop_identity_foundation(conn: &Connection, user_version: i32) {
    drop_source_writer_schema(conn);
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS library_source_observations;
         DROP TABLE IF EXISTS library_legacy_attributions;
         DROP TABLE IF EXISTS library_source_incarnations;
         DROP TABLE IF EXISTS library_generation_authority;
         PRAGMA user_version = {user_version};"
    ))
    .expect("remove P0 objects from historical fixture");
}

fn source_incarnations_sql_with_case_changed_verification_check(conn: &Connection) -> String {
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema \
             WHERE type = 'table' AND name = 'library_source_incarnations'",
            [],
            |row| row.get(0),
        )
        .expect("read source incarnation DDL");
    let expected_check = "verification_state <> 'verified' OR";
    assert_eq!(sql.matches(expected_check).count(), 1);
    let changed_sql = sql.replace(expected_check, "verification_state <> 'VERIFIED' OR");
    assert_ne!(changed_sql, sql, "fixture must change the CHECK literal");
    changed_sql
}

fn sqlite_value_snapshot(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "<null>".to_string(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => format!("{value:.17}"),
        ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned(),
        ValueRef::Blob(value) => value
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(""),
    }
}

fn table_snapshot(conn: &Connection, table: &str) -> Vec<Vec<String>> {
    let mut statement = conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
        .expect("prepare legacy table snapshot");
    let column_count = statement.column_count();
    statement
        .query_map([], |row| {
            (0..column_count)
                .map(|index| Ok(sqlite_value_snapshot(row.get_ref(index)?)))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .expect("query legacy table snapshot")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect legacy table snapshot")
}

fn full_schema_snapshot(conn: &Connection) -> Vec<(String, String, String, Option<String>)> {
    let mut statement = conn
        .prepare(
            "SELECT type, name, tbl_name, sql FROM sqlite_schema \
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .expect("prepare legacy schema snapshot");
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("query legacy schema snapshot")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect schema snapshot")
}

fn legacy_schema_snapshot(conn: &Connection) -> Vec<(String, String, String, Option<String>)> {
    let p0_names = identity_foundation_object_names();
    let source_writer_names = source_writer_object_names();
    full_schema_snapshot(conn)
        .into_iter()
        .filter(|(_, name, _, _)| {
            !p0_names.contains(&name.as_str()) && !source_writer_names.contains(&name.as_str())
        })
        .collect()
}

fn fts_rows_matching(conn: &Connection, query: &str) -> Vec<(i64, String, String)> {
    let mut statement = conn
        .prepare("SELECT rowid, name, path FROM files_fts WHERE files_fts MATCH ?1 ORDER BY rowid")
        .expect("prepare FTS result snapshot");
    statement
        .query_map([query], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query FTS result snapshot")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect FTS result snapshot")
}

fn seed_legacy_contract_rows(conn: &Connection) {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        INSERT INTO files (id, path, name, extension, size, mtime, is_dir, state_code)
        VALUES (
            'legacy-file', '/legacy/p0-preservation/migrationtoken-legacy.txt',
            'migrationtoken-legacy.txt', 'txt', 31, 1700000000, 0, 0
        );
        INSERT INTO user_tags (id, display_name, normalized_name, color_token, created_at, updated_at, revision)
        VALUES ('legacy-tag', 'Owner Tag', 'owner tag', 'blue', 100, 200, 5);
        INSERT INTO file_user_tags (file_id, tag_id, created_at)
        VALUES ('legacy-file', 'legacy-tag', 150);
        INSERT INTO file_fingerprints (
            file_id, path_snapshot, identity_status, size, fingerprint_status,
            captured_at, last_verified_at
        ) VALUES (
            'legacy-file', '/legacy/p0-preservation/migrationtoken-legacy.txt',
            'path_only', 31, 'complete', 100, 200
        );
        INSERT INTO dedupe_runs (
            id, request_key, scope_json, scope_hash, status, phase, created_at, updated_at
        ) VALUES ('legacy-dedupe-run', 'legacy-dedupe-request', '{}', 'legacy-scope',
                 'completed', 'completed', 100, 200);
        INSERT INTO duplicate_groups (
            id, size_each, full_hash, member_count, physical_copy_count,
            potential_reclaimable_bytes, reclaimable_confidence, status,
            last_built_run_id, created_at, updated_at, last_verified_at
        ) VALUES (
            'legacy-duplicate-group', 31, 'legacy-content-hash', 1, 1, 0,
            'unknown', 'active', 'legacy-dedupe-run', 100, 200, 200
        );
        INSERT INTO duplicate_group_members (
            group_id, file_id, path_snapshot, identity_status, size, verified_at
        ) VALUES (
            'legacy-duplicate-group', 'legacy-file',
            '/legacy/p0-preservation/migrationtoken-legacy.txt', 'path_only', 31, 200
        );
        INSERT INTO content_runs (
            id, scope_json, scope_fingerprint, mode, provider_mode, status,
            expected_library_revision, policy_fingerprint, created_at, updated_at
        ) VALUES (
            'legacy-content-run', '{}', 'legacy-content-scope', 'local', 'none',
            'completed', 1, 'legacy-policy', 100, 200
        );
        INSERT INTO content_run_items (
            id, run_id, file_id, ordinal, status, source_size, source_mtime,
            created_at, updated_at
        ) VALUES (
            'legacy-content-item', 'legacy-content-run', 'legacy-file', 0,
            'completed', 31, 1700000000, 100, 200
        );
        INSERT INTO content_artifacts (
            id, file_id, source_size, source_mtime, source_is_dir,
            extractor_family, extractor_version, policy_revision,
            content_fingerprint, status, created_at, updated_at
        ) VALUES (
            'legacy-content-artifact', 'legacy-file', 31, 1700000000, 0,
            'plain_text', '1', 1, 'legacy-content-fingerprint', 'current', 100, 200
        );
        INSERT INTO operation_batches (id, created_at, status)
        VALUES ('legacy-operation-batch', 100, 'completed');
        INSERT INTO operation_logs (
            id, batch_id, operation_type, source_path, target_path, old_name, new_name,
            status, created_at, path_before, path_after, name_before, name_after
        ) VALUES (
            'legacy-operation-log', 'legacy-operation-batch', 'move',
            '/legacy/source.txt', '/legacy/target.txt', 'source.txt', 'target.txt',
            'success', 100, '/legacy/source.txt', '/legacy/target.txt',
            'source.txt', 'target.txt'
        );
        INSERT INTO organization_plans (
            id, title, status, source_kind, source_snapshot_revision,
            requested_count, materialized_count, planner_version, revision,
            created_at, updated_at
        ) VALUES (
            'legacy-organization-plan', 'Legacy plan', 'ready', 'explicit',
            1, 1, 1, 1, 1, 100, 200
        );
        INSERT INTO organization_plan_items (
            id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
            source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
            source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
            proposed_target_directory, proposed_name, proposed_target_path,
            decision, validity, confidence, risk_level, requires_confirmation,
            revision, created_at, updated_at
        ) VALUES (
            'legacy-organization-item', 'legacy-organization-plan', 0,
            'legacy-file', '/legacy/p0-preservation/migrationtoken-legacy.txt',
            'migrationtoken-legacy.txt', 31, 1700000000, 0, 'legacy-proposal',
            'keep', '/legacy/p0-preservation', 'migrationtoken-legacy.txt',
            '/legacy/p0-preservation/migrationtoken-legacy.txt', 'undecided',
            'ready', 0.9, 'Normal', 0, 1, 100, 200
        );
        "#,
    )
    .expect("seed representative legacy facts");
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
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision;
        DROP INDEX IF EXISTS idx_library_source_root_claims_source_state;
        DROP INDEX IF EXISTS idx_library_source_root_claims_root_path;
        DROP INDEX IF EXISTS idx_library_source_root_claims_path_state;
        DROP INDEX IF EXISTS idx_library_source_physical_verified;
        DROP INDEX IF EXISTS idx_library_source_physical_lookup;
        DROP TABLE IF EXISTS library_source_root_claims;
        DROP TABLE IF EXISTS library_root_epoch_state;
        PRAGMA user_version = 16;
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
    drop_source_writer_schema(&conn);
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
        DROP INDEX IF EXISTS idx_library_source_root_claims_source_state;
        DROP INDEX IF EXISTS idx_library_source_root_claims_root_path;
        DROP INDEX IF EXISTS idx_library_source_root_claims_path_state;
        DROP INDEX IF EXISTS idx_library_source_physical_verified;
        DROP INDEX IF EXISTS idx_library_source_physical_lookup;
        DROP TABLE IF EXISTS library_source_root_claims;
        DROP TABLE IF EXISTS library_root_epoch_state;
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
    assert_eq!(version, 41);
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
fn schema_41_reopen_is_idempotent_and_future_schema_42_is_rejected() {
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
        .execute_batch("PRAGMA user_version = 42;")
        .expect("set future schema version");
    drop(second);
    let error = match Database::open(&path) {
        Ok(_) => panic!("future schema must remain rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("newer than this app supports"));
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

    assert_eq!(version, 41);
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
        assert_eq!(migrated_version, 41);
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
    drop_identity_foundation(&conn, 19);
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
    drop_source_writer_schema(&conn);
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
    drop_source_writer_schema(&conn);
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
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision;
        DROP INDEX IF EXISTS idx_library_source_root_claims_source_state;
        DROP INDEX IF EXISTS idx_library_source_root_claims_root_path;
        DROP INDEX IF EXISTS idx_library_source_root_claims_path_state;
        DROP INDEX IF EXISTS idx_library_source_physical_verified;
        DROP INDEX IF EXISTS idx_library_source_physical_lookup;
        DROP TABLE IF EXISTS library_source_root_claims;
        DROP TABLE IF EXISTS library_root_epoch_state;
        PRAGMA user_version = 22;
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
        41
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

#[test]
fn p0_t01_fresh_and_repeated_open_keep_identity_authority_dormant() {
    let path = test_db_path("p0-t01-fresh");
    let db = Database::open(&path).expect("create fresh schema 41 database");
    db.init().expect("repeat initialization is idempotent");
    drop(db);

    let conn = Connection::open(&path).expect("inspect fresh schema 41 database");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read schema version");
    assert_eq!(version, 41);
    let tables: Vec<String> = {
        let mut statement = conn
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' \
                 AND name IN ('library_source_incarnations', 'library_source_observations', \
                              'library_legacy_attributions', 'library_generation_authority', \
                              'library_root_epoch_state', 'library_source_root_claims') \
                 ORDER BY name",
            )
            .expect("prepare P0 table list");
        statement
            .query_map([], |row| row.get(0))
            .expect("query P0 table list")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect P0 table list")
    };
    let mut expected_tables = identity_foundation_table_names()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    expected_tables.extend([
        "library_root_epoch_state".to_string(),
        "library_source_root_claims".to_string(),
    ]);
    expected_tables.sort();
    assert_eq!(tables, expected_tables);

    let object_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name IN \
             ('library_source_incarnations', 'library_source_observations', \
              'idx_library_observations_legacy_id', 'idx_library_observations_path', \
              'library_legacy_attributions', 'idx_library_attributions_legacy', \
              'idx_library_attributions_source', 'library_generation_authority', \
              'library_root_epoch_state', 'library_source_root_claims', \
              'idx_library_source_root_claims_source_state', \
              'idx_library_source_root_claims_root_path', \
              'idx_library_source_root_claims_path_state', \
              'idx_library_source_physical_verified', \
              'idx_library_source_physical_lookup')",
            [],
            |row| row.get(0),
        )
        .expect("count P0 schema objects");
    assert_eq!(object_count, 15);

    let index_properties: Vec<(String, i64, i64)> = {
        let mut statement = conn
            .prepare("PRAGMA index_list('library_source_incarnations')")
            .expect("prepare SourceUID index list");
        statement
            .query_map([], |row| Ok((row.get(1)?, row.get(2)?, row.get(4)?)))
            .expect("query SourceUID index list")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect SourceUID index list")
    };
    assert!(index_properties.contains(&("idx_library_source_physical_lookup".to_string(), 0, 0,)));
    assert!(index_properties.contains(&("idx_library_source_physical_verified".to_string(), 1, 1,)));
    let lookup_columns: Vec<String> = {
        let mut statement = conn
            .prepare("PRAGMA index_info('idx_library_source_physical_lookup')")
            .expect("prepare SourceUID lookup index columns");
        statement
            .query_map([], |row| row.get(2))
            .expect("query SourceUID lookup index columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect SourceUID lookup index columns")
    };
    assert_eq!(
        lookup_columns,
        vec![
            "physical_platform",
            "physical_volume_id",
            "physical_object_id",
            "created_at",
            "source_uid",
        ]
    );

    let authority: (i64, String, i64, Option<String>, i64) = conn
        .query_row(
            "SELECT singleton_id, read_mode, active_catalog_epoch, active_fts_slot_id, revision \
             FROM library_generation_authority",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("read initial legacy authority");
    assert_eq!(authority, (1, "legacy".to_string(), 0, None, 1));
    for table in [
        "library_source_incarnations",
        "library_source_observations",
        "library_legacy_attributions",
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count dormant identity rows");
        assert_eq!(count, 0, "{table} must start empty");
    }
    drop(conn);
    Database::open(&path).expect("second open is idempotent");
}

#[test]
fn p0_t02_schema_37_migration_preserves_representative_legacy_user_facts_and_fts() {
    let path = test_db_path("p0-t02-preservation");
    let db = Database::open(&path).expect("create current schema fixture");
    drop(db);
    let conn = Connection::open(&path).expect("seed schema 37 preservation fixture");
    seed_legacy_contract_rows(&conn);
    drop_identity_foundation(&conn, 37);

    let preserved_tables = [
        "files",
        "user_tags",
        "file_user_tags",
        "file_fingerprints",
        "dedupe_runs",
        "duplicate_groups",
        "duplicate_group_members",
        "content_runs",
        "content_run_items",
        "content_artifacts",
        "content_catalog",
        "operation_batches",
        "operation_logs",
        "organization_plans",
        "organization_plan_items",
    ];
    let before_rows = preserved_tables
        .iter()
        .map(|table| (*table, table_snapshot(&conn, table)))
        .collect::<Vec<_>>();
    let before_schema = legacy_schema_snapshot(&conn);
    let before_fts = fts_rows_matching(&conn, "migrationtoken");
    assert_eq!(before_fts.len(), 1);
    drop(conn);

    let started = Instant::now();
    let migrated = Database::open(&path).expect("migrate schema 37 fixture through schema 41");
    let migration_elapsed = started.elapsed();
    drop(migrated);

    let conn = Connection::open(&path).expect("inspect migrated preservation fixture");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read migrated version");
    assert_eq!(version, 41);
    for (table, expected) in before_rows {
        assert_eq!(table_snapshot(&conn, table), expected, "{table} changed");
    }
    assert_eq!(legacy_schema_snapshot(&conn), before_schema);
    assert_eq!(fts_rows_matching(&conn, "migrationtoken"), before_fts);
    let authority: (String, i64, Option<String>, i64) = conn
        .query_row(
            "SELECT read_mode, active_catalog_epoch, active_fts_slot_id, revision \
             FROM library_generation_authority WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read authority after migration");
    assert_eq!(authority, ("legacy".to_string(), 0, None, 1));
    eprintln!(
        "[issue381-p0] T02 schema37_to_38_open_elapsed_ms={}",
        migration_elapsed.as_millis()
    );
}

#[test]
fn p0_t03_schema_36_migrates_to_38_without_rewriting_files() {
    let path = test_db_path("p0-t03-schema36");
    let db = Database::open(&path).expect("create current schema fixture");
    drop(db);
    let conn = Connection::open(&path).expect("create schema 36 fixture");
    conn.execute(
        "INSERT INTO files (id, path, name, extension, size, mtime, is_dir, state_code) \
         VALUES ('v36-file', '/legacy/v36preserved.txt', 'v36preserved.txt', 'txt', 12, 9, 0, 0)",
        [],
    )
    .expect("seed schema 36 file");
    let before_file = table_snapshot(&conn, "files");
    let before_fts = fts_rows_matching(&conn, "v36preserved");
    drop_identity_foundation(&conn, 36);
    conn.execute_batch(
        r#"
         DROP TABLE IF EXISTS automation_trigger_state;
         DROP TABLE IF EXISTS automation_runs;
         DROP TABLE IF EXISTS automation_intents;
         ALTER TABLE files DROP COLUMN filesystem_observation_key;
         ALTER TABLE scan_roots DROP COLUMN library_change_revision;
         CREATE TABLE automation_intents (
             id TEXT PRIMARY KEY,
             revision INTEGER NOT NULL CHECK(revision >= 1),
             title TEXT NOT NULL CHECK(length(trim(title)) BETWEEN 1 AND 120),
             workflow_kind TEXT NOT NULL CHECK(workflow_kind = 'organize_plan'),
             scope_query_json TEXT NOT NULL CHECK(json_valid(scope_query_json)),
             scope_fingerprint TEXT NOT NULL,
             trigger_json TEXT NOT NULL CHECK(json_valid(trigger_json) AND
                 coalesce(json_extract(trigger_json, '$.version') = 1 AND
                 json_extract(trigger_json, '$.kind') = 'manual' AND
                 json_remove(trigger_json, '$.version', '$.kind') = '{}', 0)),
             policy_json TEXT NOT NULL CHECK(json_valid(policy_json) AND
                 coalesce(json_extract(policy_json, '$.version') = 1 AND
                 json_extract(policy_json, '$.review') = 'required' AND
                 json_type(policy_json, '$.autoExecute') = 'false' AND
                 json_remove(policy_json, '$.version', '$.review', '$.autoExecute') = '{}', 0)),
             enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             archived_at INTEGER
         );
         CREATE TABLE automation_runs (
             id TEXT PRIMARY KEY,
             request_key TEXT NOT NULL UNIQUE,
             intent_id TEXT NOT NULL REFERENCES automation_intents(id) ON DELETE RESTRICT,
             intent_revision INTEGER NOT NULL CHECK(intent_revision >= 1),
             trigger_kind TEXT NOT NULL CHECK(trigger_kind = 'manual'),
             scope_fingerprint TEXT NOT NULL,
             library_snapshot_revision INTEGER,
             status TEXT NOT NULL CHECK(status IN ('completed','blocked','failed')),
             result_plan_id TEXT,
             queued_analysis_count INTEGER NOT NULL DEFAULT 0 CHECK(queued_analysis_count >= 0),
             requires_plan_refresh INTEGER NOT NULL CHECK(requires_plan_refresh IN (0,1)),
             analysis_blocker_code TEXT,
             error_code TEXT,
             created_at INTEGER NOT NULL,
             completed_at INTEGER NOT NULL
         );
         CREATE INDEX idx_automation_intents_active ON automation_intents(archived_at, updated_at DESC, id);
         CREATE INDEX idx_automation_runs_recent ON automation_runs(created_at DESC, id);
         CREATE INDEX idx_automation_runs_intent ON automation_runs(intent_id, created_at DESC, id);
         "#,
    )
    .expect("construct schema 36 automation tables");
    drop(conn);

    let db = Database::open(&path).expect("migrate schema 36 through schema 41");
    drop(db);
    let conn = Connection::open(&path).expect("inspect migrated schema 36 fixture");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read migrated version"),
        41
    );
    assert_eq!(table_snapshot(&conn, "files"), before_file);
    assert_eq!(fts_rows_matching(&conn, "v36preserved"), before_fts);
    let authority: (String, i64, Option<String>, i64) = conn
        .query_row(
            "SELECT read_mode, active_catalog_epoch, active_fts_slot_id, revision \
             FROM library_generation_authority WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read migrated authority");
    assert_eq!(authority, ("legacy".to_string(), 0, None, 1));
}

#[test]
fn p0_t05_provisional_schema_38_files_au_database_is_rejected_without_mutation() {
    let path = test_db_path("p0-t05-schema38-conflict");
    let db = Database::open(&path).expect("create current schema fixture");
    drop(db);
    let conn = Connection::open(&path).expect("create provisional schema 38 fixture");
    seed_legacy_contract_rows(&conn);
    drop_identity_foundation(&conn, 38);
    conn.execute_batch(
        r#"
        DROP TRIGGER files_au;
        CREATE TRIGGER files_au AFTER UPDATE ON files
        WHEN old.rowid IS NOT new.rowid
          OR old.name IS NOT new.name
          OR old.path IS NOT new.path
        BEGIN
            INSERT INTO files_fts(files_fts, rowid, name, path)
            VALUES('delete', old.rowid, old.name, old.path);
            INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
        END;
        PRAGMA user_version = 38;
        "#,
    )
    .expect("install the provisional #383 schema 38 trigger");
    let before_schema = full_schema_snapshot(&conn);
    let before_files = table_snapshot(&conn, "files");
    let before_tags = table_snapshot(&conn, "user_tags");
    let before_fts = fts_rows_matching(&conn, "migrationtoken");
    drop(conn);

    let error = match Database::open(&path) {
        Ok(_) => panic!("the incompatible provisional schema 38 must be rejected"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("library_schema38_identity_foundation_conflict"));

    let conn = Connection::open(&path).expect("inspect rejected schema 38 fixture");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read rejected schema version"),
        38
    );
    assert_eq!(full_schema_snapshot(&conn), before_schema);
    assert_eq!(table_snapshot(&conn, "files"), before_files);
    assert_eq!(table_snapshot(&conn, "user_tags"), before_tags);
    assert_eq!(fts_rows_matching(&conn, "migrationtoken"), before_fts);
    for name in identity_foundation_object_names() {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .expect("check rejected fixture was not healed");
        assert_eq!(count, 0, "unexpected identity object {name}");
    }
}

#[test]
fn p0_t06_schema_37_collision_is_rejected_without_touching_legacy_data() {
    let path = test_db_path("p0-t06-name-collision");
    let db = Database::open(&path).expect("create current schema fixture");
    drop(db);
    let conn = Connection::open(&path).expect("create schema 37 collision fixture");
    seed_legacy_contract_rows(&conn);
    drop_identity_foundation(&conn, 37);
    conn.execute_batch("CREATE TABLE library_source_incarnations (conflicting TEXT);")
        .expect("install conflicting P0 table name");
    let before_schema = full_schema_snapshot(&conn);
    let before_files = table_snapshot(&conn, "files");
    let before_tags = table_snapshot(&conn, "user_tags");
    let before_fts = fts_rows_matching(&conn, "migrationtoken");
    drop(conn);

    let error = match Database::open(&path) {
        Ok(_) => panic!("the conflicting schema 37 table must be rejected"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("library_schema38_identity_foundation_conflict"));
    let conn = Connection::open(&path).expect("inspect rejected schema 37 fixture");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read schema version"),
        37
    );
    assert_eq!(full_schema_snapshot(&conn), before_schema);
    assert_eq!(table_snapshot(&conn, "files"), before_files);
    assert_eq!(table_snapshot(&conn, "user_tags"), before_tags);
    assert_eq!(fts_rows_matching(&conn, "migrationtoken"), before_fts);
}

#[test]
fn p0_r1_schema_38_reopen_rejects_case_changed_check_without_mutation() {
    let path = test_db_path("p0-r1-schema38-case-check");
    let db = Database::open(&path).expect("create current schema fixture");
    drop(db);
    let conn = Connection::open(&path).expect("open schema 38 case-check fixture");
    seed_legacy_contract_rows(&conn);

    let changed_source_sql = source_incarnations_sql_with_case_changed_verification_check(&conn);
    conn.execute_batch("DROP TABLE library_source_incarnations;")
        .expect("remove source table for DDL-only fixture change");
    conn.execute_batch(&changed_source_sql)
        .expect("install CHECK with case-sensitive semantic change");
    conn.execute_batch("PRAGMA user_version = 38;")
        .expect("reopen malformed schema 38 fixture");
    let before_schema = full_schema_snapshot(&conn);
    let before_files = table_snapshot(&conn, "files");
    let before_tags = table_snapshot(&conn, "user_tags");
    let before_file_tags = table_snapshot(&conn, "file_user_tags");
    let before_authority = table_snapshot(&conn, "library_generation_authority");
    let before_fts = fts_rows_matching(&conn, "migrationtoken");
    drop(conn);

    let error = match Database::open(&path) {
        Ok(_) => panic!("schema 38 with a case-changed CHECK must be rejected"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "library_schema38_identity_foundation_conflict: library_source_incarnations"
    );

    let conn = Connection::open(&path).expect("inspect rejected schema 38 fixture");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read rejected schema version"),
        38
    );
    assert_eq!(full_schema_snapshot(&conn), before_schema);
    assert_eq!(table_snapshot(&conn, "files"), before_files);
    assert_eq!(table_snapshot(&conn, "user_tags"), before_tags);
    assert_eq!(table_snapshot(&conn, "file_user_tags"), before_file_tags);
    assert_eq!(
        table_snapshot(&conn, "library_generation_authority"),
        before_authority
    );
    assert_eq!(fts_rows_matching(&conn, "migrationtoken"), before_fts);
}

#[test]
fn schema_38_to_41_adds_source_and_operation_fencing_without_changing_global_index_data() {
    let path = test_db_path("schema38-to-41-global-index");
    let db = Database::open(&path).expect("create schema 41 fixture");
    let volume = GlobalVolume {
        id: "schema39-volume".into(),
        platform: "windows".into(),
        stable_volume_id: "schema39-stable-volume".into(),
        display_name: "C:".into(),
        mount_path: "C:\\".into(),
        filesystem_type: "ntfs".into(),
        drive_kind: "fixed".into(),
        enabled: true,
        provider: PROVIDER_WINDOWS_MFT_USN.into(),
        index_status: "discovered".into(),
        last_error: None,
        journal_id: None,
        journal_cursor: None,
        last_full_index_at: None,
        last_incremental_sync_at: None,
        entry_count: 0,
        created_at: 1,
        updated_at: 1,
    };
    db.upsert_global_volume(&volume).expect("seed volume");
    db.update_global_volume_state(
        &volume.id,
        "rebuild_required",
        Some("journal identity changed"),
        Some("journal-17"),
        Some("cursor-93"),
        Some(101),
        Some(202),
    )
    .expect("seed committed source health and checkpoint");
    db.upsert_global_entries_batch(&[GlobalEntryInput {
        volume_id: volume.id.clone(),
        platform_file_id: "file-17".into(),
        parent_platform_file_id: "root".into(),
        name: "preserved.txt".into(),
        path: "C:\\preserved.txt".into(),
        extension: "txt".into(),
        is_directory: false,
        size: 42,
        created_at_fs: Some(3),
        modified_at_fs: Some(5),
        file_attributes: 0,
        is_hidden: false,
        is_system: false,
        source_provider: PROVIDER_WINDOWS_MFT_USN.into(),
        last_seen_at: 7,
    }])
    .expect("seed active Global Search entry");
    drop(db);

    let conn = Connection::open(&path).expect("open schema 41 database as schema 38");
    conn.execute_batch(
        r#"
        ALTER TABLE global_volumes DROP COLUMN rebuild_completed_revision;
        ALTER TABLE global_volumes DROP COLUMN rebuild_intent_revision;
        ALTER TABLE global_volumes DROP COLUMN operation_state;
        ALTER TABLE global_volumes DROP COLUMN operation_kind;
        ALTER TABLE global_volumes DROP COLUMN operation_owner_route;
        ALTER TABLE global_volumes DROP COLUMN operation_owner_started_at;
        ALTER TABLE global_volumes DROP COLUMN operation_owner_pid;
        ALTER TABLE global_volumes DROP COLUMN operation_owner_id;
        ALTER TABLE global_volumes DROP COLUMN operation_revision;
        DROP INDEX idx_library_source_root_claims_source_state;
        DROP INDEX idx_library_source_root_claims_root_path;
        DROP INDEX idx_library_source_root_claims_path_state;
        DROP INDEX idx_library_source_physical_verified;
        DROP INDEX idx_library_source_physical_lookup;
        DROP TABLE library_source_root_claims;
        DROP TABLE library_root_epoch_state;
        PRAGMA user_version = 38;
        "#,
    )
    .expect("construct pre-migration schema 38 fixture");
    drop(conn);

    let migrated = Database::open(&path).expect("migrate schema 38 to schema 41");
    let stored = migrated
        .get_global_volume(&volume.id)
        .expect("read migrated volume")
        .expect("volume survives migration");
    assert_eq!(stored.index_status, "rebuild_required");
    assert_eq!(
        stored.last_error.as_deref(),
        Some("journal identity changed")
    );
    assert_eq!(stored.journal_id.as_deref(), Some("journal-17"));
    assert_eq!(stored.journal_cursor.as_deref(), Some("cursor-93"));
    assert_eq!(stored.last_full_index_at, Some(101));
    assert_eq!(stored.last_incremental_sync_at, Some(202));
    drop(migrated);
    let conn = Connection::open(&path).expect("inspect migrated entry and version");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read migrated schema version");
    let entry: (String, i64) = conn
        .query_row(
            "SELECT name, is_stale FROM global_entries WHERE volume_id = ?1",
            [&volume.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read preserved Global Search entry");
    let operation: (i64, i64, i64) = conn
        .query_row(
            "SELECT operation_revision, rebuild_intent_revision, rebuild_completed_revision \
             FROM global_volumes WHERE id = ?1",
            [&volume.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read default operation state");
    assert_eq!(version, 41);
    assert_eq!(entry, ("preserved.txt".into(), 0));
    assert_eq!(operation, (0, 0, 0));
}

#[test]
fn schema_39_to_41_adds_writer_claims_without_changing_legacy_file_authority() {
    let path = test_db_path("schema39-to-41-source-writer");
    let db = Database::open(&path).expect("create schema 41 fixture");
    db.insert_file(InsertFileRequest {
        id: "schema40-preserved".into(),
        path: "schema40-preserved".into(),
        name: "schema40-preserved.txt".into(),
        extension: "txt".into(),
        size: 17,
        mtime: 23,
        ctime: 19,
        is_dir: false,
        state_code: 0,
    })
    .expect("seed legacy file row");
    drop(db);

    let conn = Connection::open(&path).expect("open schema 41 as schema 39");
    let before_file = table_snapshot(&conn, "files");
    conn.execute_batch(
        r#"
        DROP INDEX idx_library_source_root_claims_source_state;
        DROP INDEX idx_library_source_root_claims_root_path;
        DROP INDEX idx_library_source_root_claims_path_state;
        DROP INDEX idx_library_source_physical_verified;
        DROP INDEX idx_library_source_physical_lookup;
        DROP TABLE library_source_root_claims;
        DROP TABLE library_root_epoch_state;
        PRAGMA user_version = 39;
        "#,
    )
    .expect("construct schema 39 fixture");
    drop(conn);

    let migrated = Database::open(&path).expect("migrate schema 39 to schema 41");
    drop(migrated);
    let conn = Connection::open(&path).expect("inspect schema 41 result");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read migrated version"),
        41
    );
    assert_eq!(table_snapshot(&conn, "files"), before_file);
    assert_eq!(
        conn.query_row(
            "SELECT read_mode, active_catalog_epoch, active_fts_slot_id FROM library_generation_authority WHERE singleton_id=1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, Option<String>>(2)?)),
        )
        .expect("read dormant library authority"),
        ("legacy".to_string(), 0, None)
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM library_source_root_claims",
            [],
            |row| row.get::<_, i64>(0)
        )
        .expect("count new membership rows"),
        0
    );
}

#[test]
fn schema_40_to_41_adds_only_lookup_index_and_preserves_existing_facts() {
    let path = test_db_path("schema40-to-41-preserve-facts");
    let db = Database::open(&path).expect("create Schema41 fixture");
    db.insert_file(InsertFileRequest {
        id: "schema41-legacy-file".into(),
        path: "/schema41/legacy.txt".into(),
        name: "legacy.txt".into(),
        extension: "txt".into(),
        size: 23,
        mtime: 29,
        ctime: 31,
        is_dir: false,
        state_code: 0,
    })
    .expect("seed legacy file row");
    let tag = db
        .create_user_tag(CreateUserTagRequest {
            display_name: "Schema41 preserved tag".into(),
            color_token: "blue".into(),
        })
        .expect("create user tag");
    db.mutate_file_user_tags(MutateFileUserTagsRequest {
        selection: LibrarySelectionV1::Explicit {
            file_ids: vec!["schema41-legacy-file".into()],
        },
        tag_ids: vec![tag.id],
        operation: UserTagMutationOperation::Add,
        expected_count: Some(1),
    })
    .expect("attach preserved user tag");

    drop(db);
    let conn = Connection::open(&path).expect("open Schema41 fixture connection");
    conn.execute(
        "INSERT INTO scan_roots (
            id, normalized_path, display_name, source_kind, enabled,
            health_status, needs_reconciliation, created_at, updated_at
         ) VALUES ('schema41-root', '/schema41', 'Schema41 root', 'file_library', 1,
                   'healthy', 0, 1, 1)",
        [],
    )
    .expect("seed managed root");
    conn.execute(
        "INSERT INTO library_source_incarnations (
            source_uid, verification_state, source_revision, created_at, updated_at
         ) VALUES ('schema41-source', 'unverified', 1, 11, 11)",
        [],
    )
    .expect("seed source incarnation");
    conn.execute(
        "INSERT INTO library_source_observations (
            source_uid, observation_revision, legacy_file_id_snapshot, observed_path,
            name, extension, size, mtime, ctime, is_dir, physical_claim_digest,
            observation_digest, observed_at
         ) VALUES ('schema41-source', 1, 'schema41-legacy-file', '/schema41/legacy.txt',
                  'legacy.txt', 'txt', 23, 29, 31, 0, NULL, 'observation-digest', 11)",
        [],
    )
    .expect("seed source observation");
    conn.execute(
        "INSERT INTO library_legacy_attributions (
            claim_id, legacy_file_id, legacy_path_snapshot, source_uid, claim_state,
            evidence_digest, decision_revision, created_at, updated_at
         ) VALUES ('schema41-attribution', 'schema41-legacy-file', '/schema41/legacy.txt',
                  NULL, 'unresolved', NULL, 1, 11, 11)",
        [],
    )
    .expect("seed unresolved legacy attribution");
    conn.execute(
        "INSERT INTO library_root_epoch_state (
            root_id, epoch, last_owner_kind, last_owner_revision, updated_at
         ) VALUES ('schema41-root', 1, 'scan', 1, 11)",
        [],
    )
    .expect("seed root epoch");
    conn.execute(
        "INSERT INTO library_source_root_claims (
            root_id, source_uid, observed_path, root_epoch, owner_kind, owner_revision,
            observation_revision, claim_state, observed_at
         ) VALUES ('schema41-root', 'schema41-source', '/schema41/legacy.txt', 1,
                  'scan', 1, 1, 'unverified', 11)",
        [],
    )
    .expect("seed root claim");
    conn.execute_batch("DROP INDEX idx_library_source_physical_lookup; PRAGMA user_version = 40;")
        .expect("construct Schema40 fixture");
    let preserved_tables = [
        "files",
        "user_tags",
        "file_user_tags",
        "scan_roots",
        "library_source_incarnations",
        "library_source_observations",
        "library_legacy_attributions",
        "library_root_epoch_state",
        "library_source_root_claims",
    ];
    let before = preserved_tables
        .iter()
        .map(|table| (*table, table_snapshot(&conn, table)))
        .collect::<Vec<_>>();
    let verified_index_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'index' \
             AND name = 'idx_library_source_physical_verified'",
            [],
            |row| row.get(0),
        )
        .expect("snapshot verified unique index");
    drop(conn);

    let migrated = Database::open(&path).expect("migrate exact Schema40 fixture to 41");
    drop(migrated);
    let conn = Connection::open(&path).expect("inspect Schema41 migrated fixture");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read migrated schema version"),
        41
    );
    for (table, expected) in before {
        assert_eq!(table_snapshot(&conn, table), expected, "{table} changed");
    }
    let verified_index_sql_after: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'index' \
             AND name = 'idx_library_source_physical_verified'",
            [],
            |row| row.get(0),
        )
        .expect("read verified unique index after migration");
    assert_eq!(verified_index_sql_after, verified_index_sql);
    let (lookup_unique, lookup_partial): (i64, i64) = conn
        .query_row(
            "SELECT \"unique\", partial FROM pragma_index_list('library_source_incarnations') \
             WHERE name = 'idx_library_source_physical_lookup'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read new lookup index properties");
    assert_eq!((lookup_unique, lookup_partial), (0, 0));
    drop(conn);
    let _ = fs::remove_file(path);
}

#[test]
fn p0_r1_schema_37_case_changed_same_name_collision_is_rejected_without_mutation() {
    let path = test_db_path("p0-r1-schema37-case-collision");
    let db = Database::open(&path).expect("create current schema fixture");
    drop(db);
    let conn = Connection::open(&path).expect("open schema 37 case-collision fixture");
    seed_legacy_contract_rows(&conn);
    let changed_source_sql = source_incarnations_sql_with_case_changed_verification_check(&conn);
    drop_identity_foundation(&conn, 37);
    conn.execute_batch(&changed_source_sql)
        .expect("install same-name table with case-changed CHECK");
    let before_schema = full_schema_snapshot(&conn);
    let before_files = table_snapshot(&conn, "files");
    let before_tags = table_snapshot(&conn, "user_tags");
    let before_file_tags = table_snapshot(&conn, "file_user_tags");
    let before_fts = fts_rows_matching(&conn, "migrationtoken");
    drop(conn);

    let error = match Database::open(&path) {
        Ok(_) => panic!("schema 37 with a case-changed same-name CHECK must be rejected"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "library_schema38_identity_foundation_conflict: library_source_incarnations"
    );

    let conn = Connection::open(&path).expect("inspect rejected schema 37 fixture");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read rejected schema version"),
        37
    );
    assert_eq!(full_schema_snapshot(&conn), before_schema);
    assert_eq!(table_snapshot(&conn, "files"), before_files);
    assert_eq!(table_snapshot(&conn, "user_tags"), before_tags);
    assert_eq!(table_snapshot(&conn, "file_user_tags"), before_file_tags);
    assert_eq!(fts_rows_matching(&conn, "migrationtoken"), before_fts);
}

#[test]
fn p0_t07_same_path_observations_do_not_inherit_legacy_tags_or_delete_claims() {
    let path = test_db_path("p0-t07-no-inheritance");
    let db = Database::open(&path).expect("create current schema fixture");
    drop(db);
    let conn = Connection::open(&path).expect("open P0 identity fixture");
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        INSERT INTO files (id, path, name, extension, size, mtime, is_dir, state_code)
        VALUES ('legacy-file', '/same/path/reused.txt', 'reused.txt', 'txt', 10, 1, 0, 0);
        INSERT INTO user_tags (id, display_name, normalized_name, created_at, updated_at, revision)
        VALUES ('legacy-tag', 'Owner Tag', 'owner tag', 1, 1, 1);
        INSERT INTO file_user_tags (file_id, tag_id, created_at)
        VALUES ('legacy-file', 'legacy-tag', 1);
        INSERT INTO library_source_incarnations (source_uid, created_at, updated_at)
        VALUES ('source-one', 1, 1), ('source-two', 1, 1);
        INSERT INTO library_source_observations (
            source_uid, observation_revision, legacy_file_id_snapshot, observed_path,
            name, extension, size, mtime, ctime, is_dir, observation_digest, observed_at
        ) VALUES
            ('source-one', 1, 'legacy-file', '/same/path/reused.txt', 'reused.txt', 'txt', 10, 1, 1, 0, 'obs-one', 1),
            ('source-two', 1, 'legacy-file', '/same/path/reused.txt', 'reused.txt', 'txt', 10, 1, 1, 0, 'obs-two', 2);
        "#,
    )
    .expect("seed repeated path observations without attribution");

    let observation_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM library_source_observations WHERE observed_path = '/same/path/reused.txt'",
            [],
            |row| row.get(0),
        )
        .expect("count same-path observations");
    let tag_membership_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_user_tags", [], |row| row.get(0))
        .expect("count legacy tag assignments");
    let attribution_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM library_legacy_attributions",
            [],
            |row| row.get(0),
        )
        .expect("count legacy attribution claims");
    assert_eq!(observation_count, 2);
    assert_eq!(tag_membership_count, 1);
    assert_eq!(attribution_count, 0);

    conn.execute(
        "INSERT INTO library_legacy_attributions \
         (claim_id, legacy_file_id, legacy_path_snapshot, claim_state, created_at, updated_at) \
         VALUES ('legacy-claim', 'legacy-file', '/same/path/reused.txt', 'unresolved', 1, 1)",
        [],
    )
    .expect("record unresolved historical attribution");
    conn.execute("DELETE FROM files WHERE id = 'legacy-file'", [])
        .expect("delete legacy row under its existing ownership rules");
    let retained_claim: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM library_legacy_attributions WHERE legacy_file_id = 'legacy-file'",
            [],
            |row| row.get(0),
        )
        .expect("legacy attribution must not cascade from files");
    assert_eq!(retained_claim, 1);
    assert!(conn
        .execute(
            "DELETE FROM library_source_incarnations WHERE source_uid = 'source-one'",
            []
        )
        .is_err());
    let authority: (String, i64, Option<String>, i64) = conn
        .query_row(
            "SELECT read_mode, active_catalog_epoch, active_fts_slot_id, revision \
             FROM library_generation_authority WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read dormant authority after same-path observations");
    assert_eq!(authority, ("legacy".to_string(), 0, None, 1));
}

#[test]
fn p0_t08_identity_foundation_foreign_keys_and_checks_reject_invalid_rows() {
    let path = test_db_path("p0-t08-constraints");
    let db = Database::open(&path).expect("create current schema fixture");
    drop(db);
    let conn = Connection::open(&path).expect("open disposable constraint fixture");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys for constraint fixture");

    assert!(conn
        .execute(
            "INSERT INTO library_source_incarnations (source_uid, created_at, updated_at) \
             VALUES (NULL, 1, 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_source_incarnations \
             (source_uid, verification_state, created_at, updated_at) VALUES ('bad-enum', 'unknown', 1, 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_source_incarnations \
             (source_uid, source_revision, created_at, updated_at) VALUES ('bad-revision', 0, 1, 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_source_incarnations \
             (source_uid, verification_state, created_at, updated_at) VALUES ('forged-verified', 'verified', 1, 1)",
            [],
        )
        .is_err());

    conn.execute(
        "INSERT INTO library_source_incarnations (source_uid, created_at, updated_at) \
         VALUES ('source-valid', 1, 1)",
        [],
    )
    .expect("insert unverified source identity");
    assert!(conn
        .execute(
            "INSERT INTO library_source_incarnations (source_uid, created_at, updated_at) \
             VALUES ('source-valid', 2, 2)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_source_observations (
                 source_uid, observation_revision, observed_path, name, size, mtime,
                 ctime, is_dir, observation_digest, observed_at
             ) VALUES ('unknown-source', 1, '/x', 'x', 0, 0, 0, 0, 'digest', 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_source_observations (
                 source_uid, observation_revision, observed_path, name, size, mtime,
                 ctime, is_dir, observation_digest, observed_at
             ) VALUES ('source-valid', 0, '/x', 'x', 0, 0, 0, 0, 'digest', 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_source_observations (
                 source_uid, observation_revision, observed_path, name, size, mtime,
                 ctime, is_dir, observation_digest, observed_at
             ) VALUES ('source-valid', 1, '/x', 'x', -1, 0, 0, 0, 'digest', 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_source_observations (
                 source_uid, observation_revision, observed_path, name, size, mtime,
                 ctime, is_dir, observation_digest, observed_at
             ) VALUES ('source-valid', 1, '/x', 'x', 0, 0, 0, 2, 'digest', 1)",
            [],
        )
        .is_err());
    let observation_sql = "INSERT INTO library_source_observations (
        source_uid, observation_revision, observed_path, name, size, mtime,
        ctime, is_dir, observation_digest, observed_at
    ) VALUES ('source-valid', 1, '/x', 'x', 0, 0, 0, 0, 'digest', 1)";
    conn.execute(observation_sql, [])
        .expect("insert first source observation");
    assert!(conn.execute(observation_sql, []).is_err());

    assert!(conn
        .execute(
            "INSERT INTO library_legacy_attributions (
                 claim_id, legacy_file_id, legacy_path_snapshot, claim_state, created_at, updated_at
             ) VALUES ('forged-claim', 'legacy-id', '/legacy/path', 'verified', 1, 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_legacy_attributions (
                 claim_id, legacy_file_id, legacy_path_snapshot, source_uid, claim_state,
                 evidence_digest, created_at, updated_at
             ) VALUES ('empty-evidence', 'legacy-id', '/legacy/path', 'source-valid', 'user_confirmed', '', 1, 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_legacy_attributions (
                 claim_id, legacy_file_id, legacy_path_snapshot, source_uid,
                 created_at, updated_at
             ) VALUES ('unknown-claim-source', 'legacy-id', '/legacy/path', 'missing-source', 1, 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_legacy_attributions (
                 claim_id, legacy_file_id, legacy_path_snapshot, claim_state, created_at, updated_at
             ) VALUES ('invalid-claim-state', 'legacy-id', '/legacy/path', 'automatic', 1, 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO library_generation_authority \
             (singleton_id, read_mode, active_catalog_epoch, revision, updated_at) \
             VALUES (2, 'legacy', 0, 1, 1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "UPDATE library_generation_authority SET active_catalog_epoch = 1 WHERE singleton_id = 1",
            [],
        )
        .is_err());
}

#[test]
fn p0_t09_file_library_search_tags_scan_upsert_and_restore_keep_legacy_authority() {
    let path = test_db_path("p0-t09-legacy-flows");
    let root = path.with_extension("library-root");
    fs::create_dir_all(&root).expect("create indexed root");
    let original_path = root.join("legacysearchtoken.txt");
    let moved_path = root.join("restored-target-token.txt");
    fs::write(&original_path, b"legacy file library and scan token").expect("write indexed file");

    let db = Database::open(&path).expect("create current schema");
    assert_eq!(
        db.upsert_files_by_paths(&[original_path.to_string_lossy().into_owned()])
            .expect("scanner upsert path"),
        1
    );
    let canonical_root = fs::canonicalize(&root)
        .expect("canonicalize indexed root")
        .to_string_lossy()
        .replace('\\', "/");
    let normalized_root = if let Some(unc_root) = canonical_root.strip_prefix("//?/UNC/") {
        format!("//{unc_root}")
    } else if let Some(root) = canonical_root.strip_prefix("//?/") {
        root.to_string()
    } else {
        canonical_root
    };
    let normalized_file_path = original_path.to_string_lossy().replace('\\', "/");
    let conn = Connection::open(&path).expect("open root fixture");
    let file_id: String = conn
        .query_row(
            "SELECT id FROM files WHERE path = ?1",
            [&normalized_file_path],
            |row| row.get(0),
        )
        .expect("read scanner file ID");
    drop(conn);

    assert_eq!(
        db.search_files("legacysearchtoken", Some(10))
            .expect("search indexed file")
            .len(),
        1
    );

    let tag = db
        .create_user_tag(CreateUserTagRequest {
            display_name: "P0 legacy tag".to_string(),
            color_token: "blue".to_string(),
        })
        .expect("create legacy user tag");
    let mutation = db
        .mutate_file_user_tags(MutateFileUserTagsRequest {
            selection: LibrarySelectionV1::Explicit {
                file_ids: vec![file_id.clone()],
            },
            tag_ids: vec![tag.id.clone()],
            operation: UserTagMutationOperation::Add,
            expected_count: Some(1),
        })
        .expect("mutate legacy file tag");
    assert_eq!(mutation.applied_count, 1);

    fs::rename(&original_path, &moved_path).expect("simulate committed move");
    assert!(db
        .update_file_after_successful_operation(
            &file_id,
            &original_path.to_string_lossy(),
            &moved_path.to_string_lossy(),
            "restored-target-token.txt",
        )
        .expect("update legacy file after move"));
    fs::rename(&moved_path, &original_path).expect("simulate committed restore");
    assert!(db
        .update_file_after_successful_restore(&OperationLogDto {
            id: "p0-t09-restore".to_string(),
            batch_id: "p0-t09-batch".to_string(),
            operation_type: "move".to_string(),
            source_path: original_path.to_string_lossy().into_owned(),
            target_path: moved_path.to_string_lossy().into_owned(),
            old_name: "legacysearchtoken.txt".to_string(),
            new_name: "restored-target-token.txt".to_string(),
            status: "success".to_string(),
            error_message: None,
            created_at: "1".to_string(),
            can_undo: true,
            path_before: original_path.to_string_lossy().into_owned(),
            path_after: moved_path.to_string_lossy().into_owned(),
            name_before: "legacysearchtoken.txt".to_string(),
            name_after: "restored-target-token.txt".to_string(),
            can_restore: true,
            restored_at: None,
            restore_status: "restored".to_string(),
            restore_error: None,
            source_size: None,
            source_modified_ns: None,
            source_platform_file_id: None,
            source_platform_volume_id: None,
            source_quick_hash: None,
            source_full_hash: None,
            target_platform_file_id: None,
            target_platform_volume_id: None,
            target_full_hash: None,
            source_claim_path: None,
            operation_phase: "completed".to_string(),
            claim_created_at: None,
            claim_platform_file_id: None,
            claim_platform_volume_id: None,
            claim_full_hash: None,
            restore_claim_path: None,
            restore_phase: "completed".to_string(),
            restore_claim_created_at: None,
            restore_claim_platform_file_id: None,
            restore_claim_platform_volume_id: None,
            restore_claim_full_hash: None,
        })
        .expect("restore legacy indexed file"));
    let conn = Connection::open(&path).expect("open root fixture");
    conn.execute(
        "INSERT INTO scan_roots (
             id, normalized_path, display_name, source_kind, enabled, health_status,
             current_generation, needs_reconciliation, created_at, updated_at
         ) VALUES ('p0-t09-root', ?1, 'P0 test root', 'file_library', 1, 'healthy', 1, 0, 1, 1)",
        [&normalized_root],
    )
    .expect("register File Library root");
    drop(conn);
    let file_library = db
        .query_file_library_v2(FileQueryRequestV2 {
            version: 2,
            request_id: "p0-t09-list".to_string(),
            query: FileQuerySpecV2 {
                scope: FileLibraryScopeV2::Roots {
                    scan_root_ids: vec!["p0-t09-root".to_string()],
                },
                text: Some("legacysearchtoken".to_string()),
                filters: FileQueryFiltersV2::default(),
                sort: Default::default(),
            },
            page_size: 10,
            cursor: None,
        })
        .expect("list through File Library V2");
    assert_eq!(file_library.files.len(), 1);
    assert_eq!(file_library.files[0].id, file_id);
    assert_eq!(
        db.search_files("legacysearchtoken", Some(10))
            .expect("search after restore")
            .len(),
        1
    );
    assert_eq!(
        db.list_user_tags().expect("list legacy tags")[0].usage_count,
        1
    );

    let conn = Connection::open(&path).expect("inspect dormant state after legacy flows");
    let authority: (String, i64, Option<String>, i64) = conn
        .query_row(
            "SELECT read_mode, active_catalog_epoch, active_fts_slot_id, revision \
             FROM library_generation_authority WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read authority after legacy flows");
    assert_eq!(authority, ("legacy".to_string(), 0, None, 1));
    for table in [
        "library_source_incarnations",
        "library_source_observations",
        "library_legacy_attributions",
    ] {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                .get::<_, i64>(0))
                .expect("count dormant identity rows"),
            0
        );
    }
}
