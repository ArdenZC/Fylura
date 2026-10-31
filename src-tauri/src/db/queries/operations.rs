use super::super::*;
use crate::file_ops::OperationLogDto;
use rusqlite::{params, OptionalExtension, Row};
use std::time::{SystemTime, UNIX_EPOCH};

impl Database {
    pub fn get_operation_logs(&self, limit: Option<u32>) -> Result<Vec<OperationLogDto>, DbError> {
        let limit = i64::from(limit.unwrap_or(500).clamp(1, 1000));
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT
                id,
                batch_id,
                operation_type,
                source_path,
                target_path,
                old_name,
                new_name,
                status,
                error_message,
                created_at,
                can_undo,
                path_before,
                path_after,
                name_before,
                name_after,
                can_restore,
                restored_at,
                restore_status,
                restore_error,
                source_size, source_modified_ns, source_platform_file_id, source_quick_hash,
                source_full_hash, target_platform_file_id, target_full_hash,
                source_claim_path, operation_phase, claim_created_at,
                claim_platform_file_id, claim_full_hash,
                restore_claim_path, restore_phase, restore_claim_created_at,
                restore_claim_platform_file_id, restore_claim_full_hash,
                source_platform_volume_id, target_platform_volume_id,
                claim_platform_volume_id, restore_claim_platform_volume_id
            FROM operation_logs
            ORDER BY created_at DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map(params![limit], operation_log_from_row)?;

        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }

    pub fn get_operation_log_by_id(&self, id: &str) -> Result<Option<OperationLogDto>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            r#"
            SELECT
                id,
                batch_id,
                operation_type,
                source_path,
                target_path,
                old_name,
                new_name,
                status,
                error_message,
                created_at,
                can_undo,
                path_before,
                path_after,
                name_before,
                name_after,
                can_restore,
                restored_at,
                restore_status,
                restore_error,
                source_size, source_modified_ns, source_platform_file_id, source_quick_hash,
                source_full_hash, target_platform_file_id, target_full_hash,
                source_claim_path, operation_phase, claim_created_at,
                claim_platform_file_id, claim_full_hash,
                restore_claim_path, restore_phase, restore_claim_created_at,
                restore_claim_platform_file_id, restore_claim_full_hash,
                source_platform_volume_id, target_platform_volume_id,
                claim_platform_volume_id, restore_claim_platform_volume_id
            FROM operation_logs
            WHERE id = ?1
            "#,
            params![id],
            operation_log_from_row,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub fn get_restorable_operation_logs_by_ids(
        &self,
        ids: &[String],
    ) -> Result<Vec<OperationLogDto>, DbError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT
                id, batch_id, operation_type, source_path, target_path, old_name, new_name,
                status, error_message, created_at, can_undo, path_before, path_after,
                name_before, name_after, can_restore, restored_at, restore_status, restore_error,
                source_size, source_modified_ns, source_platform_file_id, source_quick_hash,
                source_full_hash, target_platform_file_id, target_full_hash,
                source_claim_path, operation_phase, claim_created_at,
                claim_platform_file_id, claim_full_hash,
                restore_claim_path, restore_phase, restore_claim_created_at,
                restore_claim_platform_file_id, restore_claim_full_hash,
                source_platform_volume_id, target_platform_volume_id,
                claim_platform_volume_id, restore_claim_platform_volume_id
            FROM operation_logs
            WHERE id = ?1
              AND status = 'success'
              AND can_restore = 1
              AND restore_status = 'not_restored'
            "#,
        )?;
        let mut logs = Vec::with_capacity(ids.len());
        for id in ids {
            let log = stmt
                .query_row(params![id], operation_log_from_row)
                .optional()?;
            if let Some(log) = log {
                logs.push(log);
            }
        }
        Ok(logs)
    }

    pub fn get_pending_operation_logs(&self) -> Result<Vec<OperationLogDto>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT
                id, batch_id, operation_type, source_path, target_path, old_name, new_name,
                status, error_message, created_at, can_undo, path_before, path_after,
                name_before, name_after, can_restore, restored_at, restore_status, restore_error,
                source_size, source_modified_ns, source_platform_file_id, source_quick_hash,
                source_full_hash, target_platform_file_id, target_full_hash,
                source_claim_path, operation_phase, claim_created_at,
                claim_platform_file_id, claim_full_hash,
                restore_claim_path, restore_phase, restore_claim_created_at,
                restore_claim_platform_file_id, restore_claim_full_hash,
                source_platform_volume_id, target_platform_volume_id,
                claim_platform_volume_id, restore_claim_platform_volume_id
            FROM operation_logs
            WHERE status = 'pending'
            ORDER BY created_at ASC
            "#,
        )?;
        let rows = stmt.query_map([], operation_log_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }

    pub fn get_pending_restore_logs(&self) -> Result<Vec<OperationLogDto>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT
                id, batch_id, operation_type, source_path, target_path, old_name, new_name,
                status, error_message, created_at, can_undo, path_before, path_after,
                name_before, name_after, can_restore, restored_at, restore_status, restore_error,
                source_size, source_modified_ns, source_platform_file_id, source_quick_hash,
                source_full_hash, target_platform_file_id, target_full_hash,
                source_claim_path, operation_phase, claim_created_at,
                claim_platform_file_id, claim_full_hash,
                restore_claim_path, restore_phase, restore_claim_created_at,
                restore_claim_platform_file_id, restore_claim_full_hash,
                source_platform_volume_id, target_platform_volume_id,
                claim_platform_volume_id, restore_claim_platform_volume_id
            FROM operation_logs
            WHERE restore_status = 'pending'
               OR (
                    restore_status = 'manual_review'
                    AND restore_phase = 'completed'
               )
            ORDER BY created_at ASC
            "#,
        )?;
        let rows = stmt.query_map([], operation_log_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }

    pub fn prepare_operation_restores(&self, logs: &[OperationLogDto]) -> Result<(), DbError> {
        if logs.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
                UPDATE operation_logs
                SET restore_status = 'pending',
                    restore_phase = 'prepared',
                    restore_error = NULL,
                    restore_claim_path = ?2,
                    restore_claim_created_at = ?3,
                    restore_claim_platform_file_id = ?4,
                    restore_claim_platform_volume_id = ?5,
                    restore_claim_full_hash = ?6
                WHERE id = ?1
                  AND status = 'success'
                  AND can_restore = 1
                  AND restore_status = 'not_restored'
                "#,
            )?;
            for log in logs {
                let Some(claim_path) = log.restore_claim_path.as_deref() else {
                    return Err(DbError::Validation(format!(
                        "Restore claim path is required before filesystem mutation: {}",
                        log.id
                    )));
                };
                if stmt.execute(params![
                    log.id,
                    claim_path,
                    log.restore_claim_created_at,
                    log.restore_claim_platform_file_id,
                    log.restore_claim_platform_volume_id,
                    log.restore_claim_full_hash
                ])? != 1
                {
                    return Err(DbError::Validation(format!(
                        "Operation log is no longer restorable: {}",
                        log.id
                    )));
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn mark_operation_restores_pending(&self, ids: &[String]) -> Result<(), DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
                UPDATE operation_logs
                SET restore_status = 'pending', restore_phase = 'prepared', restore_error = NULL
                WHERE id = ?1 AND status = 'success' AND can_restore = 1
                  AND restore_status = 'not_restored'
                "#,
            )?;
            for id in ids {
                if stmt.execute(params![id])? != 1 {
                    return Err(DbError::Validation(format!(
                        "Operation log is no longer restorable: {id}"
                    )));
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn save_operation_logs(
        &self,
        batch_id: &str,
        logs: &[OperationLogDto],
    ) -> Result<(), DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let created_at = logs
            .first()
            .map(|log| parse_operation_timestamp(&log.created_at))
            .unwrap_or_else(current_timestamp_ms);
        let batch_status = if logs.iter().any(|log| log.status == "pending") {
            "pending"
        } else if logs
            .iter()
            .any(|log| matches!(log.status.as_str(), "failed" | "manual_review"))
        {
            "partial_failed"
        } else if logs.iter().all(|log| log.status == "skipped") {
            "skipped"
        } else {
            "success"
        };

        tx.execute(
            r#"
            INSERT INTO operation_batches (id, created_at, status)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(id) DO UPDATE SET
                created_at = excluded.created_at,
                status = excluded.status
            "#,
            params![batch_id, created_at, batch_status],
        )?;

        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO operation_logs (
                    id,
                    batch_id,
                    operation_type,
                    source_path,
                    target_path,
                    old_name,
                    new_name,
                    status,
                    error_message,
                    created_at,
                    can_undo,
                    path_before,
                    path_after,
                    name_before,
                    name_after,
                    can_restore,
                    restored_at,
                    restore_status,
                    restore_error,
                    source_size,
                    source_modified_ns,
                    source_platform_file_id,
                    source_quick_hash,
                    source_full_hash,
                    target_platform_file_id,
                    target_full_hash,
                    source_claim_path,
                    operation_phase,
                    claim_created_at,
                    claim_platform_file_id,
                    claim_full_hash,
                    restore_claim_path,
                    restore_phase,
                    restore_claim_created_at,
                    restore_claim_platform_file_id,
                    restore_claim_full_hash,
                    source_platform_volume_id,
                    target_platform_volume_id,
                    claim_platform_volume_id,
                    restore_claim_platform_volume_id
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40)
                ON CONFLICT(id) DO UPDATE SET
                    batch_id = excluded.batch_id,
                    operation_type = excluded.operation_type,
                    source_path = excluded.source_path,
                    target_path = excluded.target_path,
                    old_name = excluded.old_name,
                    new_name = excluded.new_name,
                    status = excluded.status,
                    error_message = excluded.error_message,
                    created_at = excluded.created_at,
                    can_undo = excluded.can_undo,
                    path_before = excluded.path_before,
                    path_after = excluded.path_after,
                    name_before = excluded.name_before,
                    name_after = excluded.name_after,
                    can_restore = excluded.can_restore,
                    restored_at = excluded.restored_at,
                    restore_status = excluded.restore_status,
                    restore_error = excluded.restore_error,
                    source_size = excluded.source_size,
                    source_modified_ns = excluded.source_modified_ns,
                    source_platform_file_id = excluded.source_platform_file_id,
                    source_quick_hash = excluded.source_quick_hash,
                    source_full_hash = excluded.source_full_hash,
                    target_platform_file_id = excluded.target_platform_file_id,
                    target_full_hash = excluded.target_full_hash,
                    source_claim_path = excluded.source_claim_path,
                    operation_phase = excluded.operation_phase,
                    claim_created_at = excluded.claim_created_at,
                    claim_platform_file_id = excluded.claim_platform_file_id,
                    claim_full_hash = excluded.claim_full_hash,
                    restore_claim_path = excluded.restore_claim_path,
                    restore_phase = excluded.restore_phase,
                    restore_claim_created_at = excluded.restore_claim_created_at,
                    restore_claim_platform_file_id = excluded.restore_claim_platform_file_id,
                    restore_claim_full_hash = excluded.restore_claim_full_hash,
                    source_platform_volume_id = excluded.source_platform_volume_id,
                    target_platform_volume_id = excluded.target_platform_volume_id,
                    claim_platform_volume_id = excluded.claim_platform_volume_id,
                    restore_claim_platform_volume_id = excluded.restore_claim_platform_volume_id
                "#,
            )?;

            for log in logs {
                stmt.execute(params![
                    log.id,
                    log.batch_id,
                    log.operation_type,
                    log.source_path,
                    log.target_path,
                    log.old_name,
                    log.new_name,
                    log.status,
                    log.error_message,
                    parse_operation_timestamp(&log.created_at),
                    bool_to_i64(log.can_undo),
                    log.path_before,
                    log.path_after,
                    log.name_before,
                    log.name_after,
                    bool_to_i64(log.can_restore),
                    log.restored_at
                        .as_deref()
                        .and_then(parse_optional_operation_timestamp),
                    log.restore_status,
                    log.restore_error,
                    log.source_size.map(|value| value as i64),
                    log.source_modified_ns,
                    log.source_platform_file_id,
                    log.source_quick_hash,
                    log.source_full_hash,
                    log.target_platform_file_id,
                    log.target_full_hash,
                    log.source_claim_path,
                    log.operation_phase,
                    log.claim_created_at,
                    log.claim_platform_file_id,
                    log.claim_full_hash,
                    log.restore_claim_path,
                    log.restore_phase,
                    log.restore_claim_created_at,
                    log.restore_claim_platform_file_id,
                    log.restore_claim_full_hash,
                    log.source_platform_volume_id,
                    log.target_platform_volume_id,
                    log.claim_platform_volume_id,
                    log.restore_claim_platform_volume_id
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn update_operation_phase(&self, log: &OperationLogDto) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE operation_logs
            SET status = ?2,
                operation_phase = ?3,
                error_message = ?4,
                source_claim_path = ?5,
                claim_created_at = ?6,
                claim_platform_file_id = ?7,
                claim_platform_volume_id = ?8,
                claim_full_hash = ?9,
                target_platform_file_id = ?10,
                target_platform_volume_id = ?11,
                target_full_hash = ?12
            WHERE id = ?1
            "#,
            params![
                log.id,
                log.status,
                log.operation_phase,
                log.error_message,
                log.source_claim_path,
                log.claim_created_at,
                log.claim_platform_file_id,
                log.claim_platform_volume_id,
                log.claim_full_hash,
                log.target_platform_file_id,
                log.target_platform_volume_id,
                log.target_full_hash
            ],
        )?;
        Ok(())
    }

    /// Persist the URLs selected by a native coordinator before the filesystem
    /// primitive is allowed to claim or publish anything.  This is deliberately
    /// separate from phase updates: the actual URL callback runs before the
    /// first claim, while phase changes may happen later in the transaction.
    pub fn update_operation_actual_paths(&self, log: &OperationLogDto) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE operation_logs
            SET source_path = ?2,
                target_path = ?3,
                old_name = ?4,
                new_name = ?5,
                path_before = ?6,
                path_after = ?7,
                name_before = ?8,
                name_after = ?9,
                source_claim_path = ?10
            WHERE id = ?1
            "#,
            params![
                log.id,
                log.source_path,
                log.target_path,
                log.old_name,
                log.new_name,
                log.path_before,
                log.path_after,
                log.name_before,
                log.name_after,
                log.source_claim_path,
            ],
        )?;
        Ok(())
    }

    pub fn update_operation_restore_phase(&self, log: &OperationLogDto) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE operation_logs
            SET restore_status = ?2,
                restore_phase = ?3,
                restore_error = ?4,
                restore_claim_path = ?5,
                restore_claim_created_at = ?6,
                restore_claim_platform_file_id = ?7,
                restore_claim_platform_volume_id = ?8,
                restore_claim_full_hash = ?9,
                target_full_hash = ?10
            WHERE id = ?1
            "#,
            params![
                log.id,
                log.restore_status,
                log.restore_phase,
                log.restore_error,
                log.restore_claim_path,
                log.restore_claim_created_at,
                log.restore_claim_platform_file_id,
                log.restore_claim_platform_volume_id,
                log.restore_claim_full_hash,
                log.target_full_hash
            ],
        )?;
        Ok(())
    }

    pub fn update_operation_restore_logs(&self, logs: &[OperationLogDto]) -> Result<(), DbError> {
        if logs.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
                UPDATE operation_logs
                SET status = ?2,
                    can_restore = ?3,
                    restored_at = ?4,
                    restore_status = ?5,
                    restore_error = ?6,
                    can_undo = ?7,
                    restore_phase = ?8,
                    restore_claim_path = ?9,
                    restore_claim_created_at = ?10,
                    restore_claim_platform_file_id = ?11,
                    restore_claim_platform_volume_id = ?12,
                    restore_claim_full_hash = ?13
                WHERE id = ?1
                "#,
            )?;

            for log in logs {
                let updated = stmt.execute(params![
                    log.id,
                    log.status,
                    bool_to_i64(log.can_restore),
                    log.restored_at
                        .as_deref()
                        .and_then(parse_optional_operation_timestamp),
                    log.restore_status,
                    log.restore_error,
                    bool_to_i64(log.can_undo),
                    log.restore_phase,
                    log.restore_claim_path,
                    log.restore_claim_created_at,
                    log.restore_claim_platform_file_id,
                    log.restore_claim_platform_volume_id,
                    log.restore_claim_full_hash
                ])?;
                if updated != 1 {
                    return Err(DbError::Validation(format!(
                        "operation restore journal row disappeared during finalization: {}",
                        log.id
                    )));
                }
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn finalize_operation_restore_outcome(
        &self,
        logs: &[OperationLogDto],
    ) -> Result<(), DbError> {
        self.update_operation_restore_logs(logs)
    }

    pub fn finalize_operation_recovery_action(
        &self,
        log: &OperationLogDto,
        expected_claim_path: Option<&str>,
    ) -> Result<(), DbError> {
        let conn = self.conn()?;
        let updated = conn.execute(
            r#"
            UPDATE operation_logs
            SET status = ?2,
                can_restore = ?3,
                restored_at = ?4,
                restore_status = ?5,
                restore_error = ?6,
                can_undo = ?7,
                restore_phase = ?8,
                restore_claim_path = ?9,
                restore_claim_created_at = ?10,
                restore_claim_platform_file_id = ?11,
                restore_claim_platform_volume_id = ?12,
                restore_claim_full_hash = ?13
            WHERE id = ?1
              AND status = 'manual_review'
              AND restore_status = 'manual_review'
              AND (
                    (?14 IS NULL AND restore_claim_path IS NULL)
                    OR restore_claim_path = ?14
              )
            "#,
            params![
                log.id,
                log.status,
                bool_to_i64(log.can_restore),
                log.restored_at
                    .as_deref()
                    .and_then(parse_optional_operation_timestamp),
                log.restore_status,
                log.restore_error,
                bool_to_i64(log.can_undo),
                log.restore_phase,
                log.restore_claim_path,
                log.restore_claim_created_at,
                log.restore_claim_platform_file_id,
                log.restore_claim_platform_volume_id,
                log.restore_claim_full_hash,
                expected_claim_path
            ],
        )?;
        if updated != 1 {
            return Err(DbError::Validation(
                "recovery action journal changed before finalization; review the new operation log"
                    .to_string(),
            ));
        }
        Ok(())
    }

    pub fn finalize_source_cleanup_retry(
        &self,
        log_id: &str,
        expected_claim_path: &str,
        can_restore: bool,
    ) -> Result<(), DbError> {
        let conn = self.conn()?;
        let updated = conn.execute(
            r#"
            UPDATE operation_logs
            SET status = 'success',
                error_message = NULL,
                operation_phase = 'completed',
                source_claim_path = NULL,
                can_undo = ?2,
                can_restore = ?2,
                restore_status = 'not_restored',
                restore_phase = 'idle',
                restore_error = NULL
            WHERE id = ?1
              AND status = 'manual_review'
              AND operation_phase = 'source_cleanup_pending'
              AND source_claim_path = ?3
            "#,
            params![log_id, bool_to_i64(can_restore), expected_claim_path],
        )?;
        if updated != 1 {
            return Err(DbError::Validation(
                "source cleanup retry journal changed before finalization; review the operation log"
                    .to_string(),
            ));
        }
        Ok(())
    }

    pub fn finalize_source_cleanup_recovery_action(
        &self,
        log: &OperationLogDto,
        expected_claim_path: &str,
    ) -> Result<(), DbError> {
        let conn = self.conn()?;
        let updated = conn.execute(
            r#"
            UPDATE operation_logs
            SET status = ?2,
                operation_phase = ?3,
                error_message = ?4,
                can_restore = ?5,
                restored_at = ?6,
                restore_status = ?7,
                restore_error = ?8,
                can_undo = ?9,
                restore_phase = ?10,
                source_claim_path = NULL,
                restore_claim_path = ?11,
                restore_claim_created_at = ?12,
                restore_claim_platform_file_id = ?13,
                restore_claim_platform_volume_id = ?14,
                restore_claim_full_hash = ?15
            WHERE id = ?1
              AND status = 'manual_review'
              AND operation_phase = 'source_cleanup_pending'
              AND source_claim_path = ?16
            "#,
            params![
                log.id,
                log.status,
                log.operation_phase,
                log.error_message,
                bool_to_i64(log.can_restore),
                log.restored_at
                    .as_deref()
                    .and_then(parse_optional_operation_timestamp),
                log.restore_status,
                log.restore_error,
                bool_to_i64(log.can_undo),
                log.restore_phase,
                log.restore_claim_path,
                log.restore_claim_created_at,
                log.restore_claim_platform_file_id,
                log.restore_claim_platform_volume_id,
                log.restore_claim_full_hash,
                expected_claim_path
            ],
        )?;
        if updated != 1 {
            return Err(DbError::Validation(
                "source cleanup recovery journal changed before finalization; review the operation log"
                    .to_string(),
            ));
        }
        Ok(())
    }

    pub fn prune_operation_logs(&self, retention_days: i64) -> Result<(), DbError> {
        let retention_days = retention_days.max(0);
        let retention_ms = retention_days.saturating_mul(24 * 60 * 60 * 1000);
        let prune_before = current_timestamp_ms().saturating_sub(retention_ms);
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;

        tx.execute(
            "DELETE FROM operation_logs WHERE created_at < ?1",
            params![prune_before],
        )?;
        tx.execute(
            r#"
            DELETE FROM operation_batches
            WHERE NOT EXISTS (
                SELECT 1
                FROM operation_logs
                WHERE operation_logs.batch_id = operation_batches.id
            )
            "#,
            [],
        )?;

        tx.commit()?;
        Ok(())
    }
}

fn parse_operation_timestamp(value: &str) -> i64 {
    value
        .parse::<i64>()
        .unwrap_or_else(|_| current_timestamp_ms())
}

fn parse_optional_operation_timestamp(value: &str) -> Option<i64> {
    value.parse::<i64>().ok()
}

fn operation_log_from_row(row: &Row<'_>) -> rusqlite::Result<OperationLogDto> {
    let created_at: i64 = row.get(9)?;
    let restored_at: Option<i64> = row.get(16)?;
    Ok(OperationLogDto {
        id: row.get(0)?,
        batch_id: row.get(1)?,
        operation_type: row.get(2)?,
        source_path: row.get(3)?,
        target_path: row.get(4)?,
        old_name: row.get(5)?,
        new_name: row.get(6)?,
        status: row.get(7)?,
        error_message: row.get(8)?,
        created_at: created_at.to_string(),
        can_undo: row.get::<_, i64>(10)? != 0,
        path_before: row.get(11)?,
        path_after: row.get(12)?,
        name_before: row.get(13)?,
        name_after: row.get(14)?,
        can_restore: row.get::<_, i64>(15)? != 0,
        restored_at: restored_at.map(|value| value.to_string()),
        restore_status: row.get(17)?,
        restore_error: row.get(18)?,
        source_size: row.get::<_, Option<i64>>(19)?.map(|value| value as u64),
        source_modified_ns: row.get(20)?,
        source_platform_file_id: row.get(21)?,
        source_quick_hash: row.get(22)?,
        source_full_hash: row.get(23)?,
        target_platform_file_id: row.get(24)?,
        target_full_hash: row.get(25)?,
        source_claim_path: row.get(26)?,
        operation_phase: row.get(27)?,
        claim_created_at: row.get(28)?,
        claim_platform_file_id: row.get(29)?,
        claim_full_hash: row.get(30)?,
        restore_claim_path: row.get(31)?,
        restore_phase: row.get(32)?,
        restore_claim_created_at: row.get(33)?,
        restore_claim_platform_file_id: row.get(34)?,
        restore_claim_full_hash: row.get(35)?,
        source_platform_volume_id: row.get(36)?,
        target_platform_volume_id: row.get(37)?,
        claim_platform_volume_id: row.get(38)?,
        restore_claim_platform_volume_id: row.get(39)?,
    })
}

fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
