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
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    OutcomeUnknown(String),
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
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            Self::OutcomeUnknown(error) => formatter.write_str(error),
            Self::Atomic(error) => error.fmt(formatter),
        }
    }
}

impl FileMutationError {
    pub(crate) fn journal_phase(&self) -> &'static str {
        match self {
            Self::Validation(_) => "rolled_back",
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            Self::OutcomeUnknown(_) => "manual_review",
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
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            Self::OutcomeUnknown(_) => true,
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
        observer(crate::fs_safety::FileOperationEvent::Phase(
            "target_committed".to_string(),
        ))
        .map_err(FileMutationError::Atomic)?;
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

pub(crate) fn create_empty_file_with_safety(
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
    let file = parent
        .create_child_file(std::ffi::OsStr::new(&name))
        .map_err(|error| FileMutationError::Validation(error.to_string()))?;
    let target = parent.path().join(&name);
    file.sync_all().map_err(|_| {
        FileMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown,
        )
    })?;
    parent.sync().map_err(|_| {
        FileMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown,
        )
    })?;
    if let Some(observer) = actual_path_observer {
        observer(parent.path(), &target, None).map_err(FileMutationError::Atomic)?;
    }
    if let Some(observer) = phase_observer {
        observer(crate::fs_safety::FileOperationEvent::Phase(
            "target_committed".to_string(),
        ))
        .map_err(FileMutationError::Atomic)?;
    }
    parent.ensure_unchanged().map_err(|_| {
        FileMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown,
        )
    })?;
    let file_is_empty = file
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == 0);
    let handle_identity =
        crate::fs_safety::identity::capture_namespace_identity_from_handle(&file, &target, None);
    let path_identity = crate::fs_safety::capture_namespace_identity_only(&target, None);
    if !file_is_empty
        || !matches!((handle_identity, path_identity), (Ok(handle), Ok(path)) if crate::fs_safety::identity_matches(&handle, &path))
    {
        return Err(FileMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown,
        ));
    }
    Ok(FileOperationResult {
        operation: "create_file".into(),
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

pub(crate) fn move_to_system_trash_with_safety(
    source_path: String,
    app_data_dir: Option<&Path>,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    cancel_flag: Option<&AtomicBool>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<FileOperationResult, FileMutationError> {
    move_to_system_trash_with_safety_before_handoff(
        source_path,
        app_data_dir,
        expected_identity,
        cancel_flag,
        phase_observer,
        |_| {},
    )
}

fn move_to_system_trash_with_safety_before_handoff(
    source_path: String,
    app_data_dir: Option<&Path>,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    cancel_flag: Option<&AtomicBool>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    before_handoff: impl FnOnce(&Path),
) -> Result<FileOperationResult, FileMutationError> {
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (
            source_path,
            app_data_dir,
            expected_identity,
            cancel_flag,
            phase_observer,
            before_handoff,
        );
        Err(FileMutationError::Validation(
            "system_trash_unsupported_on_platform".into(),
        ))
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        if cancel_flag.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(FileMutationError::Atomic(
                crate::fs_safety::AtomicMoveError::Cancelled,
            ));
        }
        let source = validate_cleanup_trash_source(&PathBuf::from(source_path), app_data_dir)
            .map_err(FileMutationError::Validation)?;
        ensure_not_safe_trash_operation(&source).map_err(FileMutationError::Validation)?;
        ensure_system_trash_source_supported(&source).map_err(FileMutationError::Validation)?;

        let expected = expected_identity.ok_or_else(|| {
            FileMutationError::Validation("system_trash_identity_required".into())
        })?;
        require_system_trash_physical_identity(expected).map_err(FileMutationError::Validation)?;
        let parent_path = source.parent().ok_or_else(|| {
            FileMutationError::Validation(FileOpError::TargetParentMissing.to_string())
        })?;
        let parent = VerifiedDirectory::open_existing(parent_path).map_err(|error| {
            FileMutationError::Validation(format!("system_trash_parent_unavailable: {error}"))
        })?;
        validate_system_trash_source_identity(&source, expected, cancel_flag)
            .map_err(FileMutationError::Validation)?;
        parent
            .ensure_unchanged()
            .map_err(|_| FileMutationError::Validation("system_trash_parent_changed".into()))?;
        if cancel_flag.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(FileMutationError::Atomic(
                crate::fs_safety::AtomicMoveError::Cancelled,
            ));
        }

        // Kept as a narrow seam so native integration tests can perform a
        // real same-path replacement at the last possible point. Production
        // passes a no-op: the OS APIs below accept paths, not our verified
        // handle, so the documented TOCTOU boundary remains.
        before_handoff(&source);

        // This is the final path-based handoff to the OS. `trash` uses
        // IFileOperation + FOF_ALLOWUNDO on Windows and Finder's native Trash
        // operation on macOS. Both preserve the original restore location and
        // return per-item/native execution failures. Neither accepts our open
        // file handle, so a replacement in the tiny interval after the
        // identity check and before the OS resolves the path remains a TOCTOU
        // boundary; the operation is revalidated immediately before handoff.
        match trash::delete(&source) {
            Ok(()) => {
                if let Some(observer) = phase_observer {
                    observer(crate::fs_safety::FileOperationEvent::Phase(
                        "target_committed".into(),
                    ))
                    .map_err(|error| {
                        FileMutationError::OutcomeUnknown(format!(
                            "system_trash_result_uncertain: the OS accepted the item but the journal phase could not be persisted: {error}"
                        ))
                    })?;
                }
                match system_trash_source_state(&source, expected) {
                    SystemTrashSourceState::Missing => Ok(FileOperationResult {
                        operation: "move_to_system_trash".into(),
                        source_path: normalize_path(&source),
                        target_path: system_trash_target_label(),
                    }),
                    state => Err(FileMutationError::OutcomeUnknown(format!(
                        "system_trash_result_uncertain: the OS returned success but the source path is {state:?}; check the system Recycle Bin or Trash before retrying"
                    ))),
                }
            }
            Err(error) => {
                let directory =
                    fs::symlink_metadata(&source).is_ok_and(|metadata| metadata.is_dir());
                match system_trash_source_state(&source, expected) {
                    SystemTrashSourceState::Original if !directory => Err(
                        FileMutationError::Validation(format!("system_trash_failed: {error}")),
                    ),
                    state => Err(FileMutationError::OutcomeUnknown(format!(
                        "system_trash_result_uncertain: the OS reported {error} and the source path is {state:?}; check the system Recycle Bin or Trash before retrying"
                    ))),
                }
            }
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemTrashSourceState {
    Original,
    Changed,
    Missing,
    Unreadable,
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
fn namespace_identity(
    expected: &crate::fs_safety::ExpectedFileIdentity,
) -> crate::fs_safety::ExpectedFileIdentity {
    crate::fs_safety::ExpectedFileIdentity {
        size: expected.size,
        modified_ns: expected.modified_ns,
        platform_volume_id: expected.platform_volume_id.clone(),
        platform_file_id: expected.platform_file_id.clone(),
        sample_hash: None,
        full_hash: None,
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
fn require_system_trash_physical_identity(
    expected: &crate::fs_safety::ExpectedFileIdentity,
) -> Result<(), String> {
    if expected
        .platform_volume_id
        .as_deref()
        .is_none_or(str::is_empty)
        || expected
            .platform_file_id
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err("system_trash_physical_identity_unavailable".into());
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
fn validate_system_trash_source_identity(
    source: &Path,
    expected: &crate::fs_safety::ExpectedFileIdentity,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(), String> {
    require_system_trash_physical_identity(expected)?;
    let actual = crate::fs_safety::capture_namespace_identity_only(source, cancel_flag)
        .map_err(|error| error.to_string())?;
    if !crate::fs_safety::identity_matches(&namespace_identity(expected), &actual) {
        return Err("system_trash_source_changed".into());
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn system_trash_source_state(
    source: &Path,
    expected: &crate::fs_safety::ExpectedFileIdentity,
) -> SystemTrashSourceState {
    match crate::fs_safety::capture_namespace_identity_only(source, None) {
        Ok(actual)
            if crate::fs_safety::identity_matches(&namespace_identity(expected), &actual) =>
        {
            SystemTrashSourceState::Original
        }
        Ok(_) => SystemTrashSourceState::Changed,
        Err(crate::fs_safety::IdentityError::SourceMissing) => SystemTrashSourceState::Missing,
        Err(_) => SystemTrashSourceState::Unreadable,
    }
}

/// Performs the inexpensive, read-only platform gate used by both Preview and
/// execution. Provider/cloud items and volumes whose system-trash behavior is
/// not established are rejected rather than hydrated or silently redirected.
pub(crate) fn ensure_system_trash_source_supported(source: &Path) -> Result<(), String> {
    ensure_general_file_operation_allowed(source)?;
    ensure_cleanup_operation_allowed(source, None)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW,
        };

        crate::file_ops::manual_transfer::ensure_transfer_source_content_ready(source)?;
        let path = source
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let mut root = vec![0u16; 32_768];
        if unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return Err("system_trash_volume_unavailable".into());
        }
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 3 {
            return Err("system_trash_local_ntfs_required".into());
        }
        let mut filesystem = vec![0u16; 64];
        let mut flags = 0u32;
        if unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut flags,
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            )
        } == 0
        {
            return Err("system_trash_volume_unavailable".into());
        }
        let end = filesystem
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(filesystem.len());
        let filesystem = String::from_utf16_lossy(&filesystem[..end]);
        if !filesystem.eq_ignore_ascii_case("ntfs") || flags & 0x0008_0000 != 0 {
            return Err("system_trash_local_ntfs_required".into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        use crate::platform::macos::MacCloudBacking;
        use std::os::unix::fs::MetadataExt;

        let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("system_trash_symlink_unsupported".into());
        }
        let semantics = crate::platform::macos::file_semantics::inspect(source);
        if semantics.backing_kind != MacCloudBacking::Local {
            return Err("system_trash_cloud_or_provider_item_unsupported".into());
        }
        let filesystem = semantics
            .volume
            .filesystem_type
            .as_deref()
            .ok_or("system_trash_volume_unavailable")?;
        if semantics.volume.is_local != Some(true)
            || semantics.volume.is_read_only != Some(false)
            || !matches!(
                filesystem.to_ascii_lowercase().as_str(),
                "apfs" | "hfs" | "hfsx"
            )
            || metadata.dev() == 0
        {
            return Err("system_trash_local_volume_unsupported".into());
        }
        let parent = source
            .parent()
            .ok_or_else(|| FileOpError::TargetParentMissing.to_string())?;
        crate::platform::macos::mutation::ensure_path_eligible(source, parent)
            .map_err(|error| format!("system_trash_source_unsupported:{error}"))?;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = source;
        Err("unsupported_platform_linux".into())
    }
    #[cfg(any(windows, target_os = "macos"))]
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn system_trash_target_label() -> String {
    if cfg!(target_os = "windows") {
        "System Recycle Bin".into()
    } else {
        "System Trash".into()
    }
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
    expected_target_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
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
    crate::fs_safety::atomic_move::atomic_replace_with_expected_target_identity(
        &source,
        &target,
        expected_identity,
        expected_target_identity,
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

/// Restore a directory move through the transfer path that verifies the
/// destination parent while allowing the existing staged copy/verify/retire
/// transaction when the original and restored locations are on different
/// volumes. Files and ordinary Move callers keep their same-volume policy.
#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_directory_move_with_identity(
    source: &Path,
    target: &Path,
    expected_identity: Option<&crate::fs_safety::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel_flag: Option<&AtomicBool>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), FileMutationError> {
    let target_parent_path = target.parent().ok_or_else(|| {
        FileMutationError::Validation(FileOpError::TargetParentMissing.to_string())
    })?;
    let target_parent = VerifiedDirectory::open_existing(target_parent_path)
        .map_err(|error| FileMutationError::Validation(error.to_string()))?;
    crate::fs_safety::atomic_move::atomic_move_with_verified_destination_for_transfer(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel_flag,
        phase_observer,
        crate::fs_safety::atomic_move::AtomicMoveOperation::Restore,
        None,
        Some(target_parent.identity()),
    )
    .map(|_| ())
    .map_err(FileMutationError::Atomic)
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

#[cfg(all(test, not(any(target_os = "windows", target_os = "macos"))))]
mod unsupported_system_trash_tests {
    use super::move_to_system_trash_with_safety;
    use crate::ids::new_job_id;

    #[test]
    fn unsupported_system_trash_fails_without_removing_the_source() {
        let path = std::env::temp_dir().join(format!(
            "fylura-system-trash-unsupported-{}.txt",
            new_job_id("test")
        ));
        std::fs::write(&path, "preserve me").expect("write fixture");

        let error = move_to_system_trash_with_safety(
            path.to_string_lossy().into_owned(),
            None,
            None,
            None,
            None,
        )
        .expect_err("unsupported platform must fail closed");

        assert!(error
            .to_string()
            .contains("system_trash_unsupported_on_platform"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("source remains"),
            "preserve me"
        );
        std::fs::remove_file(path).expect("cleanup fixture");
    }
}

#[cfg(test)]
mod system_trash_identity_tests {
    use super::validate_system_trash_source_identity;
    use crate::fs_safety::{capture_namespace_identity_only, ExpectedFileIdentity};
    use crate::ids::new_job_id;
    use std::fs;
    use std::path::Path;

    #[test]
    fn system_trash_revalidation_rejects_same_path_replacement() {
        let root = std::env::temp_dir().join(format!(
            "fylura-system-trash-binding-{}",
            new_job_id("test")
        ));
        fs::create_dir(&root).expect("create fixture root");

        let file = root.join("file.txt");
        fs::write(&file, b"previewed object").expect("write previewed source");
        let expected = capture_namespace_identity_only(&file, None).expect("capture identity");
        fs::remove_file(&file).expect("remove previewed source");
        fs::write(&file, b"replacement object").expect("write replacement source");
        let error = validate_system_trash_source_identity(&file, &expected, None)
            .expect_err("replacement must be rejected before native handoff");
        assert!(error.contains("system_trash_source_changed"));
        assert_eq!(
            fs::read(&file).expect("replacement remains"),
            b"replacement object"
        );
        fs::remove_dir_all(root).expect("cleanup fixture");
    }

    #[test]
    fn system_trash_revalidation_rejects_renamed_or_missing_source() {
        let root = std::env::temp_dir().join(format!(
            "fylura-system-trash-renamed-{}",
            new_job_id("test")
        ));
        fs::create_dir(&root).expect("create fixture root");
        let source = root.join("source.txt");
        let renamed = root.join("renamed.txt");
        fs::write(&source, b"source").expect("write source");
        let expected: ExpectedFileIdentity =
            capture_namespace_identity_only(&source, None).expect("capture identity");
        fs::rename(&source, &renamed).expect("rename source");
        assert!(validate_system_trash_source_identity(&source, &expected, None).is_err());
        assert_eq!(fs::read(&renamed).expect("renamed file remains"), b"source");
        fs::remove_file(renamed).expect("remove fixture");
        fs::remove_dir(root).expect("remove fixture root");
    }

    #[test]
    fn system_trash_identity_projection_uses_namespace_facts_without_content_hashes() {
        let root = std::env::temp_dir().join(format!(
            "fylura-system-trash-namespace-{}",
            new_job_id("test")
        ));
        fs::create_dir(&root).expect("create fixture root");
        let source = root.join("source.txt");
        fs::write(&source, b"same object").expect("write source");
        let expected = capture_namespace_identity_only(&source, None).expect("capture identity");
        assert!(validate_system_trash_source_identity(Path::new(&source), &expected, None).is_ok());
        fs::remove_dir_all(root).expect("cleanup fixture");
    }
}

#[cfg(all(test, any(target_os = "windows", target_os = "macos")))]
fn native_system_trash_test_root(label: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".tmp-tests")
        .join(format!(
            "fylura-system-trash-{label}-{}",
            crate::ids::new_job_id("test")
        ))
}

#[cfg(all(test, any(target_os = "windows", target_os = "macos")))]
mod system_trash_preflight_native_tests {
    use super::move_to_system_trash_with_safety;
    use crate::fs_safety::capture_namespace_identity_only;
    use std::fs;

    #[test]
    fn native_system_trash_rejects_same_path_replacement_before_handoff() {
        let root = super::native_system_trash_test_root("preflight");
        fs::create_dir_all(&root).expect("create fixture root");
        let source = root.join("entry.txt");
        fs::write(&source, b"previewed object").expect("write previewed source");
        let expected = capture_namespace_identity_only(&source, None).expect("capture identity");
        fs::remove_file(&source).expect("remove previewed source");
        fs::write(&source, b"replacement object").expect("write replacement");

        let error = move_to_system_trash_with_safety(
            source.to_string_lossy().into_owned(),
            None,
            Some(&expected),
            None,
            None,
        )
        .expect_err("changed source must be rejected before the OS resolves the path");
        assert!(error.to_string().contains("system_trash_source_changed"));
        assert_eq!(
            fs::read(&source).expect("replacement remains"),
            b"replacement object"
        );
        fs::remove_dir_all(root).expect("cleanup fixture");
    }
}

#[cfg(all(test, windows))]
mod windows_system_trash_native_tests {
    use super::{
        move_to_system_trash_with_safety, move_to_system_trash_with_safety_before_handoff,
    };
    use crate::fs_safety::capture_namespace_identity_only;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;

    static NATIVE_TRASH_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn same_windows_path(left: &Path, right: &Path) -> bool {
        fn normalized(path: &Path) -> String {
            let path = path.to_string_lossy().replace('/', "\\");
            path.strip_prefix("\\\\?\\")
                .unwrap_or(&path)
                .to_ascii_lowercase()
        }

        normalized(left) == normalized(right)
    }

    fn trash_and_restore(path: &Path) {
        let matches = trash::os_limited::list()
            .expect("list real Windows Recycle Bin")
            .into_iter()
            .filter(|item| same_windows_path(&item.original_path(), path))
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "Recycle Bin should retain the original path"
        );
        trash::os_limited::restore_all(matches).expect("restore through Windows Shell");
    }

    #[test]
    fn windows_recycle_bin_restores_file_and_directory_to_original_paths() {
        let _guard = NATIVE_TRASH_TEST_LOCK
            .lock()
            .expect("serialize Windows Recycle Bin tests");
        let root = super::native_system_trash_test_root("windows-native");
        fs::create_dir_all(&root).expect("create fixture root");

        let file = root.join("native-file.txt");
        fs::write(&file, b"system recycle bin round trip").expect("write file");
        let identity = capture_namespace_identity_only(&file, None).expect("file identity");
        let result = move_to_system_trash_with_safety(
            file.to_string_lossy().into_owned(),
            None,
            Some(&identity),
            None,
            None,
        )
        .expect("send file to the real Recycle Bin");
        assert_eq!(result.target_path, "System Recycle Bin");
        assert!(!file.exists());
        trash_and_restore(&file);
        assert_eq!(
            fs::read(&file).expect("restored file"),
            b"system recycle bin round trip"
        );

        let directory = root.join("native-directory");
        fs::create_dir_all(directory.join("nested")).expect("create nested directory");
        fs::write(directory.join("nested/child.txt"), b"folder tree").expect("write child");
        let identity =
            capture_namespace_identity_only(&directory, None).expect("directory identity");
        let result = move_to_system_trash_with_safety(
            directory.to_string_lossy().into_owned(),
            None,
            Some(&identity),
            None,
            None,
        )
        .expect("send directory to the real Recycle Bin");
        assert_eq!(result.target_path, "System Recycle Bin");
        assert!(!directory.exists());
        trash_and_restore(&directory);
        assert_eq!(
            fs::read(directory.join("nested/child.txt")).expect("restored nested file"),
            b"folder tree"
        );
        fs::remove_dir_all(root).expect("cleanup restored fixture");
    }

    #[test]
    fn windows_native_reproduces_same_path_replacement_after_final_revalidation() {
        let _guard = NATIVE_TRASH_TEST_LOCK
            .lock()
            .expect("serialize Windows Recycle Bin tests");
        let root = super::native_system_trash_test_root("windows-handoff-race");
        fs::create_dir_all(&root).expect("create fixture root");
        let source = root.join("race-entry.txt");
        let displaced_original = root.join("validated-original.txt");
        fs::write(&source, b"validated original").expect("write original file");
        let identity = capture_namespace_identity_only(&source, None).expect("capture identity");

        let result = move_to_system_trash_with_safety_before_handoff(
            source.to_string_lossy().into_owned(),
            None,
            Some(&identity),
            None,
            None,
            |checked_path| {
                fs::rename(checked_path, &displaced_original)
                    .expect("competitor moves the validated object");
                fs::write(checked_path, b"same-path replacement")
                    .expect("competitor recreates the checked path");
            },
        )
        .expect("native path API accepts the replacement in the documented race window");

        assert_eq!(result.target_path, "System Recycle Bin");
        assert_eq!(
            fs::read(&displaced_original).expect("validated object remains outside Trash"),
            b"validated original"
        );
        assert!(!source.exists(), "the replacement path was sent to Trash");
        trash_and_restore(&source);
        assert_eq!(
            fs::read(&source).expect("restore the replacement from Recycle Bin"),
            b"same-path replacement"
        );
        eprintln!("SYSTEM_TRASH_TOC_TOU_NATIVE_PASS platform=windows replacement_after_final_revalidation=true path_api_accepted_replacement=true");
        fs::remove_dir_all(root).expect("cleanup race fixture");
    }

    #[test]
    fn windows_staged_trash_restore_returns_to_staging_path() {
        let _guard = NATIVE_TRASH_TEST_LOCK
            .lock()
            .expect("serialize Windows Recycle Bin tests");
        let root = super::native_system_trash_test_root("windows-staging-restore");
        let original_parent = root.join("original-location");
        let staging_parent = root.join("fylura-staging");
        fs::create_dir_all(&original_parent).expect("create original location");
        fs::create_dir_all(&staging_parent).expect("create staging location");
        let original = original_parent.join("staged-entry.txt");
        let staged = staging_parent.join("staged-entry.txt");
        fs::write(&original, b"B staging semantics").expect("write original file");
        fs::rename(&original, &staged).expect("same-volume move into controlled staging");

        trash::delete(&staged).expect("send staged path to the real Recycle Bin");
        assert!(!staged.exists());
        trash_and_restore(&staged);
        assert_eq!(
            fs::read(&staged).expect("Shell restores to the path handed to it"),
            b"B staging semantics"
        );
        assert!(
            !original.exists(),
            "native restore targets the staged path, not the pre-staging source"
        );
        eprintln!("SYSTEM_TRASH_STAGING_NATIVE_PASS platform=windows restored_to_staging=true original_path_preserved=false");
        fs::remove_dir_all(root).expect("cleanup staged restore fixture");
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_system_trash_native_tests {
    use super::{
        move_to_system_trash_with_safety, move_to_system_trash_with_safety_before_handoff,
    };
    use crate::fs_safety::capture_namespace_identity_only;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::sync::Mutex;

    static NATIVE_TRASH_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn applescript_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    fn restore_with_finder(path: &Path) {
        let name = path
            .file_name()
            .expect("fixture has a file name")
            .to_string_lossy();
        let parent = path
            .parent()
            .expect("fixture has a parent")
            .to_string_lossy();
        let script = format!(
            "tell application \"Finder\"\n\
             activate\n\
             set targetItem to first item of trash whose name is \"{}\"\n\
             set targetFolder to POSIX file \"{}\" as alias\n\
             move targetItem to targetFolder\n\
             end tell",
            applescript_string(&name),
            applescript_string(&parent)
        );
        let output = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .output()
            .expect("run Finder restore operation");
        assert!(
            output.status.success(),
            "Finder failed to restore the item: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn finder_trash_round_trips_local_file_and_directory() {
        let _guard = NATIVE_TRASH_TEST_LOCK
            .lock()
            .expect("serialize Finder Trash tests");
        let root = super::native_system_trash_test_root("macos-native");
        fs::create_dir_all(&root).expect("create fixture root");
        let unique_name = root
            .file_name()
            .expect("fixture root has a name")
            .to_string_lossy();

        let file = root.join(format!("{unique_name}-native-file.txt"));
        fs::write(&file, b"Finder Trash integration fixture").expect("write file");
        let identity = capture_namespace_identity_only(&file, None).expect("file identity");
        let result = move_to_system_trash_with_safety(
            file.to_string_lossy().into_owned(),
            None,
            Some(&identity),
            None,
            None,
        )
        .expect("send local file to Finder Trash");
        assert_eq!(result.target_path, "System Trash");
        assert!(!file.exists());
        restore_with_finder(&file);
        assert_eq!(
            fs::read(&file).expect("Finder-restored file"),
            b"Finder Trash integration fixture"
        );

        let directory = root.join(format!("{unique_name}-native-directory"));
        fs::create_dir_all(directory.join("nested")).expect("create nested directory");
        fs::write(directory.join("nested/child.txt"), b"folder tree").expect("write child");
        let identity =
            capture_namespace_identity_only(&directory, None).expect("directory identity");
        let result = move_to_system_trash_with_safety(
            directory.to_string_lossy().into_owned(),
            None,
            Some(&identity),
            None,
            None,
        )
        .expect("send directory tree to Finder Trash");
        assert_eq!(result.target_path, "System Trash");
        assert!(!directory.exists());
        restore_with_finder(&directory);
        assert_eq!(
            fs::read(directory.join("nested/child.txt")).expect("Finder-restored nested file"),
            b"folder tree"
        );
        fs::remove_dir_all(root).expect("cleanup remaining test files");
    }

    #[test]
    fn finder_native_reproduces_same_path_replacement_after_final_revalidation() {
        let _guard = NATIVE_TRASH_TEST_LOCK
            .lock()
            .expect("serialize Finder Trash tests");
        let root = super::native_system_trash_test_root("macos-handoff-race");
        fs::create_dir_all(&root).expect("create fixture root");
        let root_name = root
            .file_name()
            .expect("fixture root has a name")
            .to_string_lossy();
        let source = root.join(format!("{root_name}-race-entry.txt"));
        let displaced_original = root.join("validated-original.txt");
        fs::write(&source, b"validated original").expect("write original file");
        let identity = capture_namespace_identity_only(&source, None).expect("capture identity");

        let result = move_to_system_trash_with_safety_before_handoff(
            source.to_string_lossy().into_owned(),
            None,
            Some(&identity),
            None,
            None,
            |checked_path| {
                fs::rename(checked_path, &displaced_original)
                    .expect("competitor moves the validated object");
                fs::write(checked_path, b"same-path replacement")
                    .expect("competitor recreates the checked path");
            },
        )
        .expect("Finder path API accepts the replacement in the documented race window");

        assert_eq!(result.target_path, "System Trash");
        assert_eq!(
            fs::read(&displaced_original).expect("validated object remains outside Trash"),
            b"validated original"
        );
        assert!(!source.exists(), "the replacement path was sent to Trash");
        restore_with_finder(&source);
        assert_eq!(
            fs::read(&source).expect("restore the replacement from Finder Trash"),
            b"same-path replacement"
        );
        eprintln!("SYSTEM_TRASH_TOC_TOU_NATIVE_PASS platform=macos replacement_after_final_revalidation=true path_api_accepted_replacement=true");
        fs::remove_dir_all(root).expect("cleanup race fixture");
    }

    #[test]
    fn finder_staged_trash_restore_returns_to_staging_path() {
        let _guard = NATIVE_TRASH_TEST_LOCK
            .lock()
            .expect("serialize Finder Trash tests");
        let root = super::native_system_trash_test_root("macos-staging-restore");
        fs::create_dir_all(&root).expect("create fixture root");
        let root_name = root
            .file_name()
            .expect("fixture root has a name")
            .to_string_lossy();
        let original_parent = root.join("original-location");
        let staging_parent = root.join("fylura-staging");
        fs::create_dir_all(&original_parent).expect("create original location");
        fs::create_dir_all(&staging_parent).expect("create staging location");
        let name = format!("{root_name}-staged-entry.txt");
        let original = original_parent.join(&name);
        let staged = staging_parent.join(&name);
        fs::write(&original, b"B staging semantics").expect("write original file");
        fs::rename(&original, &staged).expect("same-volume move into controlled staging");

        trash::delete(&staged).expect("send staged path to Finder Trash");
        assert!(!staged.exists());
        restore_with_finder(&staged);
        assert_eq!(
            fs::read(&staged).expect("Finder restores to the path handed to it"),
            b"B staging semantics"
        );
        assert!(
            !original.exists(),
            "Finder restore targets the staged path, not the pre-staging source"
        );
        eprintln!("SYSTEM_TRASH_STAGING_NATIVE_PASS platform=macos restored_to_staging=true original_path_preserved=false");
        fs::remove_dir_all(root).expect("cleanup staged restore fixture");
    }
}
