use super::*;

use std::cell::RefCell;

#[derive(Debug, Clone)]
struct ActualOperationPaths {
    source: PathBuf,
    target: PathBuf,
    claim: Option<PathBuf>,
}

fn update_operation_log_with_actual_paths(
    log: &mut OperationLogDto,
    operation_type: &str,
    paths: &ActualOperationPaths,
) {
    let source = normalize_path(&paths.source);
    log.source_path = source.clone();
    log.path_before = source;
    log.name_before = paths
        .source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&log.name_before)
        .to_string();
    log.old_name = log.name_before.clone();

    if operation_type != "permanent_delete" {
        let target = normalize_path(&paths.target);
        log.target_path = target.clone();
        log.path_after = target;
        log.name_after = paths
            .target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&log.name_after)
            .to_string();
        log.new_name = log.name_after.clone();
    }
    if let Some(claim) = paths.claim.as_deref() {
        log.source_claim_path = Some(normalize_path(claim));
    }
}

pub(crate) fn execute_moves_with_persistence_with_progress_and_app_data(
    db: &Database,
    request: ExecuteMovesRequest,
    cancel_flag: Arc<AtomicBool>,
    emitter: &impl OperationProgressEmitter,
    app_data_dir: Option<PathBuf>,
    caller_batch_id: Option<String>,
) -> Result<ExecuteMovesResult, String> {
    crate::fs_safety::platform_support::ensure_supported_file_mutation()
        .map_err(|error| error.to_string())?;
    let operations = request.operations.clone();
    let batch_id = caller_batch_id.unwrap_or_else(|| new_job_id("operation-batch"));
    let created_at = current_timestamp_ms().to_string();
    let prepared_operations =
        persist_pending_operation_journal(db, &request, &batch_id, &created_at)?;
    let mut result = execute_moves_core_with_identity(
        request,
        cancel_flag,
        emitter,
        app_data_dir,
        batch_id,
        created_at,
        OperationPersistenceContext {
            prepared_operations: Some(&prepared_operations),
            journal_db: Some(db),
        },
    );

    for (operation, log) in operations.iter().zip(result.logs.iter_mut()) {
        if let Some(prepared) = prepared_operations.get(&operation.id) {
            apply_source_fingerprint(log, &prepared.fingerprint);
            log.source_claim_path = Some(normalize_path(&prepared.claim_path));
            log.claim_created_at = Some(prepared.claim_created_at.clone());
        }
        if log.status != "success" {
            continue;
        }
        let identity_path = if operation.operation_type == "replace" {
            crate::fs_safety::atomic_move::replacement_backup_path(
                Path::new(&log.path_before),
                Path::new(&log.path_after),
            )
        } else {
            PathBuf::from(&log.path_after)
        };
        if let Ok(target_fingerprint) =
            file_operation_fingerprint(&identity_path, &operation.operation_type)
        {
            apply_target_fingerprint(log, &target_fingerprint);
        }
        if operation.operation_type == "move"
            && identity_path
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_dir())
        {
            match crate::file_ops::manual_transfer::directory_tree_snapshot(&identity_path)
                .and_then(|tree| {
                    crate::file_ops::manual_transfer::encode_directory_tree_journal_fingerprint(
                        &tree,
                    )
                    .ok_or_else(|| "directory tree fingerprint encoding failed".into())
                }) {
                Ok(tree) => log.target_full_hash = Some(tree),
                Err(error) => {
                    log.status = "manual_review".to_string();
                    log.can_undo = false;
                    log.can_restore = false;
                    log.operation_phase = "target_committed".to_string();
                    log.error_message = Some(format!(
                        "target_committed_identity_unreadable: directory tree proof could not be captured: {error}"
                    ));
                }
            }
        }
        if cfg!(target_os = "macos") && operation.operation_type == "replace" {
            if let Err(error) = refresh_replacement_journal_identities(log) {
                log.status = "manual_review".to_string();
                log.can_undo = false;
                log.can_restore = false;
                log.operation_phase = "target_committed".to_string();
                log.error_message = Some(format!(
                    "target_committed_identity_unreadable: replacement identities could not be captured: {error}"
                ));
            }
        }
        if log.status != "success" {
            continue;
        }
        if matches!(
            operation.operation_type.as_str(),
            "move_to_trash" | "copy" | "duplicate" | "permanent_delete"
        ) || operation.file_id == super::RECOVERY_ACTION_FILE_ID
        {
            continue;
        }

        if let Err(error) = db.update_file_after_successful_operation(
            &operation.file_id,
            &log.path_before,
            &log.path_after,
            &log.name_after,
        ) {
            let warning = format!("file index sync failed: {error}");
            eprintln!("{warning}");
            append_operation_log_error(log, warning);
        }
    }

    for log in &mut result.logs {
        log.operation_phase = operation_phase_for_log(log).to_string();
    }

    #[cfg(any(test, feature = "native-qa"))]
    if take_operation_test_fault(OperationTestFaultPoint::AfterCompletedPhaseBeforeFinalLogPersist)
    {
        return Err("injected after_completed_phase_before_final_log_persist failure".to_string());
    }

    db.save_operation_logs(&result.batch_id, &result.logs)
        .map_err(|error| format!("operation completed but failed to persist logs: {error}"))?;
    Ok(result)
}

pub(crate) fn execute_moves_core_with_progress_and_app_data(
    request: ExecuteMovesRequest,
    cancel_flag: Arc<AtomicBool>,
    emitter: &impl OperationProgressEmitter,
    app_data_dir: Option<PathBuf>,
) -> ExecuteMovesResult {
    let batch_id = new_job_id("operation-batch");
    let created_at = current_timestamp_ms().to_string();
    execute_moves_core_with_identity(
        request,
        cancel_flag,
        emitter,
        app_data_dir,
        batch_id,
        created_at,
        OperationPersistenceContext {
            prepared_operations: None,
            journal_db: None,
        },
    )
}

pub(crate) struct OperationPersistenceContext<'a> {
    prepared_operations: Option<&'a std::collections::HashMap<String, PreparedOperation>>,
    journal_db: Option<&'a Database>,
}

fn execute_moves_core_with_identity(
    request: ExecuteMovesRequest,
    cancel_flag: Arc<AtomicBool>,
    emitter: &impl OperationProgressEmitter,
    app_data_dir: Option<PathBuf>,
    batch_id: String,
    created_at: String,
    persistence: OperationPersistenceContext<'_>,
) -> ExecuteMovesResult {
    let prepared_operations = persistence.prepared_operations;
    let journal_db = persistence.journal_db;
    let total = request.operations.len() as u64;
    let mut progress = OperationProgressBuffer::new("execute", batch_id.clone(), total);
    let mut logs = Vec::with_capacity(request.operations.len());

    for (index, operation) in request.operations.iter().enumerate() {
        let log = if is_operation_cancelled(&cancel_flag) {
            make_canceled_operation_log(&batch_id, &created_at, index, operation)
        } else {
            let prepared = prepared_operations.and_then(|items| items.get(&operation.id));
            let expected_identity =
                prepared.map(|item| expected_identity_from_fingerprint(&item.fingerprint));
            let phase_log = RefCell::new(prepared.map(|item| item.journal_log.clone()));
            let actual_paths = RefCell::new(None::<ActualOperationPaths>);
            let mut observed_phase = None;
            let mut phase_observer = |event: crate::fs_safety::FileOperationEvent| {
                match event {
                    crate::fs_safety::FileOperationEvent::Phase(phase) => {
                        observed_phase = Some(phase.clone());
                        if let Some(db) = journal_db {
                            let mut phase_log = phase_log.borrow_mut();
                            if let Some(log) = phase_log.as_mut() {
                                log.operation_phase = phase.clone();
                                // Keep the row pending until the final log
                                // transaction succeeds.
                                log.status = "pending".to_string();
                                log.error_message = None;
                                if operation.operation_type == "move"
                                    && phase == "target_committed"
                                    && Path::new(&log.path_after)
                                        .symlink_metadata()
                                        .is_ok_and(|metadata| metadata.is_dir())
                                {
                                    let target = PathBuf::from(&log.path_after);
                                    let fingerprint =
                                        file_operation_fingerprint(&target, "move").map_err(|_| {
                                            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown
                                        })?;
                                    apply_target_fingerprint(log, &fingerprint);
                                    let tree = crate::file_ops::manual_transfer::directory_tree_snapshot(
                                        &target,
                                    )
                                    .and_then(|tree| {
                                        crate::file_ops::manual_transfer::encode_directory_tree_journal_fingerprint(
                                            &tree,
                                        )
                                        .ok_or_else(|| "directory tree fingerprint encoding failed".into())
                                    })
                                    .map_err(|_| {
                                        crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown
                                    })?;
                                    log.target_full_hash = Some(tree);
                                }
                                if cfg!(target_os = "macos")
                                    && operation.operation_type == "replace"
                                    && matches!(
                                        phase.as_str(),
                                        "target_committed" | "source_cleanup_pending" | "completed"
                                    )
                                {
                                    refresh_replacement_journal_identities(log).map_err(|_| {
                                        crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown
                                    })?;
                                }
                                db.update_operation_phase(log).map_err(|error| {
                                    let message = format!("journal phase persistence failed: {error}");
                                    if matches!(
                                        phase.as_str(),
                                        "target_committed" | "source_cleanup_pending" | "completed"
                                    ) {
                                        crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown
                                    } else {
                                        crate::fs_safety::AtomicMoveError::SourceClaimRecoveryRequired(message)
                                    }
                                })?;
                            }
                        }
                        emitter.emit_progress(OperationProgressPayload {
                            kind: "execute".to_string(),
                            batch_id: batch_id.clone(),
                            processed: index as u64,
                            total,
                            current_path: operation.source_path.clone(),
                            phase: Some(phase),
                            processed_items: None,
                            total_items: None,
                            processed_bytes: None,
                            total_bytes: None,
                        });
                    }
                    crate::fs_safety::FileOperationEvent::Progress(progress) => {
                        emitter.emit_progress(OperationProgressPayload {
                            kind: "execute".to_string(),
                            batch_id: batch_id.clone(),
                            processed: index as u64,
                            total,
                            current_path: progress.current_path,
                            phase: Some(progress.stage),
                            processed_items: Some(progress.processed_items),
                            total_items: progress.total_items,
                            processed_bytes: Some(progress.processed_bytes),
                            total_bytes: progress.total_bytes,
                        });
                    }
                }
                Ok(())
            };
            let mut actual_path_observer =
                |actual_source: &Path, actual_target: &Path, actual_claim: Option<&Path>| {
                    let paths = ActualOperationPaths {
                        source: actual_source.to_path_buf(),
                        target: actual_target.to_path_buf(),
                        claim: actual_claim.map(Path::to_path_buf),
                    };
                    actual_paths.replace(Some(paths.clone()));
                    if let Some(db) = journal_db {
                        let mut phase_log = phase_log.borrow_mut();
                        let Some(log) = phase_log.as_mut() else {
                            return Ok(());
                        };
                        // Windows keeps its existing handle-bound path
                        // representation in the journal. The actual-URL
                        // callback is a macOS coordination contract; using
                        // the Windows extended path spelling here would
                        // change persisted log text without changing the
                        // mutation authority.
                        if cfg!(target_os = "macos") {
                            update_operation_log_with_actual_paths(
                                log,
                                &operation.operation_type,
                                &paths,
                            );
                        }
                        db.update_operation_actual_paths(log).map_err(|error| {
                            crate::fs_safety::AtomicMoveError::SourceClaimRecoveryRequired(format!(
                                "actual coordinated path persistence failed: {error}"
                            ))
                        })?;
                    }
                    Ok(())
                };
            let mut log = execute_preview_operation_with_app_data(
                &batch_id,
                &created_at,
                index,
                operation,
                OperationExecutionContext {
                    cancel_flag: Some(cancel_flag.as_ref()),
                    app_data_dir: app_data_dir.as_deref(),
                    expected_identity: expected_identity.as_ref(),
                    planned_claim_path: prepared.map(|item| item.claim_path.as_path()),
                    phase_observer: Some(
                        &mut phase_observer as &mut crate::fs_safety::PhaseObserver<'_>,
                    ),
                    actual_path_observer: Some(
                        &mut actual_path_observer as &mut crate::fs_safety::ActualPathObserver<'_>,
                    ),
                },
            );
            // A post-commit failure is returned as a fresh operation log by
            // `execute_preview_operation_with_app_data`. Preserve the
            // identity proof written by the phase observer before the final
            // batch save; otherwise that save can erase the target identity
            // needed to distinguish a recoverable source-cleanup interruption
            // from an ambiguous filesystem state after restart.
            drop(phase_observer);
            drop(actual_path_observer);
            if let Some(journal_log) = phase_log.into_inner() {
                log.source_claim_path = log.source_claim_path.or(journal_log.source_claim_path);
                log.claim_created_at = log.claim_created_at.or(journal_log.claim_created_at);
                log.claim_platform_file_id = log
                    .claim_platform_file_id
                    .or(journal_log.claim_platform_file_id);
                log.claim_platform_volume_id = log
                    .claim_platform_volume_id
                    .or(journal_log.claim_platform_volume_id);
                log.claim_full_hash = log.claim_full_hash.or(journal_log.claim_full_hash);
                log.target_platform_file_id = log
                    .target_platform_file_id
                    .or(journal_log.target_platform_file_id);
                log.target_platform_volume_id = log
                    .target_platform_volume_id
                    .or(journal_log.target_platform_volume_id);
                log.target_full_hash = log.target_full_hash.or(journal_log.target_full_hash);
            }
            if let Some(paths) = actual_paths.into_inner() {
                if cfg!(target_os = "macos") {
                    update_operation_log_with_actual_paths(
                        &mut log,
                        &operation.operation_type,
                        &paths,
                    );
                }
            }
            if let Some(phase) = observed_phase {
                log.operation_phase = phase;
                if log.status != "success"
                    && !matches!(
                        log.operation_phase.as_str(),
                        "prepared" | "source_claimed" | "copying" | "rolled_back"
                    )
                {
                    log.status = "manual_review".to_string();
                    log.can_undo = false;
                    log.can_restore = false;
                }
            }
            log
        };
        let current_path = operation.source_path.clone();
        logs.push(log);
        progress.record(emitter, (index + 1) as u64, current_path);
    }

    ExecuteMovesResult { logs, batch_id }
}
