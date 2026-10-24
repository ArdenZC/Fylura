use super::super::*;
use super::dedupe::{invalidate_file_in_transaction, invalidate_stale_files_in_transaction};
use super::library::{current_library_revision, selection_where, LibrarySelectionV1};
use super::library_identity::{
    advance_root_epoch_tx, has_source_uid_attribution_tx, managed_root_ids_for_path_tx,
    mark_root_path_absent_tx, observe_source_for_root_tx, verify_legacy_source_identity_tx,
    SourceObservationContext,
};
use super::*;
use crate::file_naming::{
    normalize_proposed_file_name, split_filename_from_target_directory, ExtensionChangePolicy,
};
use crate::file_ops::OperationLogDto;
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, Transaction,
};
use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    time::Instant,
};
use sysinfo::Disks;

fn fence_source_root_for_user_mutation_tx(
    tx: &Transaction<'_>,
    root_id: &str,
    observed_at: i64,
) -> Result<i64, DbError> {
    let changed = tx.execute(
        r#"
        UPDATE scan_roots
        SET revision = revision + 1,
            watcher_revision = watcher_revision + 1,
            library_change_revision = library_change_revision + 1,
            needs_reconciliation = 1,
            health_status = CASE
                WHEN health_status IN ('missing','permission_required') THEN health_status
                ELSE 'reconciliation_required'
            END,
            updated_at = ?1
        WHERE id = ?2 AND source_kind = 'file_library'
        "#,
        params![observed_at, root_id],
    )?;
    if changed != 1 {
        return Err(DbError::Validation(format!(
            "Source identity mutation lost its managed root: {root_id}"
        )));
    }
    tx.query_row(
        "SELECT revision FROM scan_roots WHERE id = ?1",
        params![root_id],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

fn resolve_insert_file_row_id(
    tx: &Transaction<'_>,
    file: &InsertFileRequest,
) -> Result<String, DbError> {
    let existing_identity = tx
        .query_row(
            "SELECT path FROM files WHERE id = ?1",
            params![file.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(archived_path) = existing_identity else {
        return Ok(file.id.clone());
    };
    if !super::is_legacy_unassigned_archive_path(&archived_path) {
        return Ok(file.id.clone());
    }

    let active_path_id = tx
        .query_row(
            "SELECT id FROM files WHERE path = ?1 AND is_stale = 0 ORDER BY id LIMIT 1",
            params![file.path],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    active_path_id.ok_or_else(|| {
        DbError::Validation(
            "Path upsert cannot revive a parked legacy row without a current active row; a managed scan must establish the current source identity first.".to_string(),
        )
    })
}

#[derive(Debug, Clone)]
struct SearchMatchSql {
    cte: String,
    params: Vec<SqlValue>,
}

impl Database {
    pub fn verify_indexed_file_identity(&self, file_id: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        let (path, expected_size, expected_mtime): (String, i64, i64) = conn.query_row(
            "SELECT path, size, mtime FROM files WHERE id = ?1 AND is_stale = 0",
            params![file_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let metadata = fs::metadata(&path)?;
        if !metadata.is_dir() && i64::try_from(metadata.len()).unwrap_or(i64::MAX) != expected_size
        {
            return Err(DbError::Validation(format!(
                "File changed after preview (size mismatch): {path}"
            )));
        }
        let actual_mtime = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|value| value.as_secs() as i64)
            .unwrap_or(0);
        if expected_mtime > 0 && actual_mtime != expected_mtime {
            return Err(DbError::Validation(format!(
                "File changed after preview (modified time mismatch): {path}"
            )));
        }
        Ok(())
    }

    pub(crate) fn get_indexed_file_naming(
        &self,
        file_id: &str,
    ) -> Result<(String, String, bool), DbError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT name, extension, is_dir FROM files WHERE id = ?1 AND is_stale = 0",
            params![file_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0)),
        )
        .map_err(DbError::from)
    }

    pub fn insert_file(&self, file: InsertFileRequest) -> Result<(), DbError> {
        self.insert_files(&[file])
    }

    pub fn insert_files(&self, files: &[InsertFileRequest]) -> Result<(), DbError> {
        if files.is_empty() {
            return Ok(());
        }
        let last_seen_at = current_unix_seconds();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
            INSERT INTO files (
                id, path, name, extension, size, mtime, ctime, is_dir, state_code,
                file_type, suggested_name, classification_status, is_stale, last_seen_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13)
            ON CONFLICT(id) DO UPDATE SET
                path = excluded.path,
                name = excluded.name,
                extension = excluded.extension,
                size = excluded.size,
                mtime = excluded.mtime,
                ctime = excluded.ctime,
                is_dir = excluded.is_dir,
                state_code = excluded.state_code,
                file_type = excluded.file_type,
                suggested_name = CASE
                    WHEN files.suggested_name = '' OR files.suggested_name = files.name
                    THEN excluded.suggested_name
                    ELSE files.suggested_name
                END,
                content_hash = CASE
                    WHEN files.path != excluded.path
                      OR files.size != excluded.size
                      OR files.mtime != excluded.mtime
                      OR files.is_dir != excluded.is_dir
                    THEN ''
                    ELSE files.content_hash
                END,
                is_stale = 0,
                last_seen_at = excluded.last_seen_at
            "#,
            )?;

            for file in files {
                let mut resolved_file = file.clone();
                resolved_file.id = resolve_insert_file_row_id(&tx, file)?;
                let previous = tx
                    .query_row(
                        "SELECT path, size, mtime, is_dir, is_stale FROM files WHERE id = ?1",
                        params![resolved_file.id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, i64>(2)?,
                                row.get::<_, i64>(3)?,
                                row.get::<_, i64>(4)?,
                            ))
                        },
                    )
                    .optional()?;
                if previous.is_some() && has_source_uid_attribution_tx(&tx, &resolved_file.id)? {
                    let physical = crate::fs_safety::capture_source_object_identity(Path::new(
                        &resolved_file.path,
                    ))
                    .ok();
                    if verify_legacy_source_identity_tx(&tx, &resolved_file.id, physical.as_ref())?
                        .is_none()
                    {
                        return Err(DbError::Validation(format!(
                            "Path upsert refused for SourceUID-attributed row {}: physical identity is missing, changed, or unverified; Managed Scan/Watcher reconciliation is required.",
                            resolved_file.id
                        )));
                    }
                }
                let file_type = infer_file_type(&resolved_file.extension, resolved_file.is_dir);
                stmt.execute(params![
                    resolved_file.id,
                    resolved_file.path,
                    resolved_file.name,
                    resolved_file.extension,
                    resolved_file.size,
                    resolved_file.mtime,
                    resolved_file.ctime,
                    bool_to_i64(resolved_file.is_dir),
                    resolved_file.state_code,
                    file_type,
                    resolved_file.name,
                    CLASSIFICATION_STATUS_UNCLASSIFIED,
                    last_seen_at
                ])?;
                if let Some((old_path, old_size, old_mtime, old_is_dir, old_is_stale)) = previous {
                    let changed = old_path != resolved_file.path
                        || old_size != resolved_file.size
                        || old_mtime != resolved_file.mtime
                        || old_is_dir != bool_to_i64(resolved_file.is_dir);
                    if changed {
                        invalidate_file_in_transaction(
                            &tx,
                            &resolved_file.id,
                            if old_is_stale != 0 {
                                "missing"
                            } else {
                                "stale"
                            },
                        )?;
                    }
                }
            }
        }
        super::library::bump_library_query_revision_in_transaction(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Compatibility command path for watcher removals: mark matching files stale instead of
    /// deleting rows, so transient file-system events do not destroy index history.
    pub fn remove_files_by_paths(&self, paths: &[String]) -> Result<usize, DbError> {
        if paths.is_empty() {
            return Ok(0);
        }

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut removed = 0;
        {
            let mut stmt = tx.prepare(
                r#"
                UPDATE files
                SET is_stale = 1
                WHERE is_stale = 0
                  AND (
                    path = ?1
                    OR path LIKE ?2 ESCAPE '~'
                    OR path LIKE ?3 ESCAPE '~'
                  )
                "#,
            )?;

            for path in paths
                .iter()
                .map(|path| path.trim())
                .filter(|path| !path.is_empty())
            {
                let path = trim_trailing_path_separators(path);
                if path.is_empty() {
                    continue;
                }

                let normalized_path = normalize_path_text(path);
                for candidate in path_lookup_candidates(path, &normalized_path) {
                    let escaped_path = escape_like_pattern(&candidate);
                    let slash_descendants = format!("{escaped_path}/%");
                    let backslash_descendants = format!("{escaped_path}\\%");
                    removed +=
                        stmt.execute(params![candidate, slash_descendants, backslash_descendants])?;
                }
            }
        }
        invalidate_stale_files_in_transaction(&tx)?;
        if removed > 0 {
            super::library::bump_library_query_revision_in_transaction(&tx)?;
        }
        tx.commit()?;
        Ok(removed)
    }

    pub fn mark_missing_files_stale_after_scan(
        &self,
        root: &str,
        scan_started_at: i64,
    ) -> Result<usize, DbError> {
        let root = trim_trailing_path_separators(root.trim());
        if root.is_empty() {
            return Ok(0);
        }

        let normalized_root = normalize_path_text(root);
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut marked = 0;
        {
            let mut stmt = tx.prepare(
                r#"
                UPDATE files
                SET is_stale = 1
                WHERE is_stale = 0
                  AND last_seen_at < ?1
                  AND (
                    path = ?2
                    OR path LIKE ?3 ESCAPE '~'
                    OR path LIKE ?4 ESCAPE '~'
                  )
                "#,
            )?;

            for candidate in path_lookup_candidates(root, &normalized_root) {
                let escaped_path = escape_like_pattern(&candidate);
                marked += stmt.execute(params![
                    scan_started_at,
                    candidate,
                    descendant_like_pattern(&escaped_path, '/'),
                    descendant_like_pattern(&escaped_path, '\\')
                ])?;
            }
        }
        invalidate_stale_files_in_transaction(&tx)?;
        if marked > 0 {
            super::library::bump_library_query_revision_in_transaction(&tx)?;
        }
        tx.commit()?;
        Ok(marked)
    }

    pub fn upsert_files_by_paths(&self, paths: &[String]) -> Result<usize, DbError> {
        upsert_files_by_paths_with_optional_optimize(self, paths)
    }

    pub fn optimize_search_index(&self) -> Result<u128, DbError> {
        let started = Instant::now();
        let conn = self.conn()?;
        conn.execute_batch("PRAGMA optimize;")?;
        Ok(started.elapsed().as_millis())
    }

    pub fn update_file_after_successful_operation(
        &self,
        file_id: &str,
        source_path: &str,
        target_path: &str,
        new_name: &str,
    ) -> Result<bool, DbError> {
        self.update_file_record_after_path_change(
            path_lookup_candidates(source_path, source_path),
            vec![file_id.to_string()],
            target_path,
            new_name,
        )
    }

    pub fn update_file_after_successful_restore(
        &self,
        log: &OperationLogDto,
    ) -> Result<bool, DbError> {
        if log.restore_status != "restored" {
            return Ok(false);
        }

        self.update_file_record_after_path_change(
            path_lookup_candidates_for_values(&[
                log.path_after.as_str(),
                log.target_path.as_str(),
                log.source_path.as_str(),
            ]),
            Vec::new(),
            &log.path_before,
            &log.name_before,
        )
    }

    /// Finalize an ordinary restore only after the filesystem commit has been
    /// verified.  The file-index path update and the restore journal update
    /// share one SQLite transaction so a restart can reconcile either the
    /// still-pending journal or the fully committed result, never a half-
    /// finalized restore.
    pub fn finalize_successful_operation_restore(
        &self,
        log: &OperationLogDto,
    ) -> Result<(), DbError> {
        if log.operation_type == "replace" {
            return self.finalize_successful_replace_restore(log);
        }
        if log.restore_status != "restored" || log.restore_phase != "completed" {
            return Err(DbError::Validation(
                "successful restore finalization requires restore_status=restored and restore_phase=completed"
                    .to_string(),
            ));
        }

        crate::file_ops::validate_operation_restore_final_identity(log)
            .map_err(DbError::Validation)?;

        let target = PathBuf::from(&log.path_before);
        crate::fs_safety::identity::ensure_supported_entry(&target).map_err(|error| {
            DbError::Validation(crate::recovery::format_recovery_message(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                &format!("restore target became unsupported during finalization: {error}"),
            ))
        })?;
        let metadata = fs::symlink_metadata(&target)?;
        let normalized_target = normalize_path_for_db(&target);
        let name = resolved_file_name(&log.path_before, &log.name_before);
        let extension = extension_from_file_name(&name);
        let size = if metadata.is_file() {
            i64::try_from(metadata.len()).unwrap_or(i64::MAX)
        } else {
            0
        };
        let mtime = metadata
            .modified()
            .ok()
            .and_then(system_time_to_unix_seconds)
            .unwrap_or_else(current_unix_seconds);
        let ctime = metadata
            .created()
            .ok()
            .and_then(system_time_to_unix_seconds)
            .unwrap_or(mtime);
        let is_dir = metadata.is_dir();
        let restored_at = current_unix_seconds();
        let target_candidates = path_lookup_candidates(&log.path_before, &normalized_target);
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let physical = crate::fs_safety::capture_source_object_identity(&target).ok();
        let (target_row_id, source_row_id) =
            Self::resolve_restore_index_rows(&tx, log, &target_candidates)?;
        if let Some(target_row_id) = target_row_id.as_deref() {
            let (indexed_size, indexed_mtime, indexed_is_dir): (i64, i64, i64) = tx.query_row(
                "SELECT size, mtime, is_dir FROM files WHERE id = ?1",
                params![target_row_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            if indexed_size != size
                || indexed_mtime != mtime
                || indexed_is_dir != bool_to_i64(is_dir)
            {
                return Err(DbError::Validation(format!(
                    "restore target index conflict: {}",
                    normalized_target
                )));
            }
        }

        let (current_id, watcher_target_id) = match (&target_row_id, &source_row_id) {
            (Some(target_row_id), Some(source_row_id)) if target_row_id != source_row_id => {
                (Some(source_row_id.clone()), Some(target_row_id.clone()))
            }
            _ => (
                source_row_id.clone().or_else(|| target_row_id.clone()),
                None,
            ),
        };

        let prior_source_uid = current_id
            .as_deref()
            .map(|file_id| verify_legacy_source_identity_tx(&tx, file_id, physical.as_ref()))
            .transpose()?
            .flatten();
        let old_path = normalize_path_for_db(Path::new(&log.path_after));
        let old_claim_roots = {
            let mut statement = tx.prepare(
                r#"
                SELECT DISTINCT claim.root_id
                FROM library_source_root_claims AS claim
                WHERE claim.observed_path = ?1
                  AND claim.claim_state IN ('present','unverified')
                ORDER BY claim.root_id
                "#,
            )?;
            let rows = statement
                .query_map(params![old_path], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let target_roots = managed_root_ids_for_path_tx(&tx, &normalized_target)?;
        let source_tracking_required = !old_claim_roots.is_empty() || !target_roots.is_empty();
        if source_tracking_required && current_id.is_some() && prior_source_uid.is_none() {
            return Err(DbError::Validation(
                "Restore source identity is unverified; legacy facts were retained for owner review."
                    .to_string(),
            ));
        }
        let mut root_epochs = HashMap::<String, (i64, i64)>::new();
        for root_id in &old_claim_roots {
            let owner_revision = fence_source_root_for_user_mutation_tx(&tx, root_id, restored_at)?;
            let epoch =
                advance_root_epoch_tx(&tx, root_id, "restore", owner_revision, restored_at)?;
            root_epochs.insert(root_id.clone(), (epoch, owner_revision));
            if old_path != normalized_target {
                mark_root_path_absent_tx(
                    &tx,
                    root_id,
                    &old_path,
                    epoch,
                    "restore",
                    owner_revision,
                    restored_at,
                )?;
            }
        }
        let restore_observation = InsertFileRequest {
            id: current_id
                .clone()
                .unwrap_or_else(|| normalized_target.clone()),
            path: normalized_target.clone(),
            name: name.clone(),
            extension: extension.clone(),
            size,
            mtime,
            ctime,
            is_dir,
            state_code: 0,
        };
        let mut restored_source_uid = prior_source_uid.clone();
        for (root_id, _) in target_roots {
            let owner_revision = if let Some((_, revision)) = root_epochs.get(&root_id) {
                *revision
            } else {
                let revision = fence_source_root_for_user_mutation_tx(&tx, &root_id, restored_at)?;
                let epoch = advance_root_epoch_tx(&tx, &root_id, "restore", revision, restored_at)?;
                root_epochs.insert(root_id.clone(), (epoch, revision));
                revision
            };
            let root_epoch = root_epochs[&root_id].0;
            let outcome = observe_source_for_root_tx(
                &tx,
                &restore_observation,
                physical.as_ref(),
                SourceObservationContext {
                    root_id: &root_id,
                    root_epoch,
                    owner_kind: "restore",
                    owner_revision,
                    legacy_row_exists: current_id.is_some(),
                    observed_at: restored_at,
                },
            )?;
            if outcome.block_legacy_row
                || prior_source_uid
                    .as_deref()
                    .is_some_and(|uid| uid != outcome.source_uid)
            {
                return Err(DbError::Validation(
                    "Restore source identity changed; legacy facts were retained for owner review."
                        .to_string(),
                ));
            }
            restored_source_uid = Some(outcome.source_uid);
        }
        if let Some(watcher_target_id) = watcher_target_id {
            let deleted = tx.execute(
                "DELETE FROM files WHERE id = ?1",
                params![watcher_target_id],
            )?;
            if deleted != 1 {
                return Err(DbError::Validation(format!(
                    "restore watcher target row disappeared during merge: {}",
                    watcher_target_id
                )));
            }
        }
        let file_type = infer_file_type(&extension, is_dir);
        let restored_file_id = current_id
            .as_deref()
            .filter(|file_id| is_neutral_source_file_id(file_id))
            .map(str::to_string)
            .unwrap_or_else(|| normalized_target.clone());
        if let Some(current_id) = current_id.as_deref() {
            // A compatibility row may keep a SourceUID-based id while its
            // path changes. Preserve that id so parked legacy rows whose
            // primary key is still the old path cannot capture its facts.
            let updated = tx.execute(
                r#"
                UPDATE files
                SET id = ?1,
                    path = ?2,
                    name = ?3,
                    extension = ?4,
                    size = ?5,
                    mtime = ?6,
                    ctime = ?7,
                    is_dir = ?8,
                    file_type = ?9,
                    is_stale = 0,
                    last_seen_at = ?10
                WHERE id = ?11
                "#,
                params![
                    restored_file_id,
                    normalized_target,
                    name,
                    extension,
                    size,
                    mtime,
                    ctime,
                    bool_to_i64(is_dir),
                    file_type,
                    current_unix_seconds(),
                    current_id
                ],
            )?;
            if updated != 1 {
                return Err(DbError::Validation(format!(
                    "restore index row disappeared during finalization: {}",
                    current_id
                )));
            }
        } else {
            let inserted = tx.execute(
                r#"
                INSERT INTO files (
                    id, path, name, extension, size, mtime, ctime, is_dir, state_code,
                    file_type, suggested_name, classification_status, is_stale, last_seen_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, 0, ?12)
                "#,
                params![
                    normalized_target,
                    normalized_target,
                    name,
                    extension,
                    size,
                    mtime,
                    ctime,
                    bool_to_i64(is_dir),
                    file_type,
                    name,
                    CLASSIFICATION_STATUS_UNCLASSIFIED,
                    current_unix_seconds(),
                ],
            )?;
            if inserted != 1 {
                return Err(DbError::Validation(format!(
                    "restore index row was not inserted: {}",
                    normalized_target
                )));
            }
        }

        let finalized = tx.execute(
            r#"
            UPDATE operation_logs
            SET status = 'success',
                error_message = NULL,
                can_restore = 0,
                restored_at = ?2,
                restore_status = 'restored',
                restore_error = NULL,
                can_undo = 0,
                restore_phase = 'completed',
                restore_claim_path = NULL,
                restore_claim_created_at = NULL,
                restore_claim_platform_file_id = NULL,
                restore_claim_platform_volume_id = NULL,
                restore_claim_full_hash = NULL
            WHERE id = ?1
            "#,
            params![
                log.id,
                log.restored_at
                    .as_deref()
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or_else(current_unix_seconds)
            ],
        )?;
        if finalized != 1 {
            return Err(DbError::Validation(format!(
                "restore journal row disappeared during finalization: {}",
                log.id
            )));
        }
        if let (Some(current_id), Some(source_uid)) =
            (current_id.as_deref(), restored_source_uid.as_deref())
        {
            tx.execute(
                r#"
                UPDATE library_legacy_attributions
                SET legacy_file_id = ?1,
                    legacy_path_snapshot = ?2,
                    decision_revision = decision_revision + 1,
                    updated_at = ?3
                WHERE legacy_file_id = ?4 AND source_uid = ?5
                  AND claim_state IN ('verified','user_confirmed')
                "#,
                params![
                    restored_file_id,
                    normalized_target,
                    restored_at,
                    current_id,
                    source_uid
                ],
            )?;
        }
        super::library::bump_library_query_revision_in_transaction(&tx)?;
        tx.commit()?;
        Ok(())
    }

    fn finalize_successful_replace_restore(&self, log: &OperationLogDto) -> Result<(), DbError> {
        if log.restore_status != "restored" || log.restore_phase != "completed" {
            return Err(DbError::Validation(
                "successful replacement restore finalization requires restore_status=restored and restore_phase=completed"
                    .to_string(),
            ));
        }

        crate::file_ops::validate_operation_restore_final_identity(log)
            .map_err(DbError::Validation)?;

        let restored_source = PathBuf::from(&log.path_before);
        let restored_target = PathBuf::from(&log.path_after);
        crate::fs_safety::identity::ensure_supported_entry(&restored_source).map_err(|error| {
            DbError::Validation(crate::recovery::format_recovery_message(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                &format!(
                    "replacement restore source became unsupported during finalization: {error}"
                ),
            ))
        })?;
        crate::fs_safety::identity::ensure_supported_entry(&restored_target).map_err(|error| {
            DbError::Validation(crate::recovery::format_recovery_message(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                &format!(
                    "replacement restore target became unsupported during finalization: {error}"
                ),
            ))
        })?;

        let source_request = restore_index_request(&restored_source, &log.name_before)?;
        let target_request = restore_index_request(&restored_target, &log.name_after)?;
        let before_candidates = path_lookup_candidates(&log.path_before, &source_request.path);
        let after_candidates = path_lookup_candidates(&log.path_after, &target_request.path);

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let before_row_id = Self::resolve_unique_restore_index_row(
            &tx,
            "replacement restore source",
            &before_candidates,
        )?;
        let after_row_id = Self::resolve_unique_restore_index_row(
            &tx,
            "replacement restore target",
            &after_candidates,
        )?;

        let after_is_replacement_source = after_row_id
            .as_deref()
            .map(|row_id| indexed_row_matches_restore_request(&tx, row_id, &source_request))
            .transpose()?
            .unwrap_or(false);
        let after_is_restored_target = after_row_id
            .as_deref()
            .map(|row_id| indexed_row_matches_restore_request(&tx, row_id, &target_request))
            .transpose()?
            .unwrap_or(false);

        if after_row_id.is_some() && !after_is_replacement_source && !after_is_restored_target {
            return Err(DbError::Validation(
                "replacement restore target index identity is ambiguous; manual review required"
                    .to_string(),
            ));
        }

        let current_id = if after_is_replacement_source {
            if let (Some(before_row_id), Some(after_row_id)) =
                (before_row_id.as_deref(), after_row_id.as_deref())
            {
                if before_row_id != after_row_id {
                    remove_file_index_row_in_transaction(&tx, before_row_id)?;
                }
            }
            after_row_id.clone()
        } else {
            before_row_id.clone()
        };

        if let Some(current_id) = current_id.as_deref() {
            invalidate_file_in_transaction(&tx, current_id, "stale")?;
            tx.execute(
                "DELETE FROM duplicate_group_members WHERE file_id = ?1",
                params![current_id],
            )?;
            tx.execute(
                "DELETE FROM file_fingerprints WHERE file_id = ?1",
                params![current_id],
            )?;
            let updated = tx.execute(
                r#"
                UPDATE files
                SET id = ?1,
                    path = ?1,
                    name = ?2,
                    extension = ?3,
                    size = ?4,
                    mtime = ?5,
                    ctime = ?6,
                    is_dir = ?7,
                    file_type = ?8,
                    is_stale = 0,
                    last_seen_at = ?9
                WHERE id = ?10
                "#,
                params![
                    source_request.id,
                    source_request.name,
                    source_request.extension,
                    source_request.size,
                    source_request.mtime,
                    source_request.ctime,
                    bool_to_i64(source_request.is_dir),
                    infer_file_type(&source_request.extension, source_request.is_dir),
                    current_unix_seconds(),
                    current_id,
                ],
            )?;
            if updated != 1 {
                return Err(DbError::Validation(
                    "replacement restore source index row disappeared during finalization"
                        .to_string(),
                ));
            }
        } else {
            let inserted = tx.execute(
                r#"
                INSERT INTO files (
                    id, path, name, extension, size, mtime, ctime, is_dir, state_code,
                    file_type, suggested_name, classification_status, is_stale, last_seen_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, 0, ?12)
                "#,
                params![
                    source_request.id,
                    source_request.path,
                    source_request.name,
                    source_request.extension,
                    source_request.size,
                    source_request.mtime,
                    source_request.ctime,
                    bool_to_i64(source_request.is_dir),
                    infer_file_type(&source_request.extension, source_request.is_dir),
                    source_request.name,
                    CLASSIFICATION_STATUS_UNCLASSIFIED,
                    current_unix_seconds(),
                ],
            )?;
            if inserted != 1 {
                return Err(DbError::Validation(
                    "replacement restore source index row was not inserted".to_string(),
                ));
            }
        }

        if after_is_replacement_source {
            if let Some(row_id) = Self::resolve_unique_restore_index_row(
                &tx,
                "replacement restore target after source move",
                &after_candidates,
            )? {
                return Err(DbError::Validation(format!(
                    "replacement restore target index row remained after source move: {row_id}"
                )));
            }
            insert_restore_index_row_in_transaction(&tx, &target_request)?;
        } else if !after_is_restored_target {
            insert_restore_index_row_in_transaction(&tx, &target_request)?;
        }

        let finalized = tx.execute(
            r#"
            UPDATE operation_logs
            SET status = 'success',
                error_message = NULL,
                can_restore = 0,
                restored_at = ?2,
                restore_status = 'restored',
                restore_error = NULL,
                can_undo = 0,
                restore_phase = 'completed',
                restore_claim_path = NULL,
                restore_claim_created_at = NULL,
                restore_claim_platform_file_id = NULL,
                restore_claim_platform_volume_id = NULL,
                restore_claim_full_hash = NULL
            WHERE id = ?1
            "#,
            params![
                log.id,
                log.restored_at
                    .as_deref()
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or_else(current_unix_seconds)
            ],
        )?;
        if finalized != 1 {
            return Err(DbError::Validation(format!(
                "replacement restore journal row disappeared during finalization: {}",
                log.id
            )));
        }
        super::library::bump_library_query_revision_in_transaction(&tx)?;
        tx.commit()?;
        Ok(())
    }

    fn resolve_restore_index_rows(
        tx: &Transaction<'_>,
        log: &OperationLogDto,
        target_candidates: &[String],
    ) -> Result<(Option<String>, Option<String>), DbError> {
        let target_row_id =
            Self::resolve_unique_restore_index_row(tx, "target", target_candidates)?;
        let path_after_row_id = Self::resolve_unique_restore_index_row(
            tx,
            "restore source path_after",
            &path_lookup_candidates_for_values(&[log.path_after.as_str()]),
        )?;
        let target_path_row_id = Self::resolve_unique_restore_index_row(
            tx,
            "restore source target_path",
            &path_lookup_candidates_for_values(&[log.target_path.as_str()]),
        )?;
        let source_path_row_id = Self::resolve_unique_restore_index_row(
            tx,
            "restore source source_path",
            &path_lookup_candidates_for_values(&[log.source_path.as_str()]),
        )?;

        let source_row_id = match (path_after_row_id, target_path_row_id) {
            (Some(path_after), Some(target_path)) if path_after != target_path => {
                return Err(DbError::Validation(
                    "restore source index rows are ambiguous across path_after and target_path"
                        .to_string(),
                ));
            }
            (Some(path_after), _) => Some(path_after),
            (_, Some(target_path)) => Some(target_path),
            (None, None) => source_path_row_id.clone(),
        };

        if let Some(source_path_row_id) = source_path_row_id {
            if Some(source_path_row_id.clone()) != target_row_id
                && Some(source_path_row_id) != source_row_id
            {
                return Err(DbError::Validation(
                    "restore source index rows are ambiguous across old source paths".to_string(),
                ));
            }
        }

        Ok((target_row_id, source_row_id))
    }

    fn resolve_unique_restore_index_row(
        tx: &Transaction<'_>,
        role: &str,
        candidates: &[String],
    ) -> Result<Option<String>, DbError> {
        let mut ids = Vec::new();
        for candidate in candidates {
            let mut stmt = tx.prepare(
                "SELECT id FROM files WHERE (id = ?1 OR path = ?1) AND (path = ?1 OR is_stale = 0)",
            )?;
            let rows = stmt.query_map(params![candidate], |row| row.get::<_, String>(0))?;
            for row in rows {
                let id = row?;
                if !ids.iter().any(|existing| existing == &id) {
                    ids.push(id);
                }
            }
        }
        match ids.as_slice() {
            [] => Ok(None),
            [id] => Ok(Some(id.clone())),
            _ => Err(DbError::Validation(format!(
                "restore {role} index rows are ambiguous"
            ))),
        }
    }

    fn update_file_record_after_path_change(
        &self,
        path_candidates: Vec<String>,
        identity_candidates: Vec<String>,
        target_path: &str,
        new_name: &str,
    ) -> Result<bool, DbError> {
        let target = PathBuf::from(target_path);
        let metadata = fs::metadata(&target)?;
        let normalized_target = normalize_path_for_db(&target);
        let name = resolved_file_name(target_path, new_name);
        let extension = extension_from_file_name(&name);
        let size = if metadata.is_file() {
            i64::try_from(metadata.len()).unwrap_or(i64::MAX)
        } else {
            0
        };
        let mtime = metadata
            .modified()
            .ok()
            .and_then(system_time_to_unix_seconds)
            .unwrap_or_else(current_unix_seconds);
        let ctime = metadata
            .created()
            .ok()
            .and_then(system_time_to_unix_seconds)
            .unwrap_or(mtime);
        let is_dir = metadata.is_dir();
        let last_seen_at = current_unix_seconds();
        let target_candidates = path_lookup_candidates(target_path, &normalized_target);

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let current_id = find_active_file_row_id(&tx, &path_candidates, &identity_candidates)?;
        let Some(current_id) = current_id else {
            tx.commit()?;
            return Ok(false);
        };
        let physical = crate::fs_safety::capture_source_object_identity(&target).ok();
        let old_path: String = tx.query_row(
            "SELECT path FROM files WHERE id = ?1",
            params![current_id],
            |row| row.get(0),
        )?;
        let verified_source_uid =
            verify_legacy_source_identity_tx(&tx, &current_id, physical.as_ref())?;
        let mut root_epochs = HashMap::<String, (i64, i64)>::new();
        let old_claim_roots = {
            let mut statement = tx.prepare(
                r#"
                SELECT DISTINCT claim.root_id
                FROM library_source_root_claims AS claim
                JOIN scan_roots AS root ON root.id = claim.root_id
                WHERE claim.observed_path = ?1
                  AND claim.claim_state IN ('present','unverified')
                ORDER BY claim.root_id
                "#,
            )?;
            let rows = statement
                .query_map(params![old_path], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let target_roots = managed_root_ids_for_path_tx(&tx, &normalized_target)?;
        let source_tracking_required = !old_claim_roots.is_empty() || !target_roots.is_empty();
        for root_id in &old_claim_roots {
            let owner_revision =
                fence_source_root_for_user_mutation_tx(&tx, root_id, last_seen_at)?;
            let epoch =
                advance_root_epoch_tx(&tx, root_id, "operation", owner_revision, last_seen_at)?;
            root_epochs.insert(root_id.clone(), (epoch, owner_revision));
            if old_path != normalized_target {
                mark_root_path_absent_tx(
                    &tx,
                    root_id,
                    &old_path,
                    epoch,
                    "operation",
                    owner_revision,
                    last_seen_at,
                )?;
            }
        }

        let mut source_uid = verified_source_uid.clone();
        let mut source_write_blocked = source_tracking_required && verified_source_uid.is_none();
        let observation = InsertFileRequest {
            id: current_id.clone(),
            path: normalized_target.clone(),
            name: name.clone(),
            extension: extension.clone(),
            size,
            mtime,
            ctime,
            is_dir,
            state_code: 0,
        };
        for (root_id, _) in target_roots {
            let owner_revision = if let Some((_, revision)) = root_epochs.get(&root_id) {
                *revision
            } else {
                let revision = fence_source_root_for_user_mutation_tx(&tx, &root_id, last_seen_at)?;
                let epoch =
                    advance_root_epoch_tx(&tx, &root_id, "operation", revision, last_seen_at)?;
                root_epochs.insert(root_id.clone(), (epoch, revision));
                revision
            };
            let root_epoch = root_epochs[&root_id].0;
            let outcome = observe_source_for_root_tx(
                &tx,
                &observation,
                physical.as_ref(),
                SourceObservationContext {
                    root_id: &root_id,
                    root_epoch,
                    owner_kind: "operation",
                    owner_revision,
                    legacy_row_exists: true,
                    observed_at: last_seen_at,
                },
            )?;
            source_write_blocked |= outcome.block_legacy_row
                || verified_source_uid.as_deref() != Some(outcome.source_uid.as_str());
            source_uid = Some(outcome.source_uid);
        }
        if source_write_blocked {
            tx.execute(
                "UPDATE files SET is_stale = 1 WHERE id = ?1",
                params![current_id],
            )?;
            invalidate_file_in_transaction(&tx, &current_id, "stale")?;
            super::library::bump_library_query_revision_in_transaction(&tx)?;
            tx.commit()?;
            return Err(DbError::Validation(
                "Source identity changed or is unverified after the operation; legacy facts remain non-actionable for owner review.".to_string(),
            ));
        }

        // A successful move/rename changes the indexed identity.  Remove the
        // old fingerprint and membership before changing files.id so the
        // schema-29 foreign keys cannot retain stale group ownership or block
        // the existing operation-journal path update.
        invalidate_file_in_transaction(&tx, &current_id, "stale")?;
        tx.execute(
            "DELETE FROM duplicate_group_members WHERE file_id = ?1",
            params![current_id],
        )?;
        tx.execute(
            "DELETE FROM file_fingerprints WHERE file_id = ?1",
            params![current_id],
        )?;

        for candidate in target_candidates {
            tx.execute(
                r#"
                DELETE FROM files
                WHERE (id = ?1 OR path = ?1)
                  AND id <> ?2
                  AND NOT (is_stale = 1 AND path <> ?1)
                "#,
                params![candidate, current_id],
            )?;
        }

        let target_file_id = if is_neutral_source_file_id(&current_id) {
            current_id.clone()
        } else {
            normalized_target.clone()
        };
        let updated = tx.execute(
            r#"
            UPDATE files
            SET id = ?1,
                path = ?2,
                name = ?3,
                extension = ?4,
                size = ?5,
                mtime = ?6,
                ctime = ?7,
                is_dir = ?8,
                suggested_action = 'Keep',
                requires_confirmation = 0,
                is_stale = 0,
                last_seen_at = ?9
            WHERE id = ?10
            "#,
            params![
                target_file_id,
                normalized_target,
                name,
                extension,
                size,
                mtime,
                ctime,
                bool_to_i64(is_dir),
                current_unix_seconds(),
                current_id
            ],
        )?;

        if updated > 0 {
            if let Some(source_uid) = source_uid.as_deref() {
                tx.execute(
                    r#"
                    UPDATE library_legacy_attributions
                    SET legacy_file_id = ?1,
                        legacy_path_snapshot = ?2,
                        decision_revision = decision_revision + 1,
                        updated_at = ?3
                    WHERE legacy_file_id = ?4 AND source_uid = ?5
                      AND claim_state IN ('verified','user_confirmed')
                    "#,
                    params![
                        target_file_id,
                        normalized_target,
                        last_seen_at,
                        current_id,
                        source_uid
                    ],
                )?;
            }
            super::library::bump_library_query_revision_in_transaction(&tx)?;
        }
        tx.commit()?;
        Ok(updated > 0)
    }

    pub fn search_files(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<FileRecordDto>, DbError> {
        self.search_files_in_scope(query, limit, &LibraryScope::All)
    }

    pub fn search_files_in_scope(
        &self,
        query: &str,
        limit: Option<u32>,
        scope: &LibraryScope,
    ) -> Result<Vec<FileRecordDto>, DbError> {
        let Some(fts_query) = build_fts_query(query) else {
            return Ok(Vec::new());
        };

        let limit = i64::from(limit.unwrap_or(50).clamp(1, 200));
        let now = current_timestamp_iso();
        let conn = self.conn()?;
        let scoped = scoped_files_sql(Some(scope));
        let search = search_match_sql(&fts_query, query);
        let sql = format!(
            r#"
            WITH {},
            {},
            dup_groups AS (
                SELECT file_id, size, content_hash
                FROM active_duplicate_membership
            )
            SELECT
                f.id,
                f.path,
                f.name,
                f.extension,
                f.size,
                f.mtime,
                f.ctime,
                f.is_dir,
                f.state_code,
                f.file_type,
                f.purpose,
                f.lifecycle,
                f.context,
                f.risk_level,
                f.suggested_action,
                f.suggested_target_path,
                f.suggested_name,
                f.confidence,
                f.classification_reason,
                f.classification_status,
                f.matched_rules,
                f.requires_confirmation,
                f.content_hash,
                (dg.file_id IS NOT NULL) AS is_duplicate,
                f.is_stale,
                f.last_seen_at,
                f.last_classified_at,
                f.classified_rule_version,
                f.last_classified_mtime,
                f.last_classified_size,
                bm.rank
            FROM best_matches AS bm
            JOIN scoped_files AS f ON f.rowid = bm.rowid
            LEFT JOIN dup_groups AS dg
              ON dg.file_id = f.id
            ORDER BY bm.rank ASC, f.mtime DESC, length(f.path) ASC
            LIMIT ?
            "#,
            scoped.cte, search.cte
        );
        let mut params = scoped.params.clone();
        params.extend(search.params);
        params.push(SqlValue::Integer(limit));
        let mut stmt = conn.prepare(&sql)?;

        let rows = stmt.query_map(params_from_iter(params.iter()), indexed_file_from_row)?;

        rows.map(|row| row.map(|file| file_record_from_indexed(file, &now)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    pub fn get_paged_files(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
        query: Option<&str>,
    ) -> Result<PagedFilesResult, DbError> {
        self.get_paged_files_in_scope_with_filter(limit, offset, query, &LibraryScope::All, None)
    }

    pub fn get_paged_files_in_scope(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
        query: Option<&str>,
        scope: &LibraryScope,
    ) -> Result<PagedFilesResult, DbError> {
        self.get_paged_files_in_scope_with_filter(limit, offset, query, scope, None)
    }

    pub fn get_paged_files_in_scope_with_filter(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
        query: Option<&str>,
        scope: &LibraryScope,
        filter: Option<&FileLibraryFilter>,
    ) -> Result<PagedFilesResult, DbError> {
        let limit = limit.unwrap_or(50).clamp(1, 200);
        let offset = offset.unwrap_or(0);
        let now = current_timestamp_iso();
        let conn = self.conn()?;
        let scoped =
            scoped_files_sql_with_extra_where(Some(scope), library_filter_pre_dup_clause(filter));
        let post_join_filter = library_filter_post_dup_clause(filter);
        let post_join_where = post_join_where_clause(post_join_filter);

        if let Some((raw_query, fts_query)) =
            query.and_then(|value| build_fts_query(value).map(|fts_query| (value, fts_query)))
        {
            let search = search_match_sql(&fts_query, raw_query);
            let total_sql = if post_join_filter.is_some() {
                format!(
                    r#"
                    WITH {},
                    {},
                    dup_groups AS (
                        SELECT file_id, size, content_hash
                        FROM active_duplicate_membership
                    )
                    SELECT COUNT(*)
                    FROM best_matches AS bm
                    JOIN scoped_files AS f ON f.rowid = bm.rowid
                    LEFT JOIN dup_groups AS dg
                      ON dg.file_id = f.id
                    {}
                    "#,
                    scoped.cte,
                    search.cte,
                    post_join_where.as_str()
                )
            } else {
                format!(
                    r#"
                    WITH {},
                    {}
                    SELECT COUNT(*)
                    FROM best_matches
                    "#,
                    scoped.cte, search.cte
                )
            };
            let mut total_params = scoped.params.clone();
            total_params.extend(search.params.clone());
            maybe_print_query_plan(
                &conn,
                "get_paged_files.search.total",
                &total_sql,
                &total_params,
            )?;
            let total =
                conn.query_row(&total_sql, params_from_iter(total_params.iter()), |row| {
                    row.get(0)
                })?;
            let page_sql = format!(
                r#"
                WITH {},
                {},
                dup_groups AS (
                    SELECT file_id, size, content_hash
                    FROM active_duplicate_membership
                )
                SELECT
                    f.id,
                    f.path,
                    f.name,
                    f.extension,
                    f.size,
                    f.mtime,
                    f.ctime,
                    f.is_dir,
                    f.state_code,
                    f.file_type,
                    f.purpose,
                    f.lifecycle,
                    f.context,
                    f.risk_level,
                    f.suggested_action,
                    f.suggested_target_path,
                    f.suggested_name,
                    f.confidence,
                    f.classification_reason,
                    f.classification_status,
                    f.matched_rules,
                    f.requires_confirmation,
                    f.content_hash,
                    (dg.file_id IS NOT NULL) AS is_duplicate,
                    f.is_stale,
                    f.last_seen_at,
                    f.last_classified_at,
                    f.classified_rule_version,
                    f.last_classified_mtime,
                    f.last_classified_size,
                    bm.rank
                FROM best_matches AS bm
                JOIN scoped_files AS f ON f.rowid = bm.rowid
                LEFT JOIN dup_groups AS dg
                  ON dg.file_id = f.id
                {}
                ORDER BY bm.rank ASC, f.mtime DESC, length(f.path) ASC
                LIMIT ? OFFSET ?
                "#,
                scoped.cte,
                search.cte,
                post_join_where.as_str()
            );
            let mut page_params = scoped.params.clone();
            page_params.extend(search.params);
            page_params.push(SqlValue::Integer(i64::from(limit)));
            page_params.push(SqlValue::Integer(i64::from(offset)));
            maybe_print_query_plan(
                &conn,
                "get_paged_files.search.page",
                &page_sql,
                &page_params,
            )?;
            let mut stmt = conn.prepare(&page_sql)?;
            let rows =
                stmt.query_map(params_from_iter(page_params.iter()), indexed_file_from_row)?;
            let files = rows
                .map(|row| row.map(|file| file_record_from_indexed(file, &now)))
                .collect::<Result<Vec<_>, _>>()?;

            return Ok(PagedFilesResult {
                files,
                total,
                limit,
                offset,
            });
        }

        let total_sql = if post_join_filter.is_some() {
            format!(
                r#"
                WITH {},
                dup_groups AS (
                    SELECT file_id, size, content_hash
                    FROM active_duplicate_membership
                )
                SELECT COUNT(*)
                FROM scoped_files AS f
                LEFT JOIN dup_groups AS dg
                  ON dg.file_id = f.id
                {}
                "#,
                scoped.cte,
                post_join_where.as_str()
            )
        } else {
            format!("WITH {} SELECT COUNT(*) FROM scoped_files", scoped.cte)
        };
        maybe_print_query_plan(&conn, "get_paged_files.total", &total_sql, &scoped.params)?;
        let total = conn.query_row(&total_sql, params_from_iter(scoped.params.iter()), |row| {
            row.get(0)
        })?;
        let page_sql = format!(
            r#"
            WITH {},
            dup_groups AS (
                SELECT file_id, size, content_hash
                FROM active_duplicate_membership
            )
            SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code,
                   f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action,
                   f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason,
                   f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash,
                   (dg.file_id IS NOT NULL) AS is_duplicate,
                   f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version,
                   f.last_classified_mtime, f.last_classified_size
            FROM scoped_files AS f
            LEFT JOIN dup_groups AS dg
              ON dg.file_id = f.id
            {}
            ORDER BY f.mtime DESC, f.name COLLATE NOCASE ASC
            LIMIT ? OFFSET ?
            "#,
            scoped.cte,
            post_join_where.as_str()
        );
        let mut page_params = scoped.params.clone();
        page_params.push(SqlValue::Integer(i64::from(limit)));
        page_params.push(SqlValue::Integer(i64::from(offset)));
        maybe_print_query_plan(&conn, "get_paged_files.page", &page_sql, &page_params)?;
        let mut stmt = conn.prepare(&page_sql)?;
        let rows = stmt.query_map(params_from_iter(page_params.iter()), |row| {
            indexed_file_from_row(row)
        })?;
        let files = rows
            .map(|row| row.map(|file| file_record_from_indexed(file, &now)))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(PagedFilesResult {
            files,
            total,
            limit,
            offset,
        })
    }

    #[cfg(test)]
    pub(crate) fn explain_paged_files_query_plan(
        &self,
        query: Option<&str>,
        scope: &LibraryScope,
        filter: Option<&FileLibraryFilter>,
    ) -> Result<Vec<String>, DbError> {
        let conn = self.conn()?;
        let scoped =
            scoped_files_sql_with_extra_where(Some(scope), library_filter_pre_dup_clause(filter));
        let post_join_filter = library_filter_post_dup_clause(filter);
        let post_join_where = post_join_where_clause(post_join_filter);
        let duplicate_cte = duplicate_filter_cte(post_join_filter);
        let duplicate_join = duplicate_filter_join(post_join_filter);
        if let Some((raw_query, fts_query)) =
            query.and_then(|value| build_fts_query(value).map(|fts_query| (value, fts_query)))
        {
            let search = search_match_sql(&fts_query, raw_query);
            let page_sql = format!(
                r#"
                WITH {},
                {}
                {}
                SELECT f.id
                FROM best_matches AS bm
                JOIN scoped_files AS f ON f.rowid = bm.rowid
                {}
                {}
                ORDER BY bm.rank ASC, f.mtime DESC, length(f.path) ASC
                LIMIT ? OFFSET ?
                "#,
                scoped.cte,
                search.cte,
                duplicate_cte,
                duplicate_join,
                post_join_where.as_str()
            );
            let mut params = scoped.params.clone();
            params.extend(search.params);
            params.push(SqlValue::Integer(50));
            params.push(SqlValue::Integer(0));
            return explain_query_plan(&conn, &page_sql, &params);
        }

        let page_sql = format!(
            r#"
            WITH {}
            SELECT f.id
            FROM scoped_files AS f
            {}
            {}
            ORDER BY f.mtime DESC, f.name COLLATE NOCASE ASC
            LIMIT ? OFFSET ?
            "#,
            if post_join_filter.is_some() {
                format!("{}{}", scoped.cte, duplicate_cte)
            } else {
                scoped.cte
            },
            duplicate_join,
            post_join_where.as_str()
        );
        let mut params = scoped.params.clone();
        params.push(SqlValue::Integer(50));
        params.push(SqlValue::Integer(0));
        explain_query_plan(&conn, &page_sql, &params)
    }

    pub fn get_operation_previews_for_scope(
        &self,
        scope: &LibraryScope,
        filter: Option<&FileLibraryFilter>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<OperationPreviewScopeResult, DbError> {
        let limit = limit.unwrap_or(1000).clamp(1, 2000);
        let offset = offset.unwrap_or(0);
        let extra_where = operation_preview_filter_clause(filter);
        let scoped = scoped_files_sql_with_extra_where(Some(scope), Some(&extra_where));
        let conn = self.conn()?;

        let total_sql = format!("WITH {} SELECT COUNT(*) FROM scoped_files", scoped.cte);
        let total = conn.query_row(&total_sql, params_from_iter(scoped.params.iter()), |row| {
            row.get::<_, i64>(0)
        })?;
        let page_sql = format!(
            r#"
            WITH {},
            dup_groups AS (
                SELECT file_id, size, content_hash
                FROM active_duplicate_membership
            )
            SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code,
                   f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action,
                   f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason,
                   f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash,
                   (dg.file_id IS NOT NULL) AS is_duplicate,
                   f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version,
                   f.last_classified_mtime, f.last_classified_size
            FROM scoped_files AS f
            LEFT JOIN dup_groups AS dg
              ON dg.file_id = f.id
            ORDER BY f.mtime DESC, f.name COLLATE NOCASE ASC
            LIMIT ? OFFSET ?
            "#,
            scoped.cte
        );
        let mut page_params = scoped.params.clone();
        page_params.push(SqlValue::Integer(i64::from(limit)));
        page_params.push(SqlValue::Integer(i64::from(offset)));
        let mut stmt = conn.prepare(&page_sql)?;
        let rows = stmt.query_map(params_from_iter(page_params.iter()), indexed_file_from_row)?;
        let previews = rows
            .map(|row| row.map(operation_preview_from_indexed))
            .filter_map(|row| match row {
                Ok(Some(preview)) => Some(Ok(preview)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()?;

        let has_more = i64::from(offset) + (previews.len() as i64) < total;
        Ok(OperationPreviewScopeResult {
            previews,
            total,
            limit,
            offset,
            truncated: has_more,
            has_more,
        })
    }

    pub fn get_operation_previews_by_file_ids(
        &self,
        file_ids: &[String],
    ) -> Result<Vec<OperationPreviewDto>, DbError> {
        Ok(self
            .get_indexed_file_rows_by_ids(file_ids)?
            .into_iter()
            .filter_map(operation_preview_from_indexed)
            .collect())
    }

    /// A user-specified basename is an intent, never a renderer-specified path.
    /// Rebuilt at execute admission so the existing fingerprint binds consent.
    pub fn get_manual_rename_operation_preview(
        &self,
        file_id: &str,
        new_name: &str,
    ) -> Result<OperationPreviewDto, DbError> {
        let invalid = DbError::Validation;
        let mut row = self
            .get_indexed_file_rows_by_ids(&[file_id.to_string()])?
            .into_iter()
            .next()
            .ok_or_else(|| invalid("manual_rename_source_unavailable".to_string()))?;
        self.verify_indexed_file_identity(file_id)?;
        let source = Path::new(&row.path);
        let metadata = fs::symlink_metadata(source)?;
        if row.is_dir || !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("manual_rename_regular_file_required".to_string()));
        }
        if metadata.permissions().readonly() {
            return Err(invalid("manual_rename_source_read_only".to_string()));
        }
        crate::file_ops::ensure_general_file_operation_allowed(source).map_err(invalid)?;
        crate::file_ops::file_namespace_fingerprint(source).map_err(invalid)?;
        if new_name.trim().is_empty() {
            return Err(invalid("manual_rename_name_required".to_string()));
        }
        let name = normalize_proposed_file_name(
            &row.name,
            &row.extension,
            new_name,
            false,
            ExtensionChangePolicy::Preserve,
        )
        .map_err(invalid)?;
        crate::file_ops::validate_safe_file_name(&name).map_err(invalid)?;
        row.suggested_action = "Rename".to_string();
        row.suggested_target_path = parent_directory(&row.path);
        row.suggested_name = name;
        row.requires_confirmation = true;
        row.classification_reason = "Manual rename requested by the user.".to_string();
        let mut preview = operation_preview_from_indexed_with_intent(row, Some("manual-rename"))
            .ok_or_else(|| invalid("manual_rename_name_unchanged".to_string()))?;
        preview.editable_new_name = Some(false);
        Ok(preview)
    }

    pub fn get_permanent_delete_operation_preview(
        &self,
        file_id: &str,
    ) -> Result<OperationPreviewDto, DbError> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = file_id;
            Err(DbError::Validation(
                "permanent_delete_unsupported".to_string(),
            ))
        }

        #[cfg(target_os = "macos")]
        {
            self.get_permanent_delete_operation_preview_if_available(file_id)?
                .ok_or_else(|| {
                    DbError::Validation("permanent_delete_preview_unavailable".to_string())
                })
        }
    }

    pub(crate) fn get_permanent_delete_operation_preview_if_available(
        &self,
        file_id: &str,
    ) -> Result<Option<OperationPreviewDto>, DbError> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = file_id;
            Ok(None)
        }

        #[cfg(target_os = "macos")]
        {
            let file_ids = [file_id.to_string()];
            Ok(self
                .get_indexed_file_rows_by_ids(&file_ids)?
                .into_iter()
                .next()
                .and_then(permanent_delete_preview_from_indexed))
        }
    }

    fn get_indexed_file_rows_by_ids(
        &self,
        file_ids: &[String],
    ) -> Result<Vec<IndexedFileRow>, DbError> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code,
                   f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action,
                   f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason,
                   f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash,
                   EXISTS (
                       SELECT 1 FROM active_duplicate_membership AS membership
                       WHERE membership.file_id = f.id
                   ) AS is_duplicate,
                   f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version,
                   f.last_classified_mtime, f.last_classified_size
            FROM files AS f
            WHERE f.id = ?1 AND f.is_stale = 0
            "#,
        )?;
        let mut rows = Vec::with_capacity(file_ids.len());
        for file_id in file_ids {
            let mut result = stmt.query_map(params![file_id], indexed_file_from_row)?;
            if let Some(row) = result.next() {
                rows.push(row?);
            }
        }
        Ok(rows)
    }

    pub fn get_operation_previews_for_selection(
        &self,
        selection: &LibrarySelectionV1,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<OperationPreviewScopeResult, DbError> {
        let limit = limit.unwrap_or(1000).clamp(1, 2000);
        let offset = offset.unwrap_or(0);
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let current_revision = current_library_revision(&tx)?;
        let (where_sql, selection_params, _, _, _) =
            selection_where(&tx, selection, current_revision)?;
        let total_sql = format!("SELECT COUNT(*) FROM files AS f WHERE {where_sql}");
        let total = tx.query_row(
            &total_sql,
            params_from_iter(selection_params.iter()),
            |row| row.get::<_, i64>(0),
        )?;
        let page_sql = format!(
            r#"
            WITH dup_groups AS (
                SELECT file_id, size, content_hash
                FROM active_duplicate_membership
            )
            SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code,
                   f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action,
                   f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason,
                   f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash,
                   (dg.file_id IS NOT NULL) AS is_duplicate,
                   f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version,
                   f.last_classified_mtime, f.last_classified_size
            FROM files AS f
            LEFT JOIN dup_groups AS dg
              ON dg.file_id = f.id
            WHERE {where_sql}
            ORDER BY f.mtime DESC, f.name COLLATE NOCASE ASC
            LIMIT ? OFFSET ?
            "#
        );
        let mut page_params = selection_params;
        page_params.push(SqlValue::Integer(i64::from(limit)));
        page_params.push(SqlValue::Integer(i64::from(offset)));
        let previews = {
            let mut stmt = tx.prepare(&page_sql)?;
            let rows =
                stmt.query_map(params_from_iter(page_params.iter()), indexed_file_from_row)?;
            rows.map(|row| row.map(operation_preview_from_indexed))
                .filter_map(|row| match row {
                    Ok(Some(preview)) => Some(Ok(preview)),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        clear_temp_selection_ids(&tx)?;
        tx.commit()?;
        let has_more = i64::from(offset) + (previews.len() as i64) < total;
        Ok(OperationPreviewScopeResult {
            previews,
            total,
            limit,
            offset,
            truncated: has_more,
            has_more,
        })
    }

    pub fn get_stats_summary(&self) -> Result<StatsSummary, DbError> {
        self.get_stats_summary_in_scope(&LibraryScope::All)
    }

    pub fn get_stats_summary_in_scope(
        &self,
        scope: &LibraryScope,
    ) -> Result<StatsSummary, DbError> {
        let mut conn = self.conn()?;
        let scoped = scoped_files_sql(Some(scope));
        // 一次事务内完成所有聚合，保证快照一致性
        let tx = conn.transaction()?;
        let totals_sql = format!(
            r#"
            WITH {},
            dup_groups AS (
                SELECT file_id, size, content_hash
                FROM active_duplicate_membership
            )
            SELECT
                COUNT(*)        FILTER (WHERE f.is_dir = 0),
                COALESCE(SUM(f.size) FILTER (WHERE f.is_dir = 0), 0),
                COUNT(*)        FILTER (WHERE f.is_dir = 0 AND f.size >= 104857600),
                COUNT(*)        FILTER (WHERE f.is_dir = 0
                                  AND (f.risk_level = 'Sensitive' OR f.lifecycle = 'Sensitive')),
                COUNT(*)        FILTER (WHERE f.is_dir = 0 AND f.requires_confirmation = 1),
                COUNT(*)        FILTER (WHERE f.is_dir = 0 AND dg.file_id IS NOT NULL),
                MAX(f.mtime)
            FROM scoped_files AS f
            LEFT JOIN dup_groups AS dg
              ON dg.file_id = f.id
            "#,
            scoped.cte
        );
        let (
            total_files,
            total_size,
            large_files,
            sensitive_files,
            needs_confirmation,
            duplicate_files,
            last_mtime,
        ): (i64, i64, i64, i64, i64, i64, Option<i64>) =
            tx.query_row(&totals_sql, params_from_iter(scoped.params.iter()), |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })?;
        let mut by_type = HashMap::new();
        let type_sql = format!(
            r#"
            WITH {}
            SELECT file_type, extension, is_dir, COUNT(*)
            FROM scoped_files
            GROUP BY file_type, extension, is_dir
            "#,
            scoped.cte
        );
        let mut stmt = tx.prepare(&type_sql)?;
        let type_rows = stmt.query_map(params_from_iter(scoped.params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for row in type_rows {
            let (file_type, extension, is_dir, count) = row?;
            let normalized_type = if file_type.is_empty() || file_type == "Other" {
                infer_file_type(&extension, is_dir).to_string()
            } else {
                file_type
            };
            *by_type.entry(normalized_type).or_insert(0) += count;
        }
        drop(stmt);
        let mut by_lifecycle = HashMap::new();
        let lifecycle_sql = format!(
            r#"
            WITH {}
            SELECT lifecycle, COUNT(*)
            FROM scoped_files
            WHERE is_dir = 0
            GROUP BY lifecycle
            "#,
            scoped.cte
        );
        let mut stmt = tx.prepare(&lifecycle_sql)?;
        let lifecycle_rows = stmt.query_map(params_from_iter(scoped.params.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in lifecycle_rows {
            let (lifecycle, count) = row?;
            by_lifecycle.insert(lifecycle, count);
        }
        drop(stmt);
        tx.commit()?;
        let disks = Disks::new_with_refreshed_list();
        let (disk_total, disk_free) = disks
            .iter()
            .map(|d| (d.total_space(), d.available_space()))
            .max_by_key(|(total, _)| *total)
            .unwrap_or((0, 0));
        let disk_usage_ratio = if disk_total > 0 {
            1.0 - (disk_free as f64 / disk_total as f64)
        } else {
            0.0
        };

        Ok(StatsSummary {
            total_files,
            total_size,
            disk_total_size: disk_total as i64,
            disk_free_size: disk_free as i64,
            disk_usage_ratio,
            duplicate_files,
            large_files,
            sensitive_files,
            needs_confirmation,
            by_type,
            by_lifecycle,
            last_scanned_at: last_mtime.map(unix_seconds_to_iso),
        })
    }
}

fn maybe_print_query_plan(
    conn: &Connection,
    label: &str,
    sql: &str,
    params: &[SqlValue],
) -> Result<(), DbError> {
    if !matches!(
        env::var("ZC_BENCH_EXPLAIN").as_deref(),
        Ok("1" | "true" | "TRUE" | "yes" | "YES")
    ) {
        return Ok(());
    }

    let plan = explain_query_plan(conn, sql, params)?;
    for line in plan {
        eprintln!("[ZC_BENCH_EXPLAIN] {label}: {line}");
    }
    Ok(())
}

fn explain_query_plan(
    conn: &Connection,
    sql: &str,
    params: &[SqlValue],
) -> Result<Vec<String>, DbError> {
    let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
    let mut stmt = conn.prepare(&explain_sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok(format!(
            "id={} parent={} not_used={} detail={}",
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?
        ))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
}

fn operation_preview_filter_clause(filter: Option<&FileLibraryFilter>) -> String {
    let action_clause =
        "f.is_dir = 0 AND f.suggested_action IN ('Move', 'Rename', 'MoveAndRename', 'Archive')";
    match library_filter_pre_dup_clause(filter) {
        Some(library_clause) => format!("{action_clause} AND ({library_clause})"),
        None => action_clause.to_string(),
    }
}

fn library_filter_pre_dup_clause(filter: Option<&FileLibraryFilter>) -> Option<&'static str> {
    match filter.and_then(|filter| filter.library_filter.as_ref()) {
        None | Some(LibraryFilter::All) => None,
        Some(LibraryFilter::Active) => {
            Some("f.lifecycle IN ('Active', 'Reference') OR f.suggested_action = 'Keep'")
        }
        Some(LibraryFilter::Archive) => Some("f.lifecycle = 'Archive'"),
        Some(LibraryFilter::Review) => Some(
            "f.requires_confirmation = 1 OR f.suggested_action IN ('Review', 'DeleteCandidate')",
        ),
        Some(LibraryFilter::Duplicate) => None,
        Some(LibraryFilter::Sensitive) => {
            Some("f.risk_level = 'Sensitive' OR f.lifecycle = 'Sensitive'")
        }
    }
}

fn library_filter_post_dup_clause(filter: Option<&FileLibraryFilter>) -> Option<&'static str> {
    match filter.and_then(|filter| filter.library_filter.as_ref()) {
        Some(LibraryFilter::Duplicate) => Some("dg.file_id IS NOT NULL"),
        _ => None,
    }
}

#[cfg(test)]
fn duplicate_filter_cte(clause: Option<&str>) -> &'static str {
    if clause.is_none() {
        return "";
    }

    r#",
            dup_groups AS (
                SELECT file_id, size, content_hash
                FROM active_duplicate_membership
            )"#
}

#[cfg(test)]
fn duplicate_filter_join(clause: Option<&str>) -> &'static str {
    if clause.is_none() {
        return "";
    }

    r#"
            LEFT JOIN dup_groups AS dg
              ON dg.file_id = f.id"#
}

fn post_join_where_clause(clause: Option<&str>) -> String {
    clause
        .map(|clause| format!("WHERE ({clause})"))
        .unwrap_or_default()
}

pub(crate) fn operation_preview_from_indexed(row: IndexedFileRow) -> Option<OperationPreviewDto> {
    operation_preview_from_indexed_with_intent(row, None)
}

fn operation_preview_from_indexed_with_intent(
    row: IndexedFileRow,
    intent: Option<&str>,
) -> Option<OperationPreviewDto> {
    let source_directory = parent_directory(&row.path);
    let proposed_name = if row.suggested_name.trim().is_empty() {
        row.name.clone()
    } else {
        row.suggested_name.clone()
    };
    let mut extension_blocking_reason = None;
    let mut new_name = match normalize_proposed_file_name(
        &row.name,
        &row.extension,
        &proposed_name,
        row.is_dir,
        ExtensionChangePolicy::Preserve,
    ) {
        Ok(name) => name,
        Err(error) => {
            extension_blocking_reason = Some(error);
            row.name.clone()
        }
    };
    let mut target_directory = match row.suggested_action.as_str() {
        "Rename" => {
            if row.suggested_target_path.trim().is_empty() {
                source_directory.clone()
            } else {
                row.suggested_target_path.clone()
            }
        }
        "Move" | "MoveAndRename" | "Archive" => row.suggested_target_path.clone(),
        "Copy" | "Duplicate" | "Replace" => row.suggested_target_path.clone(),
        _ => String::new(),
    };
    if let Some((parent, file_name)) = intent
        .is_none()
        .then(|| split_filename_from_target_directory(&target_directory, &row.extension))
        .flatten()
    {
        target_directory = parent;
        if row.suggested_name.trim().is_empty() || row.suggested_name == row.name {
            new_name = file_name;
        }
    }
    if extension_blocking_reason.is_none() {
        match normalize_proposed_file_name(
            &row.name,
            &row.extension,
            &new_name,
            row.is_dir,
            ExtensionChangePolicy::Preserve,
        ) {
            Ok(normalized_name) => new_name = normalized_name,
            Err(error) => {
                extension_blocking_reason = Some(error);
                new_name = row.name.clone();
            }
        }
    }
    let target_path = if target_directory.trim().is_empty() {
        row.path.clone()
    } else {
        join_path_text(&target_directory, &new_name)
    };
    if extension_blocking_reason.is_none()
        && normalize_path_for_compare_text(&row.path)
            == normalize_path_for_compare_text(&target_path)
    {
        return None;
    }

    let is_move = !target_directory.trim().is_empty()
        && normalize_path_for_compare_text(&target_directory)
            != normalize_path_for_compare_text(&source_directory);
    let is_rename = new_name != row.name;
    let operation_type = match row.suggested_action.as_str() {
        "Copy" => "copy",
        "Duplicate" => "duplicate",
        "Replace" => "replace",
        _ if is_move && is_rename => "move_rename",
        _ if is_move => "move",
        _ => "rename",
    };
    let target_exists = Path::new(&target_path).symlink_metadata().is_ok();
    let target_parent_exists = Path::new(&target_path)
        .parent()
        .map(|parent| parent.exists())
        .unwrap_or(false);
    let semantics = operation_preview_semantics(
        operation_type,
        Path::new(&row.path),
        Path::new(&target_path),
    );
    let preview_id = intent.map_or_else(
        || operation_preview_id(&row.id),
        |intent| operation_preview_id_for_intent(&row.id, intent),
    );
    let source_identity_fingerprint = operation_source_identity_fingerprint(Path::new(&row.path));
    let provider_identity_fingerprint =
        operation_provider_identity_fingerprint(Path::new(&row.path));
    let is_sensitive = row.risk_level == "Sensitive";
    let extension_blocked = extension_blocking_reason.is_some();
    let replace_operation = operation_type == "replace";
    let requires_confirmation = row.requires_confirmation
        || row.confidence < 0.7
        || is_sensitive
        || extension_blocked
        || replace_operation
        || semantics.runtime_blocking_reason.is_some();
    let is_executable = !is_sensitive
        && !extension_blocked
        && semantics.runtime_blocking_reason.is_none()
        && ((!target_exists && !replace_operation) || (target_exists && replace_operation));
    let blocking_reason = extension_blocking_reason
        .clone()
        .or_else(|| {
            is_sensitive.then(|| "Sensitive files require manual confirmation.".to_string())
        })
        .or_else(|| {
            (target_exists && !replace_operation).then(|| {
                "Target path already exists; Zen Canvas will not overwrite it.".to_string()
            })
        })
        .or_else(|| semantics.runtime_blocking_reason.map(str::to_string));
    let operation_fingerprint = operation_preview_fingerprint(
        &preview_id,
        &row.id,
        operation_type,
        &row.path,
        &target_path,
        &row.risk_level,
        requires_confirmation,
        is_executable,
        !target_parent_exists,
        blocking_reason.as_deref(),
        semantics,
        source_identity_fingerprint.as_deref(),
        provider_identity_fingerprint.as_deref(),
    );

    Some(OperationPreviewDto {
        id: preview_id,
        file_id: row.id,
        operation_type: operation_type.to_string(),
        source_path: row.path,
        target_path,
        old_name: row.name,
        new_name,
        status: "pending".to_string(),
        risk_level: row.risk_level,
        confidence: row.confidence,
        requires_confirmation,
        suggested_action: row.suggested_action,
        is_duplicate: row.is_duplicate,
        reason: row.classification_reason,
        selected_by_default: Some(is_executable && !requires_confirmation),
        is_executable: Some(is_executable),
        blocking_reason,
        editable_new_name: Some(!extension_blocked),
        target_parent_exists: Some(target_parent_exists),
        will_create_parent: Some(!target_parent_exists),
        strategy: semantics.strategy.map(str::to_string),
        conflict_policy: Some(semantics.conflict_policy.to_string()),
        will_copy: Some(semantics.will_copy),
        will_move: Some(semantics.will_move),
        will_download: Some(semantics.will_download),
        materialization_requirement: Some(
            semantics.materialization_requirement.as_str().to_string(),
        ),
        materialization_requirement_v2: Some(
            semantics.materialization_requirement.as_str().to_string(),
        ),
        operation_fingerprint: Some(operation_fingerprint),
        cross_volume_copy_required: Some(semantics.cross_volume_copy_required),
        metadata_degradation_possible: Some(semantics.metadata_degradation_possible),
        source_retirement_capability: Some(semantics.source_retirement_capability.to_string()),
        source_retirement_eligible: Some(semantics.source_retirement_eligible),
        source_retirement_probe_required: Some(semantics.source_retirement_probe_required),
        provider_coordination: Some(semantics.provider_coordination),
        source_identity_fingerprint,
        provider_identity_fingerprint,
        will_replace: Some(semantics.will_replace),
        will_trash: Some(semantics.will_trash),
    })
}

#[cfg(target_os = "macos")]
fn permanent_delete_preview_from_indexed(row: IndexedFileRow) -> Option<OperationPreviewDto> {
    if row.is_stale {
        return None;
    }

    let source = Path::new(&row.path);
    let source_available = source.symlink_metadata().is_ok();
    let cleanup_execution_forbidden =
        source_available && crate::storage_analyzer::is_cleanup_execution_forbidden(source, None);
    let source_identity_fingerprint = operation_source_identity_fingerprint(source);
    let provider_identity_fingerprint = operation_provider_identity_fingerprint(source);
    let capability = crate::platform::macos::strategy::source_retirement_capability(source);
    let capability_label = match capability.strategy {
        crate::platform::macos::strategy::MacSourceRetirementStrategy::ExclusiveClaim => {
            "exclusive_claim"
        }
        crate::platform::macos::strategy::MacSourceRetirementStrategy::ProviderCoordinated => {
            "provider_coordinated"
        }
        crate::platform::macos::strategy::MacSourceRetirementStrategy::PortableNamespaceRetirement => {
            "portable_namespace_retirement"
        }
    };
    let base_semantics = operation_preview_semantics("move_to_trash", source, source);
    let semantics = OperationPreviewSemantics {
        strategy: base_semantics.strategy,
        conflict_policy: "permanent_delete_quarantine",
        runtime_blocking_reason: base_semantics.runtime_blocking_reason,
        will_copy: false,
        will_move: true,
        will_download: false,
        materialization_requirement: MaterializationRequirement::None,
        cross_volume_copy_required: false,
        metadata_degradation_possible: false,
        source_retirement_capability: capability_label,
        source_retirement_eligible: capability.eligible,
        source_retirement_probe_required: false,
        provider_coordination: base_semantics.provider_coordination,
        will_replace: false,
        will_trash: false,
    };
    let blocking_reason = if !source_available {
        Some("Permanent Delete source is no longer available.".to_string())
    } else if source_identity_fingerprint.is_none() {
        Some("Permanent Delete requires a verifiable source identity.".to_string())
    } else if cleanup_execution_forbidden {
        Some("Permanent Delete is not allowed for this source.".to_string())
    } else if !capability.eligible {
        Some(
            capability
                .reason
                .unwrap_or("Permanent Delete requires an eligible source retirement capability.")
                .to_string(),
        )
    } else {
        base_semantics.runtime_blocking_reason.map(str::to_string)
    };
    let is_executable =
        blocking_reason.is_none() && base_semantics.runtime_blocking_reason.is_none();
    let preview_id = operation_preview_id_for_intent(&row.id, "permanent-delete");
    let target_path = "Permanent deletion quarantine".to_string();
    let operation_fingerprint = operation_preview_fingerprint(
        &preview_id,
        &row.id,
        "permanent_delete",
        &row.path,
        &target_path,
        "Sensitive",
        true,
        is_executable,
        false,
        blocking_reason.as_deref(),
        semantics,
        source_identity_fingerprint.as_deref(),
        provider_identity_fingerprint.as_deref(),
    );

    Some(OperationPreviewDto {
        id: preview_id,
        file_id: row.id,
        operation_type: "permanent_delete".to_string(),
        source_path: row.path,
        target_path,
        old_name: row.name.clone(),
        new_name: row.name,
        status: "pending".to_string(),
        risk_level: "Sensitive".to_string(),
        confidence: 1.0,
        requires_confirmation: true,
        suggested_action: "DeleteCandidate".to_string(),
        is_duplicate: row.is_duplicate,
        reason:
            "Permanent Delete requires explicit confirmation and backend identity revalidation."
                .to_string(),
        selected_by_default: Some(false),
        is_executable: Some(is_executable),
        blocking_reason,
        editable_new_name: Some(false),
        target_parent_exists: Some(true),
        will_create_parent: Some(false),
        strategy: semantics.strategy.map(str::to_string),
        conflict_policy: Some(semantics.conflict_policy.to_string()),
        will_copy: Some(false),
        will_move: Some(true),
        will_download: Some(false),
        materialization_requirement: Some(MaterializationRequirement::None.as_str().to_string()),
        materialization_requirement_v2: Some(MaterializationRequirement::None.as_str().to_string()),
        operation_fingerprint: Some(operation_fingerprint),
        cross_volume_copy_required: Some(false),
        metadata_degradation_possible: Some(false),
        source_retirement_capability: Some(semantics.source_retirement_capability.to_string()),
        source_retirement_eligible: Some(semantics.source_retirement_eligible),
        source_retirement_probe_required: Some(false),
        provider_coordination: Some(semantics.provider_coordination),
        source_identity_fingerprint,
        provider_identity_fingerprint,
        will_replace: Some(false),
        will_trash: Some(false),
    })
}

#[derive(Debug, Clone, Copy)]
struct OperationPreviewSemantics {
    strategy: Option<&'static str>,
    conflict_policy: &'static str,
    runtime_blocking_reason: Option<&'static str>,
    will_copy: bool,
    will_move: bool,
    will_download: bool,
    materialization_requirement: MaterializationRequirement,
    cross_volume_copy_required: bool,
    metadata_degradation_possible: bool,
    source_retirement_capability: &'static str,
    source_retirement_eligible: bool,
    source_retirement_probe_required: bool,
    provider_coordination: bool,
    will_replace: bool,
    will_trash: bool,
}

fn operation_preview_semantics(
    operation_type: &str,
    source: &Path,
    target: &Path,
) -> OperationPreviewSemantics {
    #[cfg(not(target_os = "macos"))]
    let _ = source;
    #[cfg(target_os = "macos")]
    let strategy =
        crate::platform::macos::strategy::select(source, target.parent().unwrap_or(target));
    #[cfg(target_os = "macos")]
    let strategy_label = Some(strategy.label());
    #[cfg(not(target_os = "macos"))]
    let strategy_label = None;

    let target_exists = target.symlink_metadata().is_ok();
    let will_replace = operation_type == "replace";
    let will_trash = matches!(operation_type, "move_to_trash" | "replace");
    let will_copy = matches!(operation_type, "copy" | "duplicate" | "replace")
        || (cfg!(target_os = "macos")
            && matches!(
                strategy_label,
                Some("cross_volume_copy_verify" | "local_portable" | "network_portable")
            ));
    let will_move = matches!(
        operation_type,
        "move" | "rename" | "move_rename" | "move_to_trash"
    );
    let materialization_requirement = if cfg!(target_os = "macos") {
        match strategy_label {
            Some("icloud_coordinated")
                if matches!(operation_type, "copy" | "duplicate" | "replace") =>
            {
                match crate::platform::macos::cloud_item::inspect(source).content_availability {
                    crate::platform::macos::types::MacContentAvailability::Local => {
                        MaterializationRequirement::None
                    }
                    crate::platform::macos::types::MacContentAvailability::NotLocal
                    | crate::platform::macos::types::MacContentAvailability::Downloading => {
                        MaterializationRequirement::ExplicitDownloadRequired
                    }
                    crate::platform::macos::types::MacContentAvailability::MetadataOnly
                    | crate::platform::macos::types::MacContentAvailability::BoundaryReadable
                    | crate::platform::macos::types::MacContentAvailability::Unknown => {
                        MaterializationRequirement::Unknown
                    }
                }
            }
            Some("icloud_coordinated") => MaterializationRequirement::MetadataOnly,
            Some("file_provider_coordinated")
                if matches!(operation_type, "copy" | "duplicate" | "replace") =>
            {
                let provider = crate::platform::macos::file_provider::inspect(source);
                match provider.content_availability {
                    crate::platform::macos::types::MacContentAvailability::Local
                    | crate::platform::macos::types::MacContentAvailability::BoundaryReadable => {
                        MaterializationRequirement::ProviderManaged
                    }
                    crate::platform::macos::types::MacContentAvailability::NotLocal
                    | crate::platform::macos::types::MacContentAvailability::Downloading
                    | crate::platform::macos::types::MacContentAvailability::MetadataOnly
                    | crate::platform::macos::types::MacContentAvailability::Unknown
                        if matches!(
                            provider.detection,
                            crate::platform::macos::file_provider::MacFileProviderDetection::CloudStorageNamespaceHint
                        ) => MaterializationRequirement::ExplicitDownloadRequired,
                    _ => MaterializationRequirement::Unknown,
                }
            }
            Some("file_provider_coordinated") => MaterializationRequirement::MetadataOnly,
            _ => MaterializationRequirement::None,
        }
    } else {
        MaterializationRequirement::None
    };
    #[cfg(target_os = "macos")]
    let (
        source_retirement_capability,
        source_retirement_eligible,
        source_retirement_probe_required,
    ) = {
        let capability = crate::platform::macos::strategy::source_retirement_capability(source);
        let label = match capability.strategy {
            crate::platform::macos::strategy::MacSourceRetirementStrategy::ExclusiveClaim => {
                "exclusive_claim"
            }
            crate::platform::macos::strategy::MacSourceRetirementStrategy::ProviderCoordinated => {
                "provider_coordinated"
            }
            crate::platform::macos::strategy::MacSourceRetirementStrategy::PortableNamespaceRetirement => {
                "portable_namespace_retirement"
            }
        };
        let probe_required = !capability.eligible
            && matches!(
                strategy_label,
                Some("cross_volume_copy_verify" | "local_portable" | "network_portable")
            )
            && crate::platform::macos::strategy::source_retirement_probe_required(source);
        (label, capability.eligible, probe_required)
    };
    #[cfg(not(target_os = "macos"))]
    let (
        source_retirement_capability,
        source_retirement_eligible,
        source_retirement_probe_required,
    ) = ("not_applicable", true, false);
    let runtime_blocking_reason = if cfg!(target_os = "macos") {
        if matches!(
            materialization_requirement,
            MaterializationRequirement::ExplicitDownloadRequired
        ) {
            Some("Materialization is required; explicitly download the source before continuing.")
        } else if matches!(
            materialization_requirement,
            MaterializationRequirement::Unknown
        ) {
            Some("Cloud content availability is unknown; review before execution.")
        } else if matches!(
            operation_type,
            "move" | "rename" | "move_rename" | "move_to_trash"
        ) && !source_retirement_eligible
            && !source_retirement_probe_required
        {
            Some("This filesystem cannot yet prove safe source retirement for this move.")
        } else {
            None
        }
    } else {
        None
    };
    // The mutation backend never starts a cloud download implicitly.  A
    // required materialization is surfaced as a precondition, so this flag
    // remains false until an explicit user-facing download action exists.
    let will_download = false;
    let cross_volume_copy_required = matches!(
        strategy_label,
        Some("cross_volume_copy_verify" | "local_portable" | "network_portable")
    );
    let metadata_degradation_possible = cross_volume_copy_required;
    let provider_coordination = matches!(
        strategy_label,
        Some("icloud_coordinated" | "file_provider_coordinated")
    );
    let conflict_policy = if will_replace {
        "replace_with_recovery_backup"
    } else if target_exists {
        "no_overwrite"
    } else if operation_type == "move_to_trash" {
        "safe_trash_recoverable"
    } else {
        "exclusive_target"
    };

    OperationPreviewSemantics {
        strategy: strategy_label,
        conflict_policy,
        runtime_blocking_reason,
        will_copy,
        will_move,
        will_download,
        materialization_requirement,
        cross_volume_copy_required,
        metadata_degradation_possible,
        source_retirement_capability,
        source_retirement_eligible,
        source_retirement_probe_required,
        provider_coordination,
        will_replace,
        will_trash,
    }
}

#[allow(clippy::too_many_arguments)]
fn operation_preview_fingerprint(
    preview_id: &str,
    file_id: &str,
    operation_type: &str,
    source: &str,
    target: &str,
    risk_level: &str,
    requires_confirmation: bool,
    is_executable: bool,
    will_create_parent: bool,
    blocking_reason: Option<&str>,
    semantics: OperationPreviewSemantics,
    source_identity_fingerprint: Option<&str>,
    provider_identity_fingerprint: Option<&str>,
) -> String {
    let payload = format!(
        "operation-preview-fingerprint:v2\0preview_id={preview_id}\0file_id={file_id}\0operation_type={operation_type}\0source={source}\0target={target}\0risk_level={risk_level}\0requires_confirmation={requires_confirmation}\0is_executable={is_executable}\0will_create_parent={will_create_parent}\0blocking_reason={}\0semantics={:?}\0source_identity={}\0provider_identity={}",
        blocking_reason.unwrap_or("none"),
        semantics,
        source_identity_fingerprint.unwrap_or("identity-unavailable"),
        provider_identity_fingerprint.unwrap_or("provider-identity-unavailable")
    );
    blake3::hash(payload.as_bytes()).to_hex().to_string()
}

fn operation_source_identity_fingerprint(source: &Path) -> Option<String> {
    let identity = crate::file_ops::file_namespace_fingerprint(source).ok()?;
    let payload = format!(
        "namespace\0{}\0{}\0{}\0{}",
        identity.size,
        identity.modified_ns.unwrap_or_default(),
        identity.platform_volume_id.as_deref().unwrap_or_default(),
        identity.platform_file_id.as_deref().unwrap_or_default(),
    );
    Some(blake3::hash(payload.as_bytes()).to_hex().to_string())
}

fn operation_provider_identity_fingerprint(source: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let probe = crate::platform::macos::file_provider::inspect(source);
        if !matches!(
            probe.domain_state,
            crate::platform::macos::file_provider::FileProviderDomainState::KnownDomain
        ) {
            return None;
        }
        crate::platform::macos::file_provider::coordinated_user_visible_url_for_execution(source)
            .ok()
            .and_then(|evidence| evidence.fingerprint())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = source;
        None
    }
}

fn operation_preview_id(file_id: &str) -> String {
    let digest = blake3::hash(file_id.as_bytes()).to_hex().to_string();
    format!("op-{}", &digest[..16])
}

fn operation_preview_id_for_intent(file_id: &str, intent: &str) -> String {
    let digest = blake3::hash(format!("operation-preview-id:v1\0{intent}\0{file_id}").as_bytes())
        .to_hex()
        .to_string();
    format!("op-{intent}-{}", &digest[..16])
}

fn restore_index_request(path: &Path, preferred_name: &str) -> Result<InsertFileRequest, DbError> {
    let metadata = fs::symlink_metadata(path)?;
    let normalized_path = normalize_path_for_db(path);
    let name = if preferred_name.trim().is_empty() {
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| resolved_file_name(&normalized_path, ""))
    } else {
        preferred_name.trim().to_string()
    };
    let extension = extension_from_file_name(&name);
    let size = if metadata.is_file() {
        i64::try_from(metadata.len()).unwrap_or(i64::MAX)
    } else {
        0
    };
    let mtime = metadata
        .modified()
        .ok()
        .and_then(system_time_to_unix_seconds)
        .unwrap_or_else(current_unix_seconds);
    let ctime = metadata
        .created()
        .ok()
        .and_then(system_time_to_unix_seconds)
        .unwrap_or(mtime);

    Ok(InsertFileRequest {
        id: normalized_path.clone(),
        path: normalized_path,
        name,
        extension,
        size,
        mtime,
        ctime,
        is_dir: metadata.is_dir(),
        state_code: 0,
    })
}

fn is_neutral_source_file_id(file_id: &str) -> bool {
    file_id.starts_with("zen-canvas-source:")
}

fn indexed_row_matches_restore_request(
    tx: &Transaction<'_>,
    row_id: &str,
    request: &InsertFileRequest,
) -> Result<bool, DbError> {
    let indexed = tx
        .query_row(
            "SELECT size, mtime, is_dir FROM files WHERE id = ?1",
            params![row_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(indexed
        .map(|(size, mtime, is_dir)| {
            size == request.size && mtime == request.mtime && is_dir == bool_to_i64(request.is_dir)
        })
        .unwrap_or(false))
}

fn remove_file_index_row_in_transaction(tx: &Transaction<'_>, row_id: &str) -> Result<(), DbError> {
    invalidate_file_in_transaction(tx, row_id, "stale")?;
    tx.execute(
        "DELETE FROM duplicate_group_members WHERE file_id = ?1",
        params![row_id],
    )?;
    tx.execute(
        "DELETE FROM file_fingerprints WHERE file_id = ?1",
        params![row_id],
    )?;
    let deleted = tx.execute("DELETE FROM files WHERE id = ?1", params![row_id])?;
    if deleted != 1 {
        return Err(DbError::Validation(format!(
            "restore index row disappeared during replacement reconciliation: {row_id}"
        )));
    }
    Ok(())
}

fn insert_restore_index_row_in_transaction(
    tx: &Transaction<'_>,
    request: &InsertFileRequest,
) -> Result<(), DbError> {
    let inserted = tx.execute(
        r#"
        INSERT INTO files (
            id, path, name, extension, size, mtime, ctime, is_dir, state_code,
            file_type, suggested_name, classification_status, is_stale, last_seen_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, 0, ?12)
        "#,
        params![
            request.id,
            request.path,
            request.name,
            request.extension,
            request.size,
            request.mtime,
            request.ctime,
            bool_to_i64(request.is_dir),
            infer_file_type(&request.extension, request.is_dir),
            request.name,
            CLASSIFICATION_STATUS_UNCLASSIFIED,
            current_unix_seconds(),
        ],
    )?;
    if inserted != 1 {
        return Err(DbError::Validation(format!(
            "replacement restore index row was not inserted: {}",
            request.path
        )));
    }
    Ok(())
}

fn join_path_text(directory: &str, name: &str) -> String {
    let separator = if directory.contains('\\') { '\\' } else { '/' };
    format!(
        "{}{}{}",
        directory.trim_end_matches(['/', '\\']),
        separator,
        name
    )
}

fn normalize_path_for_compare_text(path: &str) -> String {
    normalize_path_text(path)
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn search_match_sql(fts_query: &str, raw_query: &str) -> SearchMatchSql {
    let mut cte = String::from(
        r#"
        fts_matches AS (
            SELECT files_fts.rowid, bm25(files_fts, 6.0, 1.5) AS rank
            FROM files_fts
            WHERE files_fts MATCH ?
        ),
        "#,
    );
    let mut params = vec![SqlValue::Text(fts_query.to_string())];

    if should_use_like_fallback(raw_query) {
        let pattern = format!("%{}%", escape_like_pattern(raw_query.trim()));
        cte.push_str(
            r#"
        like_matches AS (
            SELECT f.rowid, 1000000.0 AS rank
            FROM scoped_files AS f
            WHERE f.name LIKE ? ESCAPE '~'
               OR f.path LIKE ? ESCAPE '~'
        ),
        "#,
        );
        params.push(SqlValue::Text(pattern.clone()));
        params.push(SqlValue::Text(pattern));
    } else {
        cte.push_str(
            r#"
        like_matches AS (
            SELECT NULL AS rowid, NULL AS rank
            WHERE 0
        ),
        "#,
        );
    }

    cte.push_str(
        r#"
        search_matches AS (
            SELECT f.rowid, m.rank
            FROM fts_matches AS m
            JOIN scoped_files AS f ON f.rowid = m.rowid
            UNION ALL
            SELECT rowid, rank
            FROM like_matches
        ),
        best_matches AS (
            SELECT rowid, MIN(rank) AS rank
            FROM search_matches
            GROUP BY rowid
        )
        "#,
    );

    SearchMatchSql { cte, params }
}

fn should_use_like_fallback(query: &str) -> bool {
    let trimmed = query.trim();
    !trimmed.is_empty() && trimmed.chars().filter(|ch| !ch.is_whitespace()).count() < 3
}

#[cfg(all(test, target_os = "macos"))]
mod macos_preview_tests {
    use super::operation_preview_semantics;
    use std::{collections::BTreeSet, fs, path::Path};

    #[test]
    fn operation_preview_semantics_does_not_run_namespace_write_probe() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-preview-read-only-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("preview fixture root");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        let before = entry_names(&root);

        let _ = operation_preview_semantics("move", &source, &target);

        assert_eq!(before, entry_names(&root));
        fs::remove_dir_all(root).expect("remove preview fixture");
    }

    fn entry_names(root: &Path) -> BTreeSet<String> {
        fs::read_dir(root)
            .expect("read preview fixture")
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect()
    }
}

#[cfg(test)]
mod operation_preview_fingerprint_tests {
    use super::{operation_preview_fingerprint, OperationPreviewSemantics};
    use crate::db::MaterializationRequirement;

    fn semantics() -> OperationPreviewSemantics {
        OperationPreviewSemantics {
            strategy: None,
            conflict_policy: "exclusive_target",
            runtime_blocking_reason: None,
            will_copy: false,
            will_move: true,
            will_download: false,
            materialization_requirement: MaterializationRequirement::None,
            cross_volume_copy_required: false,
            metadata_degradation_possible: false,
            source_retirement_capability: "not_applicable",
            source_retirement_eligible: true,
            source_retirement_probe_required: false,
            provider_coordination: false,
            will_replace: false,
            will_trash: false,
        }
    }

    #[test]
    fn provider_identity_is_part_of_the_authoritative_fingerprint() {
        let first = operation_preview_fingerprint(
            "preview",
            "file",
            "move",
            "/source",
            "/target",
            "Normal",
            false,
            true,
            false,
            None,
            semantics(),
            Some("source-identity"),
            Some("provider-identity-a"),
        );
        let second = operation_preview_fingerprint(
            "preview",
            "file",
            "move",
            "/source",
            "/target",
            "Normal",
            false,
            true,
            false,
            None,
            semantics(),
            Some("source-identity"),
            Some("provider-identity-b"),
        );
        assert_ne!(first, second);
    }
}
