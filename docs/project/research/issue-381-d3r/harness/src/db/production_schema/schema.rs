use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::sync::OnceLock;

/// 当前期望的 schema 版本号，每次需要改动 schema 时 +1
pub(crate) const CURRENT_SCHEMA_VERSION: i32 = 37;
static FTS5_CHECKED: OnceLock<()> = OnceLock::new();

fn assert_fts5_available(conn: &Connection) -> Result<(), DbError> {
    if FTS5_CHECKED.get().is_none() {
        conn.execute_batch(
            r#"
            CREATE VIRTUAL TABLE temp.fts5_probe USING fts5(value, tokenize='trigram');
            DROP TABLE temp.fts5_probe;
            "#,
        )?;
        let _ = FTS5_CHECKED.set(());
    }
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<i32, DbError> {
    conn.query_row("SELECT user_version FROM pragma_user_version", [], |row| {
        row.get(0)
    })
    .map_err(DbError::from)
}

fn set_schema_version(conn: &Connection, version: i32) -> Result<(), DbError> {
    // PRAGMA user_version 不支持参数绑定，用格式化字符串（整数无 SQL 注入风险）
    conn.execute_batch(&format!("PRAGMA user_version = {version}"))
        .map_err(DbError::from)
}

pub(crate) fn migrate(conn: &Connection) -> Result<(), DbError> {
    assert_fts5_available(conn)?;
    let version = schema_version(conn)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(DbError::Validation(format!(
            "Database schema version {version} is newer than this app supports ({CURRENT_SCHEMA_VERSION})."
        )));
    }
    if version == CURRENT_SCHEMA_VERSION {
        ensure_global_index_schema(conn)?;
        ensure_global_index_hardening(conn)?;
        ensure_journal_state_triggers(conn)?;
        ensure_scan_ledger_schema(conn)?;
        ensure_watcher_reconciliation_schema(conn)?;
        ensure_dedupe_schema(conn)?;
        ensure_analysis_schema(conn)?;
        ensure_file_library_schema(conn)?;
        ensure_organization_plan_schema(conn)?;
        ensure_rule_proposal_schema(conn)?;
        ensure_content_schema(conn)?;
        backfill_scan_roots_from_settings(conn)?;
        return Ok(());
    }
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let migration_result = (|| -> Result<(), DbError> {
        if version < 1 {
            // 建表 + 基础索引
            conn.execute_batch(
                r#"
            CREATE TABLE IF NOT EXISTS files (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                extension TEXT NOT NULL DEFAULT '',
                size INTEGER NOT NULL DEFAULT 0,
                mtime INTEGER NOT NULL DEFAULT 0,
                is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1)),
                state_code INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_name ON files(name);
            CREATE INDEX IF NOT EXISTS idx_files_extension ON files(extension);
            CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime DESC);
            "#,
            )?;
            set_schema_version(conn, 1)?;
        }
        if version < 2 {
            // 分类字段 + FTS + 触发器
            execute_column_migrations(
            conn,
            &[
                "ALTER TABLE files ADD COLUMN file_type TEXT NOT NULL DEFAULT 'Other';",
                "ALTER TABLE files ADD COLUMN purpose TEXT NOT NULL DEFAULT 'Unknown';",
                "ALTER TABLE files ADD COLUMN lifecycle TEXT NOT NULL DEFAULT 'Inbox';",
                "ALTER TABLE files ADD COLUMN context TEXT NOT NULL DEFAULT '';",
                "ALTER TABLE files ADD COLUMN risk_level TEXT NOT NULL DEFAULT 'Normal';",
                "ALTER TABLE files ADD COLUMN suggested_action TEXT NOT NULL DEFAULT 'Keep';",
                "ALTER TABLE files ADD COLUMN suggested_target_path TEXT NOT NULL DEFAULT '';",
                "ALTER TABLE files ADD COLUMN suggested_name TEXT NOT NULL DEFAULT '';",
                "ALTER TABLE files ADD COLUMN confidence REAL NOT NULL DEFAULT 0.5;",
                "ALTER TABLE files ADD COLUMN classification_reason TEXT NOT NULL DEFAULT 'Indexed by Zen Canvas Tauri backend.';",
                "ALTER TABLE files ADD COLUMN matched_rules TEXT NOT NULL DEFAULT '[]';",
                "ALTER TABLE files ADD COLUMN requires_confirmation INTEGER NOT NULL DEFAULT 0;",
            ],
        )?;
            conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_files_file_type ON files(file_type);
            CREATE INDEX IF NOT EXISTS idx_files_purpose ON files(purpose);
            CREATE INDEX IF NOT EXISTS idx_files_lifecycle ON files(lifecycle);
            CREATE INDEX IF NOT EXISTS idx_files_risk_level ON files(risk_level);
            CREATE INDEX IF NOT EXISTS idx_files_requires_confirmation ON files(requires_confirmation);
            "#,
        )?;
            ensure_trigram_fts(conn)?;
            ensure_fts_triggers(conn)?;
            set_schema_version(conn, 2)?;
        }
        if version < 3 {
            // 新增 ctime 字段（真实创建时间）
            execute_column_migrations(
                conn,
                &["ALTER TABLE files ADD COLUMN ctime INTEGER NOT NULL DEFAULT 0;"],
            )?;
            set_schema_version(conn, 3)?;
        }
        if version < 4 {
            conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS operation_batches (
                id TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL,
                status TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS operation_logs (
                id TEXT PRIMARY KEY,
                batch_id TEXT NOT NULL,
                operation_type TEXT NOT NULL,
                source_path TEXT NOT NULL,
                target_path TEXT NOT NULL,
                old_name TEXT NOT NULL,
                new_name TEXT NOT NULL,
                status TEXT NOT NULL,
                error_message TEXT,
                created_at INTEGER NOT NULL,
                can_undo INTEGER NOT NULL DEFAULT 0,
                path_before TEXT NOT NULL,
                path_after TEXT NOT NULL,
                name_before TEXT NOT NULL,
                name_after TEXT NOT NULL,
                can_restore INTEGER NOT NULL DEFAULT 0,
                restored_at INTEGER,
                restore_status TEXT NOT NULL DEFAULT 'not_restored',
                restore_error TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_operation_logs_batch_id ON operation_logs(batch_id);
            CREATE INDEX IF NOT EXISTS idx_operation_logs_created_at ON operation_logs(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_operation_logs_restore_status ON operation_logs(restore_status);
            "#,
        )?;
            set_schema_version(conn, 4)?;
        }
        if version < 5 {
            execute_column_migrations(
                conn,
                &[
                    "ALTER TABLE files ADD COLUMN is_stale INTEGER NOT NULL DEFAULT 0;",
                    "ALTER TABLE files ADD COLUMN last_seen_at INTEGER NOT NULL DEFAULT 0;",
                ],
            )?;
            conn.execute_batch(
                r#"
            CREATE INDEX IF NOT EXISTS idx_files_is_stale ON files(is_stale);
            CREATE INDEX IF NOT EXISTS idx_files_last_seen_at ON files(last_seen_at DESC);
            "#,
            )?;
            set_schema_version(conn, 5)?;
        }
        if version < 6 {
            execute_column_migrations(
            conn,
            &[
                "ALTER TABLE files ADD COLUMN last_classified_at INTEGER NOT NULL DEFAULT 0;",
                "ALTER TABLE files ADD COLUMN classified_rule_version TEXT NOT NULL DEFAULT '';",
                "ALTER TABLE files ADD COLUMN last_classified_mtime INTEGER NOT NULL DEFAULT 0;",
                "ALTER TABLE files ADD COLUMN last_classified_size INTEGER NOT NULL DEFAULT 0;",
            ],
        )?;
            conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_files_classified_version ON files(classified_rule_version);
            CREATE INDEX IF NOT EXISTS idx_files_last_classified_at ON files(last_classified_at DESC);
            CREATE INDEX IF NOT EXISTS idx_files_classification_fingerprint ON files(last_classified_mtime, last_classified_size);
            "#,
        )?;
            set_schema_version(conn, 6)?;
        }
        if version < 7 {
            conn.execute_batch(
                r#"
            CREATE TABLE IF NOT EXISTS rules (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'user',
                enabled INTEGER NOT NULL DEFAULT 1,
                priority REAL NOT NULL DEFAULT 0,
                weight REAL NOT NULL DEFAULT 0,
                root_operator TEXT NOT NULL DEFAULT 'AND',
                groups_json TEXT NOT NULL DEFAULT '[]',
                action_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS idx_rules_source ON rules(source);
            CREATE INDEX IF NOT EXISTS idx_rules_enabled ON rules(enabled);
            CREATE INDEX IF NOT EXISTS idx_rules_priority ON rules(priority DESC);
            "#,
            )?;
            set_schema_version(conn, 7)?;
        }
        if version < 8 {
            conn.execute_batch(
                r#"
            CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
            )?;
            conn.execute(
                r#"
            INSERT OR IGNORE INTO app_settings (key, value)
            VALUES (?1, ?2)
            "#,
                params![
                    crate::settings::APP_SETTINGS_KEY,
                    crate::settings::default_settings_json()?
                ],
            )?;
            set_schema_version(conn, 8)?;
        }
        if version < 9 {
            execute_column_migrations(
                conn,
                &["ALTER TABLE files ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';"],
            )?;
            conn.execute_batch(
                r#"
            CREATE INDEX IF NOT EXISTS idx_files_dedupe
            ON files(size, content_hash)
            WHERE is_dir = 0 AND size > 0;
            "#,
            )?;
            set_schema_version(conn, 9)?;
        }
        if version < 10 {
            execute_column_migrations(
                conn,
                &[r#"
                ALTER TABLE files ADD COLUMN classification_status TEXT NOT NULL DEFAULT 'unclassified'
                CHECK (classification_status IN ('unclassified', 'classified'));
            "#],
            )?;
            conn.execute(
                r#"
            UPDATE files
            SET classification_status = 'classified'
            WHERE last_classified_at > 0
               OR matched_rules <> '[]'
               OR purpose <> 'Unknown'
            "#,
                [],
            )?;
            set_schema_version(conn, 10)?;
        }
        if version < 11 {
            conn.execute_batch(
                r#"
            CREATE INDEX IF NOT EXISTS idx_files_active_mtime
            ON files(is_stale, mtime DESC);

            CREATE INDEX IF NOT EXISTS idx_files_lifecycle_mtime
            ON files(is_stale, lifecycle, mtime DESC);

            CREATE INDEX IF NOT EXISTS idx_files_action_mtime
            ON files(is_stale, suggested_action, mtime DESC);

            CREATE INDEX IF NOT EXISTS idx_files_review_mtime
            ON files(is_stale, requires_confirmation, suggested_action, mtime DESC);

            CREATE INDEX IF NOT EXISTS idx_files_risk_mtime
            ON files(is_stale, risk_level, mtime DESC);

            CREATE INDEX IF NOT EXISTS idx_files_scope_path
            ON files(is_stale, path);
            "#,
            )?;
            set_schema_version(conn, 11)?;
        }
        if version < 12 {
            ensure_trigram_fts(conn)?;
            ensure_fts_triggers(conn)?;
            set_schema_version(conn, 12)?;
        }
        if version < 13 {
            conn.execute_batch(
                r#"
            CREATE TABLE IF NOT EXISTS cleanup_trash_batches (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                root TEXT,
                total_items INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                status TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS cleanup_trash_items (
                id TEXT PRIMARY KEY,
                batch_id TEXT NOT NULL,
                original_path TEXT NOT NULL,
                trash_path TEXT NOT NULL,
                name TEXT NOT NULL,
                size INTEGER NOT NULL,
                moved_at TEXT NOT NULL,
                restored_at TEXT,
                status TEXT NOT NULL,
                message TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_cleanup_trash_items_batch_id
            ON cleanup_trash_items(batch_id);
            CREATE INDEX IF NOT EXISTS idx_cleanup_trash_batches_created_at
            ON cleanup_trash_batches(created_at DESC);
            "#,
            )?;
            set_schema_version(conn, 13)?;
        }
        if version < 14 {
            conn.execute_batch(
                r#"
            CREATE TABLE IF NOT EXISTS classification_history (
                id TEXT PRIMARY KEY,
                file_id TEXT NOT NULL,
                file_name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                extension TEXT NOT NULL DEFAULT '',
                source TEXT NOT NULL,
                file_type TEXT NOT NULL,
                purpose TEXT NOT NULL,
                lifecycle TEXT NOT NULL,
                context TEXT NOT NULL DEFAULT '',
                risk_level TEXT NOT NULL,
                suggested_action TEXT NOT NULL,
                suggested_target_path TEXT NOT NULL DEFAULT '',
                suggested_name TEXT NOT NULL DEFAULT '',
                confidence REAL NOT NULL DEFAULT 0.5,
                reason TEXT NOT NULL DEFAULT '',
                keywords_json TEXT NOT NULL DEFAULT '[]',
                user_confirmed INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_classification_history_file_id
            ON classification_history(file_id);
            CREATE INDEX IF NOT EXISTS idx_classification_history_name
            ON classification_history(file_name);
            CREATE INDEX IF NOT EXISTS idx_classification_history_confirmed
            ON classification_history(user_confirmed);
            CREATE INDEX IF NOT EXISTS idx_classification_history_source
            ON classification_history(source);

            CREATE TABLE IF NOT EXISTS classification_feedback (
                id TEXT PRIMARY KEY,
                file_id TEXT NOT NULL,
                file_name TEXT NOT NULL,
                original_json TEXT NOT NULL,
                corrected_json TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_classification_feedback_file_id
            ON classification_feedback(file_id);
            "#,
            )?;
            set_schema_version(conn, 14)?;
        }
        if version < 15 {
            conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS operation_batches (
                id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, status TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS operation_logs (
                id TEXT PRIMARY KEY, batch_id TEXT NOT NULL, operation_type TEXT NOT NULL,
                source_path TEXT NOT NULL, target_path TEXT NOT NULL, old_name TEXT NOT NULL,
                new_name TEXT NOT NULL, status TEXT NOT NULL, error_message TEXT,
                created_at INTEGER NOT NULL, can_undo INTEGER NOT NULL DEFAULT 0,
                path_before TEXT NOT NULL, path_after TEXT NOT NULL, name_before TEXT NOT NULL,
                name_after TEXT NOT NULL, can_restore INTEGER NOT NULL DEFAULT 0,
                restored_at INTEGER, restore_status TEXT NOT NULL DEFAULT 'not_restored',
                restore_error TEXT
            );
            CREATE TABLE IF NOT EXISTS cleanup_trash_batches (
                id TEXT PRIMARY KEY, created_at TEXT NOT NULL, root TEXT,
                total_items INTEGER NOT NULL, total_size INTEGER NOT NULL, status TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS cleanup_trash_items (
                id TEXT PRIMARY KEY, batch_id TEXT NOT NULL, original_path TEXT NOT NULL,
                trash_path TEXT NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL,
                moved_at TEXT NOT NULL, restored_at TEXT, status TEXT NOT NULL, message TEXT
            );

            CREATE TRIGGER IF NOT EXISTS operation_logs_batch_guard_insert
            BEFORE INSERT ON operation_logs
            WHEN NOT EXISTS (SELECT 1 FROM operation_batches WHERE id = NEW.batch_id)
            BEGIN SELECT RAISE(ABORT, 'operation log batch does not exist'); END;

            CREATE TRIGGER IF NOT EXISTS operation_logs_status_guard_insert
            BEFORE INSERT ON operation_logs
            WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped')
              OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled')
            BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;

            CREATE TRIGGER IF NOT EXISTS operation_logs_status_guard_update
            BEFORE UPDATE OF status, restore_status ON operation_logs
            WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped')
              OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled')
            BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;

            CREATE TRIGGER IF NOT EXISTS cleanup_items_batch_guard_insert
            BEFORE INSERT ON cleanup_trash_items
            WHEN NOT EXISTS (SELECT 1 FROM cleanup_trash_batches WHERE id = NEW.batch_id)
            BEGIN SELECT RAISE(ABORT, 'cleanup item batch does not exist'); END;

            CREATE TRIGGER IF NOT EXISTS cleanup_items_status_guard_insert
            BEFORE INSERT ON cleanup_trash_items
            WHEN NEW.status NOT IN ('pending', 'moved', 'restored', 'failed', 'missing')
            BEGIN SELECT RAISE(ABORT, 'invalid cleanup item status'); END;

            CREATE TRIGGER IF NOT EXISTS cleanup_items_status_guard_update
            BEFORE UPDATE OF status ON cleanup_trash_items
            WHEN NEW.status NOT IN ('pending', 'moved', 'restored', 'failed', 'missing')
            BEGIN SELECT RAISE(ABORT, 'invalid cleanup item status'); END;
            "#,
        )?;
            set_schema_version(conn, 15)?;
        }
        if version < 16 {
            conn.execute_batch(
            r#"
            DROP TRIGGER IF EXISTS operation_logs_status_guard_insert;
            DROP TRIGGER IF EXISTS operation_logs_status_guard_update;

            CREATE TRIGGER operation_logs_status_guard_insert
            BEFORE INSERT ON operation_logs
            WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped')
              OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled')
            BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;

            CREATE TRIGGER operation_logs_status_guard_update
            BEFORE UPDATE OF status, restore_status ON operation_logs
            WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped')
              OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled')
            BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;
            "#,
        )?;
            set_schema_version(conn, 16)?;
        }
        if version < 17 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )?;
            execute_column_migrations(
                conn,
                &["ALTER TABLE app_settings ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;"],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO app_settings (key, value, revision) VALUES (?1, ?2, 0)",
                params![
                    crate::settings::APP_SETTINGS_KEY,
                    crate::settings::default_settings_json()?
                ],
            )?;
            set_schema_version(conn, 17)?;
        }
        if version < 18 {
            execute_column_migrations(
                conn,
                &[
                    "ALTER TABLE operation_logs ADD COLUMN source_size INTEGER;",
                    "ALTER TABLE operation_logs ADD COLUMN source_modified_ns TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN source_platform_file_id TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN source_quick_hash TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN target_platform_file_id TEXT;",
                ],
            )?;
            conn.execute_batch(
                r#"
                DROP TRIGGER IF EXISTS operation_logs_status_guard_insert;
                DROP TRIGGER IF EXISTS operation_logs_status_guard_update;
                CREATE TRIGGER operation_logs_status_guard_insert
                BEFORE INSERT ON operation_logs
                WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped', 'manual_review')
                  OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled', 'manual_review')
                BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;
                CREATE TRIGGER operation_logs_status_guard_update
                BEFORE UPDATE OF status, restore_status ON operation_logs
                WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped', 'manual_review')
                  OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled', 'manual_review')
                BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;
                "#,
            )?;
            set_schema_version(conn, 18)?;
        }
        if version < 19 {
            execute_column_migrations(
                conn,
                &[
                    "ALTER TABLE cleanup_trash_items ADD COLUMN source_modified_ns TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN source_platform_file_id TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN source_quick_hash TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN trash_modified_ns TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN trash_platform_volume_id TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN trash_platform_file_id TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN trash_quick_hash TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN identity_status TEXT NOT NULL DEFAULT 'legacy_unverified';",
                ],
            )?;
            set_schema_version(conn, 19)?;
        }
        if version < 20 {
            migrate_invalid_rule_domain_values(conn)?;
            conn.execute(
                r#"
                UPDATE operation_logs
                SET can_restore = 0,
                    restore_status = 'manual_review',
                    restore_error = 'manual_review: legacy identity unavailable'
                WHERE status = 'success'
                  AND can_restore = 1
                  AND (
                    source_size IS NULL
                    OR source_modified_ns IS NULL
                    OR source_quick_hash IS NULL
                    OR target_platform_file_id IS NULL
                  )
                "#,
                [],
            )?;
            set_schema_version(conn, 20)?;
        }
        if version < 21 {
            execute_column_migrations(
                conn,
                &[
                    "ALTER TABLE operation_logs ADD COLUMN source_full_hash TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN target_full_hash TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN source_full_hash TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN trash_full_hash TEXT;",
                ],
            )?;
            conn.execute(
                r#"
                UPDATE operation_logs
                SET can_restore = 0,
                    restore_status = 'manual_review',
                    restore_error = 'manual_review: complete identity unavailable'
                WHERE status = 'success'
                  AND can_restore = 1
                  AND (source_full_hash IS NULL OR target_full_hash IS NULL)
                "#,
                [],
            )?;
            conn.execute(
                r#"
                UPDATE cleanup_trash_items
                SET identity_status = 'legacy_unverified',
                    message = COALESCE(message, 'Complete identity is unavailable; manual review is required.')
                WHERE status = 'moved'
                  AND (source_full_hash IS NULL OR trash_full_hash IS NULL)
                "#,
                [],
            )?;
            set_schema_version(conn, 21)?;
        }
        if version < 22 {
            execute_column_migrations(
                conn,
                &[
                    "ALTER TABLE operation_logs ADD COLUMN source_claim_path TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN operation_phase TEXT NOT NULL DEFAULT 'completed';",
                    "ALTER TABLE operation_logs ADD COLUMN claim_created_at TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN claim_platform_file_id TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN claim_full_hash TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN source_claim_path TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN operation_phase TEXT NOT NULL DEFAULT 'completed';",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN claim_created_at TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN claim_platform_file_id TEXT;",
                    "ALTER TABLE cleanup_trash_items ADD COLUMN claim_full_hash TEXT;",
                ],
            )?;
            conn.execute_batch(
                r#"
                DROP TRIGGER IF EXISTS operation_logs_phase_guard_insert;
                DROP TRIGGER IF EXISTS operation_logs_phase_guard_update;
                CREATE TRIGGER operation_logs_phase_guard_insert
                BEFORE INSERT ON operation_logs
                WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
                    'target_committed', 'source_cleanup_pending', 'completed',
                    'rolled_back', 'manual_review')
                BEGIN SELECT RAISE(ABORT, 'invalid operation phase'); END;
                CREATE TRIGGER operation_logs_phase_guard_update
                BEFORE UPDATE OF operation_phase ON operation_logs
                WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
                    'target_committed', 'source_cleanup_pending', 'completed',
                    'rolled_back', 'manual_review')
                BEGIN SELECT RAISE(ABORT, 'invalid operation phase'); END;

                DROP TRIGGER IF EXISTS cleanup_items_phase_guard_insert;
                DROP TRIGGER IF EXISTS cleanup_items_phase_guard_update;
                CREATE TRIGGER cleanup_items_phase_guard_insert
                BEFORE INSERT ON cleanup_trash_items
                WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
                    'target_committed', 'source_cleanup_pending', 'completed',
                    'rolled_back', 'manual_review')
                BEGIN SELECT RAISE(ABORT, 'invalid cleanup operation phase'); END;
                CREATE TRIGGER cleanup_items_phase_guard_update
                BEFORE UPDATE OF operation_phase ON cleanup_trash_items
                WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
                    'target_committed', 'source_cleanup_pending', 'completed',
                    'rolled_back', 'manual_review')
                BEGIN SELECT RAISE(ABORT, 'invalid cleanup operation phase'); END;

                DROP TRIGGER IF EXISTS cleanup_items_status_guard_insert;
                DROP TRIGGER IF EXISTS cleanup_items_status_guard_update;
                CREATE TRIGGER cleanup_items_status_guard_insert
                BEFORE INSERT ON cleanup_trash_items
                WHEN NEW.status NOT IN ('pending', 'moved', 'restored', 'failed', 'missing',
                    'manual_review', 'canceled')
                BEGIN SELECT RAISE(ABORT, 'invalid cleanup item status'); END;
                CREATE TRIGGER cleanup_items_status_guard_update
                BEFORE UPDATE OF status ON cleanup_trash_items
                WHEN NEW.status NOT IN ('pending', 'moved', 'restored', 'failed', 'missing',
                    'manual_review', 'canceled')
                BEGIN SELECT RAISE(ABORT, 'invalid cleanup item status'); END;
                "#,
            )?;
            set_schema_version(conn, 22)?;
        }
        if version < 23 {
            execute_column_migrations(
                conn,
                &[
                    "ALTER TABLE operation_logs ADD COLUMN restore_claim_path TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN restore_phase TEXT NOT NULL DEFAULT 'idle';",
                    "ALTER TABLE operation_logs ADD COLUMN restore_claim_created_at TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN restore_claim_platform_file_id TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN restore_claim_full_hash TEXT;",
                ],
            )?;
            conn.execute(
                r#"
                UPDATE operation_logs
                SET restore_phase = CASE
                    WHEN restore_status = 'pending' THEN 'prepared'
                    ELSE 'idle'
                END
                WHERE restore_phase IS NULL OR restore_phase = 'idle'
                "#,
                [],
            )?;
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_operation_logs_restore_phase ON operation_logs(restore_phase);",
            )?;
            ensure_journal_state_triggers(conn)?;
            set_schema_version(conn, 23)?;
        }
        if version < 24 {
            execute_column_migrations(
                conn,
                &[
                    "ALTER TABLE operation_logs ADD COLUMN source_platform_volume_id TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN target_platform_volume_id TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN claim_platform_volume_id TEXT;",
                    "ALTER TABLE operation_logs ADD COLUMN restore_claim_platform_volume_id TEXT;",
                ],
            )?;
            set_schema_version(conn, 24)?;
        }
        if version < 25 {
            ensure_global_index_schema(conn)?;
            set_schema_version(conn, 25)?;
        }
        if version < 26 {
            ensure_global_index_hardening(conn)?;
            set_schema_version(conn, 26)?;
        }
        if version < 27 {
            ensure_scan_ledger_schema(conn)?;
            backfill_scan_roots_from_settings(conn)?;
            set_schema_version(conn, 27)?;
        }
        if version < 28 {
            ensure_watcher_reconciliation_schema(conn)?;
            set_schema_version(conn, 28)?;
        }
        if version < 29 {
            ensure_dedupe_schema(conn)?;
            set_schema_version(conn, 29)?;
        }
        if version < 30 {
            ensure_analysis_schema(conn)?;
            set_schema_version(conn, 30)?;
        }
        if version < 31 {
            ensure_file_library_schema(conn)?;
            set_schema_version(conn, 31)?;
        }
        if version < 32 {
            ensure_organization_plan_schema(conn)?;
            set_schema_version(conn, 32)?;
        }
        if version < 33 {
            ensure_rule_proposal_schema(conn)?;
            set_schema_version(conn, 33)?;
        }
        if version < 34 {
            ensure_content_schema(conn)?;
            set_schema_version(conn, 34)?;
        }
        if version < 35 {
            migrate_cleanup_identity_encoding(conn)?;
            set_schema_version(conn, 35)?;
        }
        if version < 36 {
            super::schema_automation::ensure_automation_schema(conn)?;
            set_schema_version(conn, 36)?;
        }
        if version < 37 {
            super::schema_automation_triggers::migrate_automation_triggers(conn)?;
            set_schema_version(conn, 37)?;
        }
        Ok(())
    })();
    match migration_result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

const MACOS_CLEANUP_PHYSICAL_ID_PREFIX: &str = "macos-dev-ino:";

fn parse_legacy_cleanup_identity(value: &str) -> Option<(String, String)> {
    let payload = value.strip_prefix(MACOS_CLEANUP_PHYSICAL_ID_PREFIX)?;
    let mut parts = payload.split(':');
    let volume_id = parts.next()?;
    let file_id = parts.next()?;
    if volume_id.is_empty()
        || file_id.is_empty()
        || parts.next().is_some()
        || volume_id.chars().any(char::is_whitespace)
        || file_id.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some((volume_id.to_string(), file_id.to_string()))
}

fn migrate_cleanup_identity_encoding(conn: &Connection) -> Result<(), DbError> {
    execute_column_migrations(
        conn,
        &["ALTER TABLE cleanup_trash_items ADD COLUMN source_platform_volume_id TEXT;"],
    )?;

    let rows = {
        let mut stmt = conn.prepare(
            r#"
            SELECT id,
                   source_platform_file_id,
                   trash_platform_volume_id,
                   trash_platform_file_id,
                   identity_status,
                   claim_platform_file_id
            FROM cleanup_trash_items
            ORDER BY id
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    for (
        id,
        original_source_file_id,
        original_trash_volume_id,
        original_trash_file_id,
        identity_status,
        original_claim_file_id,
    ) in rows
    {
        let mut source_volume_id = None;
        let mut source_file_id = original_source_file_id.clone();
        let mut trash_volume_id = original_trash_volume_id.clone();
        let mut trash_file_id = original_trash_file_id.clone();
        let mut claim_file_id = original_claim_file_id.clone();
        let mut source_volume_proof = None;
        let mut ambiguous = false;

        if let Some(value) = original_source_file_id.as_deref() {
            if value.starts_with(MACOS_CLEANUP_PHYSICAL_ID_PREFIX) {
                match parse_legacy_cleanup_identity(value) {
                    Some((volume_id, file_id)) => {
                        source_volume_proof = Some(volume_id.clone());
                        source_volume_id = Some(volume_id);
                        source_file_id = Some(file_id);
                    }
                    None => ambiguous = true,
                }
            }
        }

        // A tagged source proves its own volume, but it does not prove that
        // an independently persisted, untagged Trash or Claim file ID came
        // from the same object. Keep the original fields and downgrade the
        // row instead of promoting mixed legacy evidence to a trusted row.
        if source_volume_proof.is_some() {
            if original_trash_file_id
                .as_deref()
                .is_some_and(|value| !value.starts_with(MACOS_CLEANUP_PHYSICAL_ID_PREFIX))
            {
                ambiguous = true;
            }
            if original_claim_file_id
                .as_deref()
                .is_some_and(|value| !value.starts_with(MACOS_CLEANUP_PHYSICAL_ID_PREFIX))
            {
                ambiguous = true;
            }
        }

        if let Some(value) = original_trash_file_id.as_deref() {
            if value.starts_with(MACOS_CLEANUP_PHYSICAL_ID_PREFIX) {
                match parse_legacy_cleanup_identity(value) {
                    Some((volume_id, file_id)) => {
                        if source_volume_proof.is_none() && original_source_file_id.is_some() {
                            // A tagged Trash component cannot establish the
                            // missing source volume for a row whose source
                            // component is present but untagged.
                            ambiguous = true;
                        } else if original_trash_volume_id
                            .as_deref()
                            .is_some_and(|existing| existing != volume_id.as_str())
                        {
                            ambiguous = true;
                        } else {
                            trash_volume_id = Some(volume_id);
                            trash_file_id = Some(file_id);
                        }
                    }
                    None => ambiguous = true,
                }
            }
        }

        if let Some(value) = original_claim_file_id.as_deref() {
            if value.starts_with(MACOS_CLEANUP_PHYSICAL_ID_PREFIX) {
                match parse_legacy_cleanup_identity(value) {
                    Some((volume_id, file_id))
                        if source_volume_proof.as_deref() == Some(volume_id.as_str()) =>
                    {
                        claim_file_id = Some(file_id);
                    }
                    Some(_) | None => ambiguous = true,
                }
            }
        }

        if ambiguous {
            if identity_status == "verified" {
                conn.execute(
                    r#"
                    UPDATE cleanup_trash_items
                    SET identity_status = 'legacy_unverified',
                        message = COALESCE(message, 'Cleanup physical identity could not be normalized safely; manual review is required.')
                    WHERE id = ?1
                    "#,
                    params![id],
                )?;
            } else {
                conn.execute(
                    r#"
                    UPDATE cleanup_trash_items
                    SET message = COALESCE(message, 'Cleanup physical identity could not be normalized safely; manual review is required.')
                    WHERE id = ?1
                    "#,
                    params![id],
                )?;
            }
            continue;
        }

        if source_volume_id.is_some()
            || source_file_id != original_source_file_id
            || trash_volume_id != original_trash_volume_id
            || trash_file_id != original_trash_file_id
            || claim_file_id != original_claim_file_id
        {
            conn.execute(
                r#"
                UPDATE cleanup_trash_items
                SET source_platform_volume_id = ?1,
                    source_platform_file_id = ?2,
                    trash_platform_volume_id = ?3,
                    trash_platform_file_id = ?4,
                    claim_platform_file_id = ?5
                WHERE id = ?6
                "#,
                params![
                    source_volume_id,
                    source_file_id,
                    trash_volume_id,
                    trash_file_id,
                    claim_file_id,
                    id
                ],
            )?;
        }
    }
    Ok(())
}

fn ensure_scan_ledger_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS scan_roots (
            id TEXT PRIMARY KEY,
            normalized_path TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            source_kind TEXT NOT NULL DEFAULT 'file_library',
            enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            health_status TEXT NOT NULL DEFAULT 'unknown'
                CHECK (health_status IN (
                    'unknown', 'healthy', 'scanning', 'degraded',
                    'missing', 'permission_required', 'reconciliation_required'
                )),
            current_generation INTEGER NOT NULL DEFAULT 0 CHECK (current_generation >= 0),
            active_run_id TEXT,
            active_generation INTEGER,
            revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
            last_successful_generation INTEGER,
            last_full_scan_at INTEGER,
            needs_reconciliation INTEGER NOT NULL DEFAULT 1 CHECK (needs_reconciliation IN (0, 1)),
            last_error_code TEXT,
            last_error_message TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_scan_roots_enabled_health
            ON scan_roots(enabled, health_status, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_scan_roots_active_lease
            ON scan_roots(active_run_id, active_generation);

        CREATE TABLE IF NOT EXISTS scan_sessions (
            id TEXT PRIMARY KEY,
            request_key TEXT UNIQUE,
            canonical_request_hash TEXT,
            status TEXT NOT NULL
                CHECK (status IN (
                    'queued', 'running', 'cancelling', 'cancelled',
                    'completed', 'completed_with_warnings', 'failed',
                    'interrupted', 'requires_reconciliation'
                )),
            phase TEXT NOT NULL DEFAULT 'preparing'
                CHECK (phase IN ('preparing', 'running', 'finalizing', 'completed')),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            requested_root_count INTEGER NOT NULL DEFAULT 0,
            effective_root_count INTEGER NOT NULL DEFAULT 0,
            completed_root_count INTEGER NOT NULL DEFAULT 0,
            failed_root_count INTEGER NOT NULL DEFAULT 0,
            cancelled_root_count INTEGER NOT NULL DEFAULT 0,
            covered_root_count INTEGER NOT NULL DEFAULT 0,
            unstarted_root_count INTEGER NOT NULL DEFAULT 0,
            dedupe_requested INTEGER NOT NULL DEFAULT 0 CHECK (dedupe_requested IN (0, 1)),
            dedupe_dispatch_state TEXT NOT NULL DEFAULT 'not_requested'
                CHECK (dedupe_dispatch_state IN (
                    'not_requested', 'pending', 'dispatching',
                    'dispatched', 'unknown', 'failed', 'suppressed'
                )),
            dedupe_attempt_count INTEGER NOT NULL DEFAULT 0,
            dedupe_job_id TEXT,
            dedupe_last_error TEXT,
            scanned_files INTEGER NOT NULL DEFAULT 0,
            scanned_directories INTEGER NOT NULL DEFAULT 0,
            warnings_count INTEGER NOT NULL DEFAULT 0,
            errors_count INTEGER NOT NULL DEFAULT 0,
            revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
            started_at INTEGER,
            finished_at INTEGER,
            last_checkpoint_at INTEGER,
            error_code TEXT,
            error_message TEXT,
            result_json TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_scan_sessions_status_created
            ON scan_sessions(status, created_at DESC);

        CREATE TABLE IF NOT EXISTS scan_runs (
            id TEXT PRIMARY KEY,
            scan_root_id TEXT NOT NULL REFERENCES scan_roots(id) ON DELETE RESTRICT,
            generation INTEGER NOT NULL CHECK (generation >= 1),
            parent_session_id TEXT REFERENCES scan_sessions(id) ON DELETE SET NULL,
            lease_token TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL
                CHECK (status IN (
                    'queued', 'running', 'cancelling', 'cancelled',
                    'completed', 'completed_with_warnings', 'failed',
                    'interrupted', 'requires_reconciliation'
                )),
            phase TEXT NOT NULL
                CHECK (phase IN (
                    'preparing', 'discovering', 'persisting',
                    'reconciling_missing', 'optimizing_search',
                    'finalizing', 'completed'
                )),
            scanned_files INTEGER NOT NULL DEFAULT 0,
            scanned_directories INTEGER NOT NULL DEFAULT 0,
            processed_bytes INTEGER NOT NULL DEFAULT 0,
            warnings_count INTEGER NOT NULL DEFAULT 0,
            errors_count INTEGER NOT NULL DEFAULT 0,
            metadata_error_count INTEGER NOT NULL DEFAULT 0,
            coverage_error_count INTEGER NOT NULL DEFAULT 0,
            coverage_complete INTEGER NOT NULL DEFAULT 0 CHECK (coverage_complete IN (0, 1)),
            stale_reconciliation_allowed INTEGER NOT NULL DEFAULT 0
                CHECK (stale_reconciliation_allowed IN (0, 1)),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
            started_at INTEGER,
            finished_at INTEGER,
            last_checkpoint_at INTEGER,
            error_code TEXT,
            error_message TEXT,
            result_json TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(scan_root_id, generation)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_scan_runs_one_active_per_root
            ON scan_runs(scan_root_id)
            WHERE status IN ('queued', 'running', 'cancelling');
        CREATE INDEX IF NOT EXISTS idx_scan_runs_root_created
            ON scan_runs(scan_root_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_scan_runs_session_status
            ON scan_runs(parent_session_id, status, created_at DESC);

        CREATE TABLE IF NOT EXISTS scan_session_roots (
            session_id TEXT NOT NULL REFERENCES scan_sessions(id) ON DELETE CASCADE,
            requested_index INTEGER NOT NULL CHECK (requested_index >= 0),
            requested_path TEXT NOT NULL,
            normalized_requested_path TEXT NOT NULL,
            resolution TEXT NOT NULL
                CHECK (resolution IN (
                    'effective', 'duplicate_requested',
                    'nested_under_effective', 'invalid'
                )),
            effective_root_id TEXT REFERENCES scan_roots(id) ON DELETE RESTRICT,
            effective_path TEXT,
            effective_index INTEGER,
            run_id TEXT REFERENCES scan_runs(id) ON DELETE SET NULL,
            status TEXT NOT NULL
                CHECK (status IN (
                    'pending', 'queued', 'running', 'completed',
                    'completed_with_warnings', 'failed', 'cancelled',
                    'interrupted', 'requires_reconciliation', 'covered',
                    'duplicate', 'nested', 'invalid', 'cancelled_not_started'
                )),
            reason TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(session_id, requested_index)
        );
        CREATE INDEX IF NOT EXISTS idx_scan_session_roots_effective
            ON scan_session_roots(session_id, effective_index, effective_root_id);
        CREATE INDEX IF NOT EXISTS idx_scan_session_roots_run
            ON scan_session_roots(run_id, status);

        CREATE TABLE IF NOT EXISTS scan_seen (
            run_id TEXT NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
            file_id TEXT NOT NULL,
            observed_path TEXT NOT NULL,
            observed_at INTEGER NOT NULL,
            PRIMARY KEY(run_id, file_id)
        );
        CREATE INDEX IF NOT EXISTS idx_scan_seen_run_path
            ON scan_seen(run_id, observed_path);

        CREATE TABLE IF NOT EXISTS scan_run_errors (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
            path TEXT,
            error_code TEXT NOT NULL,
            error_message TEXT,
            affects_coverage INTEGER NOT NULL DEFAULT 1
                CHECK (affects_coverage IN (0, 1)),
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_scan_run_errors_run_created
            ON scan_run_errors(run_id, created_at);
        "#,
    )?;
    Ok(())
}

fn ensure_watcher_reconciliation_schema(conn: &Connection) -> Result<(), DbError> {
    execute_column_migrations(
        conn,
        &[
            "ALTER TABLE scan_roots ADD COLUMN watcher_revision INTEGER NOT NULL DEFAULT 0 CHECK (watcher_revision >= 0);",
            "ALTER TABLE scan_roots ADD COLUMN watcher_applied_revision INTEGER NOT NULL DEFAULT 0 CHECK (watcher_applied_revision >= 0);",
            "ALTER TABLE scan_roots ADD COLUMN watcher_last_event_at INTEGER;",
            "ALTER TABLE scan_roots ADD COLUMN watcher_last_applied_at INTEGER;",
            "ALTER TABLE scan_roots ADD COLUMN watcher_last_error_code TEXT;",
            "ALTER TABLE scan_roots ADD COLUMN watcher_last_error_message TEXT;",
            "ALTER TABLE scan_runs ADD COLUMN watcher_revision_at_start INTEGER NOT NULL DEFAULT 0 CHECK (watcher_revision_at_start >= 0);",
        ],
    )?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_scan_roots_reconciliation_enabled
            ON scan_roots(enabled, needs_reconciliation, updated_at);
        "#,
    )?;
    Ok(())
}

/// Task 02 durable identity/fingerprint/dedupe ledger.
///
/// This function is intentionally called only while migrating to schema 29
/// (or when a schema-29 connection is reopened).  The watcher recovery flag
/// is kept in the same migration as the dedupe tables so a failed migration
/// leaves the complete schema-28 ledger untouched.
fn ensure_dedupe_schema(conn: &Connection) -> Result<(), DbError> {
    execute_column_migrations(
        conn,
        &[r#"
            ALTER TABLE scan_roots
            ADD COLUMN watcher_rule_recovery_required INTEGER NOT NULL DEFAULT 0
            CHECK (watcher_rule_recovery_required IN (0, 1));
        "#],
    )?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS dedupe_runs (
            id TEXT PRIMARY KEY,
            request_key TEXT NOT NULL,
            request_attempt INTEGER NOT NULL DEFAULT 1 CHECK (request_attempt > 0),
            parent_scan_session_id TEXT,
            scope_json TEXT NOT NULL,
            scope_hash TEXT NOT NULL,
            scope_snapshot_json TEXT NOT NULL DEFAULT '{}',
            scope_snapshot_hash TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL CHECK (status IN (
                'queued', 'running', 'cancelling',
                'completed', 'completed_with_warnings',
                'cancelled', 'failed', 'interrupted'
            )),
            phase TEXT NOT NULL CHECK (phase IN (
                'collecting', 'capturing_identity', 'prehashing',
                'full_hashing', 'building_groups', 'finalizing', 'completed'
            )),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            rerun_required INTEGER NOT NULL DEFAULT 0 CHECK (rerun_required IN (0, 1)),
            candidate_files INTEGER NOT NULL DEFAULT 0,
            candidate_physical_objects INTEGER NOT NULL DEFAULT 0,
            candidate_bytes INTEGER NOT NULL DEFAULT 0,
            identity_verified_files INTEGER NOT NULL DEFAULT 0,
            identity_unknown_files INTEGER NOT NULL DEFAULT 0,
            hardlink_aliases INTEGER NOT NULL DEFAULT 0,
            prehashed_files INTEGER NOT NULL DEFAULT 0,
            prehash_pruned_files INTEGER NOT NULL DEFAULT 0,
            full_hashed_files INTEGER NOT NULL DEFAULT 0,
            duplicate_groups INTEGER NOT NULL DEFAULT 0,
            duplicate_members INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            potential_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            processed_files INTEGER NOT NULL DEFAULT 0,
            processed_bytes INTEGER NOT NULL DEFAULT 0,
            total_bytes INTEGER NOT NULL DEFAULT 0,
            warning_count INTEGER NOT NULL DEFAULT 0,
            error_count INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER,
            finished_at INTEGER,
            last_checkpoint_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            error_code TEXT,
            error_message TEXT,
            UNIQUE(request_key, request_attempt)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_dedupe_runs_one_active_scope
            ON dedupe_runs(scope_hash)
            WHERE status IN ('queued', 'running', 'cancelling');
        CREATE INDEX IF NOT EXISTS idx_dedupe_runs_created
            ON dedupe_runs(created_at DESC, id);
        CREATE INDEX IF NOT EXISTS idx_dedupe_runs_parent_scan
            ON dedupe_runs(parent_scan_session_id, created_at DESC);

        CREATE TABLE IF NOT EXISTS file_fingerprints (
            file_id TEXT PRIMARY KEY,
            path_snapshot TEXT NOT NULL,
            identity_status TEXT NOT NULL CHECK (identity_status IN (
                'verified', 'path_only', 'unsupported', 'missing', 'stale', 'error'
            )),
            platform_kind TEXT NOT NULL DEFAULT '',
            platform_volume_id TEXT,
            platform_file_id TEXT,
            physical_key TEXT,
            link_count INTEGER,
            size INTEGER NOT NULL,
            modified_ns INTEGER,
            prehash TEXT,
            prehash_algorithm TEXT NOT NULL DEFAULT 'blake3-head-tail',
            prehash_version INTEGER NOT NULL DEFAULT 1,
            prehash_sample_bytes INTEGER NOT NULL DEFAULT 4096,
            full_hash TEXT,
            full_hash_algorithm TEXT NOT NULL DEFAULT 'blake3',
            full_hash_version INTEGER NOT NULL DEFAULT 1,
            fingerprint_status TEXT NOT NULL CHECK (fingerprint_status IN (
                'identity_only', 'prehash_complete', 'complete', 'stale',
                'missing', 'unsupported', 'error'
            )),
            captured_at INTEGER NOT NULL,
            prehashed_at INTEGER,
            full_hashed_at INTEGER,
            last_verified_at INTEGER NOT NULL,
            error_code TEXT,
            error_message TEXT,
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_file_fingerprints_physical
            ON file_fingerprints(physical_key) WHERE physical_key IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_file_fingerprints_validity
            ON file_fingerprints(size, modified_ns, fingerprint_status);
        CREATE INDEX IF NOT EXISTS idx_file_fingerprints_prehash
            ON file_fingerprints(size, prehash) WHERE prehash IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_file_fingerprints_full_hash
            ON file_fingerprints(size, full_hash) WHERE full_hash IS NOT NULL;

        CREATE TABLE IF NOT EXISTS dedupe_run_errors (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            file_id TEXT,
            path_snapshot TEXT NOT NULL,
            phase TEXT NOT NULL,
            error_code TEXT NOT NULL,
            error_message TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (run_id) REFERENCES dedupe_runs(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_dedupe_run_errors_run
            ON dedupe_run_errors(run_id, created_at, id);

        CREATE TABLE IF NOT EXISTS duplicate_groups (
            id TEXT PRIMARY KEY,
            size_each INTEGER NOT NULL,
            full_hash TEXT NOT NULL,
            full_hash_algorithm TEXT NOT NULL DEFAULT 'blake3',
            full_hash_version INTEGER NOT NULL DEFAULT 1,
            member_count INTEGER NOT NULL,
            physical_copy_count INTEGER NOT NULL,
            hardlink_alias_count INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER,
            potential_reclaimable_bytes INTEGER NOT NULL,
            reclaimable_confidence TEXT NOT NULL CHECK (
                reclaimable_confidence IN ('exact', 'estimated', 'unknown')
            ),
            status TEXT NOT NULL CHECK (status IN ('active', 'stale', 'superseded')),
            last_built_run_id TEXT NOT NULL,
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_verified_at INTEGER NOT NULL,
            UNIQUE(size_each, full_hash, full_hash_algorithm, full_hash_version),
            FOREIGN KEY (last_built_run_id) REFERENCES dedupe_runs(id)
        );
        CREATE INDEX IF NOT EXISTS idx_duplicate_groups_active_reclaimable
            ON duplicate_groups(status, potential_reclaimable_bytes DESC, size_each DESC, id);

        CREATE TABLE IF NOT EXISTS duplicate_group_members (
            group_id TEXT NOT NULL,
            file_id TEXT NOT NULL,
            path_snapshot TEXT NOT NULL,
            physical_key TEXT,
            identity_status TEXT NOT NULL,
            is_hardlink_alias INTEGER NOT NULL DEFAULT 0 CHECK (is_hardlink_alias IN (0, 1)),
            size INTEGER NOT NULL,
            modified_ns INTEGER,
            verified_at INTEGER NOT NULL,
            PRIMARY KEY (group_id, file_id),
            FOREIGN KEY (group_id) REFERENCES duplicate_groups(id) ON DELETE CASCADE,
            FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_duplicate_group_members_file
            ON duplicate_group_members(file_id, group_id);
        CREATE INDEX IF NOT EXISTS idx_duplicate_group_members_physical
            ON duplicate_group_members(group_id, physical_key);

        DROP VIEW IF EXISTS active_duplicate_membership;
        CREATE VIEW active_duplicate_membership AS
        SELECT
            member.file_id,
            group_row.id AS group_id,
            group_row.size_each AS size,
            group_row.full_hash AS content_hash
        FROM duplicate_group_members AS member
        JOIN duplicate_groups AS group_row ON group_row.id = member.group_id
        WHERE group_row.status = 'active';
        "#,
    )?;
    Ok(())
}

/// Task 03 durable analysis/finding ledger.
///
/// The migration deliberately creates no history rows.  In particular, the
/// dedupe authority starts in `rebuild_required`; an existing schema-29
/// duplicate group is retained for compatibility but is not silently promoted
/// to a healthy global publication.
fn ensure_analysis_schema(conn: &Connection) -> Result<(), DbError> {
    execute_column_migrations(
        conn,
        &[r#"
            ALTER TABLE dedupe_runs
            ADD COLUMN publication_mode TEXT NOT NULL DEFAULT 'diagnostic'
            CHECK (publication_mode IN ('authoritative', 'diagnostic'));
        "#],
    )?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS dedupe_authority_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            status TEXT NOT NULL CHECK (status IN ('healthy', 'rebuild_required', 'degraded')),
            last_authoritative_run_id TEXT,
            scope_hash TEXT NOT NULL DEFAULT '',
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (last_authoritative_run_id) REFERENCES dedupe_runs(id) ON DELETE SET NULL
        );
        INSERT OR IGNORE INTO dedupe_authority_state (
            id, revision, status, last_authoritative_run_id, scope_hash, updated_at
        ) VALUES (1, 1, 'rebuild_required', NULL, '', unixepoch());

        CREATE TABLE IF NOT EXISTS analysis_runs (
            id TEXT PRIMARY KEY,
            request_key TEXT NOT NULL,
            request_attempt INTEGER NOT NULL DEFAULT 1 CHECK (request_attempt > 0),
            scope_json TEXT NOT NULL,
            scope_hash TEXT NOT NULL,
            source_snapshot_json TEXT NOT NULL,
            source_snapshot_hash TEXT NOT NULL,
            detector_set_json TEXT NOT NULL,
            detector_set_hash TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN (
                'queued', 'running', 'cancelling',
                'completed', 'completed_with_warnings',
                'cancelled', 'failed', 'interrupted'
            )),
            phase TEXT NOT NULL CHECK (phase IN (
                'preparing', 'running_detectors', 'finalizing', 'completed'
            )),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            rerun_required INTEGER NOT NULL DEFAULT 0 CHECK (rerun_required IN (0, 1)),
            detectors_total INTEGER NOT NULL DEFAULT 0,
            detectors_completed INTEGER NOT NULL DEFAULT 0,
            detectors_failed INTEGER NOT NULL DEFAULT 0,
            findings_staged INTEGER NOT NULL DEFAULT 0,
            findings_published INTEGER NOT NULL DEFAULT 0,
            safe_count INTEGER NOT NULL DEFAULT 0,
            review_count INTEGER NOT NULL DEFAULT 0,
            caution_count INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            potential_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            warning_count INTEGER NOT NULL DEFAULT 0,
            error_count INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER,
            finished_at INTEGER,
            last_checkpoint_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            error_code TEXT,
            error_message TEXT,
            UNIQUE(request_key, request_attempt)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_analysis_runs_one_active_scope
            ON analysis_runs(scope_hash, detector_set_hash)
            WHERE status IN ('queued', 'running', 'cancelling');
        CREATE INDEX IF NOT EXISTS idx_analysis_runs_created
            ON analysis_runs(created_at DESC, id);

        CREATE TABLE IF NOT EXISTS analysis_run_detectors (
            run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE,
            detector_id TEXT NOT NULL,
            detector_version INTEGER NOT NULL CHECK (detector_version > 0),
            status TEXT NOT NULL CHECK (status IN (
                'queued', 'running', 'completed', 'completed_with_warnings',
                'skipped', 'cancelled', 'failed', 'interrupted'
            )),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            scanned_subjects INTEGER NOT NULL DEFAULT 0,
            findings_staged INTEGER NOT NULL DEFAULT 0,
            findings_published INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            potential_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER,
            finished_at INTEGER,
            error_code TEXT,
            error_message TEXT,
            PRIMARY KEY (run_id, detector_id)
        );
        CREATE INDEX IF NOT EXISTS idx_analysis_run_detectors_status
            ON analysis_run_detectors(run_id, status, detector_id);

        CREATE TABLE IF NOT EXISTS analysis_findings (
            id TEXT PRIMARY KEY,
            finding_key TEXT NOT NULL,
            run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE,
            detector_id TEXT NOT NULL,
            detector_version INTEGER NOT NULL CHECK (detector_version > 0),
            scope_hash TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN (
                'staged', 'active', 'stale', 'superseded', 'discarded'
            )),
            tier TEXT NOT NULL CHECK (tier IN ('safe', 'review', 'caution')),
            category TEXT NOT NULL,
            action_kind TEXT NOT NULL CHECK (action_kind IN (
                'reveal', 'review_duplicate_group', 'uninstall_advice',
                'app_internal_cleanup', 'safe_trash_candidate', 'none'
            )),
            title TEXT NOT NULL,
            reason TEXT NOT NULL,
            risk_note TEXT,
            confidence TEXT NOT NULL CHECK (confidence IN ('exact', 'estimated', 'unknown')),
            size_bytes INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER,
            potential_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            requires_confirmation INTEGER NOT NULL DEFAULT 1 CHECK (requires_confirmation IN (0, 1)),
            executable INTEGER NOT NULL DEFAULT 0 CHECK (executable IN (0, 1)),
            primary_subject_kind TEXT NOT NULL,
            primary_subject_id TEXT NOT NULL,
            path_snapshot TEXT,
            identity_snapshot_json TEXT NOT NULL DEFAULT '{}',
            evidence_summary_json TEXT NOT NULL DEFAULT '{}',
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            published_at INTEGER,
            stale_at INTEGER,
            UNIQUE(run_id, finding_key)
        );
        CREATE INDEX IF NOT EXISTS idx_analysis_findings_active_page
            ON analysis_findings(status, tier, potential_reclaimable_bytes DESC, updated_at DESC, id);
        CREATE INDEX IF NOT EXISTS idx_analysis_findings_key
            ON analysis_findings(finding_key, status, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_analysis_findings_subject
            ON analysis_findings(primary_subject_kind, primary_subject_id, status);
        CREATE INDEX IF NOT EXISTS idx_analysis_findings_run_detector
            ON analysis_findings(run_id, detector_id, status);

        CREATE TABLE IF NOT EXISTS analysis_finding_evidence (
            id TEXT PRIMARY KEY,
            finding_id TEXT NOT NULL REFERENCES analysis_findings(id) ON DELETE CASCADE,
            evidence_kind TEXT NOT NULL,
            subject_kind TEXT NOT NULL,
            subject_id TEXT,
            path_snapshot TEXT,
            value_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_analysis_finding_evidence_finding
            ON analysis_finding_evidence(finding_id, created_at, id);
        CREATE INDEX IF NOT EXISTS idx_analysis_finding_evidence_subject
            ON analysis_finding_evidence(subject_kind, subject_id, finding_id);

        CREATE TABLE IF NOT EXISTS analysis_finding_decisions (
            finding_key TEXT PRIMARY KEY,
            decision TEXT NOT NULL CHECK (decision IN ('open', 'acknowledged', 'dismissed', 'snoozed')),
            snoozed_until INTEGER,
            note TEXT,
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            CHECK (decision <> 'snoozed' OR snoozed_until IS NOT NULL)
        );
        CREATE INDEX IF NOT EXISTS idx_analysis_finding_decisions_updated
            ON analysis_finding_decisions(updated_at DESC, finding_key);
        "#,
    )?;
    Ok(())
}

fn backfill_scan_roots_from_settings(conn: &Connection) -> Result<(), DbError> {
    let Some(settings_json) = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![crate::settings::APP_SETTINGS_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(());
    };

    let settings: Value = match serde_json::from_str(&settings_json) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    let Some(roots) = settings.get("defaultScanFolders").and_then(Value::as_array) else {
        return Ok(());
    };

    let now = current_unix_seconds();
    for root in roots {
        let (path, display_name, enabled) = match root {
            Value::String(path) => (path.trim().to_string(), String::new(), true),
            Value::Object(object) => (
                object
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                object
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                object
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            ),
            _ => continue,
        };
        let path = trim_trailing_path_separators(&path).trim().to_string();
        if path.is_empty() {
            continue;
        }
        let normalized_path = normalize_path_text(&path);
        let display_name = if display_name.is_empty() {
            normalized_path
                .trim_end_matches('/')
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(&normalized_path)
                .to_string()
        } else {
            display_name
        };
        let id = format!(
            "scan-root-{}",
            blake3::hash(normalized_path.as_bytes()).to_hex()
        );
        let existing_id = if cfg!(windows) {
            conn.query_row(
                "SELECT id FROM scan_roots WHERE lower(normalized_path) = lower(?1) LIMIT 1",
                params![normalized_path],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        } else {
            conn.query_row(
                "SELECT id FROM scan_roots WHERE normalized_path = ?1 LIMIT 1",
                params![normalized_path],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        };
        if let Some(existing_id) = existing_id {
            conn.execute(
                "UPDATE scan_roots SET display_name = ?1, enabled = ?2, updated_at = ?3 WHERE id = ?4",
                params![display_name, bool_i64(enabled), now, existing_id],
            )?;
        } else {
            conn.execute(
                r#"
                INSERT INTO scan_roots (
                    id, normalized_path, display_name, source_kind, enabled,
                    health_status, current_generation, needs_reconciliation,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, 'file_library', ?4, 'unknown', 0, 1, ?5, ?5)
                "#,
                params![id, normalized_path, display_name, bool_i64(enabled), now],
            )?;
        }
    }
    Ok(())
}

fn bool_i64(value: bool) -> i64 {
    i64::from(value)
}

/// Creates the search-only index domain without coupling it to the existing
/// `files` table.  The external-content FTS table deliberately mirrors only
/// fields that are safe and useful for global search; it never stores file
/// contents or AI classification state.
fn ensure_global_index_schema(conn: &Connection) -> Result<(), DbError> {
    let fts_exists = conn
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'global_entries_fts'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some();
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS global_volumes (
            id TEXT PRIMARY KEY,
            platform TEXT NOT NULL,
            stable_volume_id TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            mount_path TEXT NOT NULL,
            filesystem_type TEXT NOT NULL,
            drive_kind TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            provider TEXT NOT NULL,
            index_status TEXT NOT NULL DEFAULT 'discovered',
            last_error TEXT,
            journal_id TEXT,
            journal_cursor TEXT,
            last_full_index_at INTEGER,
            last_incremental_sync_at INTEGER,
            entry_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_global_volumes_enabled
            ON global_volumes(enabled, index_status);
        CREATE INDEX IF NOT EXISTS idx_global_volumes_mount_path
            ON global_volumes(mount_path);

        CREATE TABLE IF NOT EXISTS global_entries (
            id TEXT PRIMARY KEY,
            volume_id TEXT NOT NULL REFERENCES global_volumes(id) ON DELETE CASCADE,
            platform_file_id TEXT NOT NULL,
            parent_platform_file_id TEXT NOT NULL DEFAULT '',
            name TEXT NOT NULL,
            name_normalized TEXT NOT NULL,
            path TEXT NOT NULL,
            path_normalized TEXT NOT NULL,
            extension TEXT NOT NULL DEFAULT '',
            is_directory INTEGER NOT NULL DEFAULT 0 CHECK (is_directory IN (0, 1)),
            size INTEGER NOT NULL DEFAULT 0,
            created_at_fs INTEGER,
            modified_at_fs INTEGER,
            file_attributes INTEGER NOT NULL DEFAULT 0,
            is_hidden INTEGER NOT NULL DEFAULT 0 CHECK (is_hidden IN (0, 1)),
            is_system INTEGER NOT NULL DEFAULT 0 CHECK (is_system IN (0, 1)),
            is_stale INTEGER NOT NULL DEFAULT 0 CHECK (is_stale IN (0, 1)),
            source_provider TEXT NOT NULL,
            last_seen_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_global_entries_volume
            ON global_entries(volume_id, is_stale);
        CREATE INDEX IF NOT EXISTS idx_global_entries_platform_identity
            ON global_entries(volume_id, platform_file_id, parent_platform_file_id);
        CREATE INDEX IF NOT EXISTS idx_global_entries_name_normalized
            ON global_entries(name_normalized);
        CREATE INDEX IF NOT EXISTS idx_global_entries_path_normalized
            ON global_entries(path_normalized);
        CREATE INDEX IF NOT EXISTS idx_global_entries_modified_at
            ON global_entries(modified_at_fs DESC);

        CREATE VIRTUAL TABLE IF NOT EXISTS global_entries_fts USING fts5(
            name,
            path,
            extension,
            content='global_entries',
            content_rowid='rowid',
            tokenize='trigram'
        );

        CREATE TRIGGER IF NOT EXISTS global_entries_ai AFTER INSERT ON global_entries BEGIN
            INSERT INTO global_entries_fts(rowid, name, path, extension)
            VALUES (new.rowid, new.name, new.path, new.extension);
        END;
        CREATE TRIGGER IF NOT EXISTS global_entries_ad AFTER DELETE ON global_entries BEGIN
            INSERT INTO global_entries_fts(global_entries_fts, rowid, name, path, extension)
            VALUES ('delete', old.rowid, old.name, old.path, old.extension);
        END;
        CREATE TRIGGER IF NOT EXISTS global_entries_au AFTER UPDATE OF name, path, extension ON global_entries
        WHEN old.name IS NOT new.name OR old.path IS NOT new.path OR old.extension IS NOT new.extension BEGIN
            INSERT INTO global_entries_fts(global_entries_fts, rowid, name, path, extension)
            VALUES ('delete', old.rowid, old.name, old.path, old.extension);
            INSERT INTO global_entries_fts(rowid, name, path, extension)
            VALUES (new.rowid, new.name, new.path, new.extension);
        END;

        CREATE TABLE IF NOT EXISTS managed_scopes (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            global_entry_id TEXT REFERENCES global_entries(id) ON DELETE SET NULL,
            enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            allow_local_ai INTEGER NOT NULL DEFAULT 1 CHECK (allow_local_ai IN (0, 1)),
            allow_cloud_ai INTEGER NOT NULL DEFAULT 0 CHECK (allow_cloud_ai IN (0, 1)),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_managed_scopes_enabled
            ON managed_scopes(enabled, updated_at DESC);

        CREATE TABLE IF NOT EXISTS managed_entries (
            id TEXT PRIMARY KEY,
            global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE CASCADE,
            managed_scope_id TEXT NOT NULL REFERENCES managed_scopes(id) ON DELETE CASCADE,
            enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(global_entry_id, managed_scope_id)
        );
        CREATE INDEX IF NOT EXISTS idx_managed_entries_scope
            ON managed_entries(managed_scope_id, enabled);
        CREATE INDEX IF NOT EXISTS idx_managed_entries_global_entry
            ON managed_entries(global_entry_id, enabled);

        CREATE TABLE IF NOT EXISTS ai_analysis_state (
            global_entry_id TEXT PRIMARY KEY REFERENCES global_entries(id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'pending',
            input_fingerprint TEXT NOT NULL DEFAULT '',
            provider TEXT NOT NULL DEFAULT 'local',
            model TEXT NOT NULL DEFAULT '',
            content_summary TEXT,
            classification_json TEXT,
            user_corrected INTEGER NOT NULL DEFAULT 0 CHECK (user_corrected IN (0, 1)),
            last_error TEXT,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ai_jobs (
            id TEXT PRIMARY KEY,
            global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE CASCADE,
            managed_scope_id TEXT NOT NULL REFERENCES managed_scopes(id) ON DELETE CASCADE,
            input_fingerprint TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL DEFAULT '',
            processing_mode TEXT NOT NULL DEFAULT 'metadata',
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'running', 'completed', 'failed', 'canceled', 'stale', 'blocked_by_policy')),
            attempt_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            created_at INTEGER NOT NULL,
            started_at INTEGER,
            completed_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_ai_jobs_status_created
            ON ai_jobs(status, created_at);
        CREATE INDEX IF NOT EXISTS idx_ai_jobs_entry_fingerprint
            ON ai_jobs(global_entry_id, input_fingerprint);

        CREATE TABLE IF NOT EXISTS ai_job_items (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES ai_jobs(id) ON DELETE CASCADE,
            global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'running', 'completed', 'failed', 'canceled', 'stale', 'blocked_by_policy')),
            last_error TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(job_id, global_entry_id)
        );
        CREATE INDEX IF NOT EXISTS idx_ai_job_items_status
            ON ai_job_items(status, updated_at);
        "#,
    )?;

    // A database can be upgraded from a partially created development build.
    // Rebuilding is needed only when the virtual table was just created; doing
    // it on every application start would turn a cheap health check into a
    // full-database operation.
    if !fts_exists {
        conn.execute(
            "INSERT INTO global_entries_fts(global_entries_fts) VALUES ('rebuild')",
            [],
        )?;
    }
    Ok(())
}

fn ensure_global_index_hardening(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS global_entries_au;
        CREATE TRIGGER global_entries_au
        AFTER UPDATE OF name, path, extension ON global_entries
        WHEN old.name IS NOT new.name
          OR old.path IS NOT new.path
          OR old.extension IS NOT new.extension
        BEGIN
            INSERT INTO global_entries_fts(global_entries_fts, rowid, name, path, extension)
            VALUES ('delete', old.rowid, old.name, old.path, old.extension);
            INSERT INTO global_entries_fts(rowid, name, path, extension)
            VALUES (new.rowid, new.name, new.path, new.extension);
        END;

        DROP TRIGGER IF EXISTS global_entries_count_ai;
        DROP TRIGGER IF EXISTS global_entries_count_ad;
        DROP TRIGGER IF EXISTS global_entries_count_au;
        CREATE TRIGGER global_entries_count_ai
        AFTER INSERT ON global_entries
        WHEN new.is_stale = 0
        BEGIN
            UPDATE global_volumes
            SET entry_count = entry_count + 1, updated_at = new.last_seen_at
            WHERE id = new.volume_id;
        END;
        CREATE TRIGGER global_entries_count_ad
        AFTER DELETE ON global_entries
        WHEN old.is_stale = 0
        BEGIN
            UPDATE global_volumes
            SET entry_count = MAX(0, entry_count - 1), updated_at = unixepoch()
            WHERE id = old.volume_id;
        END;
        CREATE TRIGGER global_entries_count_au
        AFTER UPDATE OF is_stale, volume_id ON global_entries
        WHEN old.is_stale IS NOT new.is_stale OR old.volume_id IS NOT new.volume_id
        BEGIN
            UPDATE global_volumes
            SET entry_count = MAX(0, entry_count - CASE WHEN old.is_stale = 0 THEN 1 ELSE 0 END),
                updated_at = unixepoch()
            WHERE id = old.volume_id;
            UPDATE global_volumes
            SET entry_count = entry_count + CASE WHEN new.is_stale = 0 THEN 1 ELSE 0 END,
                updated_at = unixepoch()
            WHERE id = new.volume_id;
        END;

        DROP TRIGGER IF EXISTS ai_jobs_canceled_terminal;
        CREATE TRIGGER ai_jobs_canceled_terminal
        BEFORE UPDATE OF status ON ai_jobs
        WHEN old.status = 'canceled' AND new.status <> 'canceled'
        BEGIN
            SELECT RAISE(ABORT, 'canceled AI jobs are terminal');
        END;

        -- The global-search tiers use a stable (modified_at_fs, id) order.
        -- Keep the old two-column indexes only long enough to upgrade an
        -- existing database; the *_order indexes avoid a temp sort for exact
        -- name/extension tiers and for their high-cardinality prefix cases.
        CREATE INDEX IF NOT EXISTS idx_global_entries_active_name_order
            ON global_entries(name_normalized, modified_at_fs DESC, id ASC)
            WHERE is_stale = 0;
        CREATE INDEX IF NOT EXISTS idx_global_entries_active_extension_order
            ON global_entries(extension, modified_at_fs DESC, id ASC)
            WHERE is_stale = 0;
        DROP INDEX IF EXISTS idx_global_entries_active_name;
        DROP INDEX IF EXISTS idx_global_entries_active_extension;

        UPDATE global_volumes
        SET entry_count = (
            SELECT COUNT(*) FROM global_entries entry
            WHERE entry.volume_id = global_volumes.id AND entry.is_stale = 0
        );
        "#,
    )?;
    Ok(())
}

/// Task 05 File Library metadata/query state.  This migration intentionally
/// adds only small side tables; the `files` authority and its durable IDs are
/// left untouched so existing scanner, watcher, operation and restore rows do
/// not need a backfill or rebuild.
fn ensure_file_library_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_library_files_modified
            ON files(is_stale, mtime DESC, id);
        CREATE INDEX IF NOT EXISTS idx_library_files_created
            ON files(is_stale, ctime DESC, id);
        CREATE INDEX IF NOT EXISTS idx_library_files_name
            ON files(is_stale, name COLLATE NOCASE, id);
        CREATE INDEX IF NOT EXISTS idx_library_files_size
            ON files(is_stale, size DESC, id);
        CREATE INDEX IF NOT EXISTS idx_library_files_confidence
            ON files(is_stale, confidence DESC, id);

        CREATE TABLE IF NOT EXISTS user_tags (
            id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            normalized_name TEXT NOT NULL COLLATE NOCASE UNIQUE,
            color_token TEXT NOT NULL DEFAULT 'neutral',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_user_tags_name
            ON user_tags(normalized_name COLLATE NOCASE, id);

        CREATE TABLE IF NOT EXISTS file_user_tags (
            file_id TEXT NOT NULL
                REFERENCES files(id)
                ON UPDATE CASCADE
                ON DELETE CASCADE,
            tag_id TEXT NOT NULL
                REFERENCES user_tags(id)
                ON DELETE CASCADE,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(file_id, tag_id)
        );
        CREATE INDEX IF NOT EXISTS idx_file_user_tags_tag_file
            ON file_user_tags(tag_id, file_id);

        CREATE TABLE IF NOT EXISTS library_saved_views (
            id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            normalized_name TEXT NOT NULL COLLATE NOCASE UNIQUE,
            query_spec_version INTEGER NOT NULL CHECK (query_spec_version = 2),
            query_spec_json TEXT NOT NULL,
            position INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_library_saved_views_position
            ON library_saved_views(position, updated_at DESC, id);

        CREATE TABLE IF NOT EXISTS library_query_state (
            singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
            revision INTEGER NOT NULL CHECK (revision >= 1),
            updated_at INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO library_query_state(singleton_id, revision, updated_at)
        VALUES (1, 1, strftime('%s', 'now'));
        "#,
    )?;
    Ok(())
}

/// Task 06 durable organization-plan ledger. The large `files` authority and
/// the existing operation/cleanup journals are intentionally untouched.
fn ensure_organization_plan_schema(conn: &Connection) -> Result<(), DbError> {
    execute_column_migrations(
        conn,
        &[
            "ALTER TABLE user_tags ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;",
            "ALTER TABLE library_saved_views ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;",
        ],
    )?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS organization_plans (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN (
                'draft', 'building', 'ready', 'stale', 'executing',
                'partially_completed', 'completed', 'cancelled', 'failed'
            )),
            source_kind TEXT NOT NULL CHECK (source_kind IN ('explicit', 'all_matching')),
            source_query_spec_json TEXT,
            source_query_fingerprint TEXT,
            source_snapshot_revision INTEGER NOT NULL,
            requested_count INTEGER NOT NULL CHECK (requested_count >= 0),
            materialized_count INTEGER NOT NULL CHECK (materialized_count >= 0),
            planner_version INTEGER NOT NULL,
            revision INTEGER NOT NULL CHECK (revision >= 1),
            active_execution_id TEXT,
            active_operation_batch_id TEXT,
            last_error_code TEXT,
            last_error_detail TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            ready_at INTEGER,
            completed_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_organization_plans_status_updated
            ON organization_plans(status, updated_at DESC, id);

        CREATE TABLE IF NOT EXISTS organization_plan_items (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL REFERENCES organization_plans(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL,
            file_id_snapshot TEXT NOT NULL,
            source_path_snapshot TEXT NOT NULL,
            source_name_snapshot TEXT NOT NULL,
            source_size_snapshot INTEGER NOT NULL,
            source_mtime_snapshot INTEGER NOT NULL,
            source_is_dir_snapshot INTEGER NOT NULL,
            proposal_fingerprint TEXT NOT NULL,
            proposal_kind TEXT NOT NULL CHECK (proposal_kind IN (
                'move', 'rename', 'move_rename', 'keep', 'blocked'
            )),
            proposed_target_directory TEXT NOT NULL,
            proposed_name TEXT NOT NULL,
            proposed_target_path TEXT NOT NULL,
            decision TEXT NOT NULL CHECK (decision IN (
                'undecided', 'accepted', 'kept', 'edited'
            )),
            edited_name TEXT,
            validity TEXT NOT NULL CHECK (validity IN (
                'ready', 'needs_analysis', 'needs_review', 'blocked',
                'stale', 'executing', 'executed', 'failed', 'skipped'
            )),
            confidence REAL NOT NULL,
            risk_level TEXT NOT NULL,
            requires_confirmation INTEGER NOT NULL,
            blocking_code TEXT,
            blocking_detail TEXT,
            authoritative_preview_id TEXT,
            operation_log_id TEXT,
            execution_id TEXT,
            revision INTEGER NOT NULL CHECK (revision >= 1),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(plan_id, ordinal),
            UNIQUE(plan_id, file_id_snapshot)
        );
        CREATE INDEX IF NOT EXISTS idx_organization_plan_items_plan_state
            ON organization_plan_items(plan_id, validity, decision, ordinal, id);
        CREATE INDEX IF NOT EXISTS idx_organization_plan_items_file
            ON organization_plan_items(file_id_snapshot, plan_id);
        CREATE INDEX IF NOT EXISTS idx_organization_plan_items_execution
            ON organization_plan_items(execution_id, validity, id);
        "#,
    )?;
    Ok(())
}

/// Task 07 Rule Repository V2 catalog state and durable proposal review
/// ledger. The large files authority and every operation/cleanup journal are
/// intentionally untouched.
fn ensure_rule_proposal_schema(conn: &Connection) -> Result<(), DbError> {
    // Some historical development fixtures at schema 9 omitted the v7 rules
    // table even though production migrations create it. Re-establish that
    // precondition idempotently before the additive schema-33 columns.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS rules (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'user',
            enabled INTEGER NOT NULL DEFAULT 1,
            priority REAL NOT NULL DEFAULT 0,
            weight REAL NOT NULL DEFAULT 0,
            root_operator TEXT NOT NULL DEFAULT 'AND',
            groups_json TEXT NOT NULL DEFAULT '[]',
            action_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT ''
        );
        "#,
    )?;
    // A few historical test fixtures created only the original id/name pair
    // before the schema-7 rules catalog was introduced. Reconcile every
    // additive catalog column before recreating its indexes.
    execute_column_migrations(
        conn,
        &[
            "ALTER TABLE rules ADD COLUMN source TEXT NOT NULL DEFAULT 'user';",
            "ALTER TABLE rules ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;",
            "ALTER TABLE rules ADD COLUMN priority REAL NOT NULL DEFAULT 0;",
            "ALTER TABLE rules ADD COLUMN weight REAL NOT NULL DEFAULT 0;",
            "ALTER TABLE rules ADD COLUMN root_operator TEXT NOT NULL DEFAULT 'AND';",
            "ALTER TABLE rules ADD COLUMN groups_json TEXT NOT NULL DEFAULT '[]';",
            "ALTER TABLE rules ADD COLUMN action_json TEXT NOT NULL DEFAULT '{}';",
            "ALTER TABLE rules ADD COLUMN created_at TEXT NOT NULL DEFAULT '';",
            "ALTER TABLE rules ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';",
        ],
    )?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_rules_source ON rules(source);
        CREATE INDEX IF NOT EXISTS idx_rules_enabled ON rules(enabled);
        CREATE INDEX IF NOT EXISTS idx_rules_priority ON rules(priority DESC);
        "#,
    )?;
    execute_column_migrations(
        conn,
        &[
            "ALTER TABLE rules ADD COLUMN ast_version INTEGER NOT NULL DEFAULT 1;",
            "ALTER TABLE rules ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;",
            "ALTER TABLE rules ADD COLUMN origin_proposal_id TEXT;",
        ],
    )?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS rule_catalog_state (
            singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
            revision INTEGER NOT NULL CHECK (revision >= 1),
            updated_at INTEGER NOT NULL
        );
        "#,
    )?;
    require_exact_table_columns(
        conn,
        "rule_catalog_state",
        &["singleton_id", "revision", "updated_at"],
    )?;
    conn.execute_batch(
        r#"
        INSERT OR IGNORE INTO rule_catalog_state(singleton_id, revision, updated_at)
        VALUES (1, 1, strftime('%s', 'now'));

        CREATE TABLE IF NOT EXISTS rule_proposals (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL CHECK (status IN (
                'draft', 'generating', 'ready', 'needs_clarification',
                'invalid', 'stale', 'applying', 'applied',
                'cancelled', 'failed'
            )),
            intent_kind TEXT NOT NULL CHECK (intent_kind IN ('create', 'update')),
            target_rule_id TEXT,
            base_rule_revision INTEGER,
            prompt TEXT NOT NULL,
            prompt_fingerprint TEXT NOT NULL,
            provider_kind TEXT,
            provider_preset TEXT,
            model TEXT,
            ast_version INTEGER NOT NULL,
            candidate_rule_json TEXT,
            candidate_fingerprint TEXT,
            summary TEXT,
            clarification_json TEXT NOT NULL DEFAULT '[]',
            validation_json TEXT NOT NULL DEFAULT '{}',
            applied_rule_id TEXT,
            revision INTEGER NOT NULL CHECK (revision >= 1),
            last_error_code TEXT,
            last_error_detail TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            generated_at INTEGER,
            applied_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_rule_proposals_status_updated
            ON rule_proposals(status, updated_at DESC, id);
        CREATE INDEX IF NOT EXISTS idx_rule_proposals_target
            ON rule_proposals(target_rule_id, status, updated_at DESC);
        "#,
    )?;
    require_exact_table_columns(
        conn,
        "rule_proposals",
        &[
            "id",
            "status",
            "intent_kind",
            "target_rule_id",
            "base_rule_revision",
            "prompt",
            "prompt_fingerprint",
            "provider_kind",
            "provider_preset",
            "model",
            "ast_version",
            "candidate_rule_json",
            "candidate_fingerprint",
            "summary",
            "clarification_json",
            "validation_json",
            "applied_rule_id",
            "revision",
            "last_error_code",
            "last_error_detail",
            "created_at",
            "updated_at",
            "generated_at",
            "applied_at",
        ],
    )?;
    let catalog_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM rule_catalog_state
         WHERE singleton_id = 1 AND revision >= 1",
        [],
        |row| row.get(0),
    )?;
    if catalog_rows != 1 {
        return Err(DbError::Validation(
            "rule_catalog_schema_conflict".to_string(),
        ));
    }
    Ok(())
}

/// Task 08 consent-bound local content understanding. These are additive
/// side tables: the existing `files`, operation journal, rule, plan, and
/// analysis authorities remain untouched. Content rows bind every artifact to
/// a durable file identity, source fingerprint, extractor/policy revision and
/// bounded provenance.
fn ensure_content_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS content_catalog (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
            updated_at INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO content_catalog(id, revision, updated_at)
            VALUES (1, 1, unixepoch());

        CREATE TABLE IF NOT EXISTS content_scope_policies (
            scan_root_id TEXT PRIMARY KEY REFERENCES scan_roots(id) ON DELETE CASCADE,
            enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
            extractor_families_json TEXT NOT NULL DEFAULT '["txt","md","csv","pdf_text","docx","xlsx","pptx"]',
            max_bytes INTEGER NOT NULL DEFAULT 8388608 CHECK (max_bytes BETWEEN 1024 AND 67108864),
            max_chars INTEGER NOT NULL DEFAULT 32768 CHECK (max_chars BETWEEN 256 AND 262144),
            max_pages INTEGER NOT NULL DEFAULT 100 CHECK (max_pages BETWEEN 1 AND 1000),
            max_rows INTEGER NOT NULL DEFAULT 10000 CHECK (max_rows BETWEEN 1 AND 100000),
            raw_retention_mode TEXT NOT NULL DEFAULT 'none' CHECK (raw_retention_mode IN ('none','bounded')),
            raw_retention_chars INTEGER NOT NULL DEFAULT 0 CHECK (raw_retention_chars BETWEEN 0 AND 262144),
            local_allowed INTEGER NOT NULL DEFAULT 0 CHECK (local_allowed IN (0, 1)),
            cloud_allowed INTEGER NOT NULL DEFAULT 0 CHECK (cloud_allowed IN (0, 1)),
            policy_revision INTEGER NOT NULL DEFAULT 1 CHECK (policy_revision >= 1),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_content_scope_policies_enabled
            ON content_scope_policies(enabled, updated_at DESC, scan_root_id);

        CREATE TABLE IF NOT EXISTS content_runs (
            id TEXT PRIMARY KEY,
            scope_json TEXT NOT NULL,
            scope_fingerprint TEXT NOT NULL,
            mode TEXT NOT NULL CHECK (mode IN ('local','understand','local_and_understand')),
            provider_mode TEXT NOT NULL CHECK (provider_mode IN ('none','existing_interactive_provider')),
            status TEXT NOT NULL CHECK (status IN ('building','ready','running','completed','partially_completed','cancelling','cancelled','failed','stale')),
            expected_library_revision INTEGER NOT NULL,
            policy_fingerprint TEXT NOT NULL,
            candidate_fingerprint TEXT NOT NULL DEFAULT '',
            candidate_resolver TEXT NOT NULL DEFAULT '',
            confirmation INTEGER NOT NULL DEFAULT 0 CHECK (confirmation IN (0, 1)),
            byte_budget INTEGER NOT NULL DEFAULT 0,
            char_budget INTEGER NOT NULL DEFAULT 0,
            requested_count INTEGER NOT NULL DEFAULT 0,
            materialized_count INTEGER NOT NULL DEFAULT 0,
            completed_count INTEGER NOT NULL DEFAULT 0,
            blocked_count INTEGER NOT NULL DEFAULT 0,
            skipped_count INTEGER NOT NULL DEFAULT 0,
            failed_count INTEGER NOT NULL DEFAULT 0,
            provider_revision INTEGER NOT NULL DEFAULT 1 CHECK (provider_revision >= 1),
            provider_owner TEXT,
            provider_confirmed INTEGER NOT NULL DEFAULT 0 CHECK (provider_confirmed IN (0, 1)),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
            last_error_code TEXT,
            last_error_detail TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            completed_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_content_runs_status_updated
            ON content_runs(status, updated_at DESC, id);

        CREATE TABLE IF NOT EXISTS content_run_items (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL REFERENCES content_runs(id) ON DELETE CASCADE,
            file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('pending','running','completed','blocked','failed','cancelled','stale')),
            root_id TEXT,
            source_is_dir INTEGER NOT NULL DEFAULT 0 CHECK (source_is_dir IN (0, 1)),
            extractor_family TEXT,
            extractor_version TEXT,
            artifact_id TEXT,
            provider_status TEXT NOT NULL DEFAULT 'pending'
                CHECK (provider_status IN ('pending','running','completed','blocked','failed','cancelled','stale')),
            provider_revision INTEGER NOT NULL DEFAULT 1 CHECK (provider_revision >= 1),
            provider_owner TEXT,
            provider_completed_at INTEGER,
            policy_revision INTEGER NOT NULL DEFAULT 1 CHECK (policy_revision >= 1),
            error_code TEXT,
            error_detail TEXT,
            source_size INTEGER NOT NULL,
            source_mtime INTEGER NOT NULL,
            source_hash TEXT NOT NULL DEFAULT '',
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(run_id, file_id),
            UNIQUE(run_id, ordinal)
        );
        CREATE INDEX IF NOT EXISTS idx_content_run_items_run_status
            ON content_run_items(run_id, status, ordinal, id);
        CREATE INDEX IF NOT EXISTS idx_content_run_items_file
            ON content_run_items(file_id, updated_at DESC);

        CREATE TABLE IF NOT EXISTS content_artifacts (
            id TEXT PRIMARY KEY,
            file_id TEXT NOT NULL UNIQUE REFERENCES files(id) ON DELETE CASCADE,
            scan_root_id TEXT REFERENCES scan_roots(id) ON DELETE SET NULL,
            source_size INTEGER NOT NULL,
            source_mtime INTEGER NOT NULL,
            source_is_dir INTEGER NOT NULL CHECK (source_is_dir IN (0, 1)),
            source_hash TEXT NOT NULL DEFAULT '',
            extractor_family TEXT NOT NULL,
            extractor_version TEXT NOT NULL,
            policy_revision INTEGER NOT NULL,
            provider_kind TEXT,
            provider_model TEXT,
            prompt_policy_version INTEGER,
            content_fingerprint TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('current','stale','unsupported','blocked','failed')),
            summary TEXT,
            keywords_json TEXT NOT NULL DEFAULT '[]',
            language TEXT,
            truncated INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
            text_retained INTEGER NOT NULL DEFAULT 0 CHECK (text_retained IN (0, 1)),
            raw_text TEXT,
            provenance_json TEXT NOT NULL DEFAULT '{}',
            catalog_revision INTEGER NOT NULL DEFAULT 1 CHECK (catalog_revision >= 1),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_run_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_content_artifacts_status_updated
            ON content_artifacts(status, updated_at DESC, file_id);
        CREATE INDEX IF NOT EXISTS idx_content_artifacts_root_status
            ON content_artifacts(scan_root_id, status, updated_at DESC);

        DROP TRIGGER IF EXISTS content_catalog_artifact_insert;
        CREATE TRIGGER content_catalog_artifact_insert
        AFTER INSERT ON content_artifacts
        BEGIN
            UPDATE content_catalog SET revision=revision+1, updated_at=unixepoch() WHERE id=1;
        END;
        DROP TRIGGER IF EXISTS content_catalog_artifact_update;
        CREATE TRIGGER content_catalog_artifact_update
        AFTER UPDATE ON content_artifacts
        BEGIN
            UPDATE content_catalog SET revision=revision+1, updated_at=unixepoch() WHERE id=1;
        END;
        DROP TRIGGER IF EXISTS content_catalog_artifact_delete;
        CREATE TRIGGER content_catalog_artifact_delete
        AFTER DELETE ON content_artifacts
        BEGIN
            UPDATE content_catalog SET revision=revision+1, updated_at=unixepoch() WHERE id=1;
        END;

        DROP TRIGGER IF EXISTS content_artifacts_file_changed;
        CREATE TRIGGER content_artifacts_file_changed
        AFTER UPDATE OF path, size, mtime, is_dir, is_stale, content_hash ON files
        WHEN EXISTS (SELECT 1 FROM content_artifacts WHERE file_id = NEW.id)
        BEGIN
            UPDATE content_artifacts
            SET status = 'stale', revision = revision + 1, updated_at = unixepoch()
            WHERE file_id = NEW.id;
        END;

        CREATE VIRTUAL TABLE IF NOT EXISTS content_artifact_fts USING fts5(
            artifact_id UNINDEXED,
            summary,
            keywords,
            language,
            raw_text,
            tokenize='unicode61'
        );
        "#,
    )?;
    execute_column_migrations(
        conn,
        &[
            "ALTER TABLE content_runs ADD COLUMN byte_budget INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE content_runs ADD COLUMN char_budget INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE content_runs ADD COLUMN materialized_count INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE content_runs ADD COLUMN skipped_count INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE content_runs ADD COLUMN candidate_fingerprint TEXT NOT NULL DEFAULT '';",
            "ALTER TABLE content_runs ADD COLUMN candidate_resolver TEXT NOT NULL DEFAULT '';",
            "ALTER TABLE content_runs ADD COLUMN provider_revision INTEGER NOT NULL DEFAULT 1;",
            "ALTER TABLE content_runs ADD COLUMN provider_owner TEXT;",
            "ALTER TABLE content_runs ADD COLUMN provider_confirmed INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE content_runs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE content_run_items ADD COLUMN root_id TEXT;",
            "ALTER TABLE content_run_items ADD COLUMN source_is_dir INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE content_run_items ADD COLUMN extractor_family TEXT;",
            "ALTER TABLE content_run_items ADD COLUMN extractor_version TEXT;",
            "ALTER TABLE content_run_items ADD COLUMN artifact_id TEXT;",
            "ALTER TABLE content_run_items ADD COLUMN provider_status TEXT NOT NULL DEFAULT 'pending';",
            "ALTER TABLE content_run_items ADD COLUMN provider_revision INTEGER NOT NULL DEFAULT 1;",
            "ALTER TABLE content_run_items ADD COLUMN provider_owner TEXT;",
            "ALTER TABLE content_run_items ADD COLUMN provider_completed_at INTEGER;",
            "ALTER TABLE content_run_items ADD COLUMN policy_revision INTEGER NOT NULL DEFAULT 1;",
            "ALTER TABLE content_artifacts ADD COLUMN catalog_revision INTEGER NOT NULL DEFAULT 1;",
        ],
    )?;
    require_exact_table_columns(
        conn,
        "content_scope_policies",
        &[
            "scan_root_id",
            "enabled",
            "extractor_families_json",
            "max_bytes",
            "max_chars",
            "max_pages",
            "max_rows",
            "raw_retention_mode",
            "raw_retention_chars",
            "local_allowed",
            "cloud_allowed",
            "policy_revision",
            "created_at",
            "updated_at",
        ],
    )?;
    require_exact_table_columns(conn, "content_catalog", &["id", "revision", "updated_at"])?;
    require_exact_table_columns(
        conn,
        "content_runs",
        &[
            "id",
            "scope_json",
            "scope_fingerprint",
            "mode",
            "provider_mode",
            "status",
            "expected_library_revision",
            "policy_fingerprint",
            "candidate_fingerprint",
            "candidate_resolver",
            "confirmation",
            "byte_budget",
            "char_budget",
            "requested_count",
            "materialized_count",
            "completed_count",
            "blocked_count",
            "skipped_count",
            "failed_count",
            "provider_revision",
            "provider_owner",
            "provider_confirmed",
            "cancel_requested",
            "revision",
            "last_error_code",
            "last_error_detail",
            "created_at",
            "updated_at",
            "completed_at",
        ],
    )?;
    require_exact_table_columns(
        conn,
        "content_run_items",
        &[
            "id",
            "run_id",
            "file_id",
            "ordinal",
            "status",
            "root_id",
            "source_is_dir",
            "extractor_family",
            "extractor_version",
            "artifact_id",
            "provider_status",
            "provider_revision",
            "provider_owner",
            "provider_completed_at",
            "policy_revision",
            "error_code",
            "error_detail",
            "source_size",
            "source_mtime",
            "source_hash",
            "revision",
            "created_at",
            "updated_at",
        ],
    )?;
    require_exact_table_columns(
        conn,
        "content_artifacts",
        &[
            "id",
            "file_id",
            "scan_root_id",
            "source_size",
            "source_mtime",
            "source_is_dir",
            "source_hash",
            "extractor_family",
            "extractor_version",
            "policy_revision",
            "provider_kind",
            "provider_model",
            "prompt_policy_version",
            "content_fingerprint",
            "status",
            "summary",
            "keywords_json",
            "language",
            "truncated",
            "text_retained",
            "raw_text",
            "provenance_json",
            "catalog_revision",
            "revision",
            "created_at",
            "updated_at",
            "last_run_id",
        ],
    )?;
    Ok(())
}

fn require_exact_table_columns(
    conn: &Connection,
    table: &str,
    expected: &[&str],
) -> Result<(), DbError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info('{table}')"))?;
    let actual = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if actual.len() != expected.len()
        || expected
            .iter()
            .any(|column| !actual.iter().any(|actual| actual == column))
    {
        return Err(DbError::Validation(format!("{table}_schema_conflict")));
    }
    Ok(())
}

fn ensure_journal_state_triggers(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS operation_logs_status_guard_insert;
        DROP TRIGGER IF EXISTS operation_logs_status_guard_update;
        CREATE TRIGGER operation_logs_status_guard_insert
        BEFORE INSERT ON operation_logs
        WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped', 'manual_review')
          OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;
        CREATE TRIGGER operation_logs_status_guard_update
        BEFORE UPDATE OF status, restore_status ON operation_logs
        WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped', 'manual_review')
          OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;

        DROP TRIGGER IF EXISTS operation_logs_phase_guard_insert;
        DROP TRIGGER IF EXISTS operation_logs_phase_guard_update;
        CREATE TRIGGER operation_logs_phase_guard_insert
        BEFORE INSERT ON operation_logs
        WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid operation phase'); END;
        CREATE TRIGGER operation_logs_phase_guard_update
        BEFORE UPDATE OF operation_phase ON operation_logs
        WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid operation phase'); END;

        DROP TRIGGER IF EXISTS operation_logs_restore_phase_guard_insert;
        DROP TRIGGER IF EXISTS operation_logs_restore_phase_guard_update;
        CREATE TRIGGER operation_logs_restore_phase_guard_insert
        BEFORE INSERT ON operation_logs
        WHEN NEW.restore_phase NOT IN ('idle', 'prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid restore phase'); END;
        CREATE TRIGGER operation_logs_restore_phase_guard_update
        BEFORE UPDATE OF restore_phase ON operation_logs
        WHEN NEW.restore_phase NOT IN ('idle', 'prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid restore phase'); END;

        DROP TRIGGER IF EXISTS cleanup_items_status_guard_insert;
        DROP TRIGGER IF EXISTS cleanup_items_status_guard_update;
        CREATE TRIGGER cleanup_items_status_guard_insert
        BEFORE INSERT ON cleanup_trash_items
        WHEN NEW.status NOT IN ('pending', 'moved', 'restored', 'failed', 'missing',
            'manual_review', 'canceled')
        BEGIN SELECT RAISE(ABORT, 'invalid cleanup item status'); END;
        CREATE TRIGGER cleanup_items_status_guard_update
        BEFORE UPDATE OF status ON cleanup_trash_items
        WHEN NEW.status NOT IN ('pending', 'moved', 'restored', 'failed', 'missing',
            'manual_review', 'canceled')
        BEGIN SELECT RAISE(ABORT, 'invalid cleanup item status'); END;

        DROP TRIGGER IF EXISTS cleanup_items_phase_guard_insert;
        DROP TRIGGER IF EXISTS cleanup_items_phase_guard_update;
        CREATE TRIGGER cleanup_items_phase_guard_insert
        BEFORE INSERT ON cleanup_trash_items
        WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid cleanup operation phase'); END;
        CREATE TRIGGER cleanup_items_phase_guard_update
        BEFORE UPDATE OF operation_phase ON cleanup_trash_items
        WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid cleanup operation phase'); END;
        "#,
    )?;
    Ok(())
}

fn migrate_invalid_rule_domain_values(conn: &Connection) -> Result<(), DbError> {
    let compatible_rules_columns = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('rules') WHERE name IN ('source', 'root_operator', 'groups_json', 'action_json')",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if compatible_rules_columns != 4 {
        return Ok(());
    }
    let migrations = [
        (
            "purpose",
            "'Project','Teaching','Study','Work','Personal','Career','Finance','Identity','Media','Installer','Temporary','Archive','Document','Duplicate Review','Unknown'",
        ),
        (
            "lifecycle",
            "'Inbox','Active','Reference','Archive','Disposable','Duplicate','Sensitive','TrashReview','Unknown'",
        ),
        (
            "risk_level",
            "'Normal','Sensitive','System','Caution','Unknown'",
        ),
        (
            "suggested_action",
            "'Keep','Rename','Move','MoveAndRename','Archive','Review','DeleteCandidate','Unknown'",
        ),
    ];
    for (field, allowed) in migrations {
        conn.execute(
            &format!(
                "UPDATE rules SET action_json = json_set(action_json, '$.{field}', 'Unknown') \
                 WHERE json_valid(action_json) \
                 AND json_type(action_json, '$.{field}') = 'text' \
                 AND json_extract(action_json, '$.{field}') NOT IN ({allowed})"
            ),
            [],
        )?;
    }
    conn.execute(
        "UPDATE rules SET source = 'unknown' WHERE source NOT IN ('system', 'user', 'session', 'ai', 'learned', 'unknown')",
        [],
    )?;
    conn.execute(
        "UPDATE rules SET root_operator = 'UNKNOWN' WHERE root_operator NOT IN ('AND', 'OR', 'UNKNOWN')",
        [],
    )?;

    let mut stmt = conn.prepare("SELECT id, groups_json FROM rules")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for (id, groups_json) in rows {
        let Ok(mut groups) = serde_json::from_str::<Value>(&groups_json) else {
            continue;
        };
        let mut changed = false;
        if let Some(group_values) = groups.as_array_mut() {
            for group in group_values {
                let Some(group_object) = group.as_object_mut() else {
                    continue;
                };
                let operator = group_object
                    .get("operator")
                    .and_then(Value::as_str)
                    .unwrap_or("UNKNOWN");
                if !matches!(operator, "AND" | "OR" | "UNKNOWN") {
                    group_object
                        .insert("operator".to_string(), Value::String("UNKNOWN".to_string()));
                    changed = true;
                }
                let Some(conditions) = group_object
                    .get_mut("conditions")
                    .and_then(Value::as_array_mut)
                else {
                    continue;
                };
                for condition in conditions {
                    let Some(condition_object) = condition.as_object_mut() else {
                        continue;
                    };
                    let field = condition_object
                        .get("field")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    if !matches!(
                        field,
                        "name"
                            | "extension"
                            | "file_type"
                            | "path"
                            | "directory"
                            | "size"
                            | "modified_at"
                            | "is_duplicate"
                            | "risk_level"
                            | "unknown"
                    ) {
                        condition_object
                            .insert("field".to_string(), Value::String("unknown".to_string()));
                        changed = true;
                    }
                    let operator = condition_object
                        .get("operator")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    if !matches!(
                        operator,
                        "contains"
                            | "equals"
                            | "startsWith"
                            | "endsWith"
                            | "is"
                            | "greaterThan"
                            | "lessThan"
                            | "olderThanDays"
                            | "newerThanDays"
                            | "unknown"
                    ) {
                        condition_object
                            .insert("operator".to_string(), Value::String("unknown".to_string()));
                        changed = true;
                    }
                }
            }
        }
        if changed {
            conn.execute(
                "UPDATE rules SET groups_json = ?2 WHERE id = ?1",
                params![id, serde_json::to_string(&groups)?],
            )?;
        }
    }
    Ok(())
}

fn execute_column_migrations(conn: &Connection, statements: &[&str]) -> Result<(), DbError> {
    for statement in statements {
        match conn.execute_batch(statement) {
            Ok(()) => {}
            Err(rusqlite::Error::SqliteFailure(_, Some(message)))
                if message.contains("duplicate column name") => {}
            Err(error) => return Err(DbError::Sqlite(error)),
        }
    }
    Ok(())
}

fn ensure_trigram_fts(conn: &Connection) -> Result<(), DbError> {
    let existing_sql = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'files_fts'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    if existing_sql
        .as_deref()
        .map(is_trigram_fts_definition)
        .unwrap_or(false)
    {
        return Ok(());
    }

    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS files_ai;
        DROP TRIGGER IF EXISTS files_ad;
        DROP TRIGGER IF EXISTS files_au;
        DROP TABLE IF EXISTS files_fts;

        CREATE VIRTUAL TABLE files_fts USING fts5(
            name,
            path,
            content='files',
            content_rowid='rowid',
            tokenize='trigram'
        );

        INSERT INTO files_fts(files_fts) VALUES('rebuild');
        "#,
    )?;
    Ok(())
}

fn ensure_fts_triggers(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r#"
        CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
            INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
        END;
        CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
            INSERT INTO files_fts(files_fts, rowid, name, path)
            VALUES('delete', old.rowid, old.name, old.path);
        END;
        CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE ON files BEGIN
            INSERT INTO files_fts(files_fts, rowid, name, path)
            VALUES('delete', old.rowid, old.name, old.path);
            INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
        END;
        "#,
    )?;
    Ok(())
}

fn is_trigram_fts_definition(sql: &str) -> bool {
    let normalized = sql.to_ascii_lowercase().replace(char::is_whitespace, "");
    normalized.contains("tokenize='trigram'") || normalized.contains("tokenize=\"trigram\"")
}
