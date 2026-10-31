#[cfg(windows)]
use super::copy_commit;
use super::{
    identity, platform_support, source_claim, source_claim::SourceClaimError,
    verified_directory::VerifiedDirectory,
};
use std::{
    io,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicMoveMethod {
    SameVolumeNoReplace,
    CrossVolumeCopyCommit,
    PermanentDeleteQuarantine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomicMoveOperation {
    Rename,
    Move,
    Trash,
    Restore,
}

/// Structured durability state for a filesystem mutation.
///
/// Callers must use this value when deciding whether a journal row can be
/// marked failed/rolled back.  The error text is intentionally not part of
/// the state machine because several variants carry platform error details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicMoveCommitState {
    RolledBack,
    SourceClaimed,
    TargetCommitted,
    SourceCleanupPending,
    Completed,
    ManualReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtomicMoveOutcome {
    pub method: AtomicMoveMethod,
    pub commit_state: AtomicMoveCommitState,
}

#[derive(Debug, Error)]
pub enum AtomicMoveError {
    #[error("target_exists")]
    TargetExists,
    #[error("source_missing")]
    SourceMissing,
    #[error("source_changed")]
    SourceChanged,
    #[error("cross_device")]
    CrossDevice,
    #[error("cross_volume_directory_move_unsupported")]
    CrossVolumeDirectoryMoveUnsupported,
    #[error("cross_volume_file_move_unsupported_on_macos")]
    CrossVolumeFileMoveUnsupportedOnMacos,
    #[error("atomic_noreplace_unsupported")]
    UnsupportedAtomicNoReplace,
    #[error("atomic_source_binding_unsupported")]
    AtomicSourceBindingUnsupported,
    #[error("unsupported_platform_linux")]
    UnsupportedPlatformLinux,
    #[error("macos_file_mutation_source_binding_unsupported")]
    MacosFileMutationSourceBindingUnsupported,
    #[error("{0}")]
    MacMutationNotSupported(&'static str),
    #[error("target_parent_identity_changed")]
    TargetParentIdentityChanged,
    #[error("target_parent_durability_unknown")]
    TargetParentDurabilityUnknown,
    #[error("staging_identity_changed")]
    StagingIdentityChanged,
    #[error("staging_handle_commit_unsupported")]
    StagingHandleCommitUnsupported,
    #[error("target_committed_durability_unknown")]
    TargetCommittedDurabilityUnknown,
    #[error("target_committed_identity_mismatch")]
    TargetCommittedIdentityMismatch,
    #[error("target_committed_source_cleanup_pending")]
    TargetCommittedSourceCleanupPending,
    #[error("unsafe_path")]
    UnsafePath,
    #[error("reparse_point")]
    ReparsePoint,
    #[error("symlink")]
    Symlink,
    #[error("cancelled")]
    Cancelled,
    #[error("copy_verification_failed")]
    CopyVerificationFailed,
    #[error("mac_metadata_preservation_unsupported: {0}")]
    MetadataPreservationUnsupported(&'static str),
    #[error("directory_manifest_name_encoding_failed")]
    DirectoryManifestNameEncodingFailed,
    #[error("source_claim_failed: {0}")]
    SourceClaimFailed(String),
    #[error("source_claim_mismatch")]
    SourceClaimMismatch,
    #[error("source_claim_rollback_failed: {0}")]
    SourceClaimRollbackFailed(String),
    #[error("source_claim_recovery_required: {0}")]
    SourceClaimRecoveryRequired(String),
    #[error("mac_claim_namespace_rebound")]
    MacClaimNamespaceRebound,
    #[error("mac_claim_identity_mismatch")]
    MacClaimIdentityMismatch,
    #[error("mac_claim_path_missing")]
    MacClaimPathMissing,
    #[error("mac_claim_path_unreadable")]
    MacClaimPathUnreadable,
    #[error("target_committed_source_delete_failed: {0}")]
    TargetCommittedSourceDeleteFailed(String),
    #[error("permanent_delete_quarantine_retained: {0}")]
    PermanentDeleteQuarantineRetained(String),
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

impl AtomicMoveError {
    pub fn commit_state(&self) -> AtomicMoveCommitState {
        match self {
            Self::TargetCommittedSourceDeleteFailed(_)
            | Self::TargetCommittedSourceCleanupPending
            | Self::PermanentDeleteQuarantineRetained(_) => {
                AtomicMoveCommitState::SourceCleanupPending
            }
            Self::TargetCommittedDurabilityUnknown | Self::TargetCommittedIdentityMismatch => {
                AtomicMoveCommitState::ManualReview
            }
            Self::SourceClaimRecoveryRequired(_) | Self::SourceClaimRollbackFailed(_) => {
                AtomicMoveCommitState::SourceClaimed
            }
            Self::MacClaimNamespaceRebound
            | Self::MacClaimIdentityMismatch
            | Self::MacClaimPathMissing
            | Self::MacClaimPathUnreadable => AtomicMoveCommitState::ManualReview,
            _ => AtomicMoveCommitState::RolledBack,
        }
    }

    pub fn is_post_commit(&self) -> bool {
        matches!(
            self.commit_state(),
            AtomicMoveCommitState::TargetCommitted
                | AtomicMoveCommitState::SourceCleanupPending
                | AtomicMoveCommitState::ManualReview
        )
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    pub fn is_mac_claim_safety_error(&self) -> bool {
        matches!(
            self,
            Self::MacClaimNamespaceRebound
                | Self::MacClaimIdentityMismatch
                | Self::MacClaimPathMissing
                | Self::MacClaimPathUnreadable
        )
    }
}

pub fn atomic_move_noreplace(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    cancel: Option<&AtomicBool>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_move_noreplace_with_claim_path(source, target, expected_identity, None, cancel)
}

pub fn atomic_move_noreplace_with_claim_path(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_move_noreplace_with_claim_path_and_observer(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel,
        None,
    )
}

pub(crate) fn atomic_move_noreplace_with_claim_path_and_observer(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_move_noreplace_with_claim_path_and_observer_for_operation(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel,
        observer,
        AtomicMoveOperation::Move,
    )
}

pub(crate) fn atomic_move_noreplace_with_claim_path_and_observer_for_operation(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    operation: AtomicMoveOperation,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_move_noreplace_with_claim_path_and_observer_for_operation_with_actual_paths(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel,
        observer,
        operation,
        None,
    )
}

#[cfg(any(test, feature = "native-qa"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicMoveTestOperation {
    Move,
    SafeTrash,
    Restore,
}

/// Test-only operation selector used by the native adversarial matrix. The
/// production execution paths continue to select the operation from the
/// authoritative preview and journal adapters above.
#[cfg(any(test, feature = "native-qa"))]
pub fn atomic_move_noreplace_for_test_operation(
    source: &Path,
    target: &Path,
    operation: AtomicMoveTestOperation,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    let operation = match operation {
        AtomicMoveTestOperation::Move => AtomicMoveOperation::Move,
        AtomicMoveTestOperation::SafeTrash => AtomicMoveOperation::Trash,
        AtomicMoveTestOperation::Restore => AtomicMoveOperation::Restore,
    };
    atomic_move_noreplace_with_claim_path_and_observer_for_operation(
        source, target, None, None, None, None, operation,
    )
}

#[cfg(any(test, feature = "native-qa"))]
pub fn atomic_permanent_delete_for_test(
    source: &Path,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_permanent_delete_with_claim_path_and_observer(source, None, None, None, None)
}

#[cfg(any(test, feature = "native-qa"))]
pub fn atomic_permanent_delete_for_test_with_hook(
    source: &Path,
    hook: source_claim::Hook,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    #[cfg(target_os = "macos")]
    {
        source_claim::set_claim_test_hook(Some(hook));
        let result = atomic_permanent_delete_with_claim_path_and_observer_and_hook(
            source,
            None,
            None,
            None,
            None,
            None,
            Some(hook),
        );
        source_claim::set_claim_test_hook(None);
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = hook;
        atomic_permanent_delete_with_claim_path_and_observer(source, None, None, None, None)
    }
}

#[cfg(any(test, feature = "native-qa"))]
pub fn atomic_replace_for_test(
    source: &Path,
    target: &Path,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_replace_with_claim_path_and_observer(source, target, None, None, None, "test", None)
}

/// Executes the same canonical move primitive while allowing a caller that
/// owns a durable ledger to receive the URLs supplied by a native coordinator.
/// The callback runs before the coordinated filesystem action, so subsequent
/// phase persistence can record the actual source/target pair rather than the
/// renderer's pre-coordination path guess.
#[allow(clippy::too_many_arguments)]
pub(crate) fn atomic_move_noreplace_with_claim_path_and_observer_for_operation_with_actual_paths(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    operation: AtomicMoveOperation,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_move_with_verified_destination(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel,
        observer,
        operation,
        actual_path_observer,
        None,
    )
}

/// Optional consent-bound destination guard for the bounded manual Move intent.
/// Ordinary operations keep their existing behavior through the None wrapper.
#[allow(clippy::too_many_arguments)]
pub(crate) fn atomic_move_with_verified_destination(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    operation: AtomicMoveOperation,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
    expected_target_parent: Option<&super::DirectoryIdentity>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_move_with_verified_destination_policy(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel,
        observer,
        operation,
        actual_path_observer,
        expected_target_parent,
        false,
    )
}

/// Transfer intents keep the target-parent identity guard while allowing the
/// existing copy/verify/source-retirement path when selected volumes differ.
/// Other manual Move callers retain their same-volume requirement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn atomic_move_with_verified_destination_for_transfer(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    operation: AtomicMoveOperation,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
    expected_target_parent: Option<&super::DirectoryIdentity>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_move_with_verified_destination_policy(
        source,
        target,
        expected_identity,
        planned_claim_path,
        cancel,
        observer,
        operation,
        actual_path_observer,
        expected_target_parent,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn atomic_move_with_verified_destination_policy(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    operation: AtomicMoveOperation,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
    expected_target_parent: Option<&super::DirectoryIdentity>,
    allow_cross_volume: bool,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::strategy::with_mutation_strategy(
            source,
            target,
            cancel,
            match operation {
                AtomicMoveOperation::Rename => {
                    crate::platform::macos::strategy::MacCoordinatedOperation::Rename
                }
                AtomicMoveOperation::Move => {
                    crate::platform::macos::strategy::MacCoordinatedOperation::Move
                }
                AtomicMoveOperation::Trash => {
                    crate::platform::macos::strategy::MacCoordinatedOperation::SafeTrash
                }
                AtomicMoveOperation::Restore => {
                    crate::platform::macos::strategy::MacCoordinatedOperation::Restore
                }
            },
            |coordinated_source, coordinated_target| {
                atomic_move_noreplace_with_claim_path_and_observer_uncoordinated(
                    coordinated_source,
                    coordinated_target,
                    expected_identity,
                    planned_claim_path,
                    cancel,
                    observer,
                    operation,
                    actual_path_observer,
                    expected_target_parent,
                    allow_cross_volume,
                )
            },
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        atomic_move_noreplace_with_claim_path_and_observer_uncoordinated(
            source,
            target,
            expected_identity,
            planned_claim_path,
            cancel,
            observer,
            operation,
            actual_path_observer,
            expected_target_parent,
            allow_cross_volume,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn atomic_move_noreplace_with_claim_path_and_observer_uncoordinated(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    mut observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    operation: AtomicMoveOperation,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
    expected_target_parent: Option<&super::DirectoryIdentity>,
    allow_cross_volume: bool,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    platform_support::ensure_supported_file_mutation().map_err(map_platform_error)?;
    if is_cancelled(cancel) {
        return Err(AtomicMoveError::Cancelled);
    }
    let target_parent_path = target.parent().ok_or(AtomicMoveError::UnsafePath)?;
    let target_name = target.file_name().ok_or(AtomicMoveError::UnsafePath)?;
    let target_parent =
        VerifiedDirectory::open_existing(target_parent_path).map_err(map_directory_error)?;
    if let Some(expected) = expected_target_parent {
        if target_parent.identity() != expected {
            return Err(AtomicMoveError::TargetParentIdentityChanged);
        }
        if !allow_cross_volume
            && expected_identity.and_then(|source| source.platform_volume_id.as_deref())
                != Some(expected.volume_id.as_str())
        {
            return Err(AtomicMoveError::CrossDevice);
        }
        #[cfg(target_os = "macos")]
        if !allow_cross_volume
            && !matches!(
                crate::platform::macos::strategy::select(source, target_parent.path()),
                crate::platform::macos::strategy::MacMutationStrategy::LocalApfs
            )
        {
            return Err(AtomicMoveError::MacMutationNotSupported(
                "manual_move_local_same_volume_required",
            ));
        }
    }
    #[cfg(target_os = "macos")]
    crate::platform::macos::mutation::ensure_path_eligible(source, target_parent.path())
        .map_err(AtomicMoveError::MacMutationNotSupported)?;
    #[cfg(target_os = "macos")]
    {
        let strategy = crate::platform::macos::strategy::select(source, target_parent.path());
        if matches!(
            strategy,
            crate::platform::macos::strategy::MacMutationStrategy::CrossVolume
                | crate::platform::macos::strategy::MacMutationStrategy::LocalPortable
                | crate::platform::macos::strategy::MacMutationStrategy::NetworkPortable
        ) {
            // Retirement capability is evidence for the post-commit source
            // cleanup leg, not a reason to suppress a safe target-first copy.
            // If the volume cannot prove an exclusive claim, copy/verify may
            // still succeed and the source remains recorded as
            // source_cleanup_pending for a later, identity-checked retry.
            let _retirement_capability =
                crate::platform::macos::strategy::verify_source_retirement_capability(source);
        }
        match strategy {
            crate::platform::macos::strategy::MacMutationStrategy::CrossVolume
            | crate::platform::macos::strategy::MacMutationStrategy::LocalPortable
            | crate::platform::macos::strategy::MacMutationStrategy::NetworkPortable => {
                return crate::platform::macos::copy::copy_commit_source_stable(
                    source,
                    target,
                    expected_identity,
                    planned_claim_path,
                    cancel,
                    observer,
                    true,
                    actual_path_observer,
                    expected_target_parent,
                )
                .map(|_| AtomicMoveOutcome {
                    method: AtomicMoveMethod::CrossVolumeCopyCommit,
                    commit_state: AtomicMoveCommitState::Completed,
                });
            }
            _ => {}
        }
    }
    let target_exists = std::fs::symlink_metadata(target).is_ok();
    if target_exists && !(cfg!(target_os = "macos") && same_macos_namespace_entry(source, target)) {
        return Err(AtomicMoveError::TargetExists);
    }
    // Windows reparse points must not enter a mutation path. On macOS the
    // namespace identity layer represents symlinks without following them,
    // and atomic rename preserves the link itself (not its target).
    #[cfg(not(target_os = "macos"))]
    identity::ensure_supported_entry(source).map_err(map_identity_error)?;
    let namespace_directory_operation = matches!(
        operation,
        AtomicMoveOperation::Rename | AtomicMoveOperation::Trash | AtomicMoveOperation::Restore
    ) && std::fs::symlink_metadata(source)
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
        && expected_identity.is_some_and(|expected| expected.full_hash.is_none());
    let expected = match expected_identity {
        Some(expected)
            if expected.full_hash.is_some()
                || (cfg!(target_os = "macos")
                    && expected.platform_volume_id.is_some()
                    && expected.platform_file_id.is_some())
                || (namespace_directory_operation
                    && expected.platform_volume_id.is_some()
                    && expected.platform_file_id.is_some()) =>
        {
            expected.clone()
        }
        Some(_) => {
            return Err(AtomicMoveError::SourceClaimFailed(
                "source identity is incomplete".to_string(),
            ));
        }
        None => {
            #[cfg(target_os = "macos")]
            {
                identity::capture_namespace_identity_only(source, cancel)
                    .map_err(map_identity_error)?
            }
            #[cfg(not(target_os = "macos"))]
            {
                identity::capture_namespace_identity(source, cancel).map_err(map_identity_error)?
            }
        }
    };
    let claim_path = match planned_claim_path {
        Some(path) => source_claim::rebind_claim_path(source, path).map_err(map_claim_error)?,
        None => source_claim::planned_claim_path(source, "atomic-move").map_err(map_claim_error)?,
    };
    if let Some(callback) = actual_path_observer {
        callback(source, target, Some(&claim_path))?;
    }
    #[cfg(any(test, feature = "native-qa"))]
    source_claim::run_claim_test_hook(
        source_claim::ClaimTestPoint::AfterJournalPreparedBeforeClaim,
        source,
        &claim_path,
    );
    let mut claim =
        source_claim::claim_source_at(source, &expected, &claim_path, "atomic-move", cancel)
            .map_err(map_claim_error)?;
    if let Err(error) = notify_phase(&mut observer, "source_claimed") {
        return match claim.rollback_to_original() {
            Ok(()) => Err(error),
            Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
                rollback.to_string(),
            )),
        };
    }
    if is_cancelled(cancel) {
        return match claim.rollback_to_original() {
            Ok(()) => Err(AtomicMoveError::Cancelled),
            Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
                rollback.to_string(),
            )),
        };
    }
    #[cfg(any(test, feature = "native-qa"))]
    source_claim::run_claim_test_hook(
        source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit,
        source,
        &claim_path,
    );
    #[cfg(any(test, feature = "native-qa"))]
    source_claim::run_claim_test_hook(
        match operation {
            AtomicMoveOperation::Rename => {
                source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeCommit
            }
            AtomicMoveOperation::Move => {
                source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeCommit
            }
            AtomicMoveOperation::Trash => {
                source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeTrashCommit
            }
            AtomicMoveOperation::Restore => {
                source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeRestoreCommit
            }
        },
        source,
        &claim_path,
    );

    if let Err(error) = enforce_target_volume_policy(
        claim.original_volume_id(),
        &target_parent.identity().volume_id,
        expected_target_parent.is_some(),
        allow_cross_volume,
    ) {
        return Err(rollback_after_failure(&mut claim, error));
    }
    if claim.original_volume_id() == target_parent.identity().volume_id {
        #[cfg(target_os = "macos")]
        let target_parent_path = target_parent.path().to_path_buf();
        #[cfg(target_os = "macos")]
        let target_parent_identity = target_parent.identity().clone();
        let result = claim.commit_to_with_cancel(target_parent, target_name, cancel);
        return match result {
            Ok(_committed_path) => {
                notify_phase(&mut observer, "target_committed")?;
                #[cfg(any(test, feature = "native-qa"))]
                if test_faults::take_fault(test_faults::AtomicFaultPoint::SourceCleanup) {
                    notify_phase(&mut observer, "source_cleanup_pending")?;
                    return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
                }
                #[cfg(any(test, feature = "native-qa"))]
                if test_faults::take_fault(test_faults::AtomicFaultPoint::TargetDurability) {
                    return Err(AtomicMoveError::TargetCommittedDurabilityUnknown);
                }
                claim
                    .sync()
                    .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
                claim
                    .sync_current_parent()
                    .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
                claim
                    .sync_original_parent()
                    .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
                claim
                    .current_parent_unchanged()
                    .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
                #[cfg(any(test, feature = "native-qa"))]
                if test_faults::take_fault(test_faults::AtomicFaultPoint::TargetIdentity) {
                    return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
                }
                let actual = claim
                    .verify_current_identity(cancel)
                    .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
                if !identity::identity_matches(&expected, &actual) {
                    return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
                }
                #[cfg(target_os = "macos")]
                let path_actual =
                    identity::capture_namespace_identity_only(claim.current_path(), cancel)
                        .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
                #[cfg(not(target_os = "macos"))]
                let path_actual = if namespace_directory_operation {
                    // A directory operation is authorized by the stable
                    // namespace identity above.  Capturing the general
                    // namespace identity here would recursively hash the
                    // directory tree, which is both unbounded and can report
                    // a different identity shape than the retained handle.
                    identity::capture_namespace_identity_only(claim.current_path(), cancel)
                } else {
                    identity::capture_namespace_identity(claim.current_path(), cancel)
                }
                .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
                #[cfg(target_os = "macos")]
                // The retained descriptor was already checked for full
                // content identity above; this pathname recheck is only a
                // physical namespace binding check on macOS.
                let path_identity_matches = identity::identity_matches(
                    &identity::ExpectedFileIdentity {
                        size: actual.size,
                        modified_ns: actual.modified_ns,
                        platform_volume_id: actual.platform_volume_id.clone(),
                        platform_file_id: actual.platform_file_id.clone(),
                        sample_hash: None,
                        full_hash: None,
                    },
                    &path_actual,
                );
                #[cfg(not(target_os = "macos"))]
                let path_identity_matches = identity::identity_matches(&actual, &path_actual);
                if !path_identity_matches {
                    return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
                }
                notify_phase(&mut observer, "completed")?;
                Ok(AtomicMoveOutcome {
                    method: AtomicMoveMethod::SameVolumeNoReplace,
                    commit_state: AtomicMoveCommitState::Completed,
                })
            }
            Err(error) => {
                #[cfg(target_os = "macos")]
                if expected_target_parent.is_none()
                    && matches!(error, SourceClaimError::AtomicSourceBindingUnsupported)
                {
                    let fallback_parent = VerifiedDirectory::open_existing(&target_parent_path)
                        .map_err(map_directory_error)?;
                    if fallback_parent.identity() != &target_parent_identity {
                        return Err(AtomicMoveError::TargetParentIdentityChanged);
                    }
                    return crate::platform::macos::copy::copy_commit_claim(
                        &mut claim,
                        fallback_parent,
                        target_name,
                        cancel,
                        observer,
                    )
                    .map(|_| AtomicMoveOutcome {
                        method: AtomicMoveMethod::CrossVolumeCopyCommit,
                        commit_state: AtomicMoveCommitState::Completed,
                    });
                }
                Err(rollback_after_failure(&mut claim, error))
            }
        };
    }

    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::copy::copy_commit_claim(
            &mut claim,
            target_parent,
            target_name,
            cancel,
            observer,
        )
        .map(|_| AtomicMoveOutcome {
            method: AtomicMoveMethod::CrossVolumeCopyCommit,
            commit_state: AtomicMoveCommitState::Completed,
        })
    }
    #[cfg(windows)]
    {
        if matches!(claim.kind(), source_claim::ClaimedEntryKind::Directory) {
            let _ = claim.rollback_to_original();
            return Err(AtomicMoveError::CrossVolumeDirectoryMoveUnsupported);
        }
        copy_commit::copy_commit_claim(&mut claim, target_parent, target_name, cancel, observer)
            .map(|_| AtomicMoveOutcome {
                method: AtomicMoveMethod::CrossVolumeCopyCommit,
                commit_state: AtomicMoveCommitState::Completed,
            })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = claim.rollback_to_original();
        Err(AtomicMoveError::UnsupportedPlatformLinux)
    }
}

fn enforce_target_volume_policy(
    source_volume_id: &str,
    target_volume_id: &str,
    has_verified_target_parent: bool,
    allow_cross_volume: bool,
) -> Result<(), SourceClaimError> {
    if has_verified_target_parent && !allow_cross_volume && source_volume_id != target_volume_id {
        Err(SourceClaimError::CrossDevice)
    } else {
        Ok(())
    }
}

fn notify_phase(
    observer: &mut Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    phase: &str,
) -> Result<(), AtomicMoveError> {
    if let Some(observer) = observer.as_deref_mut() {
        observer(phase)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn atomic_copy_noreplace_with_claim_path_and_observer_with_actual_paths(
    source: &Path,
    target: &Path,
    operation: &str,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_copy_with_verified_destination(
        source,
        target,
        operation,
        expected_identity,
        planned_claim_path,
        cancel,
        observer,
        actual_path_observer,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn atomic_copy_with_verified_destination(
    source: &Path,
    target: &Path,
    operation: &str,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
    expected_target_parent: Option<&super::DirectoryIdentity>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::strategy::with_mutation_strategy(
            source,
            target,
            cancel,
            match operation {
                "duplicate" => crate::platform::macos::strategy::MacCoordinatedOperation::Duplicate,
                _ => crate::platform::macos::strategy::MacCoordinatedOperation::Copy,
            },
            |coordinated_source, coordinated_target| {
                atomic_copy_noreplace_uncoordinated(
                    coordinated_source,
                    coordinated_target,
                    expected_identity,
                    planned_claim_path,
                    cancel,
                    observer,
                    actual_path_observer,
                    expected_target_parent,
                )
            },
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = operation;
        atomic_copy_noreplace_uncoordinated(
            source,
            target,
            expected_identity,
            planned_claim_path,
            cancel,
            observer,
            actual_path_observer,
            expected_target_parent,
        )
    }
}

/// Permanently removes a namespace entry through the existing claim and
/// journal boundary.  The source is first renamed to the caller's private
/// claim path (the quarantine), verified again, and only then deleted.  If
/// deletion cannot be completed the quarantine is deliberately retained for
/// manual recovery; the operation never falls back to an unverified
/// path-based delete.
#[cfg(any(test, feature = "native-qa"))]
pub(crate) fn atomic_permanent_delete_with_claim_path_and_observer(
    source: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_permanent_delete_with_claim_path_and_observer_with_actual_paths(
        source,
        expected_identity,
        planned_claim_path,
        cancel,
        observer,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn atomic_permanent_delete_with_claim_path_and_observer_with_actual_paths(
    source: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    #[cfg(target_os = "macos")]
    {
        // NSFileCoordinator may invoke the accessor on a native callback
        // thread. Carry the test-only adversarial hook into that callback
        // instead of relying on thread-local state there; production builds
        // do not compile this value or the hook path.
        #[cfg(any(test, feature = "native-qa"))]
        let claim_test_hook = source_claim::current_claim_test_hook();
        atomic_permanent_delete_with_claim_path_and_observer_and_hook(
            source,
            expected_identity,
            planned_claim_path,
            cancel,
            observer,
            actual_path_observer,
            #[cfg(any(test, feature = "native-qa"))]
            claim_test_hook,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            source,
            expected_identity,
            planned_claim_path,
            cancel,
            observer,
            actual_path_observer,
        );
        Err(AtomicMoveError::MacMutationNotSupported(
            "permanent_delete_requires_macos_quarantine",
        ))
    }
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn atomic_permanent_delete_with_claim_path_and_observer_and_hook(
    source: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
    #[cfg(any(test, feature = "native-qa"))] claim_test_hook: Option<source_claim::Hook>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    crate::platform::macos::strategy::with_mutation_strategy(
        source,
        source,
        cancel,
        crate::platform::macos::strategy::MacCoordinatedOperation::PermanentDelete,
        |coordinated_source, _coordinated_target| {
            atomic_permanent_delete_uncoordinated(
                coordinated_source,
                expected_identity,
                planned_claim_path,
                cancel,
                observer,
                actual_path_observer,
                #[cfg(any(test, feature = "native-qa"))]
                claim_test_hook,
            )
        },
    )
}

#[cfg(target_os = "macos")]
fn atomic_permanent_delete_uncoordinated(
    source: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    mut observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
    #[cfg(any(test, feature = "native-qa"))] claim_test_hook: Option<source_claim::Hook>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    platform_support::ensure_supported_file_mutation().map_err(map_platform_error)?;
    if is_cancelled(cancel) {
        return Err(AtomicMoveError::Cancelled);
    }
    let expected = match expected_identity {
        Some(expected) => expected.clone(),
        None => {
            #[cfg(target_os = "macos")]
            {
                identity::capture_namespace_identity_only(source, cancel)
                    .map_err(map_identity_error)?
            }
            #[cfg(not(target_os = "macos"))]
            {
                identity::capture_namespace_identity(source, cancel).map_err(map_identity_error)?
            }
        }
    };
    let claim_path = match planned_claim_path {
        Some(path) => source_claim::rebind_claim_path(source, path).map_err(map_claim_error)?,
        None => {
            source_claim::planned_claim_path(source, "permanent-delete").map_err(map_claim_error)?
        }
    };
    if let Some(callback) = actual_path_observer {
        callback(source, source, Some(&claim_path))?;
    }
    let mut claim =
        source_claim::claim_source_at(source, &expected, &claim_path, "permanent-delete", cancel)
            .map_err(map_claim_error)?;
    if let Err(error) = notify_phase(&mut observer, "source_claimed") {
        return match claim.rollback_to_original() {
            Ok(()) => Err(error),
            Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
                rollback.to_string(),
            )),
        };
    }
    if let Err(error) = claim
        .verify_current_identity(cancel)
        .map_err(map_claim_error)
    {
        return match claim.rollback_to_original() {
            Ok(()) => Err(error),
            Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
                rollback.to_string(),
            )),
        };
    }
    if is_cancelled(cancel) {
        return match claim.rollback_to_original() {
            Ok(()) => Err(AtomicMoveError::Cancelled),
            Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
                rollback.to_string(),
            )),
        };
    }
    claim
        .verify_current_namespace_binding()
        .map_err(map_claim_error)?;
    // Permanent Delete has a single-source transaction boundary. Run the
    // native-qa hook inside the coordinator action, immediately before the
    // normal SourceClaim delete path performs its final identity check. This
    // keeps the adversarial rebind on the same callback thread while leaving
    // the production deletion path unchanged.
    #[cfg(any(test, feature = "native-qa"))]
    let claim_test_hook = claim_test_hook.or_else(source_claim::current_claim_test_hook);
    #[cfg(any(test, feature = "native-qa"))]
    source_claim::run_claim_test_hook_with_override(
        claim_test_hook,
        source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeDelete,
        claim.original_path(),
        claim.current_path(),
    );
    #[cfg(any(test, feature = "native-qa"))]
    claim
        .verify_current_namespace_binding()
        .map_err(map_claim_error)?;
    let delete_result = if claim.kind() == source_claim::ClaimedEntryKind::Directory {
        claim.delete_claim_tree()
    } else {
        claim.delete_claim()
    };
    if let Err(error) = delete_result {
        let mapped = map_claim_error(error);
        if mapped.is_mac_claim_safety_error() {
            return Err(mapped);
        }
        return Err(AtomicMoveError::PermanentDeleteQuarantineRetained(
            mapped.to_string(),
        ));
    }
    notify_phase(&mut observer, "completed")?;
    Ok(AtomicMoveOutcome {
        method: AtomicMoveMethod::PermanentDeleteQuarantine,
        commit_state: AtomicMoveCommitState::Completed,
    })
}

/// Replaces an existing destination without overwriting it in place.  The
/// destination is first moved into a deterministic private replacement
/// backup; the source is then published through the normal verified copy or
/// handle-bound move primitive.  The backup remains available for recovery and
/// is never silently deleted by this function.
#[cfg(any(test, feature = "native-qa"))]
pub(crate) fn atomic_replace_with_claim_path_and_observer(
    source: &Path,
    target: &Path,
    expected_source_identity: Option<&identity::ExpectedFileIdentity>,
    planned_source_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    operation_id: &str,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    atomic_replace_with_claim_path_and_observer_with_actual_paths(
        source,
        target,
        expected_source_identity,
        planned_source_claim_path,
        cancel,
        operation_id,
        observer,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn atomic_replace_with_claim_path_and_observer_with_actual_paths(
    source: &Path,
    target: &Path,
    expected_source_identity: Option<&identity::ExpectedFileIdentity>,
    planned_source_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    operation_id: &str,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::strategy::with_mutation_strategy(
            source,
            target,
            cancel,
            crate::platform::macos::strategy::MacCoordinatedOperation::Replace,
            |coordinated_source, coordinated_target| {
                atomic_replace_uncoordinated(
                    coordinated_source,
                    coordinated_target,
                    expected_source_identity,
                    planned_source_claim_path,
                    cancel,
                    operation_id,
                    observer,
                    actual_path_observer,
                )
            },
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        atomic_replace_uncoordinated(
            source,
            target,
            expected_source_identity,
            planned_source_claim_path,
            cancel,
            operation_id,
            observer,
            actual_path_observer,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn atomic_replace_uncoordinated(
    source: &Path,
    target: &Path,
    expected_source_identity: Option<&identity::ExpectedFileIdentity>,
    planned_source_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    _operation_id: &str,
    mut observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    platform_support::ensure_supported_file_mutation().map_err(map_platform_error)?;
    if is_cancelled(cancel) {
        return Err(AtomicMoveError::Cancelled);
    }
    let target_parent_path = target.parent().ok_or(AtomicMoveError::UnsafePath)?;
    let target_name = target.file_name().ok_or(AtomicMoveError::UnsafePath)?;
    let target_parent =
        VerifiedDirectory::open_existing(target_parent_path).map_err(map_directory_error)?;
    let source_claim_path = match planned_source_claim_path {
        Some(path) => source_claim::rebind_claim_path(source, path),
        None => source_claim::planned_claim_path(source, "replace"),
    }
    .map_err(map_claim_error)?;
    if let Some(callback) = actual_path_observer {
        callback(source, target, Some(&source_claim_path))?;
    }
    #[cfg(target_os = "macos")]
    crate::platform::macos::mutation::ensure_path_eligible(source, target_parent.path())
        .map_err(AtomicMoveError::MacMutationNotSupported)?;
    #[cfg(target_os = "macos")]
    if !crate::platform::macos::strategy::verify_source_retirement_capability(source).eligible {
        return Err(AtomicMoveError::MacMutationNotSupported(
            crate::platform::macos::strategy::MAC_FILESYSTEM_CAPABILITY_INSUFFICIENT,
        ));
    }
    if std::fs::symlink_metadata(target).is_err() || same_macos_namespace_entry(source, target) {
        return Err(AtomicMoveError::TargetExists);
    }
    let target_identity =
        identity::capture_namespace_identity(target, cancel).map_err(map_identity_error)?;
    // The backup name must be derivable from the durable operation paths after
    // a restart.  The preview operation id is not persisted independently of
    // the batch log, so including it here would make replacement recovery
    // unable to find the retained destination object.
    let backup_path = replacement_backup_path(source, target);
    if std::fs::symlink_metadata(&backup_path).is_ok() {
        return Err(AtomicMoveError::TargetExists);
    }
    let mut backup_claim = source_claim::claim_source_at(
        target,
        &target_identity,
        &backup_path,
        "replace-backup",
        cancel,
    )
    .map_err(map_claim_error)?;
    #[cfg(any(test, feature = "native-qa"))]
    source_claim::run_claim_test_hook(
        source_claim::ClaimTestPoint::AfterReplacementBackupClaimed,
        target,
        &backup_path,
    );
    if let Err(error) = backup_claim.verify_current_namespace_binding() {
        return Err(map_claim_error(error));
    }

    let expected_source = match expected_source_identity {
        Some(expected)
            if expected.full_hash.is_some()
                || (cfg!(target_os = "macos")
                    && expected.platform_volume_id.is_some()
                    && expected.platform_file_id.is_some()) =>
        {
            expected.clone()
        }
        Some(_) => {
            let _ = backup_claim.rollback_to_original();
            return Err(AtomicMoveError::SourceClaimFailed(
                "source identity is incomplete".to_string(),
            ));
        }
        None => identity::capture_namespace_identity(source, cancel).map_err(|error| {
            let _ = backup_claim.rollback_to_original();
            map_identity_error(error)
        })?,
    };
    let mut source_claim = match source_claim::claim_source_at(
        source,
        &expected_source,
        &source_claim_path,
        "replace",
        cancel,
    ) {
        Ok(claim) => claim,
        Err(error) => {
            let _ = backup_claim.rollback_to_original();
            return Err(map_claim_error(error));
        }
    };
    if let Err(error) = notify_phase(&mut observer, "source_claimed") {
        let _ = source_claim.rollback_to_original();
        let _ = backup_claim.rollback_to_original();
        return Err(error);
    }

    #[cfg(target_os = "macos")]
    let commit_result = crate::platform::macos::copy::copy_commit_claim(
        &mut source_claim,
        target_parent,
        target_name,
        cancel,
        None,
    );
    #[cfg(windows)]
    let commit_result = {
        let same_volume = source_claim.original_volume_id() == target_parent.identity().volume_id;
        if same_volume {
            source_claim
                .commit_to_with_cancel(target_parent, target_name, cancel)
                .map(|_| ())
                .map_err(map_claim_error)
        } else {
            if source_claim.kind() != source_claim::ClaimedEntryKind::File {
                Err(AtomicMoveError::CrossVolumeDirectoryMoveUnsupported)
            } else {
                copy_commit::copy_commit_claim(
                    &mut source_claim,
                    target_parent,
                    target_name,
                    cancel,
                    None,
                )
            }
        }
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let commit_result: Result<(), AtomicMoveError> = Err(AtomicMoveError::UnsupportedPlatformLinux);

    match commit_result {
        Ok(()) => {
            notify_phase(&mut observer, "target_committed")?;
            notify_phase(&mut observer, "source_cleanup_pending")?;
            notify_phase(&mut observer, "completed")?;
            Ok(AtomicMoveOutcome {
                method: AtomicMoveMethod::CrossVolumeCopyCommit,
                commit_state: AtomicMoveCommitState::Completed,
            })
        }
        Err(error) => {
            if !error.is_post_commit() {
                let _ = source_claim.rollback_to_original();
                if backup_claim.rollback_to_original().is_err() {
                    return Err(AtomicMoveError::SourceClaimRollbackFailed(
                        "replacement backup rollback requires manual recovery".to_string(),
                    ));
                }
            }
            Err(error)
        }
    }
}

pub(crate) fn replacement_backup_path(source: &Path, target: &Path) -> std::path::PathBuf {
    let source = replacement_namespace_path(source);
    let target = replacement_namespace_path(target);
    let key = format!("{}\0{}", source.display(), target.display());
    let digest = blake3::hash(key.as_bytes()).to_hex().to_string();
    let parent = target.parent().unwrap_or(&target);
    let backup_name = format!(".zen-canvas-replace-{}", &digest[..24]);
    #[cfg(target_os = "macos")]
    {
        parent
            .join(".zen-canvas-retirement")
            .join(format!("replace-{}", &digest[..24]))
            .join(backup_name)
    }
    #[cfg(not(target_os = "macos"))]
    {
        parent.join(backup_name)
    }
}

#[cfg(target_os = "macos")]
fn replacement_namespace_path(path: &Path) -> std::path::PathBuf {
    let Some(name) = path.file_name() else {
        return path.to_path_buf();
    };
    path.parent()
        .and_then(|parent| parent.canonicalize().ok())
        .map(|parent| parent.join(name))
        .unwrap_or_else(|| path.to_path_buf())
}

#[cfg(not(target_os = "macos"))]
fn replacement_namespace_path(path: &Path) -> std::path::PathBuf {
    path.to_path_buf()
}

#[allow(clippy::too_many_arguments)]
fn atomic_copy_noreplace_uncoordinated(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    #[cfg(target_os = "macos")] observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    #[cfg(not(target_os = "macos"))] mut observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    #[cfg(target_os = "macos")] actual_path_observer: Option<
        &mut crate::fs_safety::ActualPathObserver<'_>,
    >,
    #[cfg(not(target_os = "macos"))] actual_path_observer: Option<
        &mut crate::fs_safety::ActualPathObserver<'_>,
    >,
    expected_target_parent: Option<&super::DirectoryIdentity>,
) -> Result<AtomicMoveOutcome, AtomicMoveError> {
    platform_support::ensure_supported_file_mutation().map_err(map_platform_error)?;
    if is_cancelled(cancel) {
        return Err(AtomicMoveError::Cancelled);
    }
    #[cfg(target_os = "macos")]
    {
        let target_parent_path = target.parent().ok_or(AtomicMoveError::UnsafePath)?;
        let target_parent =
            VerifiedDirectory::open_existing(target_parent_path).map_err(map_directory_error)?;
        if expected_target_parent.is_some_and(|expected| expected != target_parent.identity()) {
            return Err(AtomicMoveError::UnsafePath);
        }
        crate::platform::macos::mutation::ensure_path_eligible(source, target_parent.path())
            .map_err(AtomicMoveError::MacMutationNotSupported)?;
        crate::platform::macos::copy::copy_commit_source_stable(
            source,
            target,
            expected_identity,
            planned_claim_path,
            cancel,
            observer,
            false,
            actual_path_observer,
            expected_target_parent,
        )
        .map(|_| AtomicMoveOutcome {
            method: AtomicMoveMethod::CrossVolumeCopyCommit,
            commit_state: AtomicMoveCommitState::Completed,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let target_parent_path = target.parent().ok_or(AtomicMoveError::UnsafePath)?;
        let target_name = target.file_name().ok_or(AtomicMoveError::UnsafePath)?;
        let target_parent =
            VerifiedDirectory::open_existing(target_parent_path).map_err(map_directory_error)?;
        if expected_target_parent.is_some_and(|expected| expected != target_parent.identity()) {
            return Err(AtomicMoveError::UnsafePath);
        }
        if std::fs::symlink_metadata(target).is_ok() {
            return Err(AtomicMoveError::TargetExists);
        }
        let expected = match expected_identity {
            Some(expected) if expected.full_hash.is_some() => expected.clone(),
            Some(_) => {
                return Err(AtomicMoveError::SourceClaimFailed(
                    "source identity is incomplete".to_string(),
                ));
            }
            None => {
                identity::capture_namespace_identity(source, cancel).map_err(map_identity_error)?
            }
        };
        let claim_path = match planned_claim_path {
            Some(path) => source_claim::rebind_claim_path(source, path).map_err(map_claim_error)?,
            None => source_claim::planned_claim_path(source, "copy").map_err(map_claim_error)?,
        };
        if let Some(callback) = actual_path_observer {
            callback(source, target, Some(&claim_path))?;
        }
        let mut claim =
            source_claim::claim_source_at(source, &expected, &claim_path, "copy", cancel)
                .map_err(map_claim_error)?;
        if let Err(error) = notify_phase(&mut observer, "source_claimed") {
            return match claim.rollback_to_original() {
                Ok(()) => Err(error),
                Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
                    rollback.to_string(),
                )),
            };
        }

        #[cfg(windows)]
        {
            copy_commit::copy_commit_claim_preserving_source(
                &mut claim,
                target_parent,
                target_name,
                cancel,
                observer,
            )
            .map(|_| AtomicMoveOutcome {
                method: AtomicMoveMethod::CrossVolumeCopyCommit,
                commit_state: AtomicMoveCommitState::Completed,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = claim.rollback_to_original();
            Err(AtomicMoveError::UnsupportedPlatformLinux)
        }
    }
}

#[cfg(any(test, feature = "native-qa"))]
pub mod test_faults {
    use std::cell::RefCell;
    #[cfg(all(test, windows))]
    use std::sync::{Mutex, MutexGuard, OnceLock};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AtomicFaultPoint {
        TargetDurability,
        TargetIdentity,
        SourceCleanup,
        CancelDirectoryCopyAfterEntry,
    }

    thread_local! {
        static FAULT: RefCell<Option<AtomicFaultPoint>> = const { RefCell::new(None) };
    }

    #[cfg(all(test, windows))]
    fn serial() -> &'static Mutex<()> {
        static SERIAL: OnceLock<Mutex<()>> = OnceLock::new();
        SERIAL.get_or_init(|| Mutex::new(()))
    }

    #[cfg(all(test, windows))]
    pub(crate) fn lock() -> MutexGuard<'static, ()> {
        serial()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set_fault(point: Option<AtomicFaultPoint>) {
        FAULT.with(|fault| *fault.borrow_mut() = point);
    }

    pub(crate) fn take_fault(point: AtomicFaultPoint) -> bool {
        FAULT.with(|fault| {
            let mut current = fault.borrow_mut();
            if *current == Some(point) {
                *current = None;
                true
            } else {
                false
            }
        })
    }
}

fn map_platform_error(error: platform_support::PlatformSupportError) -> AtomicMoveError {
    match error {
        platform_support::PlatformSupportError::LinuxUnsupported => {
            AtomicMoveError::UnsupportedPlatformLinux
        }
        platform_support::PlatformSupportError::MacosFileMutationSourceBindingUnsupported => {
            AtomicMoveError::MacosFileMutationSourceBindingUnsupported
        }
    }
}

fn rollback_after_failure(
    claim: &mut source_claim::SourceClaim,
    error: SourceClaimError,
) -> AtomicMoveError {
    let mapped = map_claim_error(error);
    // Once the private claim pathname has rebound, even rollback is a
    // destructive name-based operation against an untrusted entry. Retain
    // both the attacker object and the private claim for manual review.
    if mapped.is_mac_claim_safety_error() {
        return mapped;
    }
    if matches!(mapped, AtomicMoveError::TargetExists) {
        return match claim.rollback_to_original() {
            Ok(()) => AtomicMoveError::TargetExists,
            Err(error) => AtomicMoveError::SourceClaimRollbackFailed(error.to_string()),
        };
    }
    match claim.rollback_to_original() {
        Ok(()) => mapped,
        Err(rollback_error) => {
            AtomicMoveError::SourceClaimRollbackFailed(rollback_error.to_string())
        }
    }
}

pub(crate) fn map_directory_error(error: super::PathGuardError) -> AtomicMoveError {
    match error {
        super::PathGuardError::UnsupportedPlatformLinux => {
            AtomicMoveError::UnsupportedPlatformLinux
        }
        super::PathGuardError::MacosFileMutationSourceBindingUnsupported => {
            AtomicMoveError::MacosFileMutationSourceBindingUnsupported
        }
        super::PathGuardError::IdentityChanged => AtomicMoveError::TargetParentIdentityChanged,
        super::PathGuardError::ReparsePoint => AtomicMoveError::ReparsePoint,
        super::PathGuardError::UnsafePath => AtomicMoveError::UnsafePath,
        super::PathGuardError::Io(error) => AtomicMoveError::Io(error),
    }
}

pub(crate) fn map_claim_error(error: SourceClaimError) -> AtomicMoveError {
    match error {
        SourceClaimError::UnsupportedPlatformLinux => AtomicMoveError::UnsupportedPlatformLinux,
        SourceClaimError::MacosFileMutationSourceBindingUnsupported => {
            AtomicMoveError::MacosFileMutationSourceBindingUnsupported
        }
        SourceClaimError::MacMutationNotSupported(code) => {
            AtomicMoveError::MacMutationNotSupported(code)
        }
        SourceClaimError::SourceMissing => AtomicMoveError::SourceMissing,
        SourceClaimError::SourceIdentityChanged => AtomicMoveError::SourceChanged,
        SourceClaimError::ClaimFailed(error) => AtomicMoveError::SourceClaimFailed(error),
        SourceClaimError::ClaimMismatch => AtomicMoveError::SourceClaimMismatch,
        SourceClaimError::ClaimRollbackFailed(error) => {
            AtomicMoveError::SourceClaimRollbackFailed(error)
        }
        SourceClaimError::RecoveryRequired(error) => {
            AtomicMoveError::SourceClaimRecoveryRequired(error)
        }
        SourceClaimError::MacClaimNamespaceRebound => AtomicMoveError::MacClaimNamespaceRebound,
        SourceClaimError::MacClaimIdentityMismatch => AtomicMoveError::MacClaimIdentityMismatch,
        SourceClaimError::MacClaimPathMissing => AtomicMoveError::MacClaimPathMissing,
        SourceClaimError::MacClaimPathUnreadable => AtomicMoveError::MacClaimPathUnreadable,
        SourceClaimError::TargetExists => AtomicMoveError::TargetExists,
        SourceClaimError::CrossDevice => AtomicMoveError::CrossDevice,
        SourceClaimError::AtomicSourceBindingUnsupported => {
            AtomicMoveError::AtomicSourceBindingUnsupported
        }
        SourceClaimError::ReparsePoint => AtomicMoveError::ReparsePoint,
        SourceClaimError::UnsupportedFileType => AtomicMoveError::UnsafePath,
        SourceClaimError::Cancelled => AtomicMoveError::Cancelled,
        SourceClaimError::Io(error) => AtomicMoveError::Io(error),
    }
}

fn map_identity_error(error: identity::IdentityError) -> AtomicMoveError {
    match error {
        identity::IdentityError::SourceMissing => AtomicMoveError::SourceMissing,
        identity::IdentityError::Symlink => AtomicMoveError::Symlink,
        identity::IdentityError::UnsupportedFileType => AtomicMoveError::UnsafePath,
        identity::IdentityError::DirectoryManifestNameEncodingFailed => {
            AtomicMoveError::DirectoryManifestNameEncodingFailed
        }
        identity::IdentityError::Cancelled => AtomicMoveError::Cancelled,
        identity::IdentityError::ContentReadRejected(reason) => map_content_read_rejected(reason),
        identity::IdentityError::Io(error) => AtomicMoveError::Io(error),
    }
}

fn map_content_read_rejected(reason: &'static str) -> AtomicMoveError {
    #[cfg(target_os = "macos")]
    {
        AtomicMoveError::MacMutationNotSupported(reason)
    }
    #[cfg(not(target_os = "macos"))]
    {
        AtomicMoveError::Io(io::Error::new(io::ErrorKind::PermissionDenied, reason))
    }
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Acquire))
}

#[cfg(target_os = "macos")]
fn same_macos_namespace_entry(source: &Path, target: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(source) = std::fs::symlink_metadata(source) else {
        return false;
    };
    let Ok(target) = std::fs::symlink_metadata(target) else {
        return false;
    };
    source.dev() == target.dev()
        && source.ino() == target.ino()
        && source.file_type().is_symlink() == target.file_type().is_symlink()
        && source.is_file() == target.is_file()
        && source.is_dir() == target.is_dir()
}

#[cfg(not(target_os = "macos"))]
fn same_macos_namespace_entry(_source: &Path, _target: &Path) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{fs, sync::atomic::AtomicBool};

    fn fixture(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-atomic-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("fixture");
        path
    }

    #[test]
    fn transfer_volume_policy_allows_cross_volume_only_for_transfer_intents() {
        assert!(matches!(
            enforce_target_volume_policy("source-volume", "target-volume", true, false),
            Err(SourceClaimError::CrossDevice)
        ));
        assert!(enforce_target_volume_policy("source-volume", "target-volume", true, true).is_ok());
        assert!(
            enforce_target_volume_policy("source-volume", "source-volume", true, false).is_ok()
        );
    }

    #[test]
    fn target_created_before_commit_is_never_overwritten() {
        let root = fixture("target-exists");
        let source = root.join("source");
        let target = root.join("target");
        fs::write(&source, b"source").expect("source");
        fs::write(&target, b"target").expect("target");
        let error =
            atomic_move_noreplace(&source, &target, None, None).expect_err("target conflict");
        assert!(matches!(error, AtomicMoveError::TargetExists));
        assert_eq!(fs::read(&source).expect("source bytes"), b"source");
        assert_eq!(fs::read(&target).expect("target bytes"), b"target");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_before_claim_leaves_source_and_target_untouched() {
        let root = fixture("cancel-before-claim");
        let source = root.join("source");
        let target = root.join("target");
        fs::write(&source, b"source").expect("source");
        let cancel = AtomicBool::new(true);

        let error = atomic_move_noreplace(&source, &target, None, Some(&cancel))
            .expect_err("cancelled move");

        assert!(matches!(error, AtomicMoveError::Cancelled));
        assert_eq!(fs::read(&source).expect("source bytes"), b"source");
        assert!(!target.exists());
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod mac_tests {
    use super::*;
    use std::fs;

    fn find_private_entry(root: &Path, prefix: &str) -> Option<std::path::PathBuf> {
        let entries = fs::read_dir(root).ok()?;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
            {
                return Some(path);
            }
            if fs::symlink_metadata(&path)
                .ok()
                .is_some_and(|metadata| metadata.is_dir())
            {
                if let Some(found) = find_private_entry(&path, prefix) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn rebind_move_claim(point: source_claim::ClaimTestPoint, _source: &Path, claim: &Path) {
        if !matches!(
            point,
            source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeTrashCommit
                | source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeRestoreCommit
        ) {
            return;
        }
        let saved = claim.with_file_name(".zen-canvas-attacker-move-save");
        fs::rename(claim, &saved).expect("save coordinated claim");
        fs::write(claim, b"attacker coordinated replacement")
            .expect("write coordinated replacement");
    }

    fn rebind_replacement_backup(
        point: source_claim::ClaimTestPoint,
        _source: &Path,
        claim: &Path,
    ) {
        if point != source_claim::ClaimTestPoint::AfterReplacementBackupClaimed {
            return;
        }
        let saved = claim.with_file_name(".zen-canvas-attacker-replacement-save");
        fs::rename(claim, &saved).expect("save original replacement backup");
        fs::write(claim, b"attacker replacement backup").expect("write replacement backup");
    }

    #[test]
    fn macos_atomic_move_claims_and_publishes_target() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-atomic-macos-parity-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, b"source").expect("source");

        let outcome = atomic_move_noreplace(&source, &target, None, None)
            .expect("macOS parity move should claim and publish the target");

        assert_eq!(outcome.commit_state, AtomicMoveCommitState::Completed);
        assert!(!source.exists());
        assert_eq!(fs::read(&target).expect("target bytes"), b"source");
        assert!(!fs::read_dir(&root)
            .expect("root entries")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".zen-canvas-claim-")));
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn macos_safe_trash_and_restore_rebinding_keep_the_attacker_object_manual() {
        let _serial = source_claim::lock_claim_test_hooks();
        for (label, operation) in [
            ("safe-trash", AtomicMoveOperation::Trash),
            ("restore", AtomicMoveOperation::Restore),
        ] {
            let root = std::env::temp_dir().join(format!(
                "zen-canvas-atomic-macos-{label}-rebind-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&root).expect("fixture");
            let source = root.join("source.txt");
            let target = root.join("target.txt");
            fs::write(&source, b"source").expect("source");
            source_claim::set_claim_test_hook(Some(rebind_move_claim));
            let result = atomic_move_noreplace_with_claim_path_and_observer_for_operation(
                &source, &target, None, None, None, None, operation,
            );
            source_claim::set_claim_test_hook(None);

            assert!(matches!(
                result,
                Err(AtomicMoveError::MacClaimNamespaceRebound)
            ));
            assert!(!target.exists(), "{label} committed a rebound claim");
            assert_eq!(
                fs::read(
                    find_private_entry(&root, ".zen-canvas-attacker-move-save")
                        .expect("saved original claim")
                )
                .expect("saved original claim"),
                b"source"
            );
            assert_eq!(
                fs::read(
                    find_private_entry(&root, ".zen-canvas-claim-").expect("replacement claim"),
                )
                .expect("attacker claim bytes"),
                b"attacker coordinated replacement"
            );
            fs::remove_dir_all(root).expect("remove fixture");
        }
    }

    #[test]
    fn macos_replacement_backup_rebinding_is_manual_and_keeps_all_objects() {
        let _serial = source_claim::lock_claim_test_hooks();
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-atomic-macos-replace-rebind-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, b"new source").expect("source");
        fs::write(&target, b"old target").expect("target");

        source_claim::set_claim_test_hook(Some(rebind_replacement_backup));
        let result = atomic_replace_with_claim_path_and_observer(
            &source,
            &target,
            None,
            None,
            None,
            "replacement-rebind",
            None,
        );
        source_claim::set_claim_test_hook(None);

        assert!(matches!(
            result,
            Err(AtomicMoveError::MacClaimNamespaceRebound)
        ));
        assert_eq!(fs::read(&source).expect("source remains"), b"new source");
        assert!(!target.exists());
        assert_eq!(
            fs::read(
                find_private_entry(&root, ".zen-canvas-attacker-replacement-save")
                    .expect("saved old target"),
            )
            .expect("saved old target"),
            b"old target"
        );
        let backup =
            find_private_entry(&root, ".zen-canvas-replace-").expect("replacement backup claim");
        assert_eq!(
            fs::read(backup).expect("attacker backup"),
            b"attacker replacement backup"
        );
        let _ = fs::remove_dir_all(root);
    }
}
