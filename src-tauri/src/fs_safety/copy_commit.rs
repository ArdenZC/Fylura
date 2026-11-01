#[cfg(test)]
use super::platform_support;
use super::{
    atomic_move::{map_claim_error, map_directory_error, AtomicMoveError},
    identity,
    source_claim::{self, ClaimedEntryKind, SourceClaim},
    verified_directory::VerifiedDirectory,
};
#[cfg(test)]
use std::path::Path;
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

const COPY_BUFFER_SIZE: usize = 1024 * 1024;

struct StagingFile<'a> {
    parent: &'a VerifiedDirectory,
    name: OsString,
    path: PathBuf,
    handle: File,
    verified_identity: Option<identity::ExpectedFileIdentity>,
    committed: bool,
}

impl<'a> StagingFile<'a> {
    fn create(
        parent: &'a VerifiedDirectory,
        target_name: &std::ffi::OsStr,
    ) -> Result<Self, AtomicMoveError> {
        let name = unique_staging_name(target_name);
        let path = parent.path().join(&name);
        let handle = create_staging_handle(parent, &name).map_err(map_create_error)?;
        Ok(Self {
            parent,
            name,
            path,
            handle,
            verified_identity: None,
            committed: false,
        })
    }

    fn sync_and_verify(
        &mut self,
        expected: &identity::ExpectedFileIdentity,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), AtomicMoveError> {
        self.handle.sync_all().map_err(AtomicMoveError::Io)?;
        let actual = identity::capture_identity_from_handle(&self.handle, &self.path, cancel)
            .map_err(map_identity_error)?;
        if !identity::content_identity_matches(expected, &actual) {
            return Err(AtomicMoveError::CopyVerificationFailed);
        }
        self.verified_identity = Some(actual);
        Ok(())
    }

    fn commit_noreplace(&mut self, target_name: &std::ffi::OsStr) -> Result<(), AtomicMoveError> {
        source_claim::commit_open_handle_noreplace(
            &self.handle,
            self.parent,
            &self.name,
            self.parent,
            target_name,
        )
        .map_err(|error| match error {
            source_claim::SourceClaimError::AtomicSourceBindingUnsupported => {
                AtomicMoveError::StagingHandleCommitUnsupported
            }
            other => map_claim_error(other),
        })?;
        self.committed = true;
        self.path = self.parent.path().join(target_name);
        Ok(())
    }

    fn verify_path_binding(&self, cancel: Option<&AtomicBool>) -> Result<(), AtomicMoveError> {
        let expected = self
            .verified_identity
            .as_ref()
            .ok_or(AtomicMoveError::CopyVerificationFailed)?;
        let handle_identity =
            identity::capture_identity_from_handle(&self.handle, &self.path, cancel)
                .map_err(map_identity_error)?;
        let path_identity =
            identity::capture_identity(&self.path, cancel).map_err(map_identity_error)?;
        if !identity::identity_matches(expected, &handle_identity)
            || !identity::identity_matches(&handle_identity, &path_identity)
        {
            return Err(AtomicMoveError::StagingIdentityChanged);
        }
        Ok(())
    }

    fn verify_committed(&self, cancel: Option<&AtomicBool>) -> Result<(), AtomicMoveError> {
        let expected = self
            .verified_identity
            .as_ref()
            .ok_or(AtomicMoveError::CopyVerificationFailed)?;
        let handle_identity =
            identity::capture_identity_from_handle(&self.handle, &self.path, cancel)
                .map_err(map_identity_error)?;
        let path_identity =
            identity::capture_identity(&self.path, cancel).map_err(map_identity_error)?;
        if !identity::identity_matches(expected, &handle_identity)
            || !identity::identity_matches(&handle_identity, &path_identity)
        {
            return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
        }
        Ok(())
    }
}

impl Drop for StagingFile<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = source_claim::delete_open_handle(
                &self.handle,
                self.parent,
                &self.name,
                ClaimedEntryKind::File,
            );
        }
    }
}

#[cfg(test)]
pub(crate) fn copy_commit_move(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    copy_commit_move_with_claim_path(source, target, expected_identity, None, cancel)
}

#[cfg(test)]
pub(crate) fn copy_commit_move_with_claim_path(
    source: &Path,
    target: &Path,
    expected_identity: Option<&identity::ExpectedFileIdentity>,
    planned_claim_path: Option<&Path>,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    platform_support::ensure_supported_file_mutation().map_err(|error| match error {
        platform_support::PlatformSupportError::LinuxUnsupported => {
            AtomicMoveError::UnsupportedPlatformLinux
        }
        platform_support::PlatformSupportError::MacosFileMutationSourceBindingUnsupported => {
            AtomicMoveError::MacosFileMutationSourceBindingUnsupported
        }
    })?;
    let expected = match expected_identity {
        Some(expected) if expected.full_hash.is_some() => expected.clone(),
        Some(_) => {
            return Err(AtomicMoveError::SourceClaimFailed(
                "source identity is incomplete".to_string(),
            ));
        }
        None => identity::capture_identity(source, cancel).map_err(|error| match error {
            identity::IdentityError::SourceMissing => AtomicMoveError::SourceMissing,
            identity::IdentityError::Symlink => AtomicMoveError::ReparsePoint,
            identity::IdentityError::UnsupportedFileType => AtomicMoveError::UnsafePath,
            identity::IdentityError::DirectoryManifestNameEncodingFailed => {
                AtomicMoveError::DirectoryManifestNameEncodingFailed
            }
            identity::IdentityError::Cancelled => AtomicMoveError::Cancelled,
            identity::IdentityError::ContentReadRejected(reason) => {
                map_content_read_rejected(reason)
            }
            identity::IdentityError::Io(error) => AtomicMoveError::Io(error),
        })?,
    };
    let target_parent_path = target.parent().ok_or(AtomicMoveError::UnsafePath)?;
    let target_name = target.file_name().ok_or(AtomicMoveError::UnsafePath)?;
    let target_parent =
        VerifiedDirectory::open_existing(target_parent_path).map_err(map_directory_error)?;
    let claim_path = match planned_claim_path {
        Some(path) => path.to_path_buf(),
        None => source_claim::planned_claim_path(source, "copy-commit").map_err(map_claim_error)?,
    };
    let mut claim =
        source_claim::claim_source_at(source, &expected, &claim_path, "copy-commit", cancel)
            .map_err(map_claim_error)?;
    copy_commit_claim(&mut claim, target_parent, target_name, cancel, None).map(|_| ())
}

pub(crate) fn copy_commit_claim(
    claim: &mut SourceClaim,
    target_parent: VerifiedDirectory,
    target_name: &std::ffi::OsStr,
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

pub(crate) fn copy_commit_claim_preserving_source(
    claim: &mut SourceClaim,
    target_parent: VerifiedDirectory,
    target_name: &std::ffi::OsStr,
    cancel: Option<&AtomicBool>,
    observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), AtomicMoveError> {
    copy_commit_claim_with_source_retirement(
        claim,
        target_parent,
        target_name,
        cancel,
        observer,
        false,
    )
}

fn copy_commit_claim_with_source_retirement(
    claim: &mut SourceClaim,
    target_parent: VerifiedDirectory,
    target_name: &std::ffi::OsStr,
    cancel: Option<&AtomicBool>,
    mut observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    retire_source: bool,
) -> Result<(), AtomicMoveError> {
    if let Err(error) = ensure_windows_sacl_absent_at_path(claim.current_path()) {
        return rollback_before_publish(claim, error);
    }
    if claim.kind() != ClaimedEntryKind::File {
        #[cfg(windows)]
        if claim.kind() == ClaimedEntryKind::Directory {
            return copy_directory_claim(
                claim,
                target_parent,
                target_name,
                cancel,
                observer,
                retire_source,
            );
        }
        let _ = claim.rollback_to_original();
        return Err(AtomicMoveError::CrossVolumeDirectoryMoveUnsupported);
    }
    if is_cancelled(cancel) {
        let _ = claim.rollback_to_original();
        return Err(AtomicMoveError::Cancelled);
    }
    claim
        .verify_current_identity(cancel)
        .map_err(map_claim_error)?;
    target_parent
        .ensure_unchanged()
        .map_err(map_directory_error)?;
    let mut staging = StagingFile::create(&target_parent, target_name)?;
    let mut target_committed = false;
    let result = (|| {
        if let Some(observer) = observer.as_deref_mut() {
            observer(crate::fs_safety::FileOperationEvent::Phase(
                "copying".to_string(),
            ))?;
        }
        let mut progress = crate::fs_safety::DirectoryTransferProgress::new("copying");
        progress.reset("copying", None, None, &mut observer)?;
        copy_file_from_claim(
            claim,
            &mut staging.handle,
            cancel,
            &mut progress,
            &mut observer,
        )?;
        progress.record_item(claim.current_path(), &mut observer)?;
        progress.finish(&mut observer)?;
        staging.sync_and_verify(claim.expected_identity(), cancel)?;
        target_parent
            .ensure_unchanged()
            .map_err(map_directory_error)?;
        #[cfg(any(test, feature = "native-qa"))]
        source_claim::run_claim_test_hook(
            source_claim::ClaimTestPoint::AfterStagingVerifiedBeforeCommit,
            staging.path.as_path(),
            &target_parent.path().join(target_name),
        );
        staging.verify_path_binding(cancel)?;
        ensure_windows_sacl_absent_at_path(claim.current_path())?;
        ensure_windows_sacl_absent_at_path(&staging.path)?;
        staging.commit_noreplace(target_name)?;
        target_committed = true;
        if let Some(observer) = observer.as_deref_mut() {
            observer(crate::fs_safety::FileOperationEvent::Phase(
                "target_committed".to_string(),
            ))?;
        }
        #[cfg(any(test, feature = "native-qa"))]
        if super::atomic_move::test_faults::take_fault(
            super::atomic_move::test_faults::AtomicFaultPoint::TargetDurability,
        ) {
            return Err(AtomicMoveError::TargetCommittedDurabilityUnknown);
        }
        target_parent
            .sync()
            .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
        #[cfg(any(test, feature = "native-qa"))]
        if super::atomic_move::test_faults::take_fault(
            super::atomic_move::test_faults::AtomicFaultPoint::TargetIdentity,
        ) {
            return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
        }
        staging.verify_committed(cancel)?;
        claim
            .sync()
            .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
        if retire_source {
            progress.reset(
                "source_retirement",
                Some(1),
                Some(claim.expected_identity().size),
                &mut observer,
            )?;
        }
        if let Some(observer) = observer.as_deref_mut() {
            observer(crate::fs_safety::FileOperationEvent::Phase(
                "source_cleanup_pending".to_string(),
            ))?;
        }
        #[cfg(any(test, feature = "native-qa"))]
        source_claim::run_claim_test_hook(
            source_claim::ClaimTestPoint::AfterTargetCommitBeforeSourceCleanup,
            claim.original_path(),
            claim.claim_path(),
        );
        #[cfg(any(test, feature = "native-qa"))]
        if super::atomic_move::test_faults::take_fault(
            super::atomic_move::test_faults::AtomicFaultPoint::SourceCleanup,
        ) {
            return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
        }
        if retire_source {
            if is_cancelled(cancel) {
                claim
                    .rollback_to_original()
                    .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
                claim
                    .sync_original_parent()
                    .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
                return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
            }
            if ensure_windows_sacl_absent_at_path(claim.current_path()).is_err() {
                return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
            }
            claim.delete_claim().map_err(|error| {
                AtomicMoveError::TargetCommittedSourceDeleteFailed(error.to_string())
            })?;
            claim.sync_current_parent().map_err(|error| {
                let _ = error;
                AtomicMoveError::TargetCommittedSourceCleanupPending
            })?;
            progress.record_item(claim.original_path(), &mut observer)?;
            progress.record_bytes(
                claim.original_path(),
                claim.expected_identity().size,
                &mut observer,
            )?;
            progress.finish(&mut observer)?;
        } else {
            claim
                .rollback_to_original()
                .map_err(|error| AtomicMoveError::SourceClaimRollbackFailed(error.to_string()))?;
            claim
                .sync_original_parent()
                .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
        }
        #[cfg(any(test, feature = "native-qa"))]
        source_claim::run_claim_test_hook(
            source_claim::ClaimTestPoint::AfterSourceCleanupBeforeJournalComplete,
            claim.original_path(),
            claim.claim_path(),
        );
        if let Some(observer) = observer.as_deref_mut() {
            observer(crate::fs_safety::FileOperationEvent::Phase(
                "completed".to_string(),
            ))?;
        }
        Ok(())
    })();

    if result.is_err() && !target_committed {
        if let Err(error) = claim.rollback_to_original() {
            return Err(AtomicMoveError::SourceClaimRollbackFailed(
                error.to_string(),
            ));
        }
    }
    result
}

fn rollback_before_publish(
    claim: &mut SourceClaim,
    error: AtomicMoveError,
) -> Result<(), AtomicMoveError> {
    // A failed macOS claim identity check means the private claim pathname may
    // have been rebound. Leave it intact for recovery instead of attempting a
    // second name-based mutation while handling the original error.
    if error.is_mac_claim_safety_error() {
        return Err(error);
    }
    match claim.rollback_to_original() {
        Ok(()) => Err(error),
        Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
            rollback.to_string(),
        )),
    }
}

#[cfg(windows)]
fn copy_directory_claim(
    claim: &mut SourceClaim,
    target_parent: VerifiedDirectory,
    target_name: &std::ffi::OsStr,
    cancel: Option<&AtomicBool>,
    mut observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    retire_source: bool,
) -> Result<(), AtomicMoveError> {
    if is_cancelled(cancel) {
        claim
            .rollback_to_original()
            .map_err(|error| AtomicMoveError::SourceClaimRollbackFailed(error.to_string()))?;
        return Err(AtomicMoveError::Cancelled);
    }
    if let Err(error) = claim.verify_current_identity(cancel) {
        return match claim.rollback_to_original() {
            Ok(()) => Err(map_claim_error(error)),
            Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
                rollback.to_string(),
            )),
        };
    }
    if let Err(error) = target_parent.ensure_unchanged() {
        return match claim.rollback_to_original() {
            Ok(()) => Err(map_directory_error(error)),
            Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
                rollback.to_string(),
            )),
        };
    }
    let mut progress = crate::fs_safety::DirectoryTransferProgress::new("preparing");
    let preflight = (|| {
        progress.reset("preparing", None, None, &mut observer)?;
        let source_root_metadata =
            fs::symlink_metadata(claim.current_path()).map_err(AtomicMoveError::Io)?;
        if source_root_metadata.file_type().is_symlink()
            || metadata_is_reparse_point(&source_root_metadata)
            || !source_root_metadata.is_dir()
        {
            return Err(AtomicMoveError::ReparsePoint);
        }
        ensure_supported_windows_file_attributes(&source_root_metadata)?;
        ensure_windows_no_named_streams(claim.current_path())?;
        let source_before =
            directory_content_digest(claim.current_path(), cancel, &mut progress, &mut observer)?;
        progress.finish(&mut observer)?;
        Ok(source_before)
    })();
    let source_before = match preflight {
        Ok(source_before) => source_before,
        Err(error) => return rollback_before_publish(claim, error),
    };
    if let Err(error) = progress.reset("copying", None, None, &mut observer) {
        return rollback_before_publish(claim, error);
    }
    if let Some(observer) = observer.as_deref_mut() {
        if let Err(error) = observer(crate::fs_safety::FileOperationEvent::Phase(
            "copying".to_string(),
        )) {
            return rollback_before_publish(claim, error);
        }
    }
    let mut staging = match StagingDirectory::create(&target_parent, target_name) {
        Ok(staging) => staging,
        Err(error) => {
            return match claim.rollback_to_original() {
                Ok(()) => Err(error),
                Err(rollback) => Err(AtomicMoveError::SourceClaimRollbackFailed(
                    rollback.to_string(),
                )),
            };
        }
    };
    let mut target_committed = false;
    let result = (|| {
        let mut entry_count = 0usize;
        copy_directory_contents(
            claim.current_path(),
            &staging.path,
            0,
            &mut entry_count,
            cancel,
            &mut progress,
            &mut observer,
        )?;
        progress.finish(&mut observer)?;
        copy_windows_security_descriptor(claim.current_path(), &staging.path)?;
        progress.reset("verify_source", None, None, &mut observer)?;
        let source_after =
            directory_content_digest(claim.current_path(), cancel, &mut progress, &mut observer)?;
        progress.finish(&mut observer)?;
        progress.reset("verify_staging", None, None, &mut observer)?;
        let staged = directory_content_digest(&staging.path, cancel, &mut progress, &mut observer)?;
        progress.finish(&mut observer)?;
        if source_before.hash != source_after.hash || source_before.hash != staged.hash {
            return Err(AtomicMoveError::CopyVerificationFailed);
        }
        claim
            .verify_current_identity(cancel)
            .map_err(map_claim_error)?;
        target_parent
            .ensure_unchanged()
            .map_err(map_directory_error)?;
        let staged_identity =
            identity::capture_identity(&staging.path, cancel).map_err(map_identity_error)?;
        progress.reset("publishing", None, None, &mut observer)?;
        publish_staged_directory(
            &staging.path,
            &target_parent,
            target_name,
            &staged_identity,
            cancel,
        )?;
        target_committed = true;
        staging.published = true;
        if let Some(observer) = observer.as_deref_mut() {
            observer(crate::fs_safety::FileOperationEvent::Phase(
                "target_committed".to_string(),
            ))?;
        }
        #[cfg(any(test, feature = "native-qa"))]
        if super::atomic_move::test_faults::take_fault(
            super::atomic_move::test_faults::AtomicFaultPoint::TargetDurability,
        ) {
            return Err(AtomicMoveError::TargetCommittedDurabilityUnknown);
        }
        target_parent
            .sync()
            .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
        progress.reset("verify_target", None, None, &mut observer)?;
        let target_digest = directory_content_digest(
            &target_parent.path().join(target_name),
            cancel,
            &mut progress,
            &mut observer,
        )
        .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
        progress.finish(&mut observer)?;
        if target_digest.hash != source_before.hash {
            return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
        }
        claim
            .verify_current_identity(cancel)
            .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
        claim
            .sync()
            .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
        if retire_source {
            progress.reset(
                "source_retirement",
                Some(source_before.entries),
                Some(source_before.bytes),
                &mut observer,
            )?;
            if let Some(observer) = observer.as_deref_mut() {
                observer(crate::fs_safety::FileOperationEvent::Phase(
                    "source_cleanup_pending".to_string(),
                ))?;
            }
            #[cfg(any(test, feature = "native-qa"))]
            source_claim::run_claim_test_hook(
                source_claim::ClaimTestPoint::AfterTargetCommitBeforeSourceCleanup,
                claim.original_path(),
                claim.current_path(),
            );
            #[cfg(any(test, feature = "native-qa"))]
            if super::atomic_move::test_faults::take_fault(
                super::atomic_move::test_faults::AtomicFaultPoint::SourceCleanup,
            ) {
                return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
            }
            if is_cancelled(cancel) {
                // The target is already durable. Restore the claimed source
                // pathname when no source entry has been retired yet, and
                // report a recoverable post-commit state instead of success.
                claim
                    .rollback_to_original()
                    .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
                claim
                    .sync_original_parent()
                    .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
                return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
            }
            let target_identity = crate::fs_safety::DirectoryIdentity {
                volume_id: staging.root_volume_id.clone(),
                file_id: staging.root_file_id.clone(),
            };
            retire_claimed_directory_verified_against_target(
                claim,
                &target_parent,
                target_name,
                &target_identity,
                cancel,
                &mut progress,
                &mut observer,
            )
            .map_err(|error| {
                AtomicMoveError::TargetCommittedSourceDeleteFailed(format!(
                    "source_cleanup_pending: {error}"
                ))
            })?;
            claim
                .sync_current_parent()
                .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
            progress.finish(&mut observer)?;
        } else {
            claim
                .rollback_to_original()
                .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
            claim
                .sync_original_parent()
                .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
        }
        if let Some(observer) = observer.as_deref_mut() {
            observer(crate::fs_safety::FileOperationEvent::Phase(
                "completed".to_string(),
            ))?;
        }
        Ok(())
    })();

    if result.is_err() && !target_committed {
        if let Err(error) = claim.rollback_to_original() {
            return Err(AtomicMoveError::SourceClaimRollbackFailed(
                error.to_string(),
            ));
        }
    }
    if let Err(error) = result {
        if error.is_post_commit() {
            staging.published = true;
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
struct StagingDirectory {
    path: PathBuf,
    root_volume_id: String,
    root_file_id: String,
    published: bool,
}

#[cfg(windows)]
impl StagingDirectory {
    fn create(
        parent: &VerifiedDirectory,
        _target_name: &std::ffi::OsStr,
    ) -> Result<Self, AtomicMoveError> {
        parent.ensure_unchanged().map_err(map_directory_error)?;
        for _ in 0..4 {
            let name = OsString::from(format!(".zen-canvas-stage-{}", uuid::Uuid::new_v4()));
            let path = parent.path().join(name);
            match fs::create_dir(&path) {
                Ok(()) => {
                    let verified = match VerifiedDirectory::open_existing(&path) {
                        Ok(verified) => verified,
                        Err(error) => {
                            let _ = remove_owned_staging_tree(&path, None, true);
                            return Err(map_directory_error(error));
                        }
                    };
                    if let Err(error) = parent.ensure_unchanged() {
                        let _ = remove_owned_staging_tree(
                            &path,
                            Some((&verified.identity().volume_id, &verified.identity().file_id)),
                            true,
                        );
                        return Err(map_directory_error(error));
                    }
                    return Ok(Self {
                        path,
                        root_volume_id: verified.identity().volume_id.clone(),
                        root_file_id: verified.identity().file_id.clone(),
                        published: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(AtomicMoveError::Io(error)),
            }
        }
        Err(AtomicMoveError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "staging directory name collision",
        )))
    }
}

#[cfg(windows)]
impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.published {
            let _ = remove_owned_staging_tree(
                &self.path,
                Some((&self.root_volume_id, &self.root_file_id)),
                true,
            );
        }
    }
}

#[cfg(windows)]
fn copy_directory_contents(
    source: &std::path::Path,
    target: &std::path::Path,
    depth: usize,
    entry_count: &mut usize,
    cancel: Option<&AtomicBool>,
    progress: &mut crate::fs_safety::DirectoryTransferProgress,
    observer: &mut Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), AtomicMoveError> {
    const MAX_DEPTH: usize = 64;
    const MAX_ENTRIES: usize = 5000;
    if depth >= MAX_DEPTH {
        return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
    }
    let source_directory = VerifiedDirectory::open_existing(source).map_err(map_directory_error)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(source).map_err(AtomicMoveError::Io)? {
        if entry_count.saturating_add(entries.len()) >= MAX_ENTRIES {
            return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
        }
        entries.push(entry.map_err(AtomicMoveError::Io)?);
    }
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        *entry_count += 1;
        if *entry_count > MAX_ENTRIES {
            return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
        }
        let name = entry.file_name();
        let source_child = entry.path();
        let target_child = target.join(&name);
        let metadata = fs::symlink_metadata(&source_child).map_err(AtomicMoveError::Io)?;
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            return Err(AtomicMoveError::ReparsePoint);
        }
        ensure_supported_windows_file_attributes(&metadata)?;
        ensure_windows_no_named_streams(&source_child)?;
        if metadata.is_dir() {
            fs::create_dir(&target_child).map_err(map_create_error)?;
            copy_directory_contents(
                &source_child,
                &target_child,
                depth + 1,
                entry_count,
                cancel,
                progress,
                observer,
            )?;
            copy_windows_security_descriptor(&source_child, &target_child)?;
            VerifiedDirectory::open_existing(&target_child)
                .map_err(map_directory_error)?
                .sync()
                .map_err(AtomicMoveError::Io)?;
        } else if metadata.is_file() {
            copy_regular_file(&source_child, &target_child, cancel, progress, observer)?;
        } else {
            return Err(AtomicMoveError::UnsafePath);
        }
        progress.record_item(&source_child, observer)?;
        #[cfg(any(test, feature = "native-qa"))]
        if super::atomic_move::test_faults::take_fault(
            super::atomic_move::test_faults::AtomicFaultPoint::CancelDirectoryCopyAfterEntry,
        ) {
            return Err(AtomicMoveError::Cancelled);
        }
    }
    source_directory
        .ensure_unchanged()
        .map_err(map_directory_error)?;
    Ok(())
}

#[cfg(windows)]
fn copy_regular_file(
    source: &std::path::Path,
    target: &std::path::Path,
    cancel: Option<&AtomicBool>,
    progress: &mut crate::fs_safety::DirectoryTransferProgress,
    observer: &mut Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), AtomicMoveError> {
    use std::io::{Seek, SeekFrom};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    if is_cancelled(cancel) {
        return Err(AtomicMoveError::Cancelled);
    }
    ensure_windows_sacl_absent_at_path(source)?;
    ensure_windows_no_named_streams(source)?;
    let mut source_options = OpenOptions::new();
    source_options
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let mut reader = source_options.open(source).map_err(AtomicMoveError::Io)?;
    let metadata = reader.metadata().map_err(AtomicMoveError::Io)?;
    if !metadata.is_file() || metadata_is_reparse_point(&metadata) {
        return Err(AtomicMoveError::ReparsePoint);
    }
    ensure_supported_windows_file_attributes(&metadata)?;
    let expected = identity::capture_identity_from_handle(&reader, source, cancel)
        .map_err(map_identity_error)?;
    let path_expected = identity::capture_identity(source, cancel).map_err(map_identity_error)?;
    if !identity::identity_matches(&expected, &path_expected) {
        return Err(AtomicMoveError::SourceChanged);
    }
    reader
        .seek(SeekFrom::Start(0))
        .map_err(AtomicMoveError::Io)?;
    let mut writer = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(map_create_error)?;
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"file\0");
    hasher.update(&metadata.len().to_le_bytes());
    let mut copied = 0_u64;
    loop {
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        let read = reader.read(&mut buffer).map_err(AtomicMoveError::Io)?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(AtomicMoveError::Io)?;
        hasher.update(&buffer[..read]);
        copied = copied.saturating_add(read as u64);
        progress.record_bytes(source, read as u64, observer)?;
    }
    let source_times = std::fs::FileTimes::new()
        .set_accessed(metadata.accessed().map_err(AtomicMoveError::Io)?)
        .set_modified(metadata.modified().map_err(AtomicMoveError::Io)?);
    writer
        .set_times(source_times)
        .map_err(AtomicMoveError::Io)?;
    writer
        .set_permissions(metadata.permissions())
        .map_err(AtomicMoveError::Io)?;
    writer.sync_all().map_err(AtomicMoveError::Io)?;
    set_windows_copy_attributes(target, metadata.file_attributes())?;
    let full_hash = hasher.finalize().to_hex().to_string();
    if copied != expected.size || expected.full_hash.as_deref() != Some(full_hash.as_str()) {
        return Err(AtomicMoveError::CopyVerificationFailed);
    }
    let source_after = identity::capture_identity_from_handle(&reader, source, cancel)
        .map_err(map_identity_error)?;
    if !identity::identity_matches(&expected, &source_after) {
        return Err(AtomicMoveError::SourceChanged);
    }
    let target_identity = identity::capture_identity(target, cancel).map_err(map_identity_error)?;
    if !identity::content_identity_matches(&expected, &target_identity) {
        return Err(AtomicMoveError::CopyVerificationFailed);
    }
    copy_windows_security_descriptor(source, target)?;
    Ok(())
}

#[cfg(windows)]
fn copy_windows_security_descriptor(
    source: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), AtomicMoveError> {
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Security::{SetFileSecurityW, PROTECTED_DACL_SECURITY_INFORMATION},
    };
    // This copier writes owner/group/DACL only. Never duplicate an object
    // whose audit policy would be silently stripped by that descriptor.
    ensure_windows_sacl_absent_at_path(source)?;
    let descriptor = read_windows_security_descriptor(source)?;
    let target_wide = source_claim::windows_wide_path(target);
    if unsafe {
        SetFileSecurityW(
            target_wide.as_ptr(),
            // Keep the source's current effective ACE list when the staged
            // entry is created under a different parent. Leaving inheritance
            // enabled here would silently merge the destination parent's
            // DACL into the moved tree and could broaden access.
            descriptor.information | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor.bytes.as_ptr().cast_mut().cast(),
        )
    } == 0
    {
        let error = unsafe { GetLastError() };
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
            error as i32,
        )));
    }
    ensure_windows_sacl_absent_at_path(target)?;
    Ok(())
}

/// Fail closed for Windows staged transfers when SACL state cannot be read or
/// when an audit ACL is present. The privilege is enabled only while opening
/// and inspecting each entry, then immediately restored.
#[cfg(windows)]
pub(crate) fn ensure_windows_transfer_security_supported(
    root: &std::path::Path,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    fn visit(
        current: &std::path::Path,
        depth: usize,
        entry_count: &mut usize,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), AtomicMoveError> {
        const MAX_DEPTH: usize = 64;
        const MAX_ENTRIES: usize = 5000;
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        if depth >= MAX_DEPTH {
            return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
        }
        ensure_windows_sacl_absent_at_path(current)?;
        let metadata = fs::symlink_metadata(current).map_err(AtomicMoveError::Io)?;
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            return Err(AtomicMoveError::ReparsePoint);
        }
        if !metadata.is_dir() {
            if metadata.is_file() {
                return Ok(());
            }
            return Err(AtomicMoveError::UnsafePath);
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(current).map_err(AtomicMoveError::Io)? {
            if entry_count.saturating_add(entries.len()) >= MAX_ENTRIES {
                return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
            }
            entries.push(entry.map_err(AtomicMoveError::Io)?);
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if is_cancelled(cancel) {
                return Err(AtomicMoveError::Cancelled);
            }
            *entry_count += 1;
            if *entry_count > MAX_ENTRIES {
                return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
            }
            visit(&entry.path(), depth + 1, entry_count, cancel)?;
        }
        Ok(())
    }

    let mut entry_count = 0;
    visit(root, 0, &mut entry_count, cancel)
}

#[cfg(windows)]
fn ensure_windows_sacl_absent_at_path(path: &std::path::Path) -> Result<(), AtomicMoveError> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{GetLastError, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL,
        },
        System::SystemServices::ACCESS_SYSTEM_SECURITY,
    };

    let metadata = fs::symlink_metadata(path).map_err(AtomicMoveError::Io)?;
    if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
        return Err(AtomicMoveError::ReparsePoint);
    }
    let _privilege = WindowsSaclPrivilege::enable()?;
    let wide = source_claim::windows_wide_path(path);
    let flags = FILE_FLAG_OPEN_REPARSE_POINT
        | if metadata.is_dir() {
            FILE_FLAG_BACKUP_SEMANTICS
        } else {
            0
        };
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            READ_CONTROL | ACCESS_SYSTEM_SECURITY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        let _error = unsafe { GetLastError() };
        return Err(AtomicMoveError::WindowsSaclInspectionUnavailable);
    }
    let handle = unsafe { std::fs::File::from(OwnedHandle::from_raw_handle(raw)) };
    let info = windows_handle_entry_info(&handle)?;
    if info.is_reparse || info.is_directory != metadata.is_dir() {
        return Err(AtomicMoveError::ReparsePoint);
    }
    ensure_windows_sacl_absent_from_handle(&handle, &_privilege)
}

#[cfg(windows)]
fn ensure_windows_sacl_absent_from_handle(
    file: &File,
    _privilege: &WindowsSaclPrivilege,
) -> Result<(), AtomicMoveError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{GetLastError, LocalFree, HANDLE},
        Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
        Security::{GetSecurityDescriptorSacl, SACL_SECURITY_INFORMATION},
    };
    let mut descriptor = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            SACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(AtomicMoveError::WindowsSaclInspectionUnavailable);
    }
    let mut present = 0;
    let mut sacl = std::ptr::null_mut();
    let mut defaulted = 0;
    let read_ok =
        unsafe { GetSecurityDescriptorSacl(descriptor, &mut present, &mut sacl, &mut defaulted) };
    unsafe {
        LocalFree(descriptor);
    }
    if read_ok == 0 {
        let _ = unsafe { GetLastError() };
        return Err(AtomicMoveError::WindowsSaclInspectionUnavailable);
    }
    if present != 0 {
        return Err(AtomicMoveError::WindowsSaclTransferUnsupported);
    }
    Ok(())
}

#[cfg(windows)]
struct WindowsSaclPrivilege {
    token: windows_sys::Win32::Foundation::HANDLE,
    previous: windows_sys::Win32::Security::TOKEN_PRIVILEGES,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(windows)]
impl WindowsSaclPrivilege {
    fn enable() -> Result<Self, AtomicMoveError> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, GetLastError, ERROR_NOT_ALL_ASSIGNED, HANDLE, LUID},
            Security::{
                AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES,
                SE_PRIVILEGE_ENABLED, SE_SECURITY_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES,
                TOKEN_QUERY,
            },
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        };
        #[cfg(any(test, feature = "native-qa"))]
        if super::atomic_move::test_faults::take_fault(
            super::atomic_move::test_faults::AtomicFaultPoint::WindowsSaclInspectionUnavailable,
        ) {
            return Err(AtomicMoveError::WindowsSaclInspectionUnavailable);
        }
        static PRIVILEGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let lock = PRIVILEGE_LOCK
            .lock()
            .map_err(|_| AtomicMoveError::WindowsSaclInspectionUnavailable)?;
        let mut token: HANDLE = std::ptr::null_mut();
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        } == 0
        {
            return Err(AtomicMoveError::WindowsSaclInspectionUnavailable);
        }
        let mut luid = LUID::default();
        if unsafe { LookupPrivilegeValueW(std::ptr::null(), SE_SECURITY_NAME, &mut luid) } == 0 {
            unsafe {
                CloseHandle(token);
            }
            return Err(AtomicMoveError::WindowsSaclInspectionUnavailable);
        }
        let requested = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let mut previous = TOKEN_PRIVILEGES::default();
        unsafe {
            windows_sys::Win32::Foundation::SetLastError(0);
        }
        if unsafe {
            AdjustTokenPrivileges(
                token,
                0,
                &requested,
                std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                &mut previous,
                std::ptr::null_mut(),
            )
        } == 0
            || unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED
        {
            unsafe {
                CloseHandle(token);
            }
            return Err(AtomicMoveError::WindowsSaclInspectionUnavailable);
        }
        Ok(Self {
            token,
            previous,
            _lock: lock,
        })
    }
}

#[cfg(windows)]
impl Drop for WindowsSaclPrivilege {
    fn drop(&mut self) {
        use windows_sys::Win32::{Foundation::CloseHandle, Security::AdjustTokenPrivileges};
        unsafe {
            AdjustTokenPrivileges(
                self.token,
                0,
                &self.previous,
                std::mem::size_of::<windows_sys::Win32::Security::TOKEN_PRIVILEGES>() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            CloseHandle(self.token);
        }
    }
}

#[cfg(windows)]
struct WindowsSecurityDescriptor {
    bytes: Vec<u8>,
    information: u32,
}

#[cfg(windows)]
fn read_windows_security_descriptor(
    path: &std::path::Path,
) -> Result<WindowsSecurityDescriptor, AtomicMoveError> {
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_INSUFFICIENT_BUFFER},
        Security::{
            GetFileSecurityW, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION,
            OWNER_SECURITY_INFORMATION,
        },
    };

    let information =
        OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let wide = source_claim::windows_wide_path(path);
    let mut descriptor_size = 0u32;
    let first_read = unsafe {
        GetFileSecurityW(
            wide.as_ptr(),
            information,
            ptr::null_mut(),
            0,
            &mut descriptor_size,
        )
    };
    let first_error = unsafe { GetLastError() };
    if first_read != 0 || first_error != ERROR_INSUFFICIENT_BUFFER || descriptor_size == 0 {
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
            first_error as i32,
        )));
    }
    let mut bytes = vec![0u8; descriptor_size as usize];
    let read = unsafe {
        GetFileSecurityW(
            wide.as_ptr(),
            information,
            bytes.as_mut_ptr().cast(),
            descriptor_size,
            &mut descriptor_size,
        )
    };
    if read == 0 || descriptor_size as usize > bytes.len() {
        let error = unsafe { GetLastError() };
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
            error as i32,
        )));
    }
    bytes.truncate(descriptor_size as usize);
    Ok(WindowsSecurityDescriptor { bytes, information })
}

#[cfg(windows)]
fn ensure_supported_windows_file_attributes(
    metadata: &std::fs::Metadata,
) -> Result<(), AtomicMoveError> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_EA,
        FILE_ATTRIBUTE_ENCRYPTED, FILE_ATTRIBUTE_INTEGRITY_STREAM, FILE_ATTRIBUTE_NO_SCRUB_DATA,
        FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_PINNED, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_SPARSE_FILE, FILE_ATTRIBUTE_TEMPORARY,
        FILE_ATTRIBUTE_UNPINNED, FILE_ATTRIBUTE_VIRTUAL,
    };
    let unsupported = FILE_ATTRIBUTE_COMPRESSED
        | FILE_ATTRIBUTE_DEVICE
        | FILE_ATTRIBUTE_EA
        | FILE_ATTRIBUTE_ENCRYPTED
        | FILE_ATTRIBUTE_INTEGRITY_STREAM
        | FILE_ATTRIBUTE_NO_SCRUB_DATA
        | FILE_ATTRIBUTE_OFFLINE
        | FILE_ATTRIBUTE_PINNED
        | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
        | FILE_ATTRIBUTE_RECALL_ON_OPEN
        | FILE_ATTRIBUTE_SPARSE_FILE
        | FILE_ATTRIBUTE_TEMPORARY
        | FILE_ATTRIBUTE_UNPINNED
        | FILE_ATTRIBUTE_VIRTUAL;
    if metadata.file_attributes() & unsupported != 0 {
        return Err(AtomicMoveError::Io(io::Error::new(
            io::ErrorKind::Unsupported,
            "cross-volume directory move encountered unsupported Windows file attributes",
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn set_windows_copy_attributes(
    path: &std::path::Path,
    source_attributes: u32,
) -> Result<(), AtomicMoveError> {
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Storage::FileSystem::{
            SetFileAttributesW, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN,
            FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_NOT_CONTENT_INDEXED, FILE_ATTRIBUTE_READONLY,
            FILE_ATTRIBUTE_SYSTEM,
        },
    };
    let attributes = source_attributes
        & (FILE_ATTRIBUTE_ARCHIVE
            | FILE_ATTRIBUTE_HIDDEN
            | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED
            | FILE_ATTRIBUTE_READONLY
            | FILE_ATTRIBUTE_SYSTEM);
    let attributes = if attributes == 0 {
        FILE_ATTRIBUTE_NORMAL
    } else {
        attributes
    };
    let wide = source_claim::windows_wide_path(path);
    if unsafe { SetFileAttributesW(wide.as_ptr(), attributes) } == 0 {
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
            unsafe { GetLastError() } as i32,
        )));
    }
    Ok(())
}

#[cfg(windows)]
struct DirectoryContentDigest {
    hash: String,
    entries: u64,
    bytes: u64,
}

#[cfg(windows)]
fn directory_content_digest(
    root: &std::path::Path,
    cancel: Option<&AtomicBool>,
    progress: &mut crate::fs_safety::DirectoryTransferProgress,
    observer: &mut Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<DirectoryContentDigest, AtomicMoveError> {
    fn visit(
        root: &std::path::Path,
        current: &std::path::Path,
        depth: usize,
        entry_count: &mut usize,
        byte_count: &mut u64,
        hasher: &mut blake3::Hasher,
        cancel: Option<&AtomicBool>,
        progress: &mut crate::fs_safety::DirectoryTransferProgress,
        observer: &mut Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    ) -> Result<(), AtomicMoveError> {
        use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
        const MAX_DEPTH: usize = 64;
        const MAX_ENTRIES: usize = 5000;
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        if depth >= MAX_DEPTH {
            return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
        }
        let verified = VerifiedDirectory::open_existing(current).map_err(map_directory_error)?;
        ensure_windows_sacl_absent_at_path(current)?;
        let security_descriptor = windows_security_descriptor_bytes(current)?;
        hasher.update(b"security-descriptor\0");
        hasher.update(&(security_descriptor.len() as u64).to_le_bytes());
        hasher.update(&security_descriptor);
        let relative = current
            .strip_prefix(root)
            .map_err(|_| AtomicMoveError::UnsafePath)?;
        let name = relative
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        hasher.update(b"directory\0");
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(&name);
        let mut entries = Vec::new();
        for entry in fs::read_dir(current).map_err(AtomicMoveError::Io)? {
            if entry_count.saturating_add(entries.len()) >= MAX_ENTRIES {
                return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
            }
            entries.push(entry.map_err(AtomicMoveError::Io)?);
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            *entry_count += 1;
            if *entry_count > MAX_ENTRIES {
                return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(AtomicMoveError::Io)?;
            if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
                return Err(AtomicMoveError::ReparsePoint);
            }
            ensure_supported_windows_file_attributes(&metadata)?;
            ensure_windows_sacl_absent_at_path(&path)?;
            ensure_windows_no_named_streams(&path)?;
            progress.record_item(&path, observer)?;
            if metadata.is_dir() {
                visit(
                    root,
                    &path,
                    depth + 1,
                    entry_count,
                    byte_count,
                    hasher,
                    cancel,
                    progress,
                    observer,
                )?;
            } else if metadata.is_file() {
                let security_descriptor = windows_security_descriptor_bytes(&path)?;
                hasher.update(b"security-descriptor\0");
                hasher.update(&(security_descriptor.len() as u64).to_le_bytes());
                hasher.update(&security_descriptor);
                let mut options = OpenOptions::new();
                options.read(true).custom_flags(
                    windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
                );
                let file = options.open(&path).map_err(AtomicMoveError::Io)?;
                let handle_metadata = file.metadata().map_err(AtomicMoveError::Io)?;
                if !handle_metadata.is_file() || metadata_is_reparse_point(&handle_metadata) {
                    return Err(AtomicMoveError::ReparsePoint);
                }
                let identity = identity::capture_identity_from_handle(&file, &path, cancel)
                    .map_err(map_identity_error)?;
                *byte_count = (*byte_count).saturating_add(identity.size);
                progress.record_bytes(&path, identity.size, observer)?;
                let name = path
                    .strip_prefix(root)
                    .map_err(|_| AtomicMoveError::UnsafePath)?
                    .as_os_str()
                    .encode_wide()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>();
                hasher.update(b"file\0");
                hasher.update(&(name.len() as u64).to_le_bytes());
                hasher.update(&name);
                hasher.update(&identity.size.to_le_bytes());
                hasher.update(identity.full_hash.as_deref().unwrap_or_default().as_bytes());
            } else {
                return Err(AtomicMoveError::UnsafePath);
            }
        }
        verified.ensure_unchanged().map_err(map_directory_error)?;
        Ok(())
    }

    let mut hasher = blake3::Hasher::new();
    let mut entry_count = 0usize;
    let mut byte_count = 0u64;
    visit(
        root,
        root,
        0,
        &mut entry_count,
        &mut byte_count,
        &mut hasher,
        cancel,
        progress,
        observer,
    )?;
    Ok(DirectoryContentDigest {
        hash: hasher.finalize().to_hex().to_string(),
        entries: entry_count as u64,
        bytes: byte_count,
    })
}

#[cfg(windows)]
fn windows_security_descriptor_bytes(path: &std::path::Path) -> Result<Vec<u8>, AtomicMoveError> {
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::{GetLastError, LocalFree},
        Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, SDDL_REVISION_1,
        },
    };

    let descriptor = read_windows_security_descriptor(path)?;
    let mut sddl_pointer = ptr::null_mut();
    let mut sddl_length = 0u32;
    if unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.bytes.as_ptr().cast_mut().cast(),
            SDDL_REVISION_1,
            descriptor.information,
            &mut sddl_pointer,
            &mut sddl_length,
        )
    } == 0
    {
        let error = unsafe { GetLastError() };
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
            error as i32,
        )));
    }
    let sddl = unsafe {
        let slice =
            std::slice::from_raw_parts(sddl_pointer, sddl_length.saturating_sub(1) as usize);
        String::from_utf16_lossy(slice)
    };
    unsafe {
        LocalFree(sddl_pointer.cast());
    }
    Ok(normalize_windows_security_descriptor_sddl(&sddl).into_bytes())
}

#[cfg(any(windows, test))]
fn normalize_windows_security_descriptor_sddl(sddl: &str) -> String {
    let Some(dacl_start) = sddl.find("D:").map(|index| index + 2) else {
        return sddl.to_string();
    };
    let Some(dacl_end) = sddl[dacl_start..].find('(').map(|index| dacl_start + index) else {
        return sddl.to_string();
    };
    let controls = &sddl[dacl_start..dacl_end];
    let mut index = 0;
    while index < controls.len() {
        let remaining = &controls[index..];
        if remaining.starts_with("AI") || remaining.starts_with("AR") {
            index += 2;
        } else if remaining.starts_with('P') {
            index += 1;
        } else {
            break;
        }
    }
    let without_dacl_control = format!(
        "{}D:{}{}",
        &sddl[..dacl_start - 2],
        &controls[index..],
        &sddl[dacl_end..]
    );

    // PROTECTED_DACL_SECURITY_INFORMATION materializes inherited ACEs on the
    // target. Strip only the ACE `ID` marker before comparing descriptors;
    // the ACE type, remaining inheritance flags, rights, SIDs, and conditions
    // remain part of the digest.
    let mut normalized = String::with_capacity(without_dacl_control.len());
    let mut cursor = 0;
    while let Some(relative_start) = without_dacl_control[cursor..].find('(') {
        let start = cursor + relative_start;
        normalized.push_str(&without_dacl_control[cursor..start]);
        let bytes = without_dacl_control.as_bytes();
        let mut depth = 0usize;
        let mut end = None;
        for (offset, byte) in bytes.iter().enumerate().skip(start) {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = Some(offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            normalized.push_str(&without_dacl_control[start..]);
            return normalized;
        };
        let ace = &without_dacl_control[start..end];
        let ace_bytes = ace.as_bytes();
        let mut ace_depth = 0usize;
        let mut separators = [None, None];
        let mut separator_count = 0usize;
        for (offset, byte) in ace_bytes.iter().enumerate().skip(1).take(ace.len() - 2) {
            match byte {
                b'(' => ace_depth += 1,
                b')' => ace_depth = ace_depth.saturating_sub(1),
                b';' if ace_depth == 0 && separator_count < separators.len() => {
                    separators[separator_count] = Some(offset);
                    separator_count += 1;
                }
                _ => {}
            }
        }
        if let [Some(first_separator), Some(second_separator)] = separators {
            let flags_start = first_separator + 1;
            let flags = &ace[flags_start..second_separator];
            let normalized_flags = flags.replace("ID", "");
            normalized.push_str(&ace[..flags_start]);
            normalized.push_str(&normalized_flags);
            normalized.push_str(&ace[second_separator..]);
        } else {
            normalized.push_str(ace);
        }
        cursor = end;
    }
    normalized.push_str(&without_dacl_control[cursor..]);
    normalized
}

#[cfg(windows)]
fn windows_security_descriptor_bytes_from_handle(file: &File) -> Result<Vec<u8>, AtomicMoveError> {
    use std::{os::windows::io::AsRawHandle, ptr};
    use windows_sys::Win32::{
        Foundation::{GetLastError, LocalFree, HANDLE},
        Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_FILE_OBJECT,
        },
        Security::{
            DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        },
    };

    let information =
        OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut descriptor = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            information,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
            status as i32,
        )));
    }
    let mut sddl_pointer = ptr::null_mut();
    let mut sddl_length = 0u32;
    if unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            information,
            &mut sddl_pointer,
            &mut sddl_length,
        )
    } == 0
    {
        let error = unsafe { GetLastError() };
        unsafe {
            LocalFree(descriptor);
        }
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
            error as i32,
        )));
    }
    let sddl = unsafe {
        let slice =
            std::slice::from_raw_parts(sddl_pointer, sddl_length.saturating_sub(1) as usize);
        String::from_utf16_lossy(slice)
    };
    unsafe {
        LocalFree(sddl_pointer.cast());
        LocalFree(descriptor);
    }
    Ok(normalize_windows_security_descriptor_sddl(&sddl).into_bytes())
}

/// Retires a cross-volume directory claim only after comparing each source
/// file with the already published target while holding no-write handles on
/// both entries. Directory handles are opened relative to their verified
/// parent handles, and reparse points and special entries fail closed.
#[cfg(windows)]
fn retire_claimed_directory_verified_against_target(
    claim: &mut SourceClaim,
    target_parent: &VerifiedDirectory,
    target_name: &std::ffi::OsStr,
    expected_target: &super::verified_directory::DirectoryIdentity,
    cancel: Option<&AtomicBool>,
    progress: &mut crate::fs_safety::DirectoryTransferProgress,
    observer: &mut Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), AtomicMoveError> {
    let source_path = claim.current_path().to_path_buf();
    let source_root_privilege = WindowsSaclPrivilege::enable()?;
    let source_root = open_windows_directory_path_for_retirement(&source_path)?;
    ensure_windows_sacl_absent_from_handle(&source_root, &source_root_privilege)?;
    drop(source_root_privilege);
    let source_identity = windows_handle_directory_identity(&source_root)?;
    let expected_source = claim.actual_identity();
    if source_identity.volume_id
        != expected_source
            .platform_volume_id
            .as_deref()
            .unwrap_or_default()
        || source_identity.file_id
            != expected_source
                .platform_file_id
                .as_deref()
                .unwrap_or_default()
    {
        return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
    }
    let source_path_identity = identity::capture_namespace_identity_only(&source_path, cancel)
        .map_err(map_identity_error)?;
    if source_path_identity.platform_volume_id.as_deref()
        != Some(source_identity.volume_id.as_str())
        || source_path_identity.platform_file_id.as_deref()
            != Some(source_identity.file_id.as_str())
    {
        return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
    }

    let target_root =
        open_windows_child_for_tree(target_parent.handle(), target_name, true, false)?;
    if windows_handle_directory_identity(&target_root)? != *expected_target {
        return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
    }
    if windows_security_descriptor_bytes_from_handle(&source_root)?
        != windows_security_descriptor_bytes_from_handle(&target_root)?
    {
        return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
    }
    let target_path = target_parent.path().join(target_name);
    let mut context = DirectoryRetirementContext {
        entry_count: 0,
        deletion_parent: target_parent,
        cancel,
    };
    retire_directory_entries_verified(
        &source_root,
        &source_path,
        &target_root,
        &target_path,
        0,
        &mut context,
        progress,
        observer,
    )?;
    claim.delete_claim().map_err(map_claim_error)?;
    Ok(())
}

#[cfg(windows)]
struct DirectoryRetirementContext<'a> {
    entry_count: usize,
    deletion_parent: &'a VerifiedDirectory,
    cancel: Option<&'a AtomicBool>,
}

#[cfg(windows)]
fn retire_directory_entries_verified(
    source_parent_handle: &File,
    source_parent_path: &std::path::Path,
    target_parent_handle: &File,
    target_parent_path: &std::path::Path,
    depth: usize,
    context: &mut DirectoryRetirementContext<'_>,
    progress: &mut crate::fs_safety::DirectoryTransferProgress,
    observer: &mut Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), AtomicMoveError> {
    const MAX_DEPTH: usize = 64;
    const MAX_ENTRIES: usize = 5000;
    if depth >= MAX_DEPTH {
        return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
    }
    if is_cancelled(context.cancel) {
        return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(source_parent_path).map_err(AtomicMoveError::Io)? {
        if context.entry_count.saturating_add(entries.len()) >= MAX_ENTRIES {
            return Err(AtomicMoveError::DirectoryTreeLimitExceeded);
        }
        entries.push(entry.map_err(AtomicMoveError::Io)?);
    }
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if is_cancelled(context.cancel) {
            return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
        }
        context.entry_count += 1;
        let name = entry.file_name();
        let source_path = source_parent_path.join(&name);
        let target_path = target_parent_path.join(&name);
        let metadata = fs::symlink_metadata(&source_path).map_err(AtomicMoveError::Io)?;
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            return Err(AtomicMoveError::ReparsePoint);
        }
        let is_directory = metadata.is_dir();
        if !is_directory && !metadata.is_file() {
            return Err(AtomicMoveError::UnsafePath);
        }
        let source_child =
            open_windows_child_for_tree(source_parent_handle, &name, is_directory, true)?;
        let source_child_privilege = WindowsSaclPrivilege::enable()?;
        ensure_windows_sacl_absent_from_handle(&source_child, &source_child_privilege)?;
        drop(source_child_privilege);
        let source_child_info = windows_handle_entry_info(&source_child)?;
        if source_child_info.is_reparse || source_child_info.is_directory != is_directory {
            return Err(AtomicMoveError::ReparsePoint);
        }
        let target_child =
            open_windows_child_for_tree(target_parent_handle, &name, is_directory, false)?;
        let target_child_info = windows_handle_entry_info(&target_child)?;
        if target_child_info.is_reparse || target_child_info.is_directory != is_directory {
            return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
        }
        if windows_security_descriptor_bytes_from_handle(&source_child)?
            != windows_security_descriptor_bytes_from_handle(&target_child)?
        {
            return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
        }
        if is_directory {
            retire_directory_entries_verified(
                &source_child,
                &source_path,
                &target_child,
                &target_path,
                depth + 1,
                context,
                progress,
                observer,
            )?;
        } else {
            ensure_windows_no_named_streams(&source_path)?;
            let source_identity =
                identity::capture_identity_from_handle(&source_child, &source_path, context.cancel)
                    .map_err(map_identity_error)?;
            let target_identity =
                identity::capture_identity_from_handle(&target_child, &target_path, context.cancel)
                    .map_err(map_identity_error)?;
            if !identity::content_identity_matches(&source_identity, &target_identity) {
                return Err(AtomicMoveError::SourceChanged);
            }
        }
        source_claim::delete_open_handle(
            &source_child,
            context.deletion_parent,
            &name,
            if is_directory {
                ClaimedEntryKind::Directory
            } else {
                ClaimedEntryKind::File
            },
        )
        .map_err(map_claim_error)?;
        progress.record_item(&source_path, observer)?;
        if !is_directory {
            progress.record_bytes(&source_path, metadata.len(), observer)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
struct WindowsHandleEntryInfo {
    is_directory: bool,
    is_reparse: bool,
    identity: super::verified_directory::DirectoryIdentity,
}

#[cfg(windows)]
fn windows_handle_entry_info(handle: &File) -> Result<WindowsHandleEntryInfo, AtomicMoveError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT,
    };
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
        return Err(AtomicMoveError::Io(io::Error::last_os_error()));
    }
    Ok(WindowsHandleEntryInfo {
        is_directory: info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        is_reparse: info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        identity: super::verified_directory::DirectoryIdentity {
            volume_id: info.dwVolumeSerialNumber.to_string(),
            file_id: (u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow))
                .to_string(),
        },
    })
}

#[cfg(windows)]
fn windows_handle_directory_identity(
    handle: &File,
) -> Result<super::verified_directory::DirectoryIdentity, AtomicMoveError> {
    let info = windows_handle_entry_info(handle)?;
    if !info.is_directory || info.is_reparse {
        return Err(AtomicMoveError::ReparsePoint);
    }
    Ok(info.identity)
}

#[cfg(windows)]
fn open_windows_directory_path_for_retirement(
    path: &std::path::Path,
) -> Result<File, AtomicMoveError> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{GetLastError, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
            OPEN_EXISTING, READ_CONTROL, SYNCHRONIZE,
        },
        System::SystemServices::ACCESS_SYSTEM_SECURITY,
    };
    let path = source_claim::windows_wide_path(path);
    let raw = unsafe {
        CreateFileW(
            path.as_ptr(),
            DELETE
                | FILE_LIST_DIRECTORY
                | FILE_READ_ATTRIBUTES
                | READ_CONTROL
                | SYNCHRONIZE
                | ACCESS_SYSTEM_SECURITY,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
            unsafe { GetLastError() } as i32,
        )));
    }
    let handle = unsafe { File::from(OwnedHandle::from_raw_handle(raw)) };
    windows_handle_directory_identity(&handle)?;
    Ok(handle)
}

#[cfg(windows)]
fn open_windows_child_for_tree(
    parent: &File,
    name: &std::ffi::OsStr,
    is_directory: bool,
    delete_access: bool,
) -> Result<File, AtomicMoveError> {
    use std::{
        mem,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
    };
    use windows_sys::{
        Wdk::{
            Foundation::OBJECT_ATTRIBUTES,
            Storage::FileSystem::{
                NtCreateFile, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
                FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
            },
        },
        Win32::{
            Foundation::{RtlNtStatusToDosError, OBJ_CASE_INSENSITIVE, UNICODE_STRING},
            Storage::FileSystem::{
                DELETE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_READ,
                READ_CONTROL, SYNCHRONIZE,
            },
            System::SystemServices::ACCESS_SYSTEM_SECURITY,
            System::IO::IO_STATUS_BLOCK,
        },
    };
    let mut wide = name.encode_wide().collect::<Vec<_>>();
    if wide.is_empty()
        || wide.contains(&0)
        || wide.len() > (u16::MAX as usize / 2)
        || std::path::Path::new(name).components().count() != 1
    {
        return Err(AtomicMoveError::UnsafePath);
    }
    let unicode = UNICODE_STRING {
        Length: (wide.len() * mem::size_of::<u16>()) as u16,
        MaximumLength: (wide.len() * mem::size_of::<u16>()) as u16,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &unicode,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut raw = std::ptr::null_mut();
    let mut io_status = unsafe { mem::zeroed::<IO_STATUS_BLOCK>() };
    let _privilege = delete_access
        .then(WindowsSaclPrivilege::enable)
        .transpose()?;
    let access = (if delete_access {
        DELETE | ACCESS_SYSTEM_SECURITY
    } else {
        0
    }) | FILE_READ_ATTRIBUTES
        | READ_CONTROL
        | SYNCHRONIZE
        | if is_directory {
            FILE_LIST_DIRECTORY
        } else {
            FILE_READ_DATA
        };
    let options = FILE_OPEN_REPARSE_POINT
        | FILE_SYNCHRONOUS_IO_NONALERT
        | if is_directory {
            FILE_DIRECTORY_FILE
        } else {
            FILE_NON_DIRECTORY_FILE
        };
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            access,
            &attributes,
            &mut io_status,
            std::ptr::null(),
            0,
            FILE_SHARE_READ,
            FILE_OPEN,
            options,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(status)
        }
            as i32)));
    }
    let handle = unsafe { File::from(OwnedHandle::from_raw_handle(raw)) };
    let info = windows_handle_entry_info(&handle)?;
    if info.is_reparse || info.is_directory != is_directory {
        return Err(AtomicMoveError::ReparsePoint);
    }
    Ok(handle)
}

#[cfg(windows)]
fn ensure_windows_no_named_streams(path: &std::path::Path) -> Result<(), AtomicMoveError> {
    use windows_sys::Win32::{
        Foundation::{
            GetLastError, ERROR_HANDLE_EOF, ERROR_INVALID_FUNCTION, INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{
            FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard,
            WIN32_FIND_STREAM_DATA,
        },
    };
    let metadata = fs::symlink_metadata(path).map_err(AtomicMoveError::Io)?;
    if metadata_is_reparse_point(&metadata) || !(metadata.is_file() || metadata.is_dir()) {
        return Err(AtomicMoveError::ReparsePoint);
    }
    let wide = source_claim::windows_wide_path(path);
    let mut data = WIN32_FIND_STREAM_DATA::default();
    let find = unsafe {
        FindFirstStreamW(
            wide.as_ptr(),
            FindStreamInfoStandard,
            (&mut data as *mut WIN32_FIND_STREAM_DATA).cast(),
            0,
        )
    };
    if find == INVALID_HANDLE_VALUE {
        let code = unsafe { GetLastError() };
        // Filesystems without named-stream support report ERROR_INVALID_FUNCTION.
        // Windows reports ERROR_HANDLE_EOF for a directory with no streams.
        if code == ERROR_INVALID_FUNCTION || (metadata.is_dir() && code == ERROR_HANDLE_EOF) {
            return Ok(());
        }
        return Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
            code as i32,
        )));
    }
    let mut result = Ok(());
    loop {
        let end = data
            .cStreamName
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(data.cStreamName.len());
        if String::from_utf16_lossy(&data.cStreamName[..end]) != "::$DATA" {
            result = Err(AtomicMoveError::Io(io::Error::new(
                io::ErrorKind::Unsupported,
                "cross-volume directory move does not support alternate data streams",
            )));
            break;
        }
        data = WIN32_FIND_STREAM_DATA::default();
        if unsafe { FindNextStreamW(find, (&mut data as *mut WIN32_FIND_STREAM_DATA).cast()) } == 0
        {
            let code = unsafe { GetLastError() };
            if code != 38 {
                result = Err(AtomicMoveError::Io(io::Error::from_raw_os_error(
                    code as i32,
                )));
            }
            break;
        }
    }
    unsafe { FindClose(find) };
    result
}

#[cfg(windows)]
fn publish_staged_directory(
    stage_path: &std::path::Path,
    target_parent: &VerifiedDirectory,
    target_name: &std::ffi::OsStr,
    expected: &identity::ExpectedFileIdentity,
    cancel: Option<&AtomicBool>,
) -> Result<(), AtomicMoveError> {
    use super::source_claim::ClaimedEntryKind;
    let source_parent_path = stage_path.parent().ok_or(AtomicMoveError::UnsafePath)?;
    let source_parent =
        VerifiedDirectory::open_existing(source_parent_path).map_err(map_directory_error)?;
    let handle = super::source_claim::open_source_handle(stage_path, ClaimedEntryKind::Directory)
        .map_err(map_claim_error)?
        .ok_or(AtomicMoveError::UnsafePath)?;
    let before = identity::capture_identity_from_handle(&handle, stage_path, cancel)
        .map_err(map_identity_error)?;
    if !identity::content_identity_matches(expected, &before) {
        return Err(AtomicMoveError::StagingIdentityChanged);
    }
    target_parent
        .ensure_unchanged()
        .map_err(map_directory_error)?;
    super::source_claim::commit_open_handle_noreplace(
        &handle,
        &source_parent,
        stage_path.file_name().ok_or(AtomicMoveError::UnsafePath)?,
        target_parent,
        target_name,
    )
    .map_err(map_claim_error)?;
    let target_path = target_parent.path().join(target_name);
    let after_handle = identity::capture_identity_from_handle(&handle, &target_path, cancel)
        .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
    let after_path = identity::capture_identity(&target_path, cancel)
        .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
    if !identity::content_identity_matches(expected, &after_handle)
        || !identity::identity_matches(&after_handle, &after_path)
    {
        return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
    }
    target_parent
        .sync()
        .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
    if is_cancelled(cancel) {
        return Err(AtomicMoveError::TargetCommittedSourceCleanupPending);
    }
    Ok(())
}

#[cfg(windows)]
fn remove_owned_staging_tree(
    path: &std::path::Path,
    expected_root: Option<(&str, &str)>,
    root: bool,
) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if root {
        if let Some((volume_id, file_id)) = expected_root {
            let identity = identity::capture_namespace_identity_only(path, None)
                .map_err(|error| io::Error::other(error.to_string()))?;
            if identity.platform_volume_id.as_deref() != Some(volume_id)
                || identity.platform_file_id.as_deref() != Some(file_id)
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "staging root identity changed",
                ));
            }
        }
    }
    if metadata.is_dir() && !metadata_is_reparse_point(&metadata) {
        for entry in fs::read_dir(path)? {
            remove_owned_staging_tree(&entry?.path(), None, false)?;
        }
        fs::remove_dir(path)
    } else if metadata.is_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

fn copy_file_from_claim(
    claim: &SourceClaim,
    writer: &mut File,
    cancel: Option<&AtomicBool>,
    progress: &mut crate::fs_safety::DirectoryTransferProgress,
    observer: &mut Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), AtomicMoveError> {
    let mut reader = claim.open_read().map_err(map_claim_error)?;
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"file\0");
    hasher.update(&claim.expected_identity().size.to_le_bytes());
    let mut copied = 0_u64;
    loop {
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        let read = reader.read(&mut buffer).map_err(AtomicMoveError::Io)?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(AtomicMoveError::Io)?;
        copied = copied.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
        progress.record_bytes(claim.current_path(), read as u64, observer)?;
    }
    writer.sync_all().map_err(AtomicMoveError::Io)?;
    let hash = hasher.finalize().to_hex().to_string();
    if copied != claim.expected_identity().size
        || claim.expected_identity().full_hash.as_deref() != Some(hash.as_str())
    {
        return Err(AtomicMoveError::CopyVerificationFailed);
    }
    Ok(())
}

fn unique_staging_name(_target_name: &std::ffi::OsStr) -> OsString {
    OsString::from(format!(".zen-canvas-stage-{}", uuid::Uuid::new_v4()))
}

#[cfg(windows)]
fn create_staging_handle(parent: &VerifiedDirectory, name: &std::ffi::OsStr) -> io::Result<File> {
    use std::{
        mem,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
    };
    use windows_sys::{
        Wdk::{
            Foundation::OBJECT_ATTRIBUTES,
            Storage::FileSystem::{
                NtCreateFile, FILE_CREATE, FILE_NON_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT,
                FILE_SYNCHRONOUS_IO_NONALERT,
            },
        },
        Win32::{
            Foundation::{RtlNtStatusToDosError, OBJ_CASE_INSENSITIVE, UNICODE_STRING},
            Storage::FileSystem::{
                DELETE, FILE_ATTRIBUTE_NORMAL, FILE_READ_ATTRIBUTES, FILE_READ_DATA,
                FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
                FILE_WRITE_DATA, SYNCHRONIZE,
            },
            System::IO::IO_STATUS_BLOCK,
        },
    };
    let mut wide = name.encode_wide().collect::<Vec<_>>();
    if wide.is_empty() || wide.contains(&0) || wide.len() > (u16::MAX as usize / 2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid staging name",
        ));
    }
    let unicode = UNICODE_STRING {
        Length: (wide.len() * mem::size_of::<u16>()) as u16,
        MaximumLength: (wide.len() * mem::size_of::<u16>()) as u16,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.handle().as_raw_handle(),
        ObjectName: &unicode,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut raw = std::ptr::null_mut();
    let mut io_status = unsafe { mem::zeroed::<IO_STATUS_BLOCK>() };
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            DELETE
                | FILE_READ_ATTRIBUTES
                | FILE_READ_DATA
                | FILE_WRITE_ATTRIBUTES
                | FILE_WRITE_DATA
                | SYNCHRONIZE,
            &attributes,
            &mut io_status,
            std::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_CREATE,
            FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    Ok(unsafe { File::from(OwnedHandle::from_raw_handle(raw)) })
}

#[cfg(not(windows))]
fn create_staging_handle(parent: &VerifiedDirectory, name: &std::ffi::OsStr) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(parent.path().join(name))
}

fn map_identity_error(error: identity::IdentityError) -> AtomicMoveError {
    match error {
        identity::IdentityError::SourceMissing => AtomicMoveError::SourceMissing,
        identity::IdentityError::Symlink => AtomicMoveError::ReparsePoint,
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
    match reason {
        "directory_entry_limit_exceeded" | "directory_depth_limit_exceeded" => {
            return AtomicMoveError::DirectoryTreeLimitExceeded;
        }
        "nested_volume_entry_unsupported" => {
            return AtomicMoveError::DirectoryNestedVolumeUnsupported;
        }
        _ => {}
    }
    #[cfg(target_os = "macos")]
    {
        AtomicMoveError::MacMutationNotSupported(reason)
    }
    #[cfg(not(target_os = "macos"))]
    {
        AtomicMoveError::Io(io::Error::new(io::ErrorKind::PermissionDenied, reason))
    }
}

fn map_create_error(error: io::Error) -> AtomicMoveError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        AtomicMoveError::TargetExists
    } else {
        AtomicMoveError::Io(error)
    }
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Acquire))
}

#[cfg(test)]
mod security_descriptor_sddl_tests {
    use super::normalize_windows_security_descriptor_sddl;

    #[test]
    fn comparison_ignores_only_dacl_inheritance_controls() {
        assert_eq!(
            normalize_windows_security_descriptor_sddl("O:SYG:SYD:PAI(A;CI;FA;;;SY)(A;ID;GR;;;BU)"),
            "O:SYG:SYD:(A;CI;FA;;;SY)(A;;GR;;;BU)"
        );
        assert_eq!(
            normalize_windows_security_descriptor_sddl("O:SYG:SYD:AIP(A;;GR;;;BU)"),
            "O:SYG:SYD:(A;;GR;;;BU)"
        );
        assert_eq!(
            normalize_windows_security_descriptor_sddl("O:SYG:SYD:NO_ACCESS_CONTROL"),
            "O:SYG:SYD:NO_ACCESS_CONTROL"
        );
        assert_eq!(
            normalize_windows_security_descriptor_sddl(
                "O:SYG:SYD:(A;IDCI;GR;;;BU;(Member_of(S-1-5-32-545)))"
            ),
            "O:SYG:SYD:(A;CI;GR;;;BU;(Member_of(S-1-5-32-545)))"
        );
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{fs, io::Write};

    fn fixture(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-staging-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        root
    }

    fn apply_test_audit_sacl(path: &std::path::Path) {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::{GetLastError, LocalFree},
            Security::Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            Security::{SetFileSecurityW, SACL_SECURITY_INFORMATION},
        };

        let _privilege = WindowsSaclPrivilege::enable().expect("SeSecurityPrivilege");
        let sddl = "S:(AU;SAFA;FA;;;WD)\0".encode_utf16().collect::<Vec<_>>();
        let mut descriptor = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            },
            0,
            "SACL test descriptor conversion failed: {}",
            unsafe { GetLastError() }
        );
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let set_result =
            unsafe { SetFileSecurityW(wide.as_ptr(), SACL_SECURITY_INFORMATION, descriptor) };
        let error = if set_result == 0 {
            Some(unsafe { GetLastError() })
        } else {
            None
        };
        unsafe {
            LocalFree(descriptor);
        }
        assert!(error.is_none(), "setting test SACL failed: {error:?}");
    }

    #[test]
    fn named_stream_guard_accepts_a_normal_directory() {
        let root = fixture("directory-stream-check");
        let result = ensure_windows_no_named_streams(&root);
        assert!(
            result.is_ok(),
            "normal directories must be stream checked: {result:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sacl_inspection_unavailable_fails_closed_without_touching_source() {
        let root = fixture("sacl-unavailable");
        let source = root.join("source.bin");
        fs::write(&source, b"SACL inspection must fail closed").expect("source fixture");
        super::super::atomic_move::test_faults::set_fault(Some(
            super::super::atomic_move::test_faults::AtomicFaultPoint::
                WindowsSaclInspectionUnavailable,
        ));
        let result = ensure_windows_transfer_security_supported(&source, None);
        super::super::atomic_move::test_faults::set_fault(None);
        assert!(matches!(
            result,
            Err(AtomicMoveError::WindowsSaclInspectionUnavailable)
        ));
        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"SACL inspection must fail closed"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_sacl_is_rejected_and_a_claimed_directory_is_rolled_back() {
        let root = fixture("nested-sacl");
        let source = root.join("source");
        fs::create_dir_all(source.join("nested")).expect("nested fixture");
        let nested_file = source.join("nested").join("audited.txt");
        fs::write(&nested_file, b"audited source bytes").expect("nested source");
        ensure_windows_transfer_security_supported(&source, None)
            .expect("ordinary no-SACL directory is supported");
        apply_test_audit_sacl(&nested_file);
        assert!(matches!(
            ensure_windows_transfer_security_supported(&source, None),
            Err(AtomicMoveError::WindowsSaclTransferUnsupported)
        ));

        let expected = identity::capture_namespace_identity_only(&source, None)
            .expect("source namespace identity");
        let claim_path = source_claim::planned_claim_path(&source, "nested-sacl-test")
            .expect("planned claim path");
        let mut claim = source_claim::claim_source_at(
            &source,
            &expected,
            &claim_path,
            "nested-sacl-test",
            None,
        )
        .expect("claim source directory");
        let target_parent = VerifiedDirectory::open_existing(&root).expect("target parent");
        let result = copy_commit_claim(
            &mut claim,
            target_parent,
            std::ffi::OsStr::new("target"),
            None,
            None,
        );
        assert!(matches!(
            result,
            Err(AtomicMoveError::WindowsSaclTransferUnsupported)
        ));
        assert!(source.is_dir(), "source namespace must be restored");
        assert!(
            !claim_path.exists(),
            "private claim must be removed by rollback"
        );
        assert!(!root.join("target").exists(), "no target may be published");
        assert_eq!(
            fs::read(&nested_file).expect("source content"),
            b"audited source bytes"
        );
        drop(claim);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn staging_handle_commits_relative_to_verified_parent_without_reopen() {
        let root = fixture("commit");
        let parent = VerifiedDirectory::open_existing(&root).expect("parent");
        let mut staging =
            StagingFile::create(&parent, std::ffi::OsStr::new("target.txt")).expect("staging");
        staging.handle.write_all(b"bound staging").expect("write");
        let expected = identity::capture_identity_from_handle(&staging.handle, &staging.path, None)
            .expect("identity");
        staging.verified_identity = Some(expected);
        staging
            .commit_noreplace(std::ffi::OsStr::new("target.txt"))
            .expect("commit");
        staging.verify_committed(None).expect("verify");
        assert_eq!(
            fs::read(root.join("target.txt")).expect("target"),
            b"bound staging"
        );
        drop(staging);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_commit_reports_each_persistable_filesystem_phase_in_order() {
        let root = fixture("phases");
        let source_parent = root.join("source");
        let target_parent_path = root.join("target");
        fs::create_dir(&source_parent).expect("source parent");
        fs::create_dir(&target_parent_path).expect("target parent");
        let source = source_parent.join("source.txt");
        let target = target_parent_path.join("target.txt");
        fs::write(&source, b"phase persistence").expect("source");
        let expected = identity::capture_identity(&source, None).expect("source identity");
        let claim_path =
            source_claim::planned_claim_path(&source, "phase-test").expect("claim path");
        let mut claim =
            source_claim::claim_source_at(&source, &expected, &claim_path, "phase-test", None)
                .expect("source claim");
        let target_parent =
            VerifiedDirectory::open_existing(&target_parent_path).expect("target parent handle");
        let mut phases = Vec::new();
        let mut observer = |event: crate::fs_safety::FileOperationEvent| {
            if let crate::fs_safety::FileOperationEvent::Phase(phase) = event {
                phases.push(phase);
            }
            Ok(())
        };

        copy_commit_claim(
            &mut claim,
            target_parent,
            target.file_name().expect("target name"),
            None,
            Some(&mut observer),
        )
        .expect("copy commit");

        assert_eq!(
            phases,
            [
                "copying",
                "target_committed",
                "source_cleanup_pending",
                "completed"
            ]
        );
        assert_eq!(fs::read(&target).expect("target"), b"phase persistence");
        assert!(!source.exists());
        drop(claim);
        assert!(!claim_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn staged_directory_move_preserves_nested_empty_tree_and_cleans_claim() {
        let root = fixture("directory-move");
        let source_parent = root.join("source");
        let target_parent_path = root.join("target");
        fs::create_dir(&source_parent).expect("source parent");
        fs::create_dir(&target_parent_path).expect("target parent");
        let source = source_parent.join("资料");
        fs::create_dir_all(source.join("嵌套").join("empty")).expect("nested empty tree");
        fs::write(source.join("根.txt"), b"root file bytes").expect("root file");
        fs::write(source.join("嵌套").join("内容.bin"), b"nested file bytes").expect("nested file");
        let claim_path =
            source_claim::planned_claim_path(&source, "directory-move-test").expect("claim path");
        let expected = identity::capture_namespace_identity_only(&source, None)
            .expect("directory namespace identity");
        let mut claim = source_claim::claim_source_at(
            &source,
            &expected,
            &claim_path,
            "directory-move-test",
            None,
        )
        .expect("directory source claim");
        let target_parent =
            VerifiedDirectory::open_existing(&target_parent_path).expect("target parent handle");
        let mut phases = Vec::new();
        let mut progress_events = Vec::new();
        let mut observer = |event: crate::fs_safety::FileOperationEvent| {
            match event {
                crate::fs_safety::FileOperationEvent::Phase(phase) => phases.push(phase),
                crate::fs_safety::FileOperationEvent::Progress(progress) => {
                    progress_events.push(progress)
                }
            }
            Ok(())
        };

        copy_commit_claim(
            &mut claim,
            target_parent,
            std::ffi::OsStr::new("资料"),
            None,
            Some(&mut observer),
        )
        .expect("staged directory move");

        let target = target_parent_path.join("资料");
        assert_eq!(
            fs::read(target.join("根.txt")).expect("root target"),
            b"root file bytes"
        );
        assert_eq!(
            fs::read(target.join("嵌套").join("内容.bin")).expect("nested target"),
            b"nested file bytes"
        );
        assert!(target.join("嵌套").join("empty").is_dir());
        assert!(!source.exists());
        drop(claim);
        assert!(!claim_path.exists());
        assert_eq!(
            phases,
            [
                "copying",
                "target_committed",
                "source_cleanup_pending",
                "completed"
            ]
        );
        let copied = progress_events
            .iter()
            .filter(|progress| progress.stage == "copying")
            .last()
            .expect("final copying progress");
        assert_eq!(copied.processed_items, 4);
        assert_eq!(
            copied.processed_bytes,
            b"root file bytes".len() as u64 + b"nested file bytes".len() as u64
        );
        assert_eq!(copied.total_items, None);
        assert_eq!(copied.total_bytes, None);
        let retired = progress_events
            .iter()
            .filter(|progress| progress.stage == "source_retirement")
            .last()
            .expect("final source-retirement progress");
        assert_eq!(retired.processed_items, 4);
        assert_eq!(retired.total_items, Some(4));
        assert_eq!(retired.processed_bytes, copied.processed_bytes);
        assert_eq!(retired.total_bytes, Some(copied.processed_bytes));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn staged_directory_move_cancel_before_publish_restores_source_and_removes_stage() {
        let root = fixture("directory-cancel");
        let source_parent = root.join("source");
        let target_parent_path = root.join("target");
        fs::create_dir(&source_parent).expect("source parent");
        fs::create_dir(&target_parent_path).expect("target parent");
        let source = source_parent.join("tree");
        fs::create_dir(&source).expect("source directory");
        fs::write(source.join("one.txt"), b"one").expect("first file");
        fs::write(source.join("two.txt"), b"two").expect("second file");
        let claim_path =
            source_claim::planned_claim_path(&source, "directory-cancel-test").expect("claim path");
        let expected = identity::capture_namespace_identity_only(&source, None)
            .expect("directory namespace identity");
        let mut claim = source_claim::claim_source_at(
            &source,
            &expected,
            &claim_path,
            "directory-cancel-test",
            None,
        )
        .expect("directory source claim");
        let target_parent =
            VerifiedDirectory::open_existing(&target_parent_path).expect("target parent handle");
        let _serial = super::super::atomic_move::test_faults::lock();
        super::super::atomic_move::test_faults::set_fault(Some(
            super::super::atomic_move::test_faults::AtomicFaultPoint::CancelDirectoryCopyAfterEntry,
        ));
        let result = copy_commit_claim(
            &mut claim,
            target_parent,
            std::ffi::OsStr::new("tree"),
            None,
            None,
        );
        super::super::atomic_move::test_faults::set_fault(None);

        assert!(matches!(result, Err(AtomicMoveError::Cancelled)));
        assert_eq!(
            fs::read(source.join("one.txt")).expect("source restored"),
            b"one"
        );
        assert!(!target_parent_path.join("tree").exists());
        assert!(!claim_path.exists());
        drop(claim);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn staged_directory_move_source_cleanup_fault_keeps_both_recoverable_copies() {
        let root = fixture("directory-cleanup-fault");
        let source_parent = root.join("source");
        let target_parent_path = root.join("target");
        fs::create_dir(&source_parent).expect("source parent");
        fs::create_dir(&target_parent_path).expect("target parent");
        let source = source_parent.join("tree");
        fs::create_dir(&source).expect("source directory");
        fs::write(source.join("file.txt"), b"recoverable bytes").expect("source file");
        let claim_path = source_claim::planned_claim_path(&source, "directory-cleanup-test")
            .expect("claim path");
        let expected = identity::capture_namespace_identity_only(&source, None)
            .expect("directory namespace identity");
        let mut claim = source_claim::claim_source_at(
            &source,
            &expected,
            &claim_path,
            "directory-cleanup-test",
            None,
        )
        .expect("directory source claim");
        let target_parent =
            VerifiedDirectory::open_existing(&target_parent_path).expect("target parent handle");
        let _serial = super::super::atomic_move::test_faults::lock();
        super::super::atomic_move::test_faults::set_fault(Some(
            super::super::atomic_move::test_faults::AtomicFaultPoint::SourceCleanup,
        ));
        let result = copy_commit_claim(
            &mut claim,
            target_parent,
            std::ffi::OsStr::new("tree"),
            None,
            None,
        );
        super::super::atomic_move::test_faults::set_fault(None);

        assert!(matches!(
            result,
            Err(AtomicMoveError::TargetCommittedSourceCleanupPending)
        ));
        assert_eq!(
            fs::read(target_parent_path.join("tree").join("file.txt")).expect("target"),
            b"recoverable bytes"
        );
        assert!(claim_path.exists());
        assert!(!source.exists());
        drop(claim);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    fn mutate_claimed_directory_after_target_commit(
        point: source_claim::ClaimTestPoint,
        _source: &std::path::Path,
        claim: &std::path::Path,
    ) {
        if point == source_claim::ClaimTestPoint::AfterTargetCommitBeforeSourceCleanup {
            fs::write(
                claim.join("nested").join("file.txt"),
                b"changed after publish",
            )
            .expect("mutate claimed source after target commit");
        }
    }

    #[test]
    #[cfg(windows)]
    fn source_tree_mutation_after_target_commit_preserves_claim_and_target() {
        let _serial = source_claim::lock_claim_test_hooks();
        let root = fixture("directory-source-mutation-after-commit");
        let source_parent = root.join("source");
        let target_parent_path = root.join("target");
        fs::create_dir(&source_parent).expect("source parent");
        fs::create_dir(&target_parent_path).expect("target parent");
        let source = source_parent.join("tree");
        fs::create_dir_all(source.join("nested")).expect("nested source");
        fs::write(source.join("nested").join("file.txt"), b"stable source").expect("source file");
        let target = target_parent_path.join("tree");
        let claim_path =
            source_claim::planned_claim_path(&source, "directory-mutation-after-commit")
                .expect("claim path");
        let expected = identity::capture_namespace_identity_only(&source, None)
            .expect("directory namespace identity");
        let mut claim = source_claim::claim_source_at(
            &source,
            &expected,
            &claim_path,
            "directory-mutation-after-commit",
            None,
        )
        .expect("source claim");
        let target_parent =
            VerifiedDirectory::open_existing(&target_parent_path).expect("target parent handle");

        source_claim::set_claim_test_hook(Some(mutate_claimed_directory_after_target_commit));
        let result = copy_commit_claim(
            &mut claim,
            target_parent,
            std::ffi::OsStr::new("tree"),
            None,
            None,
        );
        source_claim::set_claim_test_hook(None);

        assert!(
            matches!(
                result,
                Err(AtomicMoveError::TargetCommittedSourceDeleteFailed(_))
            ),
            "{result:?}"
        );
        assert_eq!(
            fs::read(target.join("nested").join("file.txt")).expect("committed target"),
            b"stable source"
        );
        assert_eq!(
            fs::read(claim_path.join("nested").join("file.txt")).expect("retained claim"),
            b"changed after publish"
        );
        assert!(!source.exists());
        drop(claim);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_cross_volume_directory_move_runs_when_an_isolated_volume_is_available() {
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

        let source_root = fixture("native-cross-volume-source");
        let source_identity = identity::capture_namespace_identity_only(&source_root, None)
            .expect("source volume identity");
        let drive_mask = unsafe { GetLogicalDrives() };
        let Some(target_fixture) = (b'A'..=b'Z').find_map(|letter| {
            if drive_mask & (1 << (letter - b'A')) == 0 {
                return None;
            }
            let drive_root = PathBuf::from(format!("{}:\\", letter as char));
            let wide = source_claim::windows_wide_path(&drive_root);
            // DRIVE_REMOVABLE and DRIVE_FIXED are the only filesystem classes
            // this destructive fixture accepts; network and optical targets
            // remain outside the automated native volume test.
            if !matches!(unsafe { GetDriveTypeW(wide.as_ptr()) }, 2 | 3) {
                return None;
            }
            let volume = identity::capture_namespace_identity_only(&drive_root, None).ok()?;
            if volume.platform_volume_id == source_identity.platform_volume_id {
                return None;
            }
            let fixture = drive_root.join(format!("Fylura-QA-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&fixture).ok().map(|_| fixture)
        }) else {
            eprintln!("UNVERIFIED: no writable independent local volume was available");
            let _ = fs::remove_dir_all(source_root);
            return;
        };

        let source = source_root.join("source tree");
        fs::create_dir_all(source.join("嵌套").join("empty")).expect("nested empty directory");
        fs::write(source.join("根.txt"), b"cross-volume source content").expect("root file");
        fs::write(source.join("嵌套").join("內容.bin"), b"nested content").expect("nested file");
        let target_parent_path = target_fixture.join("destination");
        fs::create_dir(&target_parent_path).expect("target parent");
        let target_parent =
            VerifiedDirectory::open_existing(&target_parent_path).expect("verified target");
        let target = target_parent_path.join("source tree");
        let claim_path = source_claim::planned_claim_path(&source, "native-cross-volume")
            .expect("source claim path");
        let expected = identity::capture_namespace_identity_only(&source, None)
            .expect("directory namespace identity");
        let mut phases = Vec::new();
        let mut observer = |event: crate::fs_safety::FileOperationEvent| {
            if let crate::fs_safety::FileOperationEvent::Phase(phase) = event {
                phases.push(phase);
            }
            Ok(())
        };

        crate::fs_safety::atomic_move::atomic_move_with_verified_destination_for_transfer(
            &source,
            &target,
            Some(&expected),
            Some(&claim_path),
            None,
            Some(&mut observer),
            super::super::atomic_move::AtomicMoveOperation::Move,
            None,
            Some(target_parent.identity()),
        )
        .expect("native cross-volume directory move");

        assert!(!source.exists());
        assert!(!claim_path.exists());
        assert_eq!(
            fs::read(target.join("根.txt")).expect("root file"),
            b"cross-volume source content"
        );
        assert_eq!(
            fs::read(target.join("嵌套").join("內容.bin")).expect("nested file"),
            b"nested content"
        );
        assert!(target.join("嵌套").join("empty").is_dir());
        assert!(phases.contains(&"source_cleanup_pending".to_string()));
        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(target_fixture);
    }

    #[test]
    fn staging_path_replacement_is_detected_and_replacement_is_never_committed() {
        let root = fixture("replacement");
        let parent = VerifiedDirectory::open_existing(&root).expect("parent");
        let mut staging =
            StagingFile::create(&parent, std::ffi::OsStr::new("target.txt")).expect("staging");
        staging
            .handle
            .write_all(b"original staging")
            .expect("write");
        staging.handle.sync_all().expect("sync");
        let expected = identity::capture_identity_from_handle(&staging.handle, &staging.path, None)
            .expect("identity");
        staging.verified_identity = Some(expected);
        let displaced = root.join("displaced-stage");
        fs::rename(&staging.path, &displaced).expect("displace staging");
        fs::write(&staging.path, b"replacement staging").expect("replacement");

        assert!(matches!(
            staging.verify_path_binding(None),
            Err(AtomicMoveError::StagingIdentityChanged)
        ));
        assert!(!root.join("target.txt").exists());
        assert_eq!(
            fs::read(&staging.path).expect("replacement"),
            b"replacement staging"
        );
        drop(staging);
        assert!(!displaced.exists());
        let _ = fs::remove_dir_all(root);
    }

    fn run_post_commit_fault(
        label: &str,
        point: super::super::atomic_move::test_faults::AtomicFaultPoint,
    ) -> (Result<(), AtomicMoveError>, PathBuf, PathBuf, PathBuf) {
        let root = fixture(label);
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, b"post-commit payload").expect("source");
        let claim = source_claim::planned_claim_path(&source, label).expect("claim path");
        super::super::atomic_move::test_faults::set_fault(Some(point));
        let result = copy_commit_move_with_claim_path(&source, &target, None, Some(&claim), None);
        super::super::atomic_move::test_faults::set_fault(None);
        (result, root, target, claim)
    }

    #[test]
    fn durability_fault_after_commit_never_reports_rollback() {
        let _serial = super::super::atomic_move::test_faults::lock();
        let (result, root, target, claim) = run_post_commit_fault(
            "durability-fault",
            super::super::atomic_move::test_faults::AtomicFaultPoint::TargetDurability,
        );
        assert!(matches!(
            result,
            Err(AtomicMoveError::TargetCommittedDurabilityUnknown)
        ));
        assert_eq!(
            fs::read(target).expect("committed target"),
            b"post-commit payload"
        );
        assert!(claim.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn identity_fault_after_commit_never_reports_rollback() {
        let _serial = super::super::atomic_move::test_faults::lock();
        let (result, root, target, claim) = run_post_commit_fault(
            "identity-fault",
            super::super::atomic_move::test_faults::AtomicFaultPoint::TargetIdentity,
        );
        assert!(matches!(
            result,
            Err(AtomicMoveError::TargetCommittedIdentityMismatch)
        ));
        assert_eq!(
            fs::read(target).expect("committed target"),
            b"post-commit payload"
        );
        assert!(claim.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_fault_after_commit_is_source_cleanup_pending() {
        let _serial = super::super::atomic_move::test_faults::lock();
        let (result, root, target, claim) = run_post_commit_fault(
            "cleanup-fault",
            super::super::atomic_move::test_faults::AtomicFaultPoint::SourceCleanup,
        );
        assert!(matches!(
            result,
            Err(AtomicMoveError::TargetCommittedSourceCleanupPending)
        ));
        assert_eq!(
            fs::read(target).expect("committed target"),
            b"post-commit payload"
        );
        assert!(claim.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    fn mutate_claimed_file_dacl_after_target_commit(
        point: source_claim::ClaimTestPoint,
        _source: &std::path::Path,
        claim: &std::path::Path,
    ) {
        if point == source_claim::ClaimTestPoint::AfterTargetCommitBeforeSourceCleanup {
            let result = std::process::Command::new("icacls")
                .arg(claim.join("nested").join("file.txt"))
                .args(["/grant", "*S-1-5-32-545:(W)"])
                .output()
                .expect("change the claimed source DACL after publication");
            assert!(
                result.status.success(),
                "could not mutate the claimed source DACL: {}",
                String::from_utf8_lossy(&result.stderr).trim()
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn source_dacl_mutation_after_target_commit_preserves_claim_and_target() {
        let _serial = source_claim::lock_claim_test_hooks();
        let root = fixture("directory-dacl-mutation-after-commit");
        let source_parent = root.join("source");
        let target_parent_path = root.join("target");
        fs::create_dir(&source_parent).expect("source parent");
        fs::create_dir(&target_parent_path).expect("target parent");
        let source = source_parent.join("tree");
        fs::create_dir_all(source.join("nested")).expect("nested source");
        fs::write(source.join("nested").join("file.txt"), b"stable source").expect("source file");
        let target = target_parent_path.join("tree");
        let claim_path = source_claim::planned_claim_path(&source, "directory-dacl-after-commit")
            .expect("claim path");
        let expected = identity::capture_namespace_identity_only(&source, None)
            .expect("directory namespace identity");
        let mut claim = source_claim::claim_source_at(
            &source,
            &expected,
            &claim_path,
            "directory-dacl-after-commit",
            None,
        )
        .expect("source claim");
        let target_parent =
            VerifiedDirectory::open_existing(&target_parent_path).expect("target parent handle");

        source_claim::set_claim_test_hook(Some(mutate_claimed_file_dacl_after_target_commit));
        let result = copy_commit_claim(
            &mut claim,
            target_parent,
            std::ffi::OsStr::new("tree"),
            None,
            None,
        );
        source_claim::set_claim_test_hook(None);

        assert!(
            matches!(
                result,
                Err(AtomicMoveError::TargetCommittedSourceDeleteFailed(_))
            ),
            "{result:?}"
        );
        assert_eq!(
            fs::read(target.join("nested").join("file.txt")).expect("committed target"),
            b"stable source"
        );
        assert_eq!(
            fs::read(claim_path.join("nested").join("file.txt")).expect("retained claim"),
            b"stable source"
        );
        assert!(!source.exists());
        drop(claim);
        let _ = fs::remove_dir_all(root);
    }
}
