use super::*;

#[derive(Debug, Clone)]
pub(crate) struct PreparedOperation {
    pub(crate) fingerprint: FileIdentityFingerprint,
    #[cfg(any(test, feature = "native-qa"))]
    pub(crate) source_path: PathBuf,
    pub(crate) claim_path: PathBuf,
    pub(crate) claim_created_at: String,
    pub(crate) journal_log: OperationLogDto,
}

pub(crate) fn apply_source_fingerprint(
    log: &mut OperationLogDto,
    fingerprint: &FileIdentityFingerprint,
) {
    log.source_size = Some(fingerprint.size);
    log.source_modified_ns = fingerprint.modified_ns.map(|value| value.to_string());
    log.source_platform_file_id = fingerprint.platform_file_id.clone();
    log.source_platform_volume_id = fingerprint.platform_volume_id.clone();
    log.source_quick_hash = fingerprint.quick_hash.clone();
    log.source_full_hash = fingerprint.full_hash.clone();
}

pub(crate) fn apply_target_fingerprint(
    log: &mut OperationLogDto,
    fingerprint: &FileIdentityFingerprint,
) {
    log.target_platform_file_id = fingerprint.platform_file_id.clone();
    log.target_platform_volume_id = fingerprint.platform_volume_id.clone();
    log.target_full_hash = fingerprint.full_hash.clone();
}

/// Refresh the two durable identities that make a completed replacement
/// restorable. `source_*` is the object published at `path_after`, while
/// `target_*` is the retained old destination at the deterministic backup
/// path. The pre-commit source proof remains in the claim fields and source
/// claim path, so no additional journal columns are required.
pub(crate) fn refresh_replacement_journal_identities(
    log: &mut OperationLogDto,
) -> Result<(), String> {
    if log.operation_type != "replace" {
        return Ok(());
    }
    let published = file_operation_fingerprint(Path::new(&log.path_after), "replace")?;
    let backup_path = crate::fs_safety::atomic_move::replacement_backup_path(
        Path::new(&log.path_before),
        Path::new(&log.path_after),
    );
    let backup = file_operation_fingerprint(&backup_path, "replace")?;
    apply_source_fingerprint(log, &published);
    apply_target_fingerprint(log, &backup);
    Ok(())
}

pub(crate) fn expected_identity_from_fingerprint(
    fingerprint: &FileIdentityFingerprint,
) -> crate::fs_safety::ExpectedFileIdentity {
    crate::fs_safety::ExpectedFileIdentity {
        size: fingerprint.size,
        modified_ns: fingerprint.modified_ns,
        platform_volume_id: fingerprint.platform_volume_id.clone(),
        platform_file_id: fingerprint.platform_file_id.clone(),
        sample_hash: fingerprint.quick_hash.clone(),
        full_hash: fingerprint.full_hash.clone(),
    }
}

pub(crate) fn expected_identity_from_log(
    log: &OperationLogDto,
) -> Option<crate::fs_safety::ExpectedFileIdentity> {
    Some(crate::fs_safety::ExpectedFileIdentity {
        size: log.source_size?,
        modified_ns: log
            .source_modified_ns
            .as_deref()
            .and_then(|value| value.parse::<i128>().ok()),
        platform_volume_id: log.source_platform_volume_id.clone(),
        platform_file_id: log.source_platform_file_id.clone(),
        sample_hash: log.source_quick_hash.clone(),
        full_hash: log.source_full_hash.clone(),
    })
}

pub(crate) fn persist_pending_operation_journal(
    db: &Database,
    request: &ExecuteMovesRequest,
    batch_id: &str,
    created_at: &str,
) -> Result<std::collections::HashMap<String, PreparedOperation>, String> {
    let logs = request
        .operations
        .iter()
        .enumerate()
        .map(|(index, operation)| {
            if operation.id.starts_with("op-transfer-") {
                manual_transfer::revalidate(operation)?;
            }
            if operation.id.starts_with("op-manual-move-") {
                manual_move::revalidate(operation)?;
            }
            #[cfg(target_os = "macos")]
            ensure_macos_mutation_eligible_before_journal(operation)?;
            // PREPARED is a namespace-only durability record on macOS.  Copy
            // and Duplicate still obtain full content evidence later, after
            // provider materialization/coordination and after the journal row
            // exists; preparation itself must not read iCloud/File Provider
            // bytes.
            let is_directory = fs::symlink_metadata(Path::new(&operation.source_path))
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            let fingerprint = if cfg!(target_os = "macos") || is_directory {
                file_namespace_fingerprint(Path::new(&operation.source_path))
            } else {
                file_identity_fingerprint(Path::new(&operation.source_path))
            }
            .map_err(|error| format!("cannot journal source identity: {error}"))?;
            let claim_path = crate::fs_safety::source_claim::planned_claim_path(
                Path::new(&operation.source_path),
                &operation.id,
            )
            .map_err(|error| format!("cannot plan source claim: {error}"))?;
            // This unique, journaled path is also the PortableSourceRetirement
            // recovery slot. A target-first portable copy may have a verified
            // target while no safe source claim primitive is available; in
            // that case the slot remains absent, the original source remains
            // authoritative, and recovery retries from path_before after an
            // identity check rather than deleting by the planned pathname.
            let mut log = make_operation_log(
                batch_id,
                created_at,
                index,
                operation,
                "pending",
                None,
                operation.target_path.clone(),
            );
            apply_source_fingerprint(&mut log, &fingerprint);
            log.source_claim_path = Some(normalize_path(&claim_path));
            log.claim_created_at = Some(created_at.to_string());
            log.claim_platform_file_id = fingerprint.platform_file_id.clone();
            log.claim_platform_volume_id = fingerprint.platform_volume_id.clone();
            log.claim_full_hash = fingerprint.full_hash.clone();
            log.operation_phase = "prepared".to_string();
            let journal_log = log.clone();
            Ok((
                log,
                PreparedOperation {
                    fingerprint,
                    #[cfg(any(test, feature = "native-qa"))]
                    source_path: PathBuf::from(&operation.source_path),
                    claim_path,
                    claim_created_at: created_at.to_string(),
                    journal_log,
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let fingerprints = logs
        .iter()
        .zip(request.operations.iter())
        .map(|((_, prepared), operation)| (operation.id.clone(), (*prepared).clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let logs = logs.into_iter().map(|(log, _)| log).collect::<Vec<_>>();
    db.save_operation_logs(batch_id, &logs).map_err(|error| {
        format!("failed to persist operation journal before execution: {error}")
    })?;
    #[cfg(any(test, feature = "native-qa"))]
    for prepared in fingerprints.values() {
        crate::fs_safety::source_claim::run_claim_test_hook(
            crate::fs_safety::source_claim::ClaimTestPoint::AfterJournalPreparedBeforeClaim,
            &prepared.source_path,
            &prepared.claim_path,
        );
    }
    Ok(fingerprints)
}

#[cfg(target_os = "macos")]
fn ensure_macos_mutation_eligible_before_journal(
    operation: &OperationPreviewRequest,
) -> Result<(), String> {
    let source = Path::new(&operation.source_path);
    // Safe Trash and Permanent Delete are source-local transactions. Their
    // display/journal target strings are not filesystem destinations, and the
    // lower permanent-delete authority coordinates `source -> source` before
    // claiming the entry into its private quarantine. Preserve the same
    // source-parent eligibility boundary here rather than interpreting the
    // sentinel target as a path.
    let target_parent = if matches!(
        operation.operation_type.as_str(),
        "move_to_trash" | "permanent_delete"
    ) {
        source
            .parent()
            .ok_or_else(|| "macos mutation rejected: mac_source_identity_changed".to_string())?
            .to_path_buf()
    } else {
        let target_parent = Path::new(&operation.target_path)
            .parent()
            .ok_or_else(|| "macos mutation rejected: mac_target_parent_changed".to_string())?;
        nearest_existing_macos_target_parent(target_parent)?
    };
    crate::platform::macos::mutation::ensure_path_eligible(source, &target_parent)
        .map_err(|code| format!("macos mutation rejected: {code}"))
}

/// PREPARED must be durable before recursive transfers create destination
/// directories. Validate the existing route prefix here; execution rechecks
/// the concrete parent after the journal and creates the missing suffix with
/// the no-links path guard.
#[cfg(target_os = "macos")]
fn nearest_existing_macos_target_parent(target_parent: &Path) -> Result<PathBuf, String> {
    use std::path::Component;

    if !target_parent.is_absolute()
        || target_parent
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err("macos mutation rejected: mac_filesystem_not_supported".to_string());
    }

    for candidate in target_parent.ancestors() {
        let metadata = match std::fs::symlink_metadata(candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("macos mutation rejected: mac_target_parent_changed".to_string()),
        };
        if metadata.file_type().is_symlink() {
            return Err("macos mutation rejected: mac_symlink_not_allowed".to_string());
        }
        if !metadata.is_dir() {
            return Err("macos mutation rejected: mac_target_parent_changed".to_string());
        }

        let verified =
            crate::fs_safety::VerifiedDirectory::open_existing(candidate).map_err(|error| {
                match error {
                    crate::fs_safety::PathGuardError::ReparsePoint => {
                        "macos mutation rejected: mac_symlink_not_allowed".to_string()
                    }
                    _ => "macos mutation rejected: mac_target_parent_changed".to_string(),
                }
            })?;
        return Ok(verified.path().to_path_buf());
    }

    Err("macos mutation rejected: mac_target_parent_changed".to_string())
}

pub(crate) fn make_canceled_operation_log(
    batch_id: &str,
    created_at: &str,
    index: usize,
    operation: &OperationPreviewRequest,
) -> OperationLogDto {
    make_operation_log(
        batch_id,
        created_at,
        index,
        operation,
        "skipped",
        None,
        operation.target_path.clone(),
    )
}

pub(crate) fn make_operation_log(
    batch_id: &str,
    created_at: &str,
    index: usize,
    operation: &OperationPreviewRequest,
    status: &str,
    error_message: Option<String>,
    actual_target_path: String,
) -> OperationLogDto {
    let success = status == "success";
    let trash_operation = operation.operation_type == "move_to_trash";
    let system_trash = trash_operation && actual_target_path == "Recycle Bin";
    let source_preserving_copy = matches!(operation.operation_type.as_str(), "copy" | "duplicate");
    let irreversible_operation = operation.operation_type == "permanent_delete";
    let can_restore = success
        && !system_trash
        && !source_preserving_copy
        && !irreversible_operation
        && operation.operation_type != "create_directory";
    let restore_status = if system_trash && success {
        "unavailable"
    } else {
        "not_restored"
    };
    let restore_error = if system_trash && success {
        Some("Restore from system trash.".to_string())
    } else {
        None
    };
    OperationLogDto {
        id: format!("{batch_id}-{index}-{}", operation.id),
        batch_id: batch_id.to_string(),
        operation_type: operation.operation_type.clone(),
        source_path: operation.source_path.clone(),
        target_path: actual_target_path.clone(),
        old_name: operation.old_name.clone(),
        new_name: operation.new_name.clone(),
        status: status.to_string(),
        error_message,
        created_at: created_at.to_string(),
        can_undo: can_restore,
        path_before: operation.source_path.clone(),
        path_after: actual_target_path,
        name_before: operation.old_name.clone(),
        name_after: operation.new_name.clone(),
        can_restore,
        restored_at: None,
        restore_status: restore_status.to_string(),
        restore_error,
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
        operation_phase: if status == "pending" {
            "prepared".to_string()
        } else {
            "completed".to_string()
        },
        claim_created_at: None,
        claim_platform_file_id: None,
        claim_platform_volume_id: None,
        claim_full_hash: None,
        restore_claim_path: None,
        restore_phase: "idle".to_string(),
        restore_claim_created_at: None,
        restore_claim_platform_file_id: None,
        restore_claim_platform_volume_id: None,
        restore_claim_full_hash: None,
    }
}

pub(crate) fn append_operation_log_error(log: &mut OperationLogDto, message: String) {
    log.error_message = Some(match log.error_message.take() {
        Some(existing) if !existing.trim().is_empty() => format!("{existing}; {message}"),
        _ => message,
    });
}
pub(crate) fn operation_phase_for_log(log: &OperationLogDto) -> &'static str {
    if log.status == "success" {
        return "completed";
    }
    match log.operation_phase.as_str() {
        "prepared" => "prepared",
        "source_claimed" => "source_claimed",
        "copying" => "copying",
        "target_committed" => "target_committed",
        "source_cleanup_pending" => "source_cleanup_pending",
        "manual_review" => "manual_review",
        "rolled_back" => "rolled_back",
        _ if log.status == "pending" => "prepared",
        _ => "rolled_back",
    }
}

pub(crate) fn current_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
