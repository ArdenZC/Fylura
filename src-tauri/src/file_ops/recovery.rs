use super::*;

pub fn reconcile_pending_operation_journal(db: &Database) -> Result<usize, String> {
    let pending = db
        .get_pending_operation_logs()
        .map_err(|error| error.to_string())?;
    let pending_restores = db
        .get_pending_restore_logs()
        .map_err(|error| error.to_string())?;
    let mut by_batch = std::collections::HashMap::<String, Vec<OperationLogDto>>::new();
    for mut log in pending {
        let before = Path::new(&log.path_before);
        let after = Path::new(&log.path_after);
        let claim = log.source_claim_path.as_deref().map(Path::new);
        let before_state =
            operation_journal_path_state(before, |path| journal_identity_matches(&log, path));
        let after_state =
            operation_journal_path_state(after, |path| journal_target_identity_matches(&log, path));
        let claim_state = claim.map_or(OperationJournalPathState::Missing, |path| {
            operation_journal_path_state(path, |candidate| {
                journal_identity_matches(&log, candidate)
            })
        });
        match (before_state, after_state, claim_state) {
            (
                OperationJournalPathState::Missing,
                OperationJournalPathState::Matches,
                OperationJournalPathState::Missing,
            ) if matches!(
                log.operation_phase.as_str(),
                "target_committed" | "source_cleanup_pending"
            ) =>
            {
                log.status = "manual_review".to_string();
                log.operation_phase = if log.operation_phase == "source_cleanup_pending" {
                    "source_cleanup_pending"
                } else {
                    "target_committed"
                }
                .to_string();
                log.can_undo = false;
                log.can_restore = false;
                log.error_message = Some(
                    "target_committed_durability_unknown: target may have committed; verify the target before retrying."
                        .to_string(),
                );
            }
            (
                OperationJournalPathState::Missing,
                OperationJournalPathState::Matches,
                OperationJournalPathState::Missing,
            ) => {
                log.status = "success".to_string();
                log.operation_phase = "completed".to_string();
                log.can_undo = log.operation_type != "move_to_trash";
                log.can_restore = log.can_undo;
                log.error_message =
                    Some("Recovered an interrupted operation journal after restart.".to_string());
            }
            (
                OperationJournalPathState::Matches,
                OperationJournalPathState::Missing,
                OperationJournalPathState::Missing,
            ) => {
                log.status = "failed".to_string();
                log.operation_phase = "rolled_back".to_string();
                log.can_undo = false;
                log.can_restore = false;
                log.error_message = Some(
                    "Operation was interrupted before the filesystem move; the source remains intact."
                        .to_string(),
                );
            }
            (
                OperationJournalPathState::Missing,
                OperationJournalPathState::Missing,
                OperationJournalPathState::Matches,
            ) => {
                log.status = "pending".to_string();
                log.operation_phase = "source_claimed".to_string();
                log.can_undo = false;
                log.can_restore = false;
                log.error_message = Some(
                    "Source claim was recovered without a committed target; manual review is required."
                        .to_string(),
                );
            }
            (
                OperationJournalPathState::Missing,
                OperationJournalPathState::Matches,
                OperationJournalPathState::Matches,
            ) => {
                log.status = "manual_review".to_string();
                log.operation_phase = "source_cleanup_pending".to_string();
                log.can_undo = false;
                log.can_restore = false;
                log.error_message = Some(
                    "Committed target and source claim were recovered together; source cleanup requires manual review."
                        .to_string(),
                );
            }
            (
                OperationJournalPathState::Matches,
                OperationJournalPathState::Matches,
                OperationJournalPathState::Missing,
            ) if log.operation_phase == "source_cleanup_pending" => {
                log.status = "manual_review".to_string();
                log.operation_phase = "source_cleanup_pending".to_string();
                log.can_undo = false;
                log.can_restore = false;
                log.error_message = Some(
                    "portable_source_retirement_pending: verified target and original source are both preserved; retry source retirement only after the volume exposes a safe exclusive claim primitive."
                        .to_string(),
                );
            }
            _ => {
                log.status = "manual_review".to_string();
                log.operation_phase = "manual_review".to_string();
                log.can_undo = false;
                log.can_restore = false;
                log.error_message = Some(
                    "Interrupted operation has ambiguous or replaced identities; manual review is required."
                        .to_string(),
                );
            }
        }
        by_batch.entry(log.batch_id.clone()).or_default().push(log);
    }
    let mut reconciled = by_batch.values().map(Vec::len).sum::<usize>();
    for (batch_id, logs) in by_batch {
        db.save_operation_logs(&batch_id, &logs)
            .map_err(|error| error.to_string())?;
    }
    if !pending_restores.is_empty() {
        for mut log in pending_restores {
            if log.operation_type == "replace" {
                let reconciled_log = reconcile_pending_replacement_restore(&log);
                if reconciled_log.restore_status == "restored" {
                    if let Err(failure) = operation_restore_final_identity_check(&reconciled_log) {
                        let mut review = reconciled_log;
                        review.restored_at = None;
                        set_restore_manual_review(
                            &mut review,
                            "target_committed",
                            failure.code,
                            failure.detail,
                        );
                        db.finalize_operation_restore_outcome(std::slice::from_ref(&review))
                            .map_err(|persist_error| persist_error.to_string())?;
                    } else if let Err(error) =
                        db.finalize_successful_operation_restore(&reconciled_log)
                    {
                        let mut review = reconciled_log;
                        review.restored_at = None;
                        set_restore_manual_review(
                            &mut review,
                            "target_committed",
                            crate::recovery::RecoveryErrorCode::TargetCommittedDurabilityUnknown,
                            format!(
                                "replacement restore was committed but final reconciliation transaction failed: {error}; do not auto retry"
                            ),
                        );
                        db.finalize_operation_restore_outcome(std::slice::from_ref(&review))
                            .map_err(|persist_error| persist_error.to_string())?;
                    }
                } else {
                    db.finalize_operation_restore_outcome(std::slice::from_ref(&reconciled_log))
                        .map_err(|error| error.to_string())?;
                }
                reconciled += 1;
                continue;
            }
            let before = Path::new(&log.path_before);
            let after = Path::new(&log.path_after);
            let claim = log.restore_claim_path.as_deref().map(Path::new);
            let source_path_reappeared = fs::symlink_metadata(after).is_ok();
            let before_state = operation_journal_path_state(before, |path| {
                operation_restore_original_identity_matches(&log, path)
            });
            let after_state = operation_journal_path_state(after, |path| {
                operation_restore_identity_matches(&log, path)
            });
            let claim_state = claim.map_or(OperationJournalPathState::Missing, |path| {
                operation_journal_path_state(path, |candidate| {
                    restore_claim_identity_matches(&log, candidate)
                })
            });

            if before_state == OperationJournalPathState::Matches
                && after_state == OperationJournalPathState::Missing
                && claim_state == OperationJournalPathState::Missing
            {
                log.can_undo = false;
                log.can_restore = false;
                log.restored_at = Some(current_timestamp_ms().to_string());
                log.restore_status = "restored".to_string();
                log.restore_phase = "completed".to_string();
                log.restore_error = None;
                if let Err(failure) = operation_restore_final_identity_check(&log) {
                    log.restored_at = None;
                    set_restore_manual_review(
                        &mut log,
                        "target_committed",
                        failure.code,
                        failure.detail,
                    );
                    db.finalize_operation_restore_outcome(std::slice::from_ref(&log))
                        .map_err(|persist_error| persist_error.to_string())?;
                } else if let Err(error) = db.finalize_successful_operation_restore(&log) {
                    log.restored_at = None;
                    set_restore_manual_review(
                        &mut log,
                        "target_committed",
                        crate::recovery::RecoveryErrorCode::TargetCommittedDurabilityUnknown,
                        format!(
                            "restore was committed but final reconciliation transaction failed: {error}; do not auto retry"
                        ),
                    );
                    db.finalize_operation_restore_outcome(std::slice::from_ref(&log))
                        .map_err(|persist_error| persist_error.to_string())?;
                }
                reconciled += 1;
                continue;
            }
            let target_commit_observed = before_state == OperationJournalPathState::Matches
                || matches!(
                    log.restore_phase.as_str(),
                    "target_committed" | "source_cleanup_pending" | "completed"
                );
            if source_path_reappeared && target_commit_observed {
                set_restore_manual_review(
                    &mut log,
                    "source_cleanup_pending",
                    crate::recovery::RecoveryErrorCode::RestoreSourcePathReappeared,
                    "restore source path reappeared after the target commit; preserve the claim and review both paths",
                );
            } else if matches!(
                claim_state,
                OperationJournalPathState::Mismatch | OperationJournalPathState::Unreadable
            ) {
                let code = if claim_state == OperationJournalPathState::Unreadable {
                    crate::recovery::RecoveryErrorCode::ClaimIdentityUnreadable
                } else {
                    crate::recovery::RecoveryErrorCode::ClaimIdentityMismatch
                };
                set_restore_manual_review(
                    &mut log,
                    if before_state == OperationJournalPathState::Matches {
                        "target_committed"
                    } else {
                        "source_claimed"
                    },
                    code,
                    "persisted restore claim identity is mismatched or unreadable; do not auto retry.",
                );
            } else if before_state == OperationJournalPathState::Mismatch
                || before_state == OperationJournalPathState::Unreadable
            {
                let code = if before_state == OperationJournalPathState::Unreadable {
                    crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable
                } else {
                    crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch
                };
                set_restore_manual_review(
                    &mut log,
                    "target_committed",
                    code,
                    "restore target or source identity is mismatched or unreadable; do not auto retry.",
                );
            } else if after_state == OperationJournalPathState::Mismatch
                || after_state == OperationJournalPathState::Unreadable
            {
                let code = if after_state == OperationJournalPathState::Unreadable {
                    crate::recovery::RecoveryErrorCode::RestoreSourceIdentityUnreadable
                } else {
                    crate::recovery::RecoveryErrorCode::RestoreSourceIdentityMismatch
                };
                set_restore_manual_review(
                    &mut log,
                    "target_committed",
                    code,
                    "restore source identity cannot be trusted; do not auto retry",
                );
            } else if before_state == OperationJournalPathState::Matches
                && after_state == OperationJournalPathState::Missing
                && claim_state == OperationJournalPathState::Matches
            {
                set_restore_manual_review(
                    &mut log,
                    "source_cleanup_pending",
                    crate::recovery::RecoveryErrorCode::TargetCommittedSourceCleanupPending,
                    "restored target and restore claim both exist; do not auto retry source cleanup.",
                );
            } else if before_state == OperationJournalPathState::Missing
                && after_state == OperationJournalPathState::Matches
                && claim_state == OperationJournalPathState::Missing
            {
                log.status = "success".to_string();
                log.can_undo = true;
                log.can_restore = true;
                log.restore_status = "not_restored".to_string();
                log.restore_phase = "rolled_back".to_string();
                log.restore_error = Some(crate::recovery::format_recovery_message(
                    crate::recovery::RecoveryErrorCode::RestorePendingReconciliation,
                    "restore was interrupted before filesystem commit; it remains available and will not be auto-retried",
                ));
                clear_restore_claim(&mut log);
            } else if before_state == OperationJournalPathState::Missing
                && after_state == OperationJournalPathState::Missing
                && claim_state == OperationJournalPathState::Matches
            {
                log.status = "manual_review".to_string();
                log.can_undo = false;
                log.can_restore = false;
                log.restore_status = "manual_review".to_string();
                log.restore_phase = "source_claimed".to_string();
                log.restore_error = Some(
                    "restore_pending_reconciliation: restore source claim was recovered without a committed target; do not auto retry."
                        .to_string(),
                );
            } else {
                set_restore_manual_review(
                    &mut log,
                    "target_committed",
                    crate::recovery::RecoveryErrorCode::TargetCommittedDurabilityUnknown,
                    "restore path state is ambiguous after the filesystem boundary; preserve the claim and do not auto retry",
                );
            }
            db.finalize_operation_restore_outcome(std::slice::from_ref(&log))
                .map_err(|error| error.to_string())?;
            reconciled += 1;
        }
    }
    Ok(reconciled)
}

fn reconcile_pending_replacement_restore(log: &OperationLogDto) -> OperationLogDto {
    let before = Path::new(&log.path_before);
    let after = Path::new(&log.path_after);
    let backup = replacement_backup_path_for_log(log);
    let claim = log.restore_claim_path.as_deref().map(Path::new);
    let before_state = operation_journal_path_state(before, |path| {
        operation_restore_original_identity_matches(log, path)
    });
    let current_after_state =
        operation_journal_path_state(after, |path| operation_restore_identity_matches(log, path));
    let restored_after_state = operation_journal_path_state(after, |path| {
        replacement_backup_identity_result(log, path)
            .map(|_| true)
            .map_err(|_| ())
    });
    let backup_state = operation_journal_path_state(&backup, |path| {
        replacement_backup_identity_result(log, path)
            .map(|_| true)
            .map_err(|_| ())
    });
    let claim_state = claim.map_or(OperationJournalPathState::Missing, |path| {
        operation_journal_path_state(path, |candidate| {
            restore_claim_identity_matches(log, candidate)
        })
    });

    if before_state == OperationJournalPathState::Matches
        && restored_after_state == OperationJournalPathState::Matches
        && backup_state == OperationJournalPathState::Missing
        && claim_state == OperationJournalPathState::Missing
    {
        let mut restored = log.clone();
        restored.status = "success".to_string();
        restored.can_undo = false;
        restored.can_restore = false;
        restored.restored_at = Some(current_timestamp_ms().to_string());
        restored.restore_status = "restored".to_string();
        restored.restore_error = None;
        restored.restore_phase = "completed".to_string();
        return restored;
    }

    if before_state == OperationJournalPathState::Missing
        && current_after_state == OperationJournalPathState::Matches
        && backup_state == OperationJournalPathState::Matches
        && claim_state == OperationJournalPathState::Missing
    {
        let mut available = log.clone();
        available.status = "success".to_string();
        available.can_undo = true;
        available.can_restore = true;
        available.restore_status = "not_restored".to_string();
        available.restore_phase = "rolled_back".to_string();
        available.restore_error = Some(
            "restore_pending_reconciliation: replacement restore was interrupted before the first filesystem commit; it remains available and will not be auto-retried."
                .to_string(),
        );
        clear_restore_claim(&mut available);
        return available;
    }

    let mut review = log.clone();
    set_restore_manual_review(
        &mut review,
        if before_state == OperationJournalPathState::Matches {
            "target_committed"
        } else {
            "source_claimed"
        },
        if matches!(
            claim_state,
            OperationJournalPathState::Mismatch | OperationJournalPathState::Unreadable
        ) {
            if claim_state == OperationJournalPathState::Unreadable {
                crate::recovery::RecoveryErrorCode::ClaimIdentityUnreadable
            } else {
                crate::recovery::RecoveryErrorCode::ClaimIdentityMismatch
            }
        } else if matches!(
            before_state,
            OperationJournalPathState::Mismatch | OperationJournalPathState::Unreadable
        ) || matches!(
            restored_after_state,
            OperationJournalPathState::Mismatch | OperationJournalPathState::Unreadable
        ) {
            if before_state == OperationJournalPathState::Unreadable
                || restored_after_state == OperationJournalPathState::Unreadable
            {
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable
            } else {
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch
            }
        } else {
            crate::recovery::RecoveryErrorCode::TargetCommittedDurabilityUnknown
        },
        "replacement restore has an ambiguous source, target, or retained-backup state; preserve all paths and do not auto retry",
    );
    review
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestoreVolumeRelation {
    SameVolume,
    CrossVolume,
    Unknown,
}

pub(crate) fn restore_volume_relation(log: &OperationLogDto) -> RestoreVolumeRelation {
    match (
        log.source_platform_volume_id.as_deref(),
        log.target_platform_volume_id.as_deref(),
    ) {
        (Some(source), Some(target)) if source == target => RestoreVolumeRelation::SameVolume,
        (Some(_), Some(_)) => RestoreVolumeRelation::CrossVolume,
        _ => RestoreVolumeRelation::Unknown,
    }
}

pub(crate) fn is_directory_move_log(log: &OperationLogDto) -> bool {
    log.operation_type == "move"
        && (crate::file_ops::manual_transfer::directory_tree_journal_fingerprint(
            log.target_full_hash.as_deref(),
        )
        .is_some()
            || crate::file_ops::manual_transfer::transfer_directory_tree_fingerprint(&log.id)
                .is_some()
            || Path::new(&log.path_after)
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_dir()))
}

pub(crate) fn directory_move_tree_fingerprint(log: &OperationLogDto) -> Option<&str> {
    crate::file_ops::manual_transfer::directory_tree_journal_fingerprint(
        log.target_full_hash.as_deref(),
    )
    .or_else(|| {
        (restore_volume_relation(log) == RestoreVolumeRelation::SameVolume)
            .then(|| crate::file_ops::manual_transfer::transfer_directory_tree_fingerprint(&log.id))
            .flatten()
    })
}

/// Identity of the path currently holding the file being restored (path_after).
/// A file ID is meaningful only when both operation volumes are known to be
/// the same. Missing volume metadata is deliberately not treated as proof of
/// same-volume semantics.
pub(crate) fn expected_restore_identity_from_log(
    log: &OperationLogDto,
) -> Option<crate::fs_safety::ExpectedFileIdentity> {
    if log.operation_type == "replace" {
        let (platform_volume_id, platform_file_id) = if cfg!(target_os = "macos") {
            (
                log.source_platform_volume_id.clone(),
                log.source_platform_file_id.clone(),
            )
        } else {
            (None, None)
        };
        return Some(crate::fs_safety::ExpectedFileIdentity {
            size: log.source_size?,
            modified_ns: None,
            platform_volume_id,
            platform_file_id,
            sample_hash: log.source_quick_hash.clone(),
            full_hash: log.source_full_hash.clone(),
        });
    }
    if is_directory_move_log(log) {
        let size = fs::symlink_metadata(&log.path_after)
            .ok()
            .map(|metadata| metadata.len())
            .or(log.source_size)?;
        return Some(crate::fs_safety::ExpectedFileIdentity {
            size,
            modified_ns: None,
            platform_volume_id: log.target_platform_volume_id.clone(),
            platform_file_id: log.target_platform_file_id.clone(),
            sample_hash: None,
            full_hash: None,
        });
    }
    let platform_file_id = if restore_volume_relation(log) == RestoreVolumeRelation::SameVolume {
        log.target_platform_file_id
            .clone()
            .or_else(|| log.source_platform_file_id.clone())
    } else {
        None
    };
    Some(crate::fs_safety::ExpectedFileIdentity {
        size: log.source_size?,
        modified_ns: None,
        platform_volume_id: log.target_platform_volume_id.clone(),
        platform_file_id,
        sample_hash: log.source_quick_hash.clone(),
        full_hash: log
            .target_full_hash
            .clone()
            .or_else(|| log.source_full_hash.clone()),
    })
}

/// Identity of the original path after a restore. A copy-commit across
/// volumes may legitimately receive a new file ID, so only a proven
/// SameVolume relation permits the old source ID to be checked.
pub(crate) fn expected_restore_original_identity_from_log(
    log: &OperationLogDto,
) -> Option<crate::fs_safety::ExpectedFileIdentity> {
    if log.operation_type == "replace" && cfg!(target_os = "macos") {
        return Some(crate::fs_safety::ExpectedFileIdentity {
            size: log.source_size?,
            modified_ns: None,
            platform_volume_id: None,
            platform_file_id: None,
            sample_hash: log.source_quick_hash.clone(),
            full_hash: log.source_full_hash.clone(),
        });
    }
    if is_directory_move_log(log) {
        let size = fs::symlink_metadata(&log.path_before)
            .ok()
            .map(|metadata| metadata.len())
            .or(log.source_size)?;
        return Some(crate::fs_safety::ExpectedFileIdentity {
            size,
            modified_ns: None,
            platform_volume_id: log.source_platform_volume_id.clone(),
            platform_file_id: (restore_volume_relation(log) == RestoreVolumeRelation::SameVolume)
                .then(|| log.source_platform_file_id.clone())
                .flatten(),
            sample_hash: None,
            full_hash: None,
        });
    }
    // Replacement publishes the source through a verified copy/clone commit.
    // Restoring that content to the original source path therefore does not
    // preserve the original source file ID, even when both paths share a
    // volume.  Content and volume identity remain useful; the physical file
    // ID must be left unbound for this transaction shape.
    let preserve_source_file_id = log.operation_type != "replace";
    Some(crate::fs_safety::ExpectedFileIdentity {
        size: log.source_size?,
        modified_ns: None,
        platform_volume_id: log.source_platform_volume_id.clone(),
        platform_file_id: (preserve_source_file_id
            && restore_volume_relation(log) == RestoreVolumeRelation::SameVolume)
            .then(|| log.source_platform_file_id.clone())
            .flatten(),
        sample_hash: log.source_quick_hash.clone(),
        full_hash: log
            .source_full_hash
            .clone()
            .or_else(|| log.target_full_hash.clone()),
    })
}

pub(crate) fn expected_restore_final_target_identity_from_log(
    log: &OperationLogDto,
) -> Option<crate::fs_safety::ExpectedFileIdentity> {
    expected_restore_original_identity_from_log(log)
}

fn capture_recovery_identity(
    path: &Path,
    expected: &crate::fs_safety::ExpectedFileIdentity,
) -> Result<crate::fs_safety::ExpectedFileIdentity, crate::fs_safety::IdentityError> {
    if expected.full_hash.is_some() || expected.sample_hash.is_some() {
        crate::fs_safety::capture_identity(path, None)
    } else {
        crate::fs_safety::capture_namespace_identity_only(path, None)
    }
}

pub(crate) fn journal_identity_matches(log: &OperationLogDto, path: &Path) -> Result<bool, ()> {
    let Some(expected) = expected_identity_from_log(log) else {
        return Err(());
    };
    let capture = capture_recovery_identity(path, &expected);
    capture
        .map(|actual| crate::fs_safety::recovery_identity_matches(&expected, &actual))
        .map_err(|_| ())
}

pub(crate) fn journal_target_identity_matches(
    log: &OperationLogDto,
    path: &Path,
) -> Result<bool, ()> {
    if is_directory_move_log(log) {
        let actual =
            crate::fs_safety::capture_namespace_identity_only(path, None).map_err(|_| ())?;
        if log.target_platform_volume_id.as_deref() != actual.platform_volume_id.as_deref()
            || log.target_platform_file_id.as_deref() != actual.platform_file_id.as_deref()
        {
            return Ok(false);
        }
        let Some(expected_tree) = directory_move_tree_fingerprint(log) else {
            return Err(());
        };
        let actual_tree =
            crate::file_ops::manual_transfer::directory_tree_snapshot(path).map_err(|_| ())?;
        return Ok(actual_tree == expected_tree);
    }
    let Some(size) = log.source_size else {
        return Err(());
    };
    let expected = if log.operation_type == "replace" {
        // Replacement target fields describe the retained old destination;
        // the visible target holds the original source after publication.
        crate::fs_safety::ExpectedFileIdentity {
            size,
            modified_ns: None,
            platform_volume_id: None,
            platform_file_id: None,
            sample_hash: log.source_quick_hash.clone(),
            full_hash: log.source_full_hash.clone(),
        }
    } else {
        crate::fs_safety::ExpectedFileIdentity {
            size,
            modified_ns: if log.target_platform_file_id.is_none() {
                log.source_modified_ns
                    .as_deref()
                    .and_then(|value| value.parse::<i128>().ok())
            } else {
                None
            },
            platform_volume_id: log.target_platform_volume_id.clone(),
            platform_file_id: log.target_platform_file_id.clone(),
            sample_hash: log.source_quick_hash.clone(),
            full_hash: log
                .target_full_hash
                .clone()
                .or_else(|| log.source_full_hash.clone()),
        }
    };
    let capture = capture_recovery_identity(path, &expected);
    capture
        .map(|actual| crate::fs_safety::recovery_identity_matches(&expected, &actual))
        .map_err(|_| ())
}

pub(crate) fn operation_restore_identity_result(
    log: &OperationLogDto,
    path: &Path,
) -> Result<(), crate::recovery::RecoveryFailure> {
    let Some(expected) = expected_restore_identity_from_log(log) else {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::RestoreSourceIdentityUnreadable,
            "restore identity is incomplete",
        ));
    };
    let actual = capture_recovery_identity(path, &expected).map_err(|error| {
        let code = match error {
            crate::fs_safety::IdentityError::SourceMissing
            | crate::fs_safety::IdentityError::Io(_) => {
                crate::recovery::RecoveryErrorCode::RestoreSourceIdentityUnreadable
            }
            crate::fs_safety::IdentityError::Symlink
            | crate::fs_safety::IdentityError::UnsupportedFileType
            | crate::fs_safety::IdentityError::DirectoryManifestNameEncodingFailed
            | crate::fs_safety::IdentityError::Cancelled
            | crate::fs_safety::IdentityError::ContentReadRejected(_) => {
                crate::recovery::RecoveryErrorCode::RestoreSourceIdentityUnreadable
            }
        };
        crate::recovery::RecoveryFailure::new(
            code,
            format!("restore source identity could not be read: {error}"),
        )
    })?;
    if !crate::fs_safety::identity_matches(&expected, &actual) {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::RestoreSourceIdentityMismatch,
            "restore source identity does not match the operation journal",
        ));
    }
    if is_directory_move_log(log) {
        let Some(expected_tree) = directory_move_tree_fingerprint(log) else {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::RestoreSourceIdentityUnreadable,
                "directory restore source tree proof is missing",
            ));
        };
        let actual_tree =
            crate::file_ops::manual_transfer::directory_tree_snapshot(path).map_err(|error| {
                crate::recovery::RecoveryFailure::new(
                    crate::recovery::RecoveryErrorCode::RestoreSourceIdentityUnreadable,
                    format!("directory restore source tree could not be verified: {error}"),
                )
            })?;
        if actual_tree != expected_tree {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::RestoreSourceIdentityMismatch,
                "directory restore source tree no longer matches its committed journal proof",
            ));
        }
    }
    Ok(())
}

/// The old destination of a replacement is retained at a deterministic
/// private path. macOS deliberately records namespace identity here so a
/// clone/replace does not trigger another full content scan. Other platforms
/// may persist a content hash instead. At least one complete identity proof
/// must be present before the retained object can participate in restore.
pub(crate) fn replacement_backup_identity_result(
    log: &OperationLogDto,
    path: &Path,
) -> Result<(), crate::recovery::RecoveryFailure> {
    let expected_hash = log.target_full_hash.as_deref();
    let has_physical_identity =
        log.target_platform_volume_id.is_some() && log.target_platform_file_id.is_some();
    if expected_hash.is_none() && !has_physical_identity {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
            "replacement backup identity is incomplete",
        ));
    }
    let actual = crate::fs_safety::capture_namespace_identity(path, None).map_err(|error| {
        crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
            format!("replacement backup identity could not be read: {error}"),
        )
    })?;
    let hash_matches =
        expected_hash.is_none_or(|expected| actual.full_hash.as_deref() == Some(expected));
    let matches = hash_matches
        && log
            .target_platform_volume_id
            .as_deref()
            .is_none_or(|expected| actual.platform_volume_id.as_deref() == Some(expected))
        && log
            .target_platform_file_id
            .as_deref()
            .is_none_or(|expected| actual.platform_file_id.as_deref() == Some(expected));
    if !matches {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch,
            "replacement backup identity does not match the operation journal",
        ));
    }
    Ok(())
}

pub(crate) fn replacement_backup_path_for_log(log: &OperationLogDto) -> PathBuf {
    crate::fs_safety::atomic_move::replacement_backup_path(
        Path::new(&log.path_before),
        Path::new(&log.path_after),
    )
}

pub(crate) fn operation_restore_original_identity_result(
    log: &OperationLogDto,
    path: &Path,
) -> Result<(), crate::recovery::RecoveryFailure> {
    let Some(expected) = expected_restore_original_identity_from_log(log) else {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
            "restore original-path identity is incomplete",
        ));
    };
    let actual = capture_recovery_identity(path, &expected).map_err(|error| {
        crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
            format!("restore original-path identity could not be read: {error}"),
        )
    })?;
    if !crate::fs_safety::identity_matches(&expected, &actual) {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch,
            "restore original-path identity does not match the operation journal",
        ));
    }
    if is_directory_move_log(log) {
        let Some(expected_tree) = directory_move_tree_fingerprint(log) else {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                "directory restore tree proof is missing",
            ));
        };
        let actual_tree =
            crate::file_ops::manual_transfer::directory_tree_snapshot(path).map_err(|error| {
                crate::recovery::RecoveryFailure::new(
                    crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                    format!("restored directory tree could not be verified: {error}"),
                )
            })?;
        if actual_tree != expected_tree {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch,
                "restored directory tree does not match its committed journal proof",
            ));
        }
    }
    Ok(())
}

pub(crate) fn operation_restore_identity_matches(
    log: &OperationLogDto,
    path: &Path,
) -> Result<bool, ()> {
    match operation_restore_identity_result(log, path) {
        Ok(()) => Ok(true),
        Err(failure) => match failure.code {
            crate::recovery::RecoveryErrorCode::RestoreSourceIdentityMismatch => Ok(false),
            _ => Err(()),
        },
    }
}

pub(crate) fn operation_restore_original_identity_matches(
    log: &OperationLogDto,
    path: &Path,
) -> Result<bool, ()> {
    match operation_restore_original_identity_result(log, path) {
        Ok(()) => Ok(true),
        Err(failure) => match failure.code {
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch => Ok(false),
            _ => Err(()),
        },
    }
}

pub(crate) fn restore_claim_identity_matches(
    log: &OperationLogDto,
    path: &Path,
) -> Result<bool, ()> {
    let Some(size) = log.source_size else {
        return Err(());
    };
    let full_hash = log
        .restore_claim_full_hash
        .clone()
        .or_else(|| log.source_full_hash.clone());
    let replacement_same_volume = log.operation_type == "replace"
        && restore_volume_relation(log) == RestoreVolumeRelation::SameVolume;
    let platform_volume_id = if log.operation_type == "replace" {
        log.restore_claim_platform_volume_id.clone().or_else(|| {
            replacement_same_volume
                .then(|| log.source_platform_volume_id.clone())
                .flatten()
        })
    } else {
        log.restore_claim_platform_volume_id
            .clone()
            .or_else(|| log.target_platform_volume_id.clone())
    };
    let platform_file_id = if log.operation_type == "replace" {
        log.restore_claim_platform_file_id.clone().or_else(|| {
            replacement_same_volume
                .then(|| log.source_platform_file_id.clone())
                .flatten()
        })
    } else {
        log.restore_claim_platform_file_id.clone().or_else(|| {
            (restore_volume_relation(log) == RestoreVolumeRelation::SameVolume)
                .then(|| {
                    log.target_platform_file_id
                        .clone()
                        .or_else(|| log.source_platform_file_id.clone())
                })
                .flatten()
        })
    };
    let expected = crate::fs_safety::ExpectedFileIdentity {
        size,
        modified_ns: None,
        platform_volume_id,
        platform_file_id,
        sample_hash: log.source_quick_hash.clone(),
        full_hash,
    };
    let capture = capture_recovery_identity(path, &expected);
    capture
        .map(|actual| crate::fs_safety::recovery_identity_matches(&expected, &actual))
        .map_err(|_| ())
}

pub(crate) fn operation_restore_final_identity_check(
    log: &OperationLogDto,
) -> Result<(), crate::recovery::RecoveryFailure> {
    if log.operation_type == "replace" {
        operation_restore_original_identity_result(log, Path::new(&log.path_before))?;
        replacement_backup_identity_result(log, Path::new(&log.path_after))?;
        return Ok(());
    }
    let source = Path::new(&log.path_after);
    match fs::symlink_metadata(source) {
        Ok(_) => {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::RestoreSourcePathReappeared,
                "restore source path reappeared after the filesystem commit; preserve the restore claim and review both paths",
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::RestoreSourceIdentityUnreadable,
                format!("restore source absence could not be verified: {error}"),
            ))
        }
    }

    let target = Path::new(&log.path_before);
    let expected = expected_restore_final_target_identity_from_log(log).ok_or_else(|| {
        crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
            "restore target identity is incomplete",
        )
    })?;
    let actual = capture_recovery_identity(target, &expected).map_err(|error| {
        crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
            format!("restore target identity could not be read: {error}"),
        )
    })?;
    if !crate::fs_safety::identity_matches(&expected, &actual) {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch,
            "restore target identity does not match the operation journal",
        ));
    }
    Ok(())
}

pub(crate) fn validate_operation_restore_final_identity(
    log: &OperationLogDto,
) -> Result<(), String> {
    operation_restore_final_identity_check(log).map_err(|failure| failure.message())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationJournalPathState {
    Missing,
    Matches,
    Mismatch,
    Unreadable,
}

pub(crate) fn operation_journal_path_state(
    path: &Path,
    identity_matches: impl FnOnce(&Path) -> Result<bool, ()>,
) -> OperationJournalPathState {
    classify_operation_journal_path_state(fs::symlink_metadata(path), || identity_matches(path))
}

pub(crate) fn classify_operation_journal_path_state(
    metadata: Result<fs::Metadata, io::Error>,
    identity_matches: impl FnOnce() -> Result<bool, ()>,
) -> OperationJournalPathState {
    match metadata {
        Ok(_) => match identity_matches() {
            Ok(true) => OperationJournalPathState::Matches,
            Ok(false) => OperationJournalPathState::Mismatch,
            Err(()) => OperationJournalPathState::Unreadable,
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => OperationJournalPathState::Missing,
        Err(_) => OperationJournalPathState::Unreadable,
    }
}
