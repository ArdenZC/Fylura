use super::*;

#[cfg(any(test, feature = "native-qa", target_os = "macos"))]
pub fn restore_moves_with_persistence(
    db: &Database,
    request: RestoreMovesRequest,
) -> Result<RestoreMovesResult, String> {
    restore_moves_with_persistence_with_progress(
        db,
        request,
        Arc::new(AtomicBool::new(false)),
        &NoopOperationProgressEmitter,
    )
}

pub(crate) fn restore_moves_with_persistence_with_progress(
    db: &Database,
    request: RestoreMovesRequest,
    cancel_flag: Arc<AtomicBool>,
    emitter: &impl OperationProgressEmitter,
) -> Result<RestoreMovesResult, String> {
    crate::fs_safety::platform_support::ensure_supported_file_mutation()
        .map_err(|error| error.to_string())?;
    if request.logs.iter().any(restore_requires_reconciliation) {
        return Err(
            "restore_pending_reconciliation: an active restore journal requires startup reconciliation before retrying."
                .to_string(),
        );
    }
    let mut prepared_logs = Vec::with_capacity(request.logs.len());
    let restore_claim_created_at = current_timestamp_ms().to_string();
    for log in &request.logs {
        let source = Path::new(&log.path_after);
        let claim_path = plan_restore_claim_path(source, &log.id)
            .map_err(|error| format!("cannot plan restore claim for {}: {error}", log.id))?;
        let expected_identity = expected_restore_identity_from_log(log).ok_or_else(|| {
            format!(
                "cannot prepare restore claim for {}: restore identity is incomplete",
                log.id
            )
        })?;
        let mut prepared = log.clone();
        prepared.restore_status = "pending".to_string();
        prepared.restore_phase = "prepared".to_string();
        prepared.restore_error = None;
        prepared.restore_claim_path = Some(normalize_path(&claim_path));
        prepared.restore_claim_created_at = Some(restore_claim_created_at.clone());
        prepared.restore_claim_platform_file_id = expected_identity.platform_file_id.clone();
        prepared.restore_claim_platform_volume_id = expected_identity.platform_volume_id.clone();
        prepared.restore_claim_full_hash = expected_identity.full_hash.clone();
        prepared_logs.push(prepared);
    }
    db.prepare_operation_restores(&prepared_logs)
        .map_err(|error| format!("failed to persist restore journal before execution: {error}"))?;
    #[cfg(any(test, feature = "native-qa"))]
    if take_operation_test_fault(OperationTestFaultPoint::AfterRestoreJournalPreparedBeforeClaim) {
        panic!("AfterRestoreJournalPreparedBeforeClaim");
    }
    let mut restored = 0_usize;
    let mut failed = 0_usize;
    let batch_id = restore_progress_batch_id(&request.logs);
    let total = request.logs.len() as u64;
    let mut progress = OperationProgressBuffer::new("restore", batch_id, total);
    let mut logs = Vec::with_capacity(request.logs.len());
    for (index, log) in prepared_logs.iter().enumerate() {
        let mut phase_log = log.clone();
        let mut phase_observer =
            |phase: &str| {
                phase_log.restore_phase = phase.to_string();
                phase_log.restore_status = "pending".to_string();
                phase_log.restore_error = None;
                db.update_operation_restore_phase(&phase_log)
                    .map_err(|error| {
                        if matches!(
                            phase,
                            "target_committed" | "source_cleanup_pending" | "completed"
                        ) {
                            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown
                        } else {
                            crate::fs_safety::AtomicMoveError::SourceClaimRecoveryRequired(format!(
                                "restore journal phase persistence failed: {error}"
                            ))
                        }
                    })?;
                #[cfg(any(test, feature = "native-qa"))]
            match phase {
                "source_claimed"
                    if take_operation_test_fault(
                        OperationTestFaultPoint::AfterRestoreSourceClaimedBeforeTargetCommit,
                    ) => panic!("AfterRestoreSourceClaimedBeforeTargetCommit"),
                "target_committed"
                    if take_operation_test_fault(
                        OperationTestFaultPoint::AfterRestoreTargetCommittedBeforeFinalPersist,
                    ) => panic!("AfterRestoreTargetCommittedBeforeFinalPersist"),
                "completed"
                    if take_operation_test_fault(
                        OperationTestFaultPoint::AfterRestoreCompletedPhaseBeforeFinalTransaction,
                    ) => panic!("AfterRestoreCompletedPhaseBeforeFinalTransaction"),
                _ => {}
            }
                Ok(())
            };
        let result = if is_operation_cancelled(&cancel_flag) {
            mark_restore_canceled(log)
        } else {
            restore_operation_log_with_observer(
                log,
                Some(cancel_flag.as_ref()),
                Some(&mut phase_observer),
            )
        };
        let result = if result.restore_status == "restored" {
            if let Err(failure) = operation_restore_final_identity_check(&result) {
                let mut review = result;
                review.restored_at = None;
                set_restore_manual_review(
                    &mut review,
                    "target_committed",
                    failure.code,
                    failure.detail,
                );
                db.finalize_operation_restore_outcome(std::slice::from_ref(&review))
                    .map_err(|persist_error| {
                        format!("restore finalization requires reconciliation: {persist_error}")
                    })?;
                failed += 1;
                review
            } else {
                match db.finalize_successful_operation_restore(&result) {
                    Ok(()) => {
                        restored += 1;
                        let mut finalized = result;
                        finalized.restore_claim_path = None;
                        finalized.restore_claim_created_at = None;
                        finalized.restore_claim_platform_file_id = None;
                        finalized.restore_claim_platform_volume_id = None;
                        finalized.restore_claim_full_hash = None;
                        finalized
                    }
                    Err(error) => {
                        let mut review = result;
                        review.restored_at = None;
                        set_restore_manual_review(
                            &mut review,
                            "target_committed",
                            crate::recovery::RecoveryErrorCode::TargetCommittedDurabilityUnknown,
                            format!(
                                "restore filesystem commit succeeded but final journal transaction failed: {error}; do not auto retry"
                            ),
                        );
                        db.finalize_operation_restore_outcome(std::slice::from_ref(&review))
                            .map_err(|persist_error| {
                                format!(
                                    "restore finalization requires reconciliation: {persist_error}"
                                )
                            })?;
                        failed += 1;
                        review
                    }
                }
            }
        } else {
            if matches!(result.restore_status.as_str(), "failed" | "manual_review") {
                failed += 1;
            }
            db.finalize_operation_restore_outcome(std::slice::from_ref(&result))
                .map_err(|error| {
                    format!("restore outcome transaction failed; reconciliation required: {error}")
                })?;
            result
        };
        progress.record(emitter, (index + 1) as u64, log.path_after.clone());
        logs.push(result);
    }
    Ok(RestoreMovesResult {
        logs,
        restored,
        failed,
    })
}

pub(crate) fn restore_requires_reconciliation(log: &OperationLogDto) -> bool {
    (log.restore_status == "pending" && log.restore_phase != "prepared")
        || (log.restore_status == "manual_review"
            && restore_phase_requires_recovery(&log.restore_phase))
}

#[cfg(all(test, windows))]
pub fn restore_moves_core(request: RestoreMovesRequest) -> RestoreMovesResult {
    restore_moves_core_with_progress(
        request,
        Arc::new(AtomicBool::new(false)),
        &NoopOperationProgressEmitter,
    )
}

#[cfg(all(test, windows))]
pub fn restore_moves_core_with_progress(
    request: RestoreMovesRequest,
    cancel_flag: Arc<AtomicBool>,
    emitter: &impl OperationProgressEmitter,
) -> RestoreMovesResult {
    let mut restored = 0_usize;
    let mut failed = 0_usize;
    let batch_id = restore_progress_batch_id(&request.logs);
    let total = request.logs.len() as u64;
    let mut progress = OperationProgressBuffer::new("restore", batch_id, total);
    let mut logs = Vec::with_capacity(request.logs.len());

    for (index, log) in request.logs.iter().enumerate() {
        let result = if is_operation_cancelled(&cancel_flag) {
            mark_restore_canceled(log)
        } else {
            restore_operation_log(log, Some(cancel_flag.as_ref()))
        };
        if result.restore_status == "restored" {
            restored += 1;
        } else if matches!(result.restore_status.as_str(), "failed" | "manual_review") {
            failed += 1;
        }
        let current_path = log.path_after.clone();
        logs.push(result);
        progress.record(emitter, (index + 1) as u64, current_path);
    }

    RestoreMovesResult {
        logs,
        restored,
        failed,
    }
}

#[cfg(all(test, windows))]
pub(crate) fn restore_operation_log(
    log: &OperationLogDto,
    cancel_flag: Option<&AtomicBool>,
) -> OperationLogDto {
    restore_operation_log_with_observer(log, cancel_flag, None)
}

pub(crate) fn restore_operation_log_with_observer(
    log: &OperationLogDto,
    cancel_flag: Option<&AtomicBool>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> OperationLogDto {
    if log.operation_type == "move_to_trash" && log.path_after == "Recycle Bin" {
        return mark_restore_unavailable(log, "Restore from system trash.");
    }
    if log.status != "success" {
        return mark_restore_unavailable(log, "Only successful operations can be restored.");
    }
    if !log.can_restore || log.restore_status == "restored" {
        return mark_restore_unavailable(log, "This operation is no longer restorable.");
    }
    if restore_requires_reconciliation(log) {
        return mark_restore_manual_review(
            log,
            "restore_pending_reconciliation: this restore has an active claim or committed-target phase; do not auto retry.",
        );
    }
    if log.path_before.trim().is_empty() || log.path_after.trim().is_empty() {
        return mark_restore_failed(log, "Restore metadata is incomplete.");
    }
    if let Err(failure) = operation_restore_identity_result(log, Path::new(&log.path_after)) {
        return mark_restore_manual_review(log, failure.message());
    }
    if log.operation_type == "replace" {
        return restore_replacement_operation_log_with_observer(log, cancel_flag, phase_observer);
    }

    let source_result = if matches!(log.operation_type.as_str(), "rename" | "move_to_trash") {
        validate_namespace_entry_path(&PathBuf::from(&log.path_after))
    } else {
        validate_source_path(&PathBuf::from(&log.path_after))
    };
    let source = match source_result {
        Ok(path) => path,
        Err(error) => return mark_restore_failed(log, error),
    };
    let restore_claim_path = match log.restore_claim_path.as_deref() {
        Some(path) => PathBuf::from(path),
        None => match plan_restore_claim_path(&source, &log.id) {
            Ok(path) => path,
            Err(error) => return mark_restore_failed(log, error),
        },
    };
    if let Err(error) = validate_restore_claim_path(&source, &restore_claim_path) {
        return mark_restore_manual_review(log, format!("claim_identity_mismatch: {error}"));
    }
    let target = match validate_target_path(&PathBuf::from(&log.path_before)) {
        Ok(path) => path,
        Err(error) => return mark_restore_failed(log, error),
    };

    if let Err(error) = ensure_general_file_operation_allowed(&source) {
        return mark_restore_failed(log, error);
    }
    if let Err(error) = ensure_general_file_operation_allowed(&target) {
        return mark_restore_failed(log, error);
    }
    let expected_identity = expected_restore_identity_from_log(log);
    if let Err(error) = move_file_no_overwrite_with_identity_for_operation(
        &source,
        &target,
        expected_identity.as_ref(),
        Some(&restore_claim_path),
        cancel_flag,
        phase_observer,
        crate::fs_safety::atomic_move::AtomicMoveOperation::Restore,
    ) {
        if error.is_cancelled() {
            return mark_restore_canceled(log);
        }
        return if error.requires_recovery() {
            mark_restore_manual_review(log, error.to_string())
        } else {
            mark_restore_failed(log, error.to_string())
        };
    }

    let mut restored = log.clone();
    restored.can_undo = false;
    restored.can_restore = false;
    restored.restored_at = Some(current_timestamp_ms().to_string());
    restored.restore_status = "restored".to_string();
    restored.restore_error = None;
    restored.restore_phase = "completed".to_string();
    restored
}

fn restore_replacement_operation_log_with_observer(
    log: &OperationLogDto,
    cancel_flag: Option<&AtomicBool>,
    mut phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> OperationLogDto {
    let source = match validate_source_path(&PathBuf::from(&log.path_after)) {
        Ok(path) => path,
        Err(error) => return mark_restore_failed(log, error),
    };
    let restore_claim_path = match log.restore_claim_path.as_deref() {
        Some(path) => PathBuf::from(path),
        None => match plan_restore_claim_path(&source, &log.id) {
            Ok(path) => path,
            Err(error) => return mark_restore_failed(log, error),
        },
    };
    if let Err(error) = validate_restore_claim_path(&source, &restore_claim_path) {
        return mark_restore_manual_review(log, format!("claim_identity_mismatch: {error}"));
    }
    let target = match validate_target_path(&PathBuf::from(&log.path_before)) {
        Ok(path) => path,
        Err(error) => return mark_restore_failed(log, error),
    };
    let backup = replacement_backup_path_for_log(log);
    if let Err(error) = ensure_general_file_operation_allowed(&source) {
        return mark_restore_failed(log, error);
    }
    if let Err(error) = ensure_general_file_operation_allowed(&target) {
        return mark_restore_failed(log, error);
    }
    if let Err(error) = ensure_general_file_operation_allowed(&backup) {
        return mark_restore_failed(log, error);
    }
    if let Err(failure) = replacement_backup_identity_result(log, &backup) {
        return mark_restore_manual_review(log, failure.message());
    }
    let backup_identity = match crate::fs_safety::capture_namespace_identity(&backup, cancel_flag) {
        Ok(identity) => identity,
        Err(error) => {
            return mark_restore_manual_review(
                log,
                format!(
                    "target_committed_identity_unreadable: replacement backup identity could not be captured: {error}"
                ),
            )
        }
    };
    let backup_claim_path = match plan_restore_claim_path(&backup, &format!("{}-backup", log.id)) {
        Ok(path) => path,
        Err(error) => return mark_restore_failed(log, error),
    };
    if let Err(error) = validate_restore_claim_path(&backup, &backup_claim_path) {
        return mark_restore_manual_review(log, format!("claim_identity_mismatch: {error}"));
    }

    let expected_source = expected_restore_identity_from_log(log);
    if let Err(error) = move_file_no_overwrite_with_identity_for_operation(
        &source,
        &target,
        expected_source.as_ref(),
        Some(&restore_claim_path),
        cancel_flag,
        phase_observer.as_deref_mut(),
        crate::fs_safety::atomic_move::AtomicMoveOperation::Restore,
    ) {
        if error.is_cancelled() {
            return mark_restore_canceled(log);
        }
        return if error.requires_recovery() {
            mark_restore_manual_review(log, error.to_string())
        } else {
            mark_restore_failed(log, error.to_string())
        };
    }

    // The first leg returns the new source object to its original path. The
    // retained old destination is then published back with its own fresh
    // identity claim. A failure on this second leg is deliberately manual
    // review: both the original source and the private backup remain
    // recoverable and must not be auto-retried.
    if let Err(error) = move_file_no_overwrite_with_identity_for_operation(
        &backup,
        Path::new(&log.path_after),
        Some(&backup_identity),
        Some(&backup_claim_path),
        cancel_flag,
        phase_observer,
        crate::fs_safety::atomic_move::AtomicMoveOperation::Restore,
    ) {
        if error.is_cancelled() {
            return mark_restore_manual_review(
                log,
                "restore_pending_reconciliation: replacement restore was canceled after the first filesystem commit; review both paths before retrying.",
            );
        }
        return mark_restore_manual_review(
            log,
            format!(
                "replacement restore requires reconciliation after the first filesystem commit: {error}"
            ),
        );
    }

    let mut restored = log.clone();
    restored.can_undo = false;
    restored.can_restore = false;
    restored.restored_at = Some(current_timestamp_ms().to_string());
    restored.restore_status = "restored".to_string();
    restored.restore_error = None;
    restored.restore_phase = "completed".to_string();
    restored
}

pub(crate) fn validate_restore_claim_path(source: &Path, claim: &Path) -> Result<(), String> {
    if !claim.is_absolute() {
        return Err("restore claim path must be absolute".to_string());
    }
    let claim_name = claim
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "restore claim path has no valid file name".to_string())?;
    #[cfg(target_os = "macos")]
    if !claim_name.starts_with(".zen-canvas-claim-")
        && !claim_name.starts_with(".zen-canvas-replace-")
    {
        return Err("restore claim path is outside the claim namespace".to_string());
    }
    let source_parent = source
        .parent()
        .ok_or_else(|| "restore source has no parent".to_string())?;
    #[cfg(target_os = "macos")]
    {
        use std::path::Component;

        let source_parent = source_parent
            .canonicalize()
            .map_err(|error| format!("restore source parent is unavailable: {error}"))?;
        let relative = claim.strip_prefix(&source_parent).map_err(|_| {
            "restore claim path is not inside the source private namespace".to_string()
        })?;
        let mut components = relative.components();
        let Component::Normal(root) = components
            .next()
            .ok_or_else(|| "restore claim path has no private retirement root".to_string())?
        else {
            return Err("restore claim path has an invalid retirement root".to_string());
        };
        let Component::Normal(session) = components
            .next()
            .ok_or_else(|| "restore claim path has no private retirement session".to_string())?
        else {
            return Err("restore claim path has an invalid retirement session".to_string());
        };
        let Component::Normal(_claim) = components
            .next()
            .ok_or_else(|| "restore claim path has no private claim entry".to_string())?
        else {
            return Err("restore claim path has an invalid claim entry".to_string());
        };
        if components.next().is_some()
            || root != std::ffi::OsStr::new(".zen-canvas-retirement")
            || session.is_empty()
        {
            return Err(
                "restore claim path is outside the private retirement namespace".to_string(),
            );
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    if !claim_name.starts_with(".zen-canvas-claim-") {
        return Err("restore claim path is outside the claim namespace".to_string());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let claim_parent = claim
            .parent()
            .ok_or_else(|| "restore claim has no parent".to_string())?
            .canonicalize()
            .map_err(|error| format!("restore claim parent is unavailable: {error}"))?;
        if normalize_path(&claim_parent) != normalize_path(source_parent) {
            return Err("restore claim path is not adjacent to the restore source".to_string());
        }
        Ok(())
    }
}

pub(crate) fn plan_restore_claim_path(
    source: &Path,
    operation_id: &str,
) -> Result<PathBuf, String> {
    if let Ok(path) = crate::fs_safety::source_claim::planned_claim_path(source, operation_id) {
        return Ok(path);
    }
    let parent = source
        .parent()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "restore source has no absolute parent".to_string())?;
    Ok(parent.join(format!(".zen-canvas-claim-{}", uuid::Uuid::new_v4())))
}

pub(crate) fn mark_restore_failed(
    log: &OperationLogDto,
    error: impl Into<String>,
) -> OperationLogDto {
    let mut failed = log.clone();
    failed.restore_status = "failed".to_string();
    failed.restore_error = Some(error.into());
    if !restore_phase_requires_recovery(&failed.restore_phase) {
        failed.restore_phase = "rolled_back".to_string();
        clear_restore_claim(&mut failed);
    }
    failed
}

pub(crate) fn mark_restore_canceled(log: &OperationLogDto) -> OperationLogDto {
    let mut canceled = log.clone();
    if restore_phase_requires_recovery(&canceled.restore_phase) {
        return mark_restore_manual_review(
            log,
            "restore_pending_reconciliation: restore cancellation occurred after the source claim boundary; do not auto retry.",
        );
    }
    canceled.restore_status = "canceled".to_string();
    canceled.restore_error = None;
    canceled.restore_phase = "rolled_back".to_string();
    clear_restore_claim(&mut canceled);
    canceled
}

pub(crate) fn mark_restore_unavailable(
    log: &OperationLogDto,
    reason: impl Into<String>,
) -> OperationLogDto {
    let mut unavailable = log.clone();
    unavailable.can_undo = false;
    unavailable.can_restore = false;
    unavailable.restore_status = "unavailable".to_string();
    unavailable.restore_error = Some(reason.into());
    unavailable
}

pub(crate) fn restore_progress_batch_id(_logs: &[OperationLogDto]) -> String {
    new_job_id("restore-batch")
}

pub(crate) fn is_operation_cancelled(cancel_flag: &Arc<AtomicBool>) -> bool {
    cancel_flag.load(Ordering::Relaxed)
}

pub(crate) fn mark_restore_manual_review(
    log: &OperationLogDto,
    error: impl Into<String>,
) -> OperationLogDto {
    let mut review = log.clone();
    let active = restore_phase_requires_recovery(&review.restore_phase);
    review.status = "manual_review".to_string();
    review.can_undo = false;
    review.can_restore = false;
    review.restore_status = "manual_review".to_string();
    if !active {
        review.restore_phase = "manual_review".to_string();
        clear_restore_claim(&mut review);
    }
    review.restore_error = Some(error.into());
    review
}

pub(crate) fn set_restore_manual_review(
    log: &mut OperationLogDto,
    phase: &str,
    code: crate::recovery::RecoveryErrorCode,
    detail: impl Into<String>,
) {
    log.status = "manual_review".to_string();
    log.can_undo = false;
    log.can_restore = false;
    log.restore_status = "manual_review".to_string();
    log.restore_phase = phase.to_string();
    log.restore_error = Some(crate::recovery::format_recovery_message(
        code,
        &detail.into(),
    ));
}
pub(crate) fn restore_phase_requires_recovery(phase: &str) -> bool {
    matches!(
        phase,
        "source_claimed" | "copying" | "target_committed" | "source_cleanup_pending" | "completed"
    )
}

pub(crate) fn clear_restore_claim(log: &mut OperationLogDto) {
    log.restore_claim_path = None;
    log.restore_claim_created_at = None;
    log.restore_claim_platform_file_id = None;
    log.restore_claim_platform_volume_id = None;
    log.restore_claim_full_hash = None;
}
