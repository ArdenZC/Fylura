use super::*;
use crate::fs_safety::{DirectoryIdentity, VerifiedDirectory};

pub(crate) struct MoveSnapshot {
    pub directory: String,
    pub identity: DirectoryIdentity,
    pub intent_id: String,
}

/// Read-only snapshot, using the existing no-link directory traversal. No
/// namespace probe, directory creation, content read or copy fallback.
pub(crate) fn snapshot(
    file_id: &str,
    source: &Path,
    directory: &Path,
    name: &str,
) -> Result<MoveSnapshot, String> {
    if !directory.is_absolute()
        || directory
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    {
        return Err("manual_move_destination_invalid".into());
    }
    validate_safe_file_name(name)?;
    let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.permissions().readonly()
    {
        return Err("manual_move_source_unavailable".into());
    }
    let source_parent = source.parent().ok_or("manual_move_source_unavailable")?;
    // Verify before canonicalizing; otherwise an alias could erase a link escape.
    let source_directory =
        VerifiedDirectory::open_existing(source_parent).map_err(|error| error.to_string())?;
    let selected =
        VerifiedDirectory::open_existing(directory).map_err(|error| error.to_string())?;
    let canonical = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let normalized = normalize_path(&canonical);
    #[cfg(windows)]
    let normalized = crate::path_identity::normalize_extended_windows_path_text(&normalized)
        .map_err(str::to_string)?
        .unwrap_or(normalized);
    let verified = VerifiedDirectory::open_existing(Path::new(&normalized))
        .map_err(|error| error.to_string())?;
    if selected.identity() != verified.identity() {
        return Err("operation_preview_stale".into());
    }
    ensure_general_file_operation_allowed(source)?;
    ensure_general_file_operation_allowed(Path::new(&normalized))?;
    let source_identity = file_namespace_fingerprint(source)?;
    let source_volume = source_identity
        .platform_volume_id
        .as_deref()
        .filter(|_| source_identity.platform_file_id.is_some())
        .ok_or("manual_move_source_unavailable")?;
    if source_volume != verified.identity().volume_id {
        return Err("manual_move_same_volume_required".into());
    }
    if source_directory.identity() == verified.identity() {
        return Err("manual_move_different_directory_required".into());
    }
    #[cfg(target_os = "macos")]
    if !matches!(
        crate::platform::macos::strategy::select(source, verified.path()),
        crate::platform::macos::strategy::MacMutationStrategy::LocalApfs
    ) {
        return Err("manual_move_local_same_volume_required".into());
    }
    let digest = blake3::hash(
        format!(
            "manual-move:v1\0{file_id}\0{}\0{normalized}\0{name}\0{source_identity:?}\0{:?}",
            normalize_path(source),
            verified.identity()
        )
        .as_bytes(),
    );
    Ok(MoveSnapshot {
        directory: normalized,
        identity: verified.identity().clone(),
        intent_id: format!("op-manual-move-{}", digest.to_hex()),
    })
}

pub(crate) fn revalidate(operation: &OperationPreviewRequest) -> Result<MoveSnapshot, String> {
    let parent = Path::new(&operation.target_path)
        .parent()
        .ok_or("operation_preview_stale")?;
    let current = snapshot(
        &operation.file_id,
        Path::new(&operation.source_path),
        parent,
        &operation.new_name,
    )?;
    if current.intent_id != operation.id {
        return Err("operation_preview_stale".into());
    }
    Ok(current)
}

pub(crate) fn execute(
    operation: &OperationPreviewRequest,
    context: OperationExecutionContext<'_>,
) -> Result<FileOperationResult, FileMutationError> {
    let current = revalidate(operation)?;
    let source = validate_source_path(Path::new(&operation.source_path))?;
    let target = validate_target_path(Path::new(&operation.target_path))?;
    crate::fs_safety::atomic_move::atomic_move_with_verified_destination(
        &source,
        &target,
        context.expected_identity,
        context.planned_claim_path,
        context.cancel_flag,
        context.phase_observer,
        crate::fs_safety::atomic_move::AtomicMoveOperation::Move,
        context.actual_path_observer,
        Some(&current.identity),
    )
    .map_err(FileMutationError::Atomic)?;
    Ok(FileOperationResult {
        operation: "move".into(),
        source_path: normalize_path(&source),
        target_path: normalize_path(&target),
    })
}
