//! Verified macOS copy/clone publication.
//!
//! This is the cross-volume leg of the existing `AtomicMove` transaction.  It
//! consumes the already-claimed source object, stages into the verified target
//! directory with an exclusive name, verifies content identity, publishes with
//! `RENAME_EXCL`, and retires the claim only after the destination is verified.

use crate::fs_safety::{
    capture_namespace_identity, identity, AtomicMoveError, ClaimedEntryKind, SourceClaim,
    VerifiedDirectory,
};
#[cfg(any(test, feature = "native-qa"))]
use std::sync::atomic::AtomicU64;
use std::{
    collections::HashMap,
    ffi::{CString, OsStr, OsString},
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, FromRawFd, RawFd},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const COPYFILE_ALL: u32 = 0x0000_000f;
// COPYFILE_METADATA == COPYFILE_ACL | COPYFILE_STAT | COPYFILE_XATTR.
// COPYFILE_DATA is 0x00000008; it must not be included in a metadata-only
// pass because fcopyfile would then attempt a second data copy into the
// already-staged destination.
const COPYFILE_METADATA: u32 = 0x0000_0007;
const STAGING_PREFIX: &str = ".zen-canvas-stage-";
const HASH_BUFFER_SIZE: usize = 1024 * 1024;
const SAMPLE_SIZE: usize = 1024 * 1024;

#[cfg(any(test, feature = "native-qa"))]
static COPY_READ_CALLS: AtomicU64 = AtomicU64::new(0);
#[cfg(any(test, feature = "native-qa"))]
static COPY_READ_BYTES: AtomicU64 = AtomicU64::new(0);

#[cfg(any(test, feature = "native-qa"))]
pub fn reset_copy_io_metrics() {
    COPY_READ_CALLS.store(0, Ordering::Relaxed);
    COPY_READ_BYTES.store(0, Ordering::Relaxed);
}

#[cfg(any(test, feature = "native-qa"))]
pub fn copy_io_metrics() -> (u64, u64) {
    (
        COPY_READ_CALLS.load(Ordering::Relaxed),
        COPY_READ_BYTES.load(Ordering::Relaxed),
    )
}

#[cfg(any(test, feature = "native-qa"))]
#[inline]
fn record_copy_read(bytes: usize) {
    COPY_READ_CALLS.fetch_add(1, Ordering::Relaxed);
    COPY_READ_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

#[cfg(not(any(test, feature = "native-qa")))]
#[inline]
fn record_copy_read(_bytes: usize) {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyVerificationPolicy {
    /// A successful clone syscall plus physical identity and bounded metadata
    /// checks is sufficient when no content hash was requested.
    PhysicalClone,
    /// Stream the source once, hash the bytes as they are written, and bind
    /// the result to the preview's expected content identity.
    StreamingHash,
    /// Retained for directory/package paths whose manifest verification still
    /// needs a full post-copy identity walk.
    FullPostVerify,
}

#[derive(Debug, Clone)]
enum CopyProof {
    /// `fclonefileat` is the native content-preserving primitive. The
    /// staging identity is retained so a later namespace rebind cannot turn
    /// the proof into a pathname-only check.
    NativeClone {
        staging_physical: crate::platform::macos::identity::MacPhysicalIdentity,
    },
    /// The source descriptor was streamed into the staging descriptor while
    /// producing a full content identity. The staged descriptor is checked
    /// against that proof before publication.
    Streamed {
        identity: identity::ExpectedFileIdentity,
        staging_physical: crate::platform::macos::identity::MacPhysicalIdentity,
    },
    Structural {
        staging_physical: crate::platform::macos::identity::MacPhysicalIdentity,
    },
}

impl CopyProof {
    fn staging_physical(&self) -> crate::platform::macos::identity::MacPhysicalIdentity {
        match self {
            Self::NativeClone { staging_physical }
            | Self::Streamed {
                staging_physical, ..
            }
            | Self::Structural { staging_physical } => *staging_physical,
        }
    }
}

fn copy_verification_policy(
    kind: ClaimedEntryKind,
    expected: Option<&identity::ExpectedFileIdentity>,
) -> CopyVerificationPolicy {
    match kind {
        ClaimedEntryKind::File if expected.is_some_and(|value| value.full_hash.is_some()) => {
            CopyVerificationPolicy::StreamingHash
        }
        ClaimedEntryKind::File | ClaimedEntryKind::Symlink => CopyVerificationPolicy::PhysicalClone,
        ClaimedEntryKind::Directory => CopyVerificationPolicy::FullPostVerify,
    }
}

pub(crate) fn copy_commit_claim(
    claim: &mut SourceClaim,
    target_parent: VerifiedDirectory,
    target_name: &OsStr,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), AtomicMoveError> {
    copy_commit_claim_with_source_retirement(
        claim,
        target_parent,
        target_name,
        cancel,
        observer,
        true,
    )
}

fn copy_commit_claim_with_source_retirement(
    claim: &mut SourceClaim,
    target_parent: VerifiedDirectory,
    target_name: &OsStr,
    cancel: Option<&AtomicBool>,
    mut observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    retire_source: bool,
) -> Result<(), AtomicMoveError> {
    target_parent
        .ensure_unchanged()
        .map_err(|_| AtomicMoveError::TargetParentIdentityChanged)?;
    if is_cancelled(cancel) {
        return rollback_before_publish(claim, AtomicMoveError::Cancelled);
    }

    let target_path = target_parent.path().join(target_name);
    if fs::symlink_metadata(&target_path).is_ok() {
        return rollback_before_publish(claim, AtomicMoveError::TargetExists);
    }
    let staging_name = OsString::from(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
    let staging_path = target_parent.path().join(&staging_name);
    let source_namespace_identity = match claim.kind() {
        ClaimedEntryKind::File | ClaimedEntryKind::Directory => {
            let handle = claim
                .clone_handle()
                .map_err(|error| rollback_claim_error(claim, map_claim_error(error)))?
                .ok_or_else(|| rollback_claim_error(claim, AtomicMoveError::UnsafePath))?;
            crate::fs_safety::identity::capture_namespace_identity_from_handle(
                &handle,
                claim.current_path(),
                cancel,
            )
            .map_err(|error| {
                rollback_claim_error(
                    claim,
                    AtomicMoveError::Io(io::Error::other(error.to_string())),
                )
            })?
        }
        ClaimedEntryKind::Symlink => {
            crate::fs_safety::identity::capture_namespace_identity(claim.current_path(), cancel)
                .map_err(|error| {
                    rollback_claim_error(
                        claim,
                        AtomicMoveError::Io(io::Error::other(error.to_string())),
                    )
                })?
        }
    };
    if !namespace_identity_matches(
        claim.expected_identity(),
        &source_namespace_identity,
        claim.kind(),
    ) {
        return rollback_before_publish(claim, AtomicMoveError::SourceChanged);
    }
    let source_identity = if matches!(claim.kind(), ClaimedEntryKind::Directory) {
        claim
            .verify_content_identity(cancel)
            .map_err(|error| rollback_claim_error(claim, map_claim_error(error)))?
    } else {
        source_namespace_identity.clone()
    };
    notify_phase(&mut observer, "copying")?;

    let copy_proof = match copy_object(
        claim,
        target_parent.raw_fd(),
        &staging_name,
        cancel,
        Some(claim.expected_identity()),
    ) {
        Ok(proof) => proof,
        Err(error) => {
            cleanup_staging_at(target_parent.raw_fd(), &staging_name, None);
            return rollback_before_publish(claim, error);
        }
    };

    let source_after_copy = if matches!(claim.kind(), ClaimedEntryKind::File) {
        let handle = match claim.clone_handle() {
            Ok(Some(handle)) => handle,
            Ok(None) => {
                cleanup_staging_at(
                    target_parent.raw_fd(),
                    &staging_name,
                    Some(copy_proof.staging_physical()),
                );
                return rollback_before_publish(claim, AtomicMoveError::UnsafePath);
            }
            Err(error) => {
                cleanup_staging_at(
                    target_parent.raw_fd(),
                    &staging_name,
                    Some(copy_proof.staging_physical()),
                );
                return rollback_before_publish(claim, map_claim_error(error));
            }
        };
        match crate::fs_safety::identity::capture_namespace_identity_from_handle(
            &handle,
            claim.current_path(),
            cancel,
        ) {
            Ok(identity) => identity,
            Err(error) => {
                cleanup_staging_at(
                    target_parent.raw_fd(),
                    &staging_name,
                    Some(copy_proof.staging_physical()),
                );
                return rollback_before_publish(
                    claim,
                    AtomicMoveError::Io(io::Error::other(error.to_string())),
                );
            }
        }
    } else {
        match claim.verify_content_identity(cancel) {
            Ok(identity) => identity,
            Err(error) => {
                cleanup_staging_at(
                    target_parent.raw_fd(),
                    &staging_name,
                    Some(copy_proof.staging_physical()),
                );
                return rollback_before_publish(claim, map_claim_error(error));
            }
        }
    };
    let source_identity_matches = if matches!(claim.kind(), ClaimedEntryKind::File) {
        namespace_identity_matches(&source_identity, &source_after_copy, ClaimedEntryKind::File)
    } else {
        identity::content_identity_matches(&source_identity, &source_after_copy)
    };
    if !source_identity_matches {
        cleanup_staging_at(
            target_parent.raw_fd(),
            &staging_name,
            Some(copy_proof.staging_physical()),
        );
        return rollback_before_publish(claim, AtomicMoveError::SourceChanged);
    }

    if let Err(error) = verify_staged_copy(
        &copy_proof,
        claim.kind(),
        &staging_path,
        target_parent.raw_fd(),
        &staging_name,
        &source_identity,
        Some(claim.expected_identity()),
        cancel,
    ) {
        cleanup_staging_at(
            target_parent.raw_fd(),
            &staging_name,
            Some(copy_proof.staging_physical()),
        );
        return rollback_before_publish(claim, error);
    }
    #[cfg(any(test, feature = "native-qa"))]
    crate::fs_safety::source_claim::run_claim_test_hook(
        crate::fs_safety::source_claim::ClaimTestPoint::AfterCopyProofVerifiedBeforePublish,
        &staging_path,
        &target_path,
    );
    #[cfg(any(test, feature = "native-qa"))]
    if crate::fs_safety::source_claim::current_claim_test_hook().is_some() {
        if let Err(error) = verify_staged_copy(
            &copy_proof,
            claim.kind(),
            &staging_path,
            target_parent.raw_fd(),
            &staging_name,
            &source_identity,
            Some(claim.expected_identity()),
            cancel,
        ) {
            cleanup_staging_at(
                target_parent.raw_fd(),
                &staging_name,
                Some(copy_proof.staging_physical()),
            );
            return rollback_before_publish(claim, error);
        }
    }
    if is_cancelled(cancel) {
        cleanup_staging_at(
            target_parent.raw_fd(),
            &staging_name,
            Some(copy_proof.staging_physical()),
        );
        return rollback_before_publish(claim, AtomicMoveError::Cancelled);
    }

    let staging_physical = copy_proof.staging_physical();

    publish_staging_exclusive(
        target_parent.raw_fd(),
        &staging_name,
        target_name,
        claim.kind(),
        staging_physical,
        cancel,
    )
    .map_err(|error| {
        cleanup_staging_at(
            target_parent.raw_fd(),
            &staging_name,
            Some(copy_proof.staging_physical()),
        );
        rollback_claim_error(claim, error)
    })?;
    notify_phase(&mut observer, "target_committed")?;

    verify_committed_copy(
        &copy_proof,
        claim.kind(),
        &target_path,
        target_parent.raw_fd(),
        target_name,
        &source_identity,
        Some(claim.expected_identity()),
        cancel,
    )?;
    target_parent
        .sync()
        .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
    notify_phase(&mut observer, "source_cleanup_pending")?;
    if retire_source {
        claim.delete_claim_tree().map_err(|error| {
            AtomicMoveError::TargetCommittedSourceDeleteFailed(format!(
                "{}: {}",
                crate::platform::macos::strategy::MAC_SOURCE_RETIREMENT_PENDING,
                error
            ))
        })?;
    } else {
        claim
            .rollback_to_original()
            .map_err(|error| AtomicMoveError::SourceClaimRollbackFailed(error.to_string()))?;
        claim
            .sync_original_parent()
            .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
    }
    notify_phase(&mut observer, "completed")?;
    Ok(())
}

/// Copies from the source namespace while the source remains at its original
/// pathname.  This is the target-first primitive for macOS Copy/Duplicate and
/// portable or cross-volume Move.  The source is not claimed until the target
/// has been published and verified; a crash or cancellation before that point
/// therefore leaves the source untouched.
// The safety context is intentionally explicit at this boundary: source and
// target observers, cancellation, and retirement state must not be hidden in a
// mutable global or an untyped options bag.
#[allow(clippy::too_many_arguments)]
pub(crate) fn copy_commit_source_stable(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
    mut observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    retire_source: bool,
    mut actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
    expected_target_parent: Option<&crate::fs_safety::DirectoryIdentity>,
) -> Result<(), AtomicMoveError> {
    let target_parent_path = target.parent().ok_or(AtomicMoveError::UnsafePath)?;
    let target_name = target.file_name().ok_or(AtomicMoveError::UnsafePath)?;
    let target_parent = VerifiedDirectory::open_existing(target_parent_path)
        .map_err(|_| AtomicMoveError::TargetParentIdentityChanged)?;
    if expected_target_parent.is_some_and(|expected| expected != target_parent.identity()) {
        return Err(AtomicMoveError::UnsafePath);
    }
    target_parent
        .ensure_unchanged()
        .map_err(|_| AtomicMoveError::TargetParentIdentityChanged)?;
    if is_cancelled(cancel) {
        return Err(AtomicMoveError::Cancelled);
    }
    if fs::symlink_metadata(target).is_ok() {
        return Err(AtomicMoveError::TargetExists);
    }
    if let Some(callback) = actual_path_observer.as_deref_mut() {
        callback(source, target, None)?;
    }

    let source_parent_path = source.parent().ok_or(AtomicMoveError::UnsafePath)?;
    let source_name = source.file_name().ok_or(AtomicMoveError::UnsafePath)?;
    let source_parent = VerifiedDirectory::open_existing(source_parent_path)
        .map_err(|_| AtomicMoveError::SourceChanged)?;
    let source_metadata = fs::symlink_metadata(source).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            AtomicMoveError::SourceMissing
        } else {
            AtomicMoveError::Io(error)
        }
    })?;
    let kind = if source_metadata.file_type().is_symlink() {
        ClaimedEntryKind::Symlink
    } else if source_metadata.is_file() {
        ClaimedEntryKind::File
    } else if source_metadata.is_dir() {
        ClaimedEntryKind::Directory
    } else {
        return Err(AtomicMoveError::UnsafePath);
    };
    let physical_before = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
        source_parent.raw_fd(),
        source_name,
    )
    .map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            AtomicMoveError::SourceMissing
        } else {
            AtomicMoveError::Io(error)
        }
    })?;
    let source_handle = match kind {
        ClaimedEntryKind::File => Some(open_file_at(source_parent.raw_fd(), source_name)?),
        ClaimedEntryKind::Directory => {
            Some(open_file_at_directory(source_parent.raw_fd(), source_name)?)
        }
        ClaimedEntryKind::Symlink => None,
    };
    if let Some(handle) = source_handle.as_ref() {
        let opened = crate::platform::macos::identity::MacPhysicalIdentity::from_fd(handle)
            .map_err(AtomicMoveError::Io)?;
        if !physical_before.matches(opened) {
            return Err(AtomicMoveError::MacClaimIdentityMismatch);
        }
    }

    let verification_policy = copy_verification_policy(kind, expected_identity);
    let source_namespace_identity = match (kind, source_handle.as_ref()) {
        (ClaimedEntryKind::File | ClaimedEntryKind::Directory, Some(handle)) => {
            crate::fs_safety::identity::capture_namespace_identity_from_handle(
                handle, source, cancel,
            )
            .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?
        }
        (_, _) => capture_namespace_identity(source, cancel)
            .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?,
    };
    if let Some(expected) = expected_identity {
        if !namespace_identity_matches(expected, &source_namespace_identity, kind) {
            return Err(AtomicMoveError::SourceChanged);
        }
    }
    // macOS operation previews deliberately persist namespace metadata only.
    // Directory content identity has a different size domain (the logical sum
    // of child content), so capture it only after the namespace binding has
    // passed and use it exclusively for copy/source-stability verification.
    let source_identity = if matches!(kind, ClaimedEntryKind::Directory) {
        let handle = source_handle.as_ref().ok_or(AtomicMoveError::UnsafePath)?;
        crate::fs_safety::capture_identity_from_handle(handle, source, cancel)
            .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?
    } else {
        source_namespace_identity.clone()
    };

    let staging_name = OsString::from(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
    notify_phase(&mut observer, "copying")?;
    let copy_proof = match kind {
        ClaimedEntryKind::File => {
            let handle = source_handle.as_ref().ok_or(AtomicMoveError::UnsafePath)?;
            if matches!(verification_policy, CopyVerificationPolicy::PhysicalClone)
                && clone_file_if_possible(handle, target_parent.raw_fd(), &staging_name).is_ok()
            {
                let destination = open_file_at(target_parent.raw_fd(), &staging_name)?;
                copy_metadata_with_native_api(handle, &destination)?;
                verify_clone_metadata(handle, &destination)?;
                let staging_physical =
                    capture_staging_physical(target_parent.raw_fd(), &staging_name)?;
                Ok(CopyProof::NativeClone { staging_physical })
            } else {
                cleanup_staging_at(target_parent.raw_fd(), &staging_name, None);
                let destination = create_staging_file(target_parent.raw_fd(), &staging_name)?;
                let actual = if matches!(verification_policy, CopyVerificationPolicy::StreamingHash)
                {
                    copy_file_contents_and_verify_expected(
                        handle,
                        &destination,
                        expected_identity,
                        cancel,
                    )?
                } else {
                    copy_file_contents_and_hash(handle, &destination, cancel)?
                };
                destination.sync_all().map_err(AtomicMoveError::Io)?;
                let staging_physical =
                    capture_staging_physical(target_parent.raw_fd(), &staging_name)?;
                Ok(CopyProof::Streamed {
                    identity: actual,
                    staging_physical,
                })
            }
        }
        ClaimedEntryKind::Directory => {
            let handle = source_handle.as_ref().ok_or(AtomicMoveError::UnsafePath)?;
            // Directory clonefile does not preserve hardlink topology on all
            // APFS implementations. Use the structural copier, which retains
            // one destination link per source (dev, inode) pair.
            cleanup_staging_at(target_parent.raw_fd(), &staging_name, None);
            let destination = create_staging_directory(target_parent.raw_fd(), &staging_name)?;
            let source_root_device = {
                use std::os::unix::fs::MetadataExt;
                handle.metadata().map_err(AtomicMoveError::Io)?.dev()
            };
            let mut state = DirectoryCopyState::new(source_root_device);
            copy_tree_from_fd(
                handle.as_raw_fd(),
                destination.as_raw_fd(),
                destination.as_raw_fd(),
                Path::new(""),
                &mut state,
                0,
                cancel,
            )?;
            let metadata = handle.metadata().map_err(AtomicMoveError::Io)?;
            copy_fd_metadata(&metadata, &destination)?;
            destination.sync_all().map_err(AtomicMoveError::Io)?;
            let staging_physical = capture_staging_physical(target_parent.raw_fd(), &staging_name)?;
            Ok(CopyProof::Structural { staging_physical })
        }
        ClaimedEntryKind::Symlink => {
            let link_target = readlink_at(source_parent.raw_fd(), source_name)?;
            symlink_at(&link_target, target_parent.raw_fd(), &staging_name)?;
            let staging_physical = capture_staging_physical(target_parent.raw_fd(), &staging_name)?;
            Ok(CopyProof::Structural { staging_physical })
        }
    };
    let copy_proof = match copy_proof {
        Ok(proof) => proof,
        Err(error) => {
            cleanup_staging_at(target_parent.raw_fd(), &staging_name, None);
            return Err(error);
        }
    };

    if let Err(error) = verify_staged_copy(
        &copy_proof,
        kind,
        &target_parent.path().join(&staging_name),
        target_parent.raw_fd(),
        &staging_name,
        &source_identity,
        expected_identity,
        cancel,
    ) {
        cleanup_staging_at(
            target_parent.raw_fd(),
            &staging_name,
            Some(copy_proof.staging_physical()),
        );
        return Err(error);
    }
    #[cfg(any(test, feature = "native-qa"))]
    crate::fs_safety::source_claim::run_claim_test_hook(
        crate::fs_safety::source_claim::ClaimTestPoint::AfterCopyProofVerifiedBeforePublish,
        &target_parent.path().join(&staging_name),
        target,
    );
    #[cfg(any(test, feature = "native-qa"))]
    if crate::fs_safety::source_claim::current_claim_test_hook().is_some() {
        if let Err(error) = verify_staged_copy(
            &copy_proof,
            kind,
            &target_parent.path().join(&staging_name),
            target_parent.raw_fd(),
            &staging_name,
            &source_identity,
            expected_identity,
            cancel,
        ) {
            cleanup_staging_at(
                target_parent.raw_fd(),
                &staging_name,
                Some(copy_proof.staging_physical()),
            );
            return Err(error);
        }
    }

    let source_after = match (kind, source_handle.as_ref(), verification_policy) {
        (ClaimedEntryKind::File, Some(handle), CopyVerificationPolicy::PhysicalClone)
        | (ClaimedEntryKind::File, Some(handle), CopyVerificationPolicy::StreamingHash) => {
            crate::fs_safety::identity::capture_namespace_identity_from_handle(
                handle, source, cancel,
            )
            .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?
        }
        (_, Some(handle), _) => {
            crate::fs_safety::capture_identity_from_handle(handle, source, cancel)
                .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?
        }
        (_, None, _) => capture_namespace_identity(source, cancel)
            .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?,
    };
    let physical_after = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
        source_parent.raw_fd(),
        source_name,
    )
    .map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            AtomicMoveError::MacClaimPathMissing
        } else {
            AtomicMoveError::MacClaimPathUnreadable
        }
    })?;
    let source_still_matches = if matches!(kind, ClaimedEntryKind::File) {
        namespace_identity_matches(&source_identity, &source_after, ClaimedEntryKind::File)
    } else {
        identity::content_identity_matches(&source_identity, &source_after)
    };
    if !physical_before.matches(physical_after) || !source_still_matches {
        cleanup_staging_at(
            target_parent.raw_fd(),
            &staging_name,
            Some(copy_proof.staging_physical()),
        );
        return Err(AtomicMoveError::MacClaimIdentityMismatch);
    }

    #[cfg(any(test, feature = "native-qa"))]
    crate::fs_safety::source_claim::run_claim_test_hook(
        crate::fs_safety::source_claim::ClaimTestPoint::AfterStagingVerifiedBeforeCommit,
        source,
        target,
    );
    if is_cancelled(cancel) {
        cleanup_staging_at(
            target_parent.raw_fd(),
            &staging_name,
            Some(copy_proof.staging_physical()),
        );
        return Err(AtomicMoveError::Cancelled);
    }

    let staging_physical = copy_proof.staging_physical();

    publish_staging_exclusive(
        target_parent.raw_fd(),
        &staging_name,
        target_name,
        kind,
        staging_physical,
        cancel,
    )
    .inspect_err(|_| {
        cleanup_staging_at(
            target_parent.raw_fd(),
            &staging_name,
            Some(copy_proof.staging_physical()),
        );
    })?;
    notify_phase(&mut observer, "target_committed")?;

    verify_committed_copy(
        &copy_proof,
        kind,
        target,
        target_parent.raw_fd(),
        target_name,
        &source_identity,
        expected_identity,
        cancel,
    )?;
    target_parent
        .sync()
        .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
    if !retire_source {
        notify_phase(&mut observer, "completed")?;
        return Ok(());
    }

    let claim_path = match planned_claim_path {
        Some(path) => crate::fs_safety::source_claim::rebind_claim_path(source, path)
            .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?,
        None => crate::fs_safety::source_claim::planned_claim_path(source, "source-retirement")
            .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?,
    };
    if let Some(callback) = actual_path_observer.as_mut() {
        (*callback)(source, target, Some(&claim_path))?;
    }
    notify_phase(&mut observer, "source_cleanup_pending")?;
    let mut claim = crate::fs_safety::source_claim::claim_source_at(
        source,
        &source_identity,
        &claim_path,
        "source-retirement",
        cancel,
    )
    .map_err(|error| {
        AtomicMoveError::TargetCommittedSourceDeleteFailed(format!(
            "{}: {}",
            crate::platform::macos::strategy::MAC_SOURCE_RETIREMENT_PENDING,
            error
        ))
    })?;
    if let Err(error) = if claim.kind() == ClaimedEntryKind::Directory {
        claim.delete_claim_tree()
    } else {
        claim.delete_claim()
    } {
        return Err(AtomicMoveError::TargetCommittedSourceDeleteFailed(format!(
            "{}: {}",
            crate::platform::macos::strategy::MAC_SOURCE_RETIREMENT_PENDING,
            error
        )));
    }
    notify_phase(&mut observer, "completed")?;
    Ok(())
}

fn copy_object(
    claim: &SourceClaim,
    target_parent_fd: RawFd,
    staging_name: &OsStr,
    cancel: Option<&AtomicBool>,
    expected: Option<&identity::ExpectedFileIdentity>,
) -> Result<CopyProof, AtomicMoveError> {
    match claim.kind() {
        ClaimedEntryKind::File => {
            let source = claim
                .open_read()
                .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?;
            let policy = copy_verification_policy(claim.kind(), expected);
            if matches!(policy, CopyVerificationPolicy::PhysicalClone)
                && clone_file_if_possible(&source, target_parent_fd, staging_name).is_ok()
            {
                let destination = open_file_at(target_parent_fd, staging_name)?;
                copy_metadata_with_native_api(&source, &destination)?;
                verify_clone_metadata(&source, &destination)?;
                let staging_physical = capture_staging_physical(target_parent_fd, staging_name)?;
                return Ok(CopyProof::NativeClone { staging_physical });
            }
            cleanup_staging_at(target_parent_fd, staging_name, None);
            let destination = create_staging_file(target_parent_fd, staging_name)?;
            let actual = if matches!(policy, CopyVerificationPolicy::StreamingHash) {
                copy_file_contents_and_verify_expected(&source, &destination, expected, cancel)?
            } else {
                copy_file_contents_and_hash(&source, &destination, cancel)?
            };
            destination.sync_all().map_err(AtomicMoveError::Io)?;
            let staging_physical = capture_staging_physical(target_parent_fd, staging_name)?;
            Ok(CopyProof::Streamed {
                identity: actual,
                staging_physical,
            })
        }
        ClaimedEntryKind::Directory => {
            let source = claim
                .clone_handle()
                .map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))?
                .ok_or(AtomicMoveError::UnsafePath)?;
            cleanup_staging_at(target_parent_fd, staging_name, None);
            let destination = create_staging_directory(target_parent_fd, staging_name)?;
            let source_root_device = {
                use std::os::unix::fs::MetadataExt;
                source.metadata().map_err(AtomicMoveError::Io)?.dev()
            };
            let mut state = DirectoryCopyState::new(source_root_device);
            copy_tree_from_fd(
                source.as_raw_fd(),
                destination.as_raw_fd(),
                destination.as_raw_fd(),
                Path::new(""),
                &mut state,
                0,
                cancel,
            )?;
            let metadata = source.metadata().map_err(AtomicMoveError::Io)?;
            copy_fd_metadata(&metadata, &destination)?;
            destination.sync_all().map_err(AtomicMoveError::Io)?;
            let staging_physical = capture_staging_physical(target_parent_fd, staging_name)?;
            Ok(CopyProof::Structural { staging_physical })
        }
        ClaimedEntryKind::Symlink => {
            let target = fs::read_link(claim.current_path()).map_err(AtomicMoveError::Io)?;
            symlink_at(&target, target_parent_fd, staging_name)?;
            let staging_physical = capture_staging_physical(target_parent_fd, staging_name)?;
            Ok(CopyProof::Structural { staging_physical })
        }
    }
}

fn capture_staging_physical(
    parent_fd: RawFd,
    staging_name: &OsStr,
) -> Result<crate::platform::macos::identity::MacPhysicalIdentity, AtomicMoveError> {
    crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent_fd, staging_name)
        .map_err(AtomicMoveError::Io)
}

fn capture_copy_identity(
    kind: ClaimedEntryKind,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<identity::ExpectedFileIdentity, AtomicMoveError> {
    let captured = if matches!(kind, ClaimedEntryKind::Symlink) {
        capture_namespace_identity(path, cancel)
    } else {
        crate::fs_safety::capture_identity(path, cancel)
    };
    captured.map_err(|error| AtomicMoveError::Io(io::Error::other(error.to_string())))
}

// Keep the namespace/content/cancellation inputs explicit so each verification
// call documents which identity is being checked at that boundary.
#[allow(clippy::too_many_arguments)]
fn verify_staged_copy(
    proof: &CopyProof,
    kind: ClaimedEntryKind,
    staging_path: &Path,
    parent_fd: RawFd,
    staging_name: &OsStr,
    source_identity: &identity::ExpectedFileIdentity,
    expected: Option<&identity::ExpectedFileIdentity>,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    let current_physical = capture_staging_physical(parent_fd, staging_name)
        .map_err(|_| AtomicMoveError::StagingIdentityChanged)?;
    if !proof.staging_physical().matches_strict(current_physical) {
        return Err(AtomicMoveError::StagingIdentityChanged);
    }
    let content_requested =
        expected.is_some_and(|value| value.sample_hash.is_some() || value.full_hash.is_some());

    match proof {
        CopyProof::NativeClone { .. } => {
            if matches!(kind, ClaimedEntryKind::File)
                && current_physical.size != source_identity.size
            {
                return Err(AtomicMoveError::CopyVerificationFailed);
            }
            if matches!(kind, ClaimedEntryKind::Directory) || content_requested {
                let actual = capture_copy_identity(kind, staging_path, cancel)?;
                let content_expected = if content_requested {
                    expected.unwrap_or(source_identity)
                } else {
                    source_identity
                };
                if !identity::content_identity_matches(content_expected, &actual) {
                    return Err(AtomicMoveError::CopyVerificationFailed);
                }
            }
        }
        CopyProof::Streamed {
            identity: proof, ..
        } => {
            let actual = capture_copy_identity(kind, staging_path, cancel)?;
            if !identity::content_identity_matches(proof, &actual)
                || (content_requested
                    && expected
                        .is_some_and(|value| !identity::content_identity_matches(value, &actual)))
            {
                return Err(AtomicMoveError::CopyVerificationFailed);
            }
        }
        CopyProof::Structural { .. } => {
            let actual = capture_copy_identity(kind, staging_path, cancel)?;
            if !identity::content_identity_matches(source_identity, &actual)
                || (content_requested
                    && expected
                        .is_some_and(|value| !identity::content_identity_matches(value, &actual)))
            {
                return Err(AtomicMoveError::CopyVerificationFailed);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn verify_committed_copy(
    proof: &CopyProof,
    kind: ClaimedEntryKind,
    target: &Path,
    parent_fd: RawFd,
    target_name: &OsStr,
    source_identity: &identity::ExpectedFileIdentity,
    expected: Option<&identity::ExpectedFileIdentity>,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    let current_physical = capture_staging_physical(parent_fd, target_name)
        .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
    if !proof.staging_physical().matches_strict(current_physical) {
        return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
    }
    let content_requested =
        expected.is_some_and(|value| value.sample_hash.is_some() || value.full_hash.is_some());

    match proof {
        CopyProof::NativeClone { .. } => {
            if matches!(kind, ClaimedEntryKind::File)
                && current_physical.size != source_identity.size
            {
                return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
            }
            if matches!(kind, ClaimedEntryKind::Directory) || content_requested {
                let actual = capture_copy_identity(kind, target, cancel)
                    .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
                let content_expected = if content_requested {
                    expected.unwrap_or(source_identity)
                } else {
                    source_identity
                };
                if !identity::content_identity_matches(content_expected, &actual) {
                    return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
                }
            }
        }
        CopyProof::Streamed {
            identity: proof, ..
        } => {
            let actual = capture_copy_identity(kind, target, cancel)
                .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
            if !identity::content_identity_matches(proof, &actual)
                || (content_requested
                    && expected
                        .is_some_and(|value| !identity::content_identity_matches(value, &actual)))
            {
                return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
            }
        }
        CopyProof::Structural { .. } => {
            let actual = capture_copy_identity(kind, target, cancel)
                .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
            if !identity::content_identity_matches(source_identity, &actual)
                || (content_requested
                    && expected
                        .is_some_and(|value| !identity::content_identity_matches(value, &actual)))
            {
                return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
            }
        }
    }
    Ok(())
}

fn clone_file_if_possible(
    source: &File,
    target_parent_fd: RawFd,
    target_name: &OsStr,
) -> Result<(), AtomicMoveError> {
    let target_name =
        CString::new(target_name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    let result = unsafe {
        fclonefileat(
            source.as_raw_fd(),
            target_parent_fd,
            target_name.as_ptr(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(AtomicMoveError::Io(io::Error::last_os_error()))
    }
}

fn namespace_identity_matches(
    expected: &identity::ExpectedFileIdentity,
    actual: &identity::ExpectedFileIdentity,
    kind: ClaimedEntryKind,
) -> bool {
    let mut namespace_expected = expected.clone();
    let content_size_is_not_namespace_size = matches!(kind, ClaimedEntryKind::Directory)
        && (expected.sample_hash.is_some() || expected.full_hash.is_some());
    namespace_expected.sample_hash = None;
    namespace_expected.full_hash = None;
    if content_size_is_not_namespace_size {
        // Directory content identities store the logical sum of descendant
        // bytes in `size`; fstat on the root handle reports the filesystem's
        // directory-entry size. Content size is verified separately against
        // the captured tree identity before and after copying.
        namespace_expected.size = actual.size;
    }
    identity::identity_matches(&namespace_expected, actual)
}

/// Copies a regular file while computing the same BLAKE3 content identity
/// used by the operation preview. The returned proof is later compared with a
/// full identity capture of the staged target so a same-inode staging mutation
/// cannot turn a source-pass proof into a pathname-only check.
fn copy_file_contents_and_verify_expected(
    source: &File,
    destination: &File,
    expected: Option<&identity::ExpectedFileIdentity>,
    cancel: Option<&AtomicBool>,
) -> Result<identity::ExpectedFileIdentity, AtomicMoveError> {
    let expected = expected.ok_or(AtomicMoveError::CopyVerificationFailed)?;
    let actual = copy_file_contents_and_hash(source, destination, cancel)?;
    if !identity::content_identity_matches(expected, &actual) {
        return Err(AtomicMoveError::SourceChanged);
    }
    Ok(actual)
}

/// Streams a regular file once while producing the same BLAKE3 identity as
/// the preview. Callers retain the proof and compare it against the staged
/// target before publication so path rebinding and same-inode mutation are
/// both detected.
fn copy_file_contents_and_hash(
    source: &File,
    destination: &File,
    cancel: Option<&AtomicBool>,
) -> Result<identity::ExpectedFileIdentity, AtomicMoveError> {
    let metadata = source.metadata().map_err(AtomicMoveError::Io)?;
    let size = metadata.len();
    let mut source = source.try_clone().map_err(AtomicMoveError::Io)?;
    source
        .seek(SeekFrom::Start(0))
        .map_err(AtomicMoveError::Io)?;
    let mut destination = destination.try_clone().map_err(AtomicMoveError::Io)?;
    destination
        .seek(SeekFrom::Start(0))
        .map_err(AtomicMoveError::Io)?;
    destination.set_len(0).map_err(AtomicMoveError::Io)?;

    let actual = stream_copy_with_hash(&mut source, &mut destination, size, cancel)?;
    copy_metadata_with_native_api(&source, &destination)?;
    Ok(actual)
}

fn stream_copy_with_hash<R: Read, W: Write>(
    source: &mut R,
    destination: &mut W,
    size: u64,
    cancel: Option<&AtomicBool>,
) -> Result<identity::ExpectedFileIdentity, AtomicMoveError> {
    let mut full_hasher = blake3::Hasher::new();
    full_hasher.update(b"file\0");
    full_hasher.update(&size.to_le_bytes());
    let mut first = Vec::with_capacity(SAMPLE_SIZE.min(size as usize));
    let mut small = (size <= (SAMPLE_SIZE * 2) as u64).then(|| Vec::with_capacity(size as usize));
    let tail_capacity = SAMPLE_SIZE.min(size as usize);
    let mut tail_ring = vec![0_u8; tail_capacity.max(1)];
    let mut total_read = 0_usize;
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    loop {
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        let read = source.read(&mut buffer).map_err(AtomicMoveError::Io)?;
        if read == 0 {
            break;
        }
        record_copy_read(read);
        let chunk = &buffer[..read];
        full_hasher.update(chunk);
        destination.write_all(chunk).map_err(AtomicMoveError::Io)?;
        if first.len() < SAMPLE_SIZE {
            let take = (SAMPLE_SIZE - first.len()).min(read);
            first.extend_from_slice(&chunk[..take]);
        }
        if let Some(small) = small.as_mut() {
            small.extend_from_slice(chunk);
        } else if tail_capacity > 0 {
            for byte in chunk {
                tail_ring[total_read % tail_capacity] = *byte;
                total_read = total_read.saturating_add(1);
            }
        }
        if small.is_some() {
            total_read = total_read.saturating_add(read);
        }
    }

    let mut sample_hasher = blake3::Hasher::new();
    sample_hasher.update(b"sample-file\0");
    sample_hasher.update(&size.to_le_bytes());
    if let Some(small) = small {
        sample_hasher.update(&small);
    } else {
        sample_hasher.update(&first);
        let start = total_read % tail_capacity.max(1);
        for index in 0..tail_capacity {
            sample_hasher.update(std::slice::from_ref(
                &tail_ring[(start + index) % tail_capacity],
            ));
        }
    }
    let actual = identity::ExpectedFileIdentity {
        size,
        modified_ns: None,
        platform_volume_id: None,
        platform_file_id: None,
        sample_hash: Some(sample_hasher.finalize().to_hex().to_string()),
        full_hash: Some(full_hasher.finalize().to_hex().to_string()),
    };
    Ok(actual)
}

fn create_staging_file(parent_fd: RawFd, name: &OsStr) -> Result<File, AtomicMoveError> {
    let name = CString::new(name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    let fd = unsafe {
        libc::openat(
            parent_fd,
            name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_RDWR,
            0o600,
        )
    };
    if fd < 0 {
        return Err(map_create_error(io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn copy_file_contents_and_metadata(
    source: &File,
    destination: &File,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    let mut native_source = source.try_clone().map_err(AtomicMoveError::Io)?;
    let native_result = unsafe {
        fcopyfile(
            native_source.as_raw_fd(),
            destination.as_raw_fd(),
            std::ptr::null_mut(),
            COPYFILE_ALL,
        )
    };
    if native_result == 0 {
        copy_fd_metadata(
            &source.metadata().map_err(AtomicMoveError::Io)?,
            destination,
        )?;
        return Ok(());
    }

    // The fallback still consumes this verified source descriptor, never
    // reopens the original pathname. A byte-only fallback is not presented as
    // metadata-preserving success.
    native_source
        .seek(SeekFrom::Start(0))
        .map_err(AtomicMoveError::Io)?;
    let mut destination = destination.try_clone().map_err(AtomicMoveError::Io)?;
    destination
        .seek(SeekFrom::Start(0))
        .map_err(AtomicMoveError::Io)?;
    destination.set_len(0).map_err(AtomicMoveError::Io)?;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        let read = native_source
            .read(&mut buffer)
            .map_err(AtomicMoveError::Io)?;
        if read == 0 {
            break;
        }
        record_copy_read(read);
        destination
            .write_all(&buffer[..read])
            .map_err(AtomicMoveError::Io)?;
    }
    copy_metadata_with_native_api(source, &destination)?;
    Ok(())
}

fn copy_metadata_with_native_api(source: &File, destination: &File) -> Result<(), AtomicMoveError> {
    let native_source = source.try_clone().map_err(AtomicMoveError::Io)?;
    if unsafe {
        fcopyfile(
            native_source.as_raw_fd(),
            destination.as_raw_fd(),
            std::ptr::null_mut(),
            COPYFILE_METADATA,
        )
    } != 0
    {
        return Err(AtomicMoveError::MetadataPreservationUnsupported(
            "native_metadata_copy_failed",
        ));
    }
    let metadata = source.metadata().map_err(AtomicMoveError::Io)?;
    copy_fd_metadata(&metadata, destination)
}

fn create_staging_directory(parent_fd: RawFd, name: &OsStr) -> Result<File, AtomicMoveError> {
    let name = CString::new(name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    if unsafe { libc::mkdirat(parent_fd, name.as_ptr(), 0o700) } != 0 {
        return Err(map_create_error(io::Error::last_os_error()));
    }
    open_directory_at(parent_fd, name.as_c_str())
}

fn open_directory_at(parent_fd: RawFd, name: &std::ffi::CStr) -> Result<File, AtomicMoveError> {
    let fd = unsafe {
        libc::openat(
            parent_fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(AtomicMoveError::Io(io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn open_file_at(parent_fd: RawFd, name: &OsStr) -> Result<File, AtomicMoveError> {
    let name = CString::new(name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    let fd = unsafe {
        libc::openat(
            parent_fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(AtomicMoveError::Io(io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn open_file_at_directory(parent_fd: RawFd, name: &OsStr) -> Result<File, AtomicMoveError> {
    let name = CString::new(name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    open_directory_at(parent_fd, name.as_c_str())
}

struct DirectoryCopyState {
    hardlinks: HashMap<(u64, u64), PathBuf>,
    source_root_device: u64,
    entry_count: usize,
}

impl DirectoryCopyState {
    fn new(source_root_device: u64) -> Self {
        Self {
            hardlinks: HashMap::new(),
            source_root_device,
            entry_count: 0,
        }
    }
}

fn copy_tree_from_fd(
    source_fd: RawFd,
    destination_fd: RawFd,
    destination_root_fd: RawFd,
    relative_prefix: &Path,
    state: &mut DirectoryCopyState,
    depth: usize,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    if depth >= crate::fs_safety::identity::DIRECTORY_IDENTITY_DEPTH_LIMIT {
        return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
    }
    let remaining = crate::fs_safety::identity::DIRECTORY_IDENTITY_ENTRY_LIMIT
        .saturating_sub(state.entry_count);
    for name in directory_entry_names_bounded(source_fd, remaining)? {
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        state.entry_count += 1;
        let source_identity =
            crate::platform::macos::identity::MacPhysicalIdentity::from_at(source_fd, &name)
                .map_err(AtomicMoveError::Io)?;
        if source_identity.dev != state.source_root_device {
            return Err(AtomicMoveError::DirectoryNestedVolumeUnsupported);
        }
        let relative_path = relative_prefix.join(&name);
        match source_identity.file_type {
            value if value == libc::S_IFLNK as u32 => {
                let target = readlink_at(source_fd, &name)?;
                symlink_at(&target, destination_fd, &name)?;
            }
            value if value == libc::S_IFDIR as u32 => {
                let source_directory = open_file_at_directory(source_fd, &name)?;
                let destination_directory = create_staging_directory(destination_fd, &name)?;
                copy_tree_from_fd(
                    source_directory.as_raw_fd(),
                    destination_directory.as_raw_fd(),
                    destination_root_fd,
                    &relative_path,
                    state,
                    depth + 1,
                    cancel,
                )?;
                let metadata = source_directory.metadata().map_err(AtomicMoveError::Io)?;
                copy_fd_metadata(&metadata, &destination_directory)?;
                copy_metadata_with_native_api(&source_directory, &destination_directory)?;
                destination_directory
                    .sync_all()
                    .map_err(AtomicMoveError::Io)?;
            }
            value if value == libc::S_IFREG as u32 => {
                let key = (source_identity.dev, source_identity.ino);
                if source_identity.nlink > 1 {
                    if let Some(first_path) = state.hardlinks.get(&key) {
                        linkat(destination_root_fd, first_path, destination_fd, &name)?;
                    } else {
                        let source_file = open_file_at(source_fd, &name)?;
                        let destination_file = create_staging_file(destination_fd, &name)?;
                        copy_file_contents_and_metadata(&source_file, &destination_file, cancel)?;
                        let metadata = source_file.metadata().map_err(AtomicMoveError::Io)?;
                        copy_fd_metadata(&metadata, &destination_file)?;
                        destination_file.sync_all().map_err(AtomicMoveError::Io)?;
                        state.hardlinks.insert(key, relative_path.clone());
                    }
                } else {
                    let source_file = open_file_at(source_fd, &name)?;
                    let destination_file = create_staging_file(destination_fd, &name)?;
                    copy_file_contents_and_metadata(&source_file, &destination_file, cancel)?;
                    let metadata = source_file.metadata().map_err(AtomicMoveError::Io)?;
                    copy_fd_metadata(&metadata, &destination_file)?;
                    destination_file.sync_all().map_err(AtomicMoveError::Io)?;
                }
            }
            _ => return Err(AtomicMoveError::UnsafePath),
        }

        let current_identity =
            crate::platform::macos::identity::MacPhysicalIdentity::from_at(source_fd, &name)
                .map_err(AtomicMoveError::Io)?;
        if !source_identity.matches(current_identity) {
            return Err(AtomicMoveError::StagingIdentityChanged);
        }
    }
    Ok(())
}

fn linkat(
    source_root_fd: RawFd,
    source_path: &Path,
    destination_fd: RawFd,
    destination_name: &OsStr,
) -> Result<(), AtomicMoveError> {
    let source_path = CString::new(source_path.as_os_str().as_bytes())
        .map_err(|_| AtomicMoveError::UnsafePath)?;
    let destination_name =
        CString::new(destination_name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    if unsafe {
        libc::linkat(
            source_root_fd,
            source_path.as_ptr(),
            destination_fd,
            destination_name.as_ptr(),
            0,
        )
    } == 0
    {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(libc::EXDEV | libc::ENOTSUP | libc::EPERM)
        ) {
            Err(AtomicMoveError::MetadataPreservationUnsupported(
                "hardlink_topology_unavailable",
            ))
        } else {
            Err(AtomicMoveError::Io(error))
        }
    }
}

fn readlink_at(parent_fd: RawFd, name: &OsStr) -> Result<PathBuf, AtomicMoveError> {
    use std::os::unix::ffi::OsStringExt;

    let name = CString::new(name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    let mut buffer = vec![0_u8; 1024];
    loop {
        let length = unsafe {
            libc::readlinkat(
                parent_fd,
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if length < 0 {
            return Err(AtomicMoveError::Io(io::Error::last_os_error()));
        }
        let length = length as usize;
        if length < buffer.len() {
            buffer.truncate(length);
            return Ok(PathBuf::from(std::ffi::OsString::from_vec(buffer)));
        }
        if buffer.len() >= 1024 * 1024 {
            return Err(AtomicMoveError::UnsafePath);
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

fn directory_entry_names(parent_fd: RawFd) -> Result<Vec<OsString>, AtomicMoveError> {
    use std::os::unix::ffi::OsStringExt;

    let scan_fd = unsafe { libc::dup(parent_fd) };
    if scan_fd < 0 {
        return Err(AtomicMoveError::Io(io::Error::last_os_error()));
    }
    let directory = unsafe { libc::fdopendir(scan_fd) };
    if directory.is_null() {
        unsafe { libc::close(scan_fd) };
        return Err(AtomicMoveError::Io(io::Error::last_os_error()));
    }
    let mut names = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(directory) };
        if entry.is_null() {
            break;
        }
        let raw_name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if raw_name == b"." || raw_name == b".." {
            continue;
        }
        names.push(OsString::from_vec(raw_name.to_vec()));
    }
    unsafe { libc::closedir(directory) };
    Ok(names)
}

fn directory_entry_names_bounded(
    parent_fd: RawFd,
    entry_limit: usize,
) -> Result<Vec<OsString>, AtomicMoveError> {
    use std::os::unix::ffi::OsStringExt;

    let scan_fd = unsafe { libc::dup(parent_fd) };
    if scan_fd < 0 {
        return Err(AtomicMoveError::Io(io::Error::last_os_error()));
    }
    let directory = unsafe { libc::fdopendir(scan_fd) };
    if directory.is_null() {
        unsafe { libc::close(scan_fd) };
        return Err(AtomicMoveError::Io(io::Error::last_os_error()));
    }
    let mut names = Vec::new();
    let mut result = Ok(());
    loop {
        let entry = unsafe { libc::readdir(directory) };
        if entry.is_null() {
            break;
        }
        let raw_name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if raw_name == b"." || raw_name == b".." {
            continue;
        }
        if names.len() >= entry_limit {
            result = Err(AtomicMoveError::DirectoryTreeLimitExceeded);
            break;
        }
        names.push(OsString::from_vec(raw_name.to_vec()));
    }
    unsafe { libc::closedir(directory) };
    result.map(|()| names)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{copy_verification_policy, namespace_identity_matches, CopyVerificationPolicy};
    use crate::fs_safety::{identity::ExpectedFileIdentity, ClaimedEntryKind};
    use std::io::{self, Read};

    #[cfg(target_os = "macos")]
    #[test]
    fn staging_cleanup_refuses_a_rebound_name() {
        use std::{fs, os::fd::AsRawFd, os::unix::ffi::OsStrExt, path::PathBuf};

        let root = std::env::temp_dir().join(format!(
            "zen-canvas-staging-rebound-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create staging rebound fixture");
        let staging_name = PathBuf::from(".zen-canvas-stage-rebound");
        let staging_path = root.join(&staging_name);
        fs::write(&staging_path, b"original staging payload").expect("write staging fixture");
        let parent = fs::File::open(&root).expect("open staging parent");
        let expected = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
            parent.as_raw_fd(),
            staging_name.as_os_str(),
        )
        .expect("capture staging identity");

        fs::remove_file(&staging_path).expect("remove original staging entry");
        fs::write(&staging_path, b"attacker staging payload").expect("rebind staging entry");
        let result = super::remove_staging_file(
            parent.as_raw_fd(),
            std::ffi::OsStr::from_bytes(staging_name.as_os_str().as_bytes()),
            expected,
        );

        assert!(matches!(
            result,
            Err(crate::fs_safety::AtomicMoveError::StagingIdentityChanged)
        ));
        assert_eq!(
            fs::read(&staging_path).expect("attacker staging remains"),
            b"attacker staging payload"
        );
        fs::remove_dir_all(&root).expect("remove staging rebound fixture");
    }

    #[test]
    fn copy_policy_streams_hashed_files_and_keeps_clone_path_bounded() {
        let expected = ExpectedFileIdentity {
            size: 10,
            modified_ns: None,
            platform_volume_id: None,
            platform_file_id: None,
            sample_hash: None,
            full_hash: Some("hash".to_string()),
        };
        assert_eq!(
            copy_verification_policy(ClaimedEntryKind::File, Some(&expected)),
            CopyVerificationPolicy::StreamingHash
        );
        assert_eq!(
            copy_verification_policy(ClaimedEntryKind::File, None),
            CopyVerificationPolicy::PhysicalClone
        );
        assert_eq!(
            copy_verification_policy(ClaimedEntryKind::Directory, Some(&expected)),
            CopyVerificationPolicy::FullPostVerify
        );
    }

    #[test]
    fn full_native_profile_streams_a_logical_ten_gib_source_once() {
        if std::env::var("ZC_MACOS_NATIVE_FULL_PROFILE").as_deref() != Ok("1") {
            println!("macOS native 10GiB stream profile: SKIPPED — FULL PROFILE NOT REQUESTED");
            return;
        }

        struct SyntheticReader {
            remaining: u64,
            reads: u64,
        }

        impl Read for SyntheticReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.remaining == 0 {
                    return Ok(0);
                }
                let count = self.remaining.min(buffer.len() as u64) as usize;
                buffer[..count].fill(0);
                self.remaining -= count as u64;
                self.reads += 1;
                Ok(count)
            }
        }

        let logical_size = 10_u64 * 1024 * 1024 * 1024;
        let mut source = SyntheticReader {
            remaining: logical_size,
            reads: 0,
        };
        let mut destination = io::sink();
        super::reset_copy_io_metrics();
        let actual =
            super::stream_copy_with_hash(&mut source, &mut destination, logical_size, None)
                .expect("synthetic stream copy");
        let (read_calls, read_bytes) = super::copy_io_metrics();
        assert_eq!(actual.size, logical_size);
        assert_eq!(source.remaining, 0);
        assert_eq!(read_calls, source.reads);
        assert_eq!(read_bytes, logical_size);
        assert!(read_calls <= logical_size / (1024 * 1024) + 1);
        eprintln!(
            "macOS native performance stream10GiB logicalBytes={logical_size} readCalls={read_calls} readBytes={read_bytes} sourcePasses=1"
        );
    }

    #[test]
    fn directory_namespace_check_distinguishes_content_size_from_root_size() {
        let expected_content = ExpectedFileIdentity {
            size: 42,
            modified_ns: Some(7),
            platform_volume_id: Some("volume".into()),
            platform_file_id: Some("directory".into()),
            sample_hash: Some("sample".into()),
            full_hash: Some("tree".into()),
        };
        let actual_namespace = ExpectedFileIdentity {
            size: 128,
            modified_ns: Some(7),
            platform_volume_id: Some("volume".into()),
            platform_file_id: Some("directory".into()),
            sample_hash: None,
            full_hash: None,
        };

        assert!(namespace_identity_matches(
            &expected_content,
            &actual_namespace,
            ClaimedEntryKind::Directory,
        ));
        assert!(!namespace_identity_matches(
            &expected_content,
            &actual_namespace,
            ClaimedEntryKind::File,
        ));

        let mut changed_metadata_only = actual_namespace.clone();
        changed_metadata_only.size = 64;
        assert!(!namespace_identity_matches(
            &actual_namespace,
            &changed_metadata_only,
            ClaimedEntryKind::Directory,
        ));

        let mut rebound_directory = actual_namespace.clone();
        rebound_directory.platform_file_id = Some("replacement".into());
        assert!(!namespace_identity_matches(
            &expected_content,
            &rebound_directory,
            ClaimedEntryKind::Directory,
        ));
    }
}

fn copy_fd_metadata(metadata: &fs::Metadata, destination: &File) -> Result<(), AtomicMoveError> {
    use std::os::unix::fs::MetadataExt;

    unsafe {
        let current = destination.metadata().map_err(AtomicMoveError::Io)?;
        if (current.uid() != metadata.uid() || current.gid() != metadata.gid())
            && libc::fchown(
                destination.as_raw_fd(),
                metadata.uid() as libc::uid_t,
                metadata.gid() as libc::gid_t,
            ) != 0
        {
            return Err(AtomicMoveError::MetadataPreservationUnsupported(
                "uid_gid_unavailable",
            ));
        }
        if libc::fchmod(
            destination.as_raw_fd(),
            (metadata.mode() & 0o7777) as libc::mode_t,
        ) != 0
        {
            return Err(AtomicMoveError::Io(io::Error::last_os_error()));
        }
        let times = [
            libc::timespec {
                tv_sec: metadata.atime(),
                tv_nsec: metadata.atime_nsec(),
            },
            libc::timespec {
                tv_sec: metadata.mtime(),
                tv_nsec: metadata.mtime_nsec(),
            },
        ];
        if libc::futimens(destination.as_raw_fd(), times.as_ptr()) != 0 {
            return Err(AtomicMoveError::Io(io::Error::last_os_error()));
        }
    }
    Ok(())
}

fn verify_basic_metadata(source: &File, destination: &File) -> Result<(), AtomicMoveError> {
    use std::os::unix::fs::MetadataExt;

    let source = source.metadata().map_err(AtomicMoveError::Io)?;
    let destination = destination.metadata().map_err(AtomicMoveError::Io)?;
    if source.mode() & 0o7777 != destination.mode() & 0o7777
        || source.uid() != destination.uid()
        || source.gid() != destination.gid()
        || source.mtime() != destination.mtime()
        || source.mtime_nsec() != destination.mtime_nsec()
    {
        return Err(AtomicMoveError::MetadataPreservationUnsupported(
            "native_clone_metadata_mismatch",
        ));
    }
    Ok(())
}

/// A clone syscall does not read the file body, so verify the bounded
/// metadata contract explicitly.  `fcopyfile(COPYFILE_METADATA)` remains the
/// authoritative preservation mechanism; this check proves that mode/owner/
/// timestamps and extended-attribute names/sizes survived publication.
fn verify_clone_metadata(source: &File, destination: &File) -> Result<(), AtomicMoveError> {
    verify_basic_metadata(source, destination)?;
    if list_xattr_sizes(source)? != list_xattr_sizes(destination)? {
        return Err(AtomicMoveError::MetadataPreservationUnsupported(
            "native_clone_xattr_mismatch",
        ));
    }
    Ok(())
}

fn list_xattr_sizes(file: &File) -> Result<Vec<(Vec<u8>, isize)>, AtomicMoveError> {
    const MAX_XATTR_NAME_BYTES: usize = 1024 * 1024;
    let size = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0, 0) };
    if size < 0 {
        return Err(AtomicMoveError::MetadataPreservationUnsupported(
            "native_xattr_list_unavailable",
        ));
    }
    let size = usize::try_from(size).map_err(|_| {
        AtomicMoveError::MetadataPreservationUnsupported("native_xattr_list_unavailable")
    })?;
    if size > MAX_XATTR_NAME_BYTES {
        return Err(AtomicMoveError::MetadataPreservationUnsupported(
            "native_xattr_list_too_large_to_verify",
        ));
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    let mut buffer = vec![0_u8; size];
    let written = unsafe {
        libc::flistxattr(
            file.as_raw_fd(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            0,
        )
    };
    if written < 0 {
        return Err(AtomicMoveError::MetadataPreservationUnsupported(
            "native_xattr_list_unavailable",
        ));
    }
    let written = usize::try_from(written).map_err(|_| {
        AtomicMoveError::MetadataPreservationUnsupported("native_xattr_list_unavailable")
    })?;
    if written > buffer.len() {
        return Err(AtomicMoveError::MetadataPreservationUnsupported(
            "native_xattr_list_changed_during_verify",
        ));
    }
    let mut result = Vec::new();
    for name in buffer[..written].split(|byte| *byte == 0) {
        if name.is_empty() {
            continue;
        }
        let name = CString::new(name).map_err(|_| {
            AtomicMoveError::MetadataPreservationUnsupported("native_xattr_name_invalid")
        })?;
        let value_size = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                0,
            )
        };
        if value_size < 0 {
            return Err(AtomicMoveError::MetadataPreservationUnsupported(
                "native_xattr_value_unavailable",
            ));
        }
        result.push((name.into_bytes(), value_size));
    }
    result.sort_unstable();
    Ok(result)
}

fn symlink_at(target: &Path, parent_fd: RawFd, name: &OsStr) -> Result<(), AtomicMoveError> {
    let target =
        CString::new(target.as_os_str().as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    let name = CString::new(name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    if unsafe { libc::symlinkat(target.as_ptr(), parent_fd, name.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(AtomicMoveError::Io(io::Error::last_os_error()))
    }
}

fn rename_noreplace(
    source_parent_fd: RawFd,
    source_name: &OsStr,
    target_parent_fd: RawFd,
    target_name: &OsStr,
) -> Result<(), AtomicMoveError> {
    let source_name =
        CString::new(source_name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    let target_name =
        CString::new(target_name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    const RENAME_EXCL: u32 = 0x0000_0004;
    if unsafe {
        renameatx_np(
            source_parent_fd,
            source_name.as_ptr(),
            target_parent_fd,
            target_name.as_ptr(),
            RENAME_EXCL,
        )
    } == 0
    {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EEXIST) => Err(AtomicMoveError::TargetExists),
            Some(libc::EINVAL | libc::ENOTSUP | libc::ENOSYS) => {
                Err(AtomicMoveError::AtomicSourceBindingUnsupported)
            }
            _ => Err(AtomicMoveError::Io(error)),
        }
    }
}

/// Publishes a completed staging object without replacing an unexpected
/// destination. APFS uses `renameatx_np(RENAME_EXCL)`. Filesystems that do
/// not implement that flag use an exclusive-create fallback: regular files
/// prefer a hard-link publication, while directories are created exclusively
/// and populated from the verified staging descriptor.
fn publish_staging_exclusive(
    parent_fd: RawFd,
    staging_name: &OsStr,
    target_name: &OsStr,
    kind: ClaimedEntryKind,
    staging_physical: crate::platform::macos::identity::MacPhysicalIdentity,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    ensure_staging_identity(parent_fd, staging_name, staging_physical)?;
    match rename_noreplace(parent_fd, staging_name, parent_fd, target_name) {
        Ok(()) => {
            let published = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
                parent_fd,
                target_name,
            )
            .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
            if !staging_physical.matches(published) {
                return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
            }
            Ok(())
        }
        Err(AtomicMoveError::AtomicSourceBindingUnsupported) => match kind {
            ClaimedEntryKind::File => publish_file_exclusive(
                parent_fd,
                staging_name,
                target_name,
                staging_physical,
                cancel,
            ),
            ClaimedEntryKind::Directory => publish_directory_exclusive(
                parent_fd,
                staging_name,
                target_name,
                staging_physical,
                cancel,
            ),
            ClaimedEntryKind::Symlink => {
                publish_symlink_exclusive(parent_fd, staging_name, target_name, staging_physical)
            }
        },
        Err(error) => Err(error),
    }
}

fn publish_file_exclusive(
    parent_fd: RawFd,
    staging_name: &OsStr,
    target_name: &OsStr,
    staging_physical: crate::platform::macos::identity::MacPhysicalIdentity,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    match link_staging_exclusive(parent_fd, staging_name, target_name, staging_physical)? {
        true => remove_staging_file(parent_fd, staging_name, staging_physical),
        false => {
            ensure_staging_identity(parent_fd, staging_name, staging_physical)?;
            let source = open_file_at(parent_fd, staging_name)?;
            let source_physical =
                crate::platform::macos::identity::MacPhysicalIdentity::from_fd(&source)
                    .map_err(|_| AtomicMoveError::StagingIdentityChanged)?;
            if !staging_physical.matches(source_physical) {
                return Err(AtomicMoveError::StagingIdentityChanged);
            }
            let destination = create_staging_file(parent_fd, target_name)?;
            copy_file_contents_and_metadata(&source, &destination, cancel)
                .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
            destination
                .sync_all()
                .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
            verify_clone_metadata(&source, &destination)
                .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
            remove_staging_file(parent_fd, staging_name, staging_physical)
        }
    }
}

fn publish_directory_exclusive(
    parent_fd: RawFd,
    staging_name: &OsStr,
    target_name: &OsStr,
    staging_physical: crate::platform::macos::identity::MacPhysicalIdentity,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    ensure_staging_identity(parent_fd, staging_name, staging_physical)?;
    let source = open_file_at_directory(parent_fd, staging_name)?;
    let source_physical = crate::platform::macos::identity::MacPhysicalIdentity::from_fd(&source)
        .map_err(|_| AtomicMoveError::StagingIdentityChanged)?;
    if !staging_physical.matches(source_physical) {
        return Err(AtomicMoveError::StagingIdentityChanged);
    }
    let destination = create_staging_directory(parent_fd, target_name)?;
    let source_root_device = {
        use std::os::unix::fs::MetadataExt;
        source.metadata().map_err(AtomicMoveError::Io)?.dev()
    };
    let mut state = DirectoryCopyState::new(source_root_device);
    if copy_tree_from_fd(
        source.as_raw_fd(),
        destination.as_raw_fd(),
        destination.as_raw_fd(),
        Path::new(""),
        &mut state,
        0,
        cancel,
    )
    .is_err()
    {
        return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
    }
    let metadata = source
        .metadata()
        .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
    copy_fd_metadata(&metadata, &destination)
        .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
    destination
        .sync_all()
        .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
    remove_staging_tree(parent_fd, staging_name, staging_physical)
}

fn publish_symlink_exclusive(
    parent_fd: RawFd,
    staging_name: &OsStr,
    target_name: &OsStr,
    staging_physical: crate::platform::macos::identity::MacPhysicalIdentity,
) -> Result<(), AtomicMoveError> {
    match link_staging_exclusive(parent_fd, staging_name, target_name, staging_physical)? {
        true => remove_staging_file(parent_fd, staging_name, staging_physical),
        false => Err(AtomicMoveError::AtomicSourceBindingUnsupported),
    }
}

fn link_staging_exclusive(
    parent_fd: RawFd,
    staging_name: &OsStr,
    target_name: &OsStr,
    expected: crate::platform::macos::identity::MacPhysicalIdentity,
) -> Result<bool, AtomicMoveError> {
    ensure_staging_identity(parent_fd, staging_name, expected)?;
    let staging_name =
        CString::new(staging_name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    let target_name_c =
        CString::new(target_name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    if unsafe {
        libc::linkat(
            parent_fd,
            staging_name.as_ptr(),
            parent_fd,
            target_name_c.as_ptr(),
            0,
        )
    } == 0
    {
        let published =
            crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent_fd, target_name)
                .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
        if !expected.matches(published) {
            return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
        }
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EEXIST) => Err(AtomicMoveError::TargetExists),
        Some(libc::EXDEV | libc::ENOTSUP | libc::EPERM) => Ok(false),
        _ => Err(AtomicMoveError::Io(error)),
    }
}

fn ensure_staging_identity(
    parent_fd: RawFd,
    name: &OsStr,
    expected: crate::platform::macos::identity::MacPhysicalIdentity,
) -> Result<(), AtomicMoveError> {
    let actual = crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent_fd, name)
        .map_err(|_| AtomicMoveError::StagingIdentityChanged)?;
    let matches = if expected.file_type == libc::S_IFDIR as u32 {
        expected.matches(actual)
    } else {
        expected.matches_strict_ignoring_link_count(actual)
    };
    if !matches {
        return Err(AtomicMoveError::StagingIdentityChanged);
    }
    Ok(())
}

fn remove_staging_file(
    parent_fd: RawFd,
    name: &OsStr,
    expected: crate::platform::macos::identity::MacPhysicalIdentity,
) -> Result<(), AtomicMoveError> {
    ensure_staging_identity(parent_fd, name, expected)?;
    let name = CString::new(name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    if unsafe { libc::unlinkat(parent_fd, name.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(AtomicMoveError::TargetCommittedSourceCleanupPending)
    }
}

fn remove_staging_tree(
    parent_fd: RawFd,
    name: &OsStr,
    expected: crate::platform::macos::identity::MacPhysicalIdentity,
) -> Result<(), AtomicMoveError> {
    ensure_staging_identity(parent_fd, name, expected)?;
    let name_c = CString::new(name.as_bytes()).map_err(|_| AtomicMoveError::UnsafePath)?;
    if expected.file_type == libc::S_IFDIR as u32 {
        let directory = open_directory_at(parent_fd, name_c.as_c_str())?;
        for child in directory_entry_names(directory.as_raw_fd())? {
            let child_expected = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
                directory.as_raw_fd(),
                &child,
            )
            .map_err(AtomicMoveError::Io)?;
            remove_staging_tree(directory.as_raw_fd(), &child, child_expected)?;
        }
        ensure_staging_identity(parent_fd, name, expected)?;
        if unsafe { libc::unlinkat(parent_fd, name_c.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
        }
        Ok(())
    } else {
        remove_staging_file(parent_fd, name, expected)
    }
}

fn cleanup_staging_at(
    parent_fd: RawFd,
    name: &OsStr,
    expected: Option<crate::platform::macos::identity::MacPhysicalIdentity>,
) {
    // Never derive the destructive cleanup identity from the pathname after
    // an operation has lost its proof. A failed clone or a pre-proof error can
    // leave a staging name that is absent, partially created, or rebound by a
    // concurrent actor; deleting whatever occupies that name would be a
    // wrong-delete. Known proofs are checked again by remove_staging_tree,
    // while an unknown object is deliberately retained for safe recovery.
    let Some(expected) = expected else {
        return;
    };
    let _ = remove_staging_tree(parent_fd, name, expected);
}

fn rollback_before_publish(
    claim: &mut SourceClaim,
    error: AtomicMoveError,
) -> Result<(), AtomicMoveError> {
    match claim.rollback_to_original() {
        Ok(()) => Err(error),
        Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
            rollback.to_string(),
        )),
    }
}

fn rollback_claim_error(claim: &mut SourceClaim, error: AtomicMoveError) -> AtomicMoveError {
    match claim.rollback_to_original() {
        Ok(()) => error,
        Err(rollback) => AtomicMoveError::SourceClaimRollbackFailed(rollback.to_string()),
    }
}

fn map_claim_error(error: crate::fs_safety::SourceClaimError) -> AtomicMoveError {
    crate::fs_safety::atomic_move::map_claim_error(error)
}

fn map_create_error(error: io::Error) -> AtomicMoveError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        AtomicMoveError::TargetExists
    } else {
        AtomicMoveError::Io(error)
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

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Acquire))
}

extern "C" {
    fn fclonefileat(
        srcfd: libc::c_int,
        dst_dir: libc::c_int,
        dst: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
    fn fcopyfile(
        from_fd: libc::c_int,
        to_fd: libc::c_int,
        state: *mut libc::c_void,
        flags: libc::c_uint,
    ) -> libc::c_int;
    fn renameatx_np(
        fromfd: libc::c_int,
        from: *const libc::c_char,
        tofd: libc::c_int,
        to: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
}
