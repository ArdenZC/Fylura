use super::*;

use crate::fs_safety::VerifiedDirectory;
use std::path::Component;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum FileOpError {
    #[error("Source file does not exist.")]
    SourceMissing,
    #[error("Source path is not a regular file.")]
    SourceNotFile,
    #[error("Source and target paths must be absolute.")]
    RelativePath,
    #[error("Target parent directory does not exist.")]
    TargetParentMissing,
    #[error("Target file already exists. Fylura will not overwrite files.")]
    TargetExists,
    #[error("The requested file name is not safe.")]
    UnsafeFileName,
    #[error("Operation rejected because it touches a protected system location: {0}")]
    ProtectedPath(String),
    #[error("Target path contains unsafe parent traversal.")]
    UnsafePathTraversal,
    #[error("File operation failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug)]
pub(crate) enum FileMutationError {
    Validation(String),
    Atomic(crate::fs_safety::AtomicMoveError),
}

impl From<String> for FileMutationError {
    fn from(error: String) -> Self {
        Self::Validation(error)
    }
}

impl From<crate::fs_safety::AtomicMoveError> for FileMutationError {
    fn from(error: crate::fs_safety::AtomicMoveError) -> Self {
        Self::Atomic(error)
    }
}

impl std::fmt::Display for FileMutationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(error) => formatter.write_str(error),
            Self::Atomic(error) => error.fmt(formatter),
        }
    }
}

impl FileMutationError {
    pub(crate) fn journal_phase(&self) -> &'static str {
        match self {
            Self::Validation(_) => "rolled_back",
            Self::Atomic(error) => match error {
                crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown
                | crate::fs_safety::AtomicMoveError::TargetCommittedIdentityMismatch => {
                    "target_committed"
                }
                crate::fs_safety::AtomicMoveError::TargetCommittedSourceCleanupPending
                | crate::fs_safety::AtomicMoveError::TargetCommittedSourceDeleteFailed(_) => {
                    "source_cleanup_pending"
                }
                _ => match error.commit_state() {
                    crate::fs_safety::AtomicMoveCommitState::RolledBack => "rolled_back",
                    crate::fs_safety::AtomicMoveCommitState::SourceClaimed => "source_claimed",
                    crate::fs_safety::AtomicMoveCommitState::TargetCommitted => "target_committed",
                    crate::fs_safety::AtomicMoveCommitState::SourceCleanupPending => {
                        "source_cleanup_pending"
                    }
                    crate::fs_safety::AtomicMoveCommitState::Completed => "completed",
                    crate::fs_safety::AtomicMoveCommitState::ManualReview => "manual_review",
                },
            },
        }
    }

    pub(crate) fn requires_recovery(&self) -> bool {
        match self {
            Self::Validation(_) => false,
            Self::Atomic(error) => !matches!(
                error.commit_state(),
                crate::fs_safety::AtomicMoveCommitState::RolledBack
            ),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        matches!(
            self,
            Self::Atomic(crate::fs_safety::AtomicMoveError::Cancelled)
        )
    }
}

pub(crate) fn rename_file_with_identity(
    source_path: String,
    new_name: String,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    cancel_flag: Option<&AtomicBool>,
    planned_claim_path: Option<&Path>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_callback: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<FileOperationResult, FileMutationError> {
    validate_safe_file_name(&new_name)?;
    let source = validate_namespace_entry_path(&PathBuf::from(source_path))?;
    ensure_not_safe_trash_operation(&source)?;
    let parent = source
        .parent()
        .ok_or(FileOpError::TargetParentMissing)
        .map_err(|error| error.to_string())?;
    let target = parent.join(new_name);
    ensure_not_safe_trash_operation(&target)?;

    if fs::symlink_metadata(&target).is_ok()
        && !(cfg!(target_os = "macos") && same_macos_namespace_entry(&source, &target))
    {
        return Err(FileMutationError::Validation(
            FileOpError::TargetExists.to_string(),
        ));
    }

    ensure_general_file_operation_allowed(&source)?;
    ensure_general_file_operation_allowed(&target)?;
    move_file_no_overwrite_with_identity_for_operation_with_actual_paths(
        &source,
        &target,
        expected_identity,
        planned_claim_path,
        cancel_flag,
        phase_observer,
        crate::fs_safety::atomic_move::AtomicMoveOperation::Rename,
        actual_path_callback,
    )?;

    Ok(FileOperationResult {
        operation: "rename".to_string(),
        source_path: normalize_path(&source),
        target_path: normalize_path(&target),
    })
}

pub(crate) fn create_directory_with_safety(
    parent_path: String,
    name: String,
    expected_parent: Option<&crate::fs_safety::ExpectedFileIdentity>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<FileOperationResult, FileMutationError> {
    validate_safe_file_name(&name)?;
    let parent = VerifiedDirectory::open_existing(Path::new(&parent_path))
        .map_err(|error| FileMutationError::Validation(error.to_string()))?;
    ensure_general_file_operation_allowed(parent.path())?;
    ensure_not_safe_trash_operation(parent.path())?;
    ensure_not_safe_trash_operation(&parent.path().join(&name))?;
    if let Some(expected) = expected_parent {
        let actual = crate::fs_safety::capture_namespace_identity_only(parent.path(), None)
            .map_err(|error| FileMutationError::Validation(error.to_string()))?;
        if !crate::fs_safety::identity_matches(expected, &actual) {
            return Err(FileMutationError::Validation(
                "operation_preview_stale".into(),
            ));
        }
    }
    let child = parent
        .create_child_directory(std::ffi::OsStr::new(&name))
        .map_err(|error| FileMutationError::Validation(error.to_string()))?;
    let target = child.path().to_path_buf();
    parent.sync().map_err(|_| {
        FileMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown,
        )
    })?;
    if let Some(observer) = actual_path_observer {
        observer(parent.path(), &target, None).map_err(FileMutationError::Atomic)?;
    }
    if let Some(observer) = phase_observer {
        observer("target_committed").map_err(FileMutationError::Atomic)?;
    }
    parent.ensure_unchanged().map_err(|_error| {
        FileMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown,
        )
    })?;
    let published = VerifiedDirectory::open_existing(&target).map_err(|_| {
        FileMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown,
        )
    })?;
    if published.identity() != child.identity() {
        return Err(FileMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown,
        ));
    }
    Ok(FileOperationResult {
        operation: "create_directory".into(),
        source_path: normalize_path(parent.path()),
        target_path: normalize_path(&target),
    })
}

pub(crate) fn validate_source_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(FileOpError::RelativePath.to_string());
    }
    if path.to_string_lossy().contains('\0')
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(FileOpError::UnsafePathTraversal.to_string());
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            FileOpError::SourceMissing.to_string()
        } else {
            FileOpError::Io(error).to_string()
        }
    })?;
    if metadata.file_type().is_symlink() && !cfg!(target_os = "macos") {
        return Err(FileOpError::ProtectedPath(normalize_path(path)).to_string());
    }
    if !metadata.is_file() && !metadata.is_dir() && !metadata.file_type().is_symlink() {
        return Err(FileOpError::SourceNotFile.to_string());
    }
    if !cfg!(target_os = "macos") && !metadata.is_file() {
        return Err(FileOpError::SourceNotFile.to_string());
    }

    let source = if cfg!(target_os = "macos") {
        canonicalize_macos_namespace_path(path)?
    } else {
        path.canonicalize()
            .map_err(|error| FileOpError::Io(error).to_string())?
    };
    ensure_general_file_operation_allowed(&source)?;

    Ok(source)
}

pub(crate) fn validate_target_path(path: &Path) -> Result<PathBuf, String> {
    validate_target_path_with_parent_policy(path, false)
}

pub(crate) fn validate_target_path_with_parent_policy(
    path: &Path,
    allow_create_parent: bool,
) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(FileOpError::RelativePath.to_string());
    }
    if path.to_string_lossy().contains('\0') {
        return Err(FileOpError::UnsafePathTraversal.to_string());
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(FileOpError::UnsafePathTraversal.to_string());
    }
    ensure_not_safe_trash_operation(path)?;
    if fs::symlink_metadata(path).is_ok() {
        return Err(FileOpError::TargetExists.to_string());
    }

    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(FileOpError::UnsafeFileName)
        .map_err(|error| error.to_string())?;
    validate_safe_file_name(name)?;

    let parent = path
        .parent()
        .ok_or(FileOpError::TargetParentMissing)
        .map_err(|error| error.to_string())?;
    let existing_ancestor = canonicalize_nearest_existing_ancestor(parent)?;
    ensure_general_file_operation_allowed(&existing_ancestor)?;
    ensure_not_safe_trash_operation(&existing_ancestor)?;
    if !parent.exists() && !allow_create_parent {
        return Err(FileOpError::TargetParentMissing.to_string());
    }
    // Any future macOS mutation surface is limited to existing,
    // descriptor-verifiable parents. Creating a parent inside an unverified
    // cloud/provider or mount namespace would create a side effect before the
    // source/target eligibility gate can run.
    if cfg!(target_os = "macos") && !parent.exists() {
        return Err(FileOpError::TargetParentMissing.to_string());
    }
    // The verified chain builder is the single parent-creation boundary.  A
    // second path-based call here used to reopen the same chain and widened
    // the TOCTOU window on Windows.
    crate::fs_safety::create_directory_chain_no_links(parent)
        .map_err(|error| format!("target parent rejected: {error}"))?;
    let parent = parent
        .canonicalize()
        .map_err(|_| FileOpError::TargetParentMissing.to_string())?;
    ensure_general_file_operation_allowed(&parent)?;
    ensure_not_safe_trash_operation(&parent)?;

    Ok(parent.join(name))
}

pub(crate) fn canonicalize_nearest_existing_ancestor(path: &Path) -> Result<PathBuf, String> {
    let ancestor = path
        .ancestors()
        .find(|ancestor| ancestor.exists())
        .ok_or(FileOpError::TargetParentMissing)
        .map_err(|error| error.to_string())?;
    ancestor
        .canonicalize()
        .map_err(|error| FileOpError::Io(error).to_string())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn move_file_with_parent_policy_with_cancel_and_identity(
    source_path: String,
    target_path: String,
    allow_create_parent: bool,
    cancel_flag: Option<&AtomicBool>,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<FileOperationResult, FileMutationError> {
    let source = validate_source_path(&PathBuf::from(source_path))?;
    let target =
        validate_target_path_with_parent_policy(&PathBuf::from(target_path), allow_create_parent)?;

    ensure_general_file_operation_allowed(&source)?;
    ensure_general_file_operation_allowed(&target)?;
    ensure_not_safe_trash_operation(&source)?;
    ensure_not_safe_trash_operation(&target)?;
    move_file_no_overwrite_with_identity_for_operation_with_actual_paths(
        &source,
        &target,
        expected_identity,
        planned_claim_path,
        cancel_flag,
        phase_observer,
        crate::fs_safety::atomic_move::AtomicMoveOperation::Move,
        actual_path_observer,
    )?;
    Ok(FileOperationResult {
        operation: "move".to_string(),
        source_path: normalize_path(&source),
        target_path: normalize_path(&target),
    })
}

pub(crate) fn move_to_trash_with_safety(
    source_path: String,
    app_data_dir: Option<&Path>,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    operation_id: &str,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<FileOperationResult, FileMutationError> {
    let source = validate_cleanup_trash_source(&PathBuf::from(source_path), app_data_dir)?;
    let (actual_source, target) = move_path_to_system_trash_with_safety(
        &source,
        expected_identity,
        planned_claim_path,
        operation_id,
        phase_observer,
        actual_path_observer,
    )?;

    Ok(FileOperationResult {
        operation: "move_to_trash".to_string(),
        source_path: normalize_path(&actual_source),
        target_path: normalize_path(&target),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn copy_file_with_identity(
    source_path: String,
    target_path: String,
    operation: String,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    cancel_flag: Option<&AtomicBool>,
    planned_claim_path: Option<&Path>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<FileOperationResult, FileMutationError> {
    let source = validate_source_path(&PathBuf::from(source_path))?;
    let target = validate_target_path(&PathBuf::from(target_path))?;
    ensure_general_file_operation_allowed(&source)?;
    ensure_general_file_operation_allowed(&target)?;
    ensure_not_safe_trash_operation(&source)?;
    ensure_not_safe_trash_operation(&target)?;
    crate::fs_safety::atomic_move::atomic_copy_noreplace_with_claim_path_and_observer_with_actual_paths(
        &source,
        &target,
        &operation,
        expected_identity,
        planned_claim_path,
        cancel_flag,
        phase_observer,
        actual_path_observer,
    )
    .map_err(FileMutationError::Atomic)?;
    Ok(FileOperationResult {
        operation,
        source_path: normalize_path(&source),
        target_path: normalize_path(&target),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn replace_file_with_identity(
    source_path: String,
    target_path: String,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    cancel_flag: Option<&AtomicBool>,
    planned_claim_path: Option<&Path>,
    operation_id: &str,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<FileOperationResult, FileMutationError> {
    let source = validate_source_path(&PathBuf::from(source_path))?;
    let target = validate_existing_target_path(&PathBuf::from(target_path))?;
    ensure_not_safe_trash_operation(&source)?;
    ensure_not_safe_trash_operation(&target)?;
    if normalize_path(&source) == normalize_path(&target)
        || (cfg!(target_os = "macos") && same_macos_namespace_entry(&source, &target))
    {
        return Err(FileMutationError::Validation(
            FileOpError::TargetExists.to_string(),
        ));
    }
    ensure_general_file_operation_allowed(&source)?;
    ensure_general_file_operation_allowed(&target)?;
    crate::fs_safety::atomic_move::atomic_replace_with_claim_path_and_observer_with_actual_paths(
        &source,
        &target,
        expected_identity,
        planned_claim_path,
        cancel_flag,
        operation_id,
        phase_observer,
        actual_path_observer,
    )
    .map_err(FileMutationError::Atomic)?;
    Ok(FileOperationResult {
        operation: "replace".to_string(),
        source_path: normalize_path(&source),
        target_path: normalize_path(&target),
    })
}

pub(crate) fn permanently_delete_with_identity(
    source_path: String,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    cancel_flag: Option<&AtomicBool>,
    planned_claim_path: Option<&Path>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<FileOperationResult, FileMutationError> {
    let source = validate_cleanup_trash_source(&PathBuf::from(source_path), None)?;
    ensure_general_file_operation_allowed(&source)?;
    crate::fs_safety::atomic_move::atomic_permanent_delete_with_claim_path_and_observer_with_actual_paths(
        &source,
        expected_identity,
        planned_claim_path,
        cancel_flag,
        phase_observer,
        actual_path_observer,
    )
    .map_err(FileMutationError::Atomic)?;
    Ok(FileOperationResult {
        operation: "permanent_delete".to_string(),
        source_path: normalize_path(&source),
        target_path: "Permanent deletion quarantine".to_string(),
    })
}

fn move_path_to_system_trash_with_safety(
    source: &Path,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    operation_id: &str,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    mut actual_path_callback: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<(PathBuf, PathBuf), FileMutationError> {
    crate::fs_safety::platform_support::ensure_supported_cleanup_mutation()
        .map_err(|error| FileMutationError::Validation(error.to_string()))?;
    let source_name = source
        .file_name()
        .ok_or_else(|| FileMutationError::Validation(FileOpError::UnsafeFileName.to_string()))?;
    let source_parent = source.parent().ok_or_else(|| {
        FileMutationError::Validation(FileOpError::TargetParentMissing.to_string())
    })?;
    let trash_root = source_parent.join(".zen-canvas-trash");
    let operation_key = blake3::hash(format!("{}\0{}", source.display(), operation_id).as_bytes())
        .to_hex()
        .to_string();
    let target_parent = trash_root.join("operations").join(&operation_key[..24]);
    crate::fs_safety::create_directory_chain_no_links(&target_parent).map_err(|error| {
        FileMutationError::Validation(format!("safe trash parent rejected: {error}"))
    })?;
    ensure_general_file_operation_allowed(&target_parent)?;
    let target = target_parent.join(source_name);
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(FileMutationError::Validation(
            FileOpError::TargetExists.to_string(),
        ));
    }
    let actual_paths = std::cell::RefCell::new(None::<(PathBuf, PathBuf)>);
    let mut actual_path_observer =
        |actual_source: &Path, actual_target: &Path, actual_claim: Option<&Path>| {
            actual_paths.replace(Some((
                actual_source.to_path_buf(),
                actual_target.to_path_buf(),
            )));
            if let Some(observer) = actual_path_callback.as_deref_mut() {
                observer(actual_source, actual_target, actual_claim)?;
            }
            Ok(())
        };
    crate::fs_safety::atomic_move::atomic_move_noreplace_with_claim_path_and_observer_for_operation_with_actual_paths(
        source,
        &target,
        expected_identity,
        planned_claim_path,
        None,
        phase_observer,
        crate::fs_safety::atomic_move::AtomicMoveOperation::Trash,
        Some(&mut actual_path_observer),
    )
    .map_err(FileMutationError::Atomic)?;
    let (actual_source, actual_target) = actual_paths
        .into_inner()
        .unwrap_or_else(|| (source.to_path_buf(), target.clone()));
    Ok((actual_source, actual_target))
}

fn validate_existing_target_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(FileOpError::RelativePath.to_string());
    }
    if path.to_string_lossy().contains('\0')
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(FileOpError::UnsafePathTraversal.to_string());
    }
    ensure_not_safe_trash_operation(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            FileOpError::TargetParentMissing.to_string()
        } else {
            FileOpError::Io(error).to_string()
        }
    })?;
    if !metadata.is_file() && !metadata.is_dir() && !metadata.file_type().is_symlink() {
        return Err(FileOpError::SourceNotFile.to_string());
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(FileOpError::UnsafeFileName)
        .map_err(|error| error.to_string())?;
    validate_safe_file_name(name)?;
    let parent = path
        .parent()
        .ok_or(FileOpError::TargetParentMissing)
        .map_err(|error| error.to_string())?
        .canonicalize()
        .map_err(|error| FileOpError::Io(error).to_string())?;
    ensure_general_file_operation_allowed(&parent)?;
    ensure_not_safe_trash_operation(&parent.join(name))?;
    Ok(parent.join(name))
}

/// Validates a regular file or directory as one namespace object. This is
/// used only for same-volume rename and Safe Trash, which never read a
/// directory tree and bind the mutation to the captured object identity.
pub(crate) fn validate_namespace_entry_path(path: &Path) -> Result<PathBuf, String> {
    validate_cleanup_path_syntax(path)?;
    ensure_general_file_operation_allowed(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            FileOpError::SourceMissing.to_string()
        } else {
            FileOpError::Io(error).to_string()
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(FileOpError::ProtectedPath(normalize_path(path)).to_string());
    }
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(FileOpError::SourceNotFile.to_string());
    }
    let source = path
        .canonicalize()
        .map_err(|error| FileOpError::Io(error).to_string())?;
    ensure_general_file_operation_allowed(&source)?;
    VerifiedDirectory::open_existing(
        source
            .parent()
            .ok_or_else(|| FileOpError::TargetParentMissing.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(source)
}

/// The Safe Trash is journal-owned storage. Ordinary transfers and renames
/// cannot source from or target within it; History Restore has its own
/// identity-checked path that deliberately allows a trashed source.
pub(crate) fn ensure_not_safe_trash_operation(path: &Path) -> Result<(), String> {
    if crate::storage_analyzer::is_zen_canvas_safe_trash_path(path) {
        return Err(FileOpError::ProtectedPath(normalize_path(path)).to_string());
    }
    Ok(())
}

pub(crate) fn validate_cleanup_path_syntax(path: &Path) -> Result<(), String> {
    let path_text = path.to_string_lossy();
    let has_wildcard = if cfg!(windows) {
        path.components().any(|component| match component {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                name.contains('*') || name.contains('?')
            }
            _ => false,
        })
    } else {
        path_text.contains('*') || path_text.contains('?')
    };
    if path.as_os_str().is_empty()
        || path_text.contains('\0')
        || has_wildcard
        || path
            .components()
            .any(|component| component == Component::ParentDir)
        || path.parent().is_none()
        || path.file_name().is_none()
    {
        return Err(FileOpError::UnsafePathTraversal.to_string());
    }
    if !path.is_absolute() {
        return Err(FileOpError::RelativePath.to_string());
    }
    Ok(())
}

pub(crate) fn validate_cleanup_trash_source(
    path: &Path,
    app_data_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    validate_cleanup_path_syntax(path)?;

    // Reject protected roots before any filesystem access. This keeps policy tests
    // deterministic and avoids touching real system trees such as System32.
    ensure_general_file_operation_allowed(path)?;

    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            FileOpError::SourceMissing.to_string()
        } else {
            FileOpError::Io(error).to_string()
        }
    })?;
    if metadata.file_type().is_symlink() && !cfg!(target_os = "macos") {
        return Err(FileOpError::ProtectedPath(normalize_path(path)).to_string());
    }

    let source = if cfg!(target_os = "macos") {
        canonicalize_macos_namespace_path(path)?
    } else {
        path.canonicalize()
            .map_err(|error| FileOpError::Io(error).to_string())?
    };

    // Repeat protection after canonicalization so links and reparse aliases cannot
    // escape the lexical check.
    ensure_general_file_operation_allowed(&source)?;
    ensure_cleanup_operation_allowed(&source, app_data_dir)?;
    Ok(source)
}

pub(crate) fn ensure_cleanup_operation_allowed(
    path: &Path,
    app_data_dir: Option<&Path>,
) -> Result<(), String> {
    if crate::storage_analyzer::is_cleanup_execution_forbidden(path, app_data_dir) {
        return Err(FileOpError::ProtectedPath(normalize_path(path)).to_string());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn canonicalize_macos_namespace_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| FileOpError::TargetParentMissing.to_string())?;
    let name = path
        .file_name()
        .ok_or_else(|| FileOpError::UnsafeFileName.to_string())?;
    let parent = parent
        .canonicalize()
        .map_err(|error| FileOpError::Io(error).to_string())?;
    Ok(parent.join(name))
}

#[cfg(not(target_os = "macos"))]
fn canonicalize_macos_namespace_path(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| FileOpError::Io(error).to_string())
}

#[cfg(target_os = "macos")]
fn same_macos_namespace_entry(left: &Path, right: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(left) = fs::symlink_metadata(left) else {
        return false;
    };
    let Ok(right) = fs::symlink_metadata(right) else {
        return false;
    };
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.file_type().is_symlink() == right.file_type().is_symlink()
        && left.is_file() == right.is_file()
        && left.is_dir() == right.is_dir()
}

#[cfg(not(target_os = "macos"))]
fn same_macos_namespace_entry(_left: &Path, _right: &Path) -> bool {
    false
}

pub(crate) fn validate_safe_file_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains("..")
        || trimmed.ends_with('.')
        || trimmed.ends_with(' ')
        || trimmed.contains('\0')
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.chars().any(|ch| ch.is_control())
    {
        return Err(FileOpError::UnsafeFileName.to_string());
    }

    if cfg!(windows) && validate_windows_filename_semantics(trimmed).is_err() {
        return Err(FileOpError::UnsafeFileName.to_string());
    }

    Ok(())
}

/// The general Windows filename contract used by rename/move targets.
///
/// Keep this limited to the original Windows reserved-device and forbidden
/// character semantics. Extended/verbatim path equivalence has stricter
/// component rules and must use `validate_windows_path_component` instead.
pub(crate) fn validate_windows_filename_semantics(name: &str) -> Result<(), &'static str> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let reserved = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    if reserved.contains(&stem.as_str())
        || name
            .chars()
            .any(|character| matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err("unsafe Windows filename");
    }

    Ok(())
}

/// Strict Windows component policy for extended/verbatim paths. This is not
/// the general rename/move filename contract: the additional dot/space and
/// traversal checks only establish Win32-equivalent path identity.
pub(crate) fn validate_windows_path_component(name: &str) -> Result<(), &'static str> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains("..")
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.contains('\0')
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(|character| character.is_control())
    {
        return Err("unsafe Windows path component");
    }

    validate_windows_filename_semantics(name).map_err(|_| "unsafe Windows path component")
}

pub(crate) fn move_file_no_overwrite(
    source: &Path,
    target: &Path,
) -> Result<(), FileMutationError> {
    move_file_no_overwrite_with_identity(source, target, None, None, None, None)
}

pub(crate) fn move_file_no_overwrite_with_identity(
    source: &Path,
    target: &Path,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel_flag: Option<&AtomicBool>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), FileMutationError> {
    move_file_no_overwrite_with_identity_for_operation(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel_flag,
        phase_observer,
        crate::fs_safety::atomic_move::AtomicMoveOperation::Move,
    )
}

pub(crate) fn move_file_no_overwrite_with_identity_for_operation(
    source: &Path,
    target: &Path,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel_flag: Option<&AtomicBool>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    operation: crate::fs_safety::atomic_move::AtomicMoveOperation,
) -> Result<(), FileMutationError> {
    move_file_no_overwrite_with_identity_for_operation_with_actual_paths(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel_flag,
        phase_observer,
        operation,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn move_file_no_overwrite_with_identity_for_operation_with_actual_paths(
    source: &Path,
    target: &Path,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel_flag: Option<&AtomicBool>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    operation: crate::fs_safety::atomic_move::AtomicMoveOperation,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<(), FileMutationError> {
    crate::fs_safety::atomic_move::atomic_move_noreplace_with_claim_path_and_observer_for_operation_with_actual_paths(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel_flag,
        phase_observer,
        operation,
        actual_path_observer,
    )
    .map(|_| ())
    .map_err(FileMutationError::Atomic)
}

#[cfg(all(test, windows))]
pub(crate) fn copy_then_delete_via_temp_with_cancel(
    source: &Path,
    target: &Path,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(), String> {
    crate::fs_safety::copy_commit::copy_commit_move(source, target, None, cancel_flag)
        .map_err(|error| error.to_string())
}

#[cfg(all(test, windows))]
pub(crate) fn copy_stream_to_temp<R: Read, W: std::io::Write>(
    reader: &mut R,
    writer: &mut W,
    cancel_flag: Option<&AtomicBool>,
    buffer_size: usize,
) -> Result<u64, String> {
    let mut buffer = vec![0; buffer_size.max(1)];
    let mut copied = 0_u64;
    loop {
        if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(crate::fs_safety::AtomicMoveError::Cancelled.to_string());
        }
        let bytes_read = reader
            .read(&mut buffer)
            .map_err(|error| FileOpError::Io(error).to_string())?;
        if bytes_read == 0 {
            return Ok(copied);
        }
        writer
            .write_all(&buffer[..bytes_read])
            .map_err(|error| FileOpError::Io(error).to_string())?;
        copied += bytes_read as u64;
        if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(crate::fs_safety::AtomicMoveError::Cancelled.to_string());
        }
    }
}

pub(crate) fn ensure_general_file_operation_allowed(path: &Path) -> Result<(), String> {
    ensure_general_file_operation_allowed_for_os(path, env::consts::OS)
}

pub(crate) fn ensure_general_file_operation_allowed_for_os(
    path: &Path,
    os: &str,
) -> Result<(), String> {
    let current_temp = if os == "macos" {
        env::temp_dir().canonicalize().ok()
    } else {
        None
    };
    ensure_general_file_operation_allowed_for_os_with_temp(path, os, current_temp.as_deref())
}

pub(crate) fn ensure_general_file_operation_allowed_for_os_with_temp(
    path: &Path,
    os: &str,
    current_temp: Option<&Path>,
) -> Result<(), String> {
    let normalized = normalize_for_compare_for_os(path, os);
    let is_current_macos_temp = os == "macos"
        && current_temp.is_some_and(|temp| {
            let normalized_temp = normalize_for_compare_for_os(temp, os);
            normalized == normalized_temp || normalized.starts_with(&format!("{normalized_temp}/"))
        });

    for root in general_file_operation_protected_roots_for_os(os) {
        let protected = normalize_for_compare_for_os(&root, os);
        if normalized == protected || normalized.starts_with(&format!("{protected}/")) {
            if is_current_macos_temp {
                continue;
            }
            return Err(FileOpError::ProtectedPath(normalize_path(&root)).to_string());
        }
    }
    Ok(())
}

#[cfg(all(test, windows))]
pub(crate) fn general_file_operation_protected_roots() -> Vec<PathBuf> {
    general_file_operation_protected_roots_for_os(env::consts::OS)
}

pub(crate) fn general_file_operation_protected_roots_for_os(os: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if os == "windows" {
        let drive = env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        for dir in [
            "Windows",
            "Program Files",
            "Program Files (x86)",
            "ProgramData",
            "System Volume Information",
            "$Recycle.Bin",
            "$WINDOWS.~BT",
            "$WinREAgent",
            "Recovery",
        ] {
            roots.push(PathBuf::from(format!("{drive}\\{dir}")));
        }
    } else if os == "macos" {
        roots.extend([
            PathBuf::from("/System"),
            PathBuf::from("/Library"),
            PathBuf::from("/Applications"),
            PathBuf::from("/bin"),
            PathBuf::from("/sbin"),
            PathBuf::from("/usr"),
            PathBuf::from("/etc"),
            PathBuf::from("/private"),
        ]);
    } else {
        roots.extend([
            PathBuf::from("/bin"),
            PathBuf::from("/boot"),
            PathBuf::from("/dev"),
            PathBuf::from("/etc"),
            PathBuf::from("/lib"),
            PathBuf::from("/lib64"),
            PathBuf::from("/proc"),
            PathBuf::from("/root"),
            PathBuf::from("/run"),
            PathBuf::from("/sbin"),
            PathBuf::from("/sys"),
            PathBuf::from("/usr"),
            PathBuf::from("/var"),
        ]);
    }

    roots
}
