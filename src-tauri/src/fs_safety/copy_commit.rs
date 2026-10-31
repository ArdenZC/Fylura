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
    if claim.kind() != ClaimedEntryKind::File {
        #[cfg(windows)]
        if claim.kind() == ClaimedEntryKind::Directory && !retire_source {
            return copy_directory_claim_preserving_source(
                claim,
                target_parent,
                target_name,
                cancel,
                observer,
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
            observer("copying")?;
        }
        copy_file_from_claim(claim, &mut staging.handle, cancel)?;
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
        staging.commit_noreplace(target_name)?;
        target_committed = true;
        if let Some(observer) = observer.as_deref_mut() {
            observer("target_committed")?;
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
        if let Some(observer) = observer.as_deref_mut() {
            observer("source_cleanup_pending")?;
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
            claim.delete_claim().map_err(|error| {
                AtomicMoveError::TargetCommittedSourceDeleteFailed(error.to_string())
            })?;
            claim.sync_current_parent().map_err(|error| {
                let _ = error;
                AtomicMoveError::TargetCommittedSourceCleanupPending
            })?;
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
            observer("completed")?;
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

#[cfg(windows)]
fn copy_directory_claim_preserving_source(
    claim: &mut SourceClaim,
    target_parent: VerifiedDirectory,
    target_name: &std::ffi::OsStr,
    cancel: Option<&AtomicBool>,
    mut observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
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
        if let Some(observer) = observer.as_deref_mut() {
            observer("copying")?;
        }
        let source_before = directory_content_digest(claim.current_path(), cancel)?;
        let mut entry_count = 0usize;
        copy_directory_contents(
            claim.current_path(),
            &staging.path,
            0,
            &mut entry_count,
            cancel,
        )?;
        let source_after = directory_content_digest(claim.current_path(), cancel)?;
        let staged = directory_content_digest(&staging.path, cancel)?;
        if source_before != source_after || source_before != staged {
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
            observer("target_committed")?;
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
        let target_digest =
            directory_content_digest(&target_parent.path().join(target_name), cancel)
                .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
        if target_digest != source_before {
            return Err(AtomicMoveError::TargetCommittedIdentityMismatch);
        }
        claim
            .verify_current_identity(cancel)
            .map_err(|_| AtomicMoveError::TargetCommittedIdentityMismatch)?;
        claim
            .sync()
            .map_err(|_| AtomicMoveError::TargetCommittedDurabilityUnknown)?;
        claim
            .rollback_to_original()
            .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
        claim
            .sync_original_parent()
            .map_err(|_| AtomicMoveError::TargetCommittedSourceCleanupPending)?;
        if let Some(observer) = observer.as_deref_mut() {
            observer("completed")?;
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
) -> Result<(), AtomicMoveError> {
    const MAX_DEPTH: usize = 64;
    const MAX_ENTRIES: usize = 5000;
    if depth >= MAX_DEPTH {
        return Err(AtomicMoveError::UnsafePath);
    }
    let source_directory = VerifiedDirectory::open_existing(source).map_err(map_directory_error)?;
    let mut entries = fs::read_dir(source)
        .map_err(AtomicMoveError::Io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AtomicMoveError::Io)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        *entry_count += 1;
        if *entry_count > MAX_ENTRIES {
            return Err(AtomicMoveError::UnsafePath);
        }
        let name = entry.file_name();
        let source_child = entry.path();
        let target_child = target.join(&name);
        let metadata = fs::symlink_metadata(&source_child).map_err(AtomicMoveError::Io)?;
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            return Err(AtomicMoveError::ReparsePoint);
        }
        if metadata.is_dir() {
            fs::create_dir(&target_child).map_err(map_create_error)?;
            copy_directory_contents(&source_child, &target_child, depth + 1, entry_count, cancel)?;
            VerifiedDirectory::open_existing(&target_child)
                .map_err(map_directory_error)?
                .sync()
                .map_err(AtomicMoveError::Io)?;
        } else if metadata.is_file() {
            copy_regular_file(&source_child, &target_child, cancel)?;
        } else {
            return Err(AtomicMoveError::UnsafePath);
        }
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
) -> Result<(), AtomicMoveError> {
    use std::io::{Seek, SeekFrom};
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    if is_cancelled(cancel) {
        return Err(AtomicMoveError::Cancelled);
    }
    let mut source_options = OpenOptions::new();
    source_options
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let mut reader = source_options.open(source).map_err(AtomicMoveError::Io)?;
    let metadata = reader.metadata().map_err(AtomicMoveError::Io)?;
    if !metadata.is_file() || metadata_is_reparse_point(&metadata) {
        return Err(AtomicMoveError::ReparsePoint);
    }
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
    }
    writer.sync_all().map_err(AtomicMoveError::Io)?;
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
    Ok(())
}

#[cfg(windows)]
fn directory_content_digest(
    root: &std::path::Path,
    cancel: Option<&AtomicBool>,
) -> Result<String, AtomicMoveError> {
    fn visit(
        root: &std::path::Path,
        current: &std::path::Path,
        depth: usize,
        entry_count: &mut usize,
        hasher: &mut blake3::Hasher,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), AtomicMoveError> {
        use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
        const MAX_DEPTH: usize = 64;
        const MAX_ENTRIES: usize = 5000;
        if is_cancelled(cancel) {
            return Err(AtomicMoveError::Cancelled);
        }
        if depth >= MAX_DEPTH {
            return Err(AtomicMoveError::UnsafePath);
        }
        let verified = VerifiedDirectory::open_existing(current).map_err(map_directory_error)?;
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
        let mut entries = fs::read_dir(current)
            .map_err(AtomicMoveError::Io)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AtomicMoveError::Io)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            *entry_count += 1;
            if *entry_count > MAX_ENTRIES {
                return Err(AtomicMoveError::UnsafePath);
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(AtomicMoveError::Io)?;
            if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
                return Err(AtomicMoveError::ReparsePoint);
            }
            if metadata.is_dir() {
                visit(root, &path, depth + 1, entry_count, hasher, cancel)?;
            } else if metadata.is_file() {
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
    visit(root, root, 0, &mut entry_count, &mut hasher, cancel)?;
    Ok(hasher.finalize().to_hex().to_string())
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
        let mut observer = |phase: &str| {
            phases.push(phase.to_string());
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
}
