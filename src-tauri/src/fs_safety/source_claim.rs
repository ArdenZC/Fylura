use super::{
    identity::{self, ExpectedFileIdentity, IdentityError},
    platform_support,
    verified_directory::VerifiedDirectory,
};
#[cfg(target_os = "macos")]
use std::io::Write;
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimedEntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, Error)]
pub enum SourceClaimError {
    #[error("unsupported_platform_linux")]
    UnsupportedPlatformLinux,
    #[error("macos_file_mutation_source_binding_unsupported")]
    MacosFileMutationSourceBindingUnsupported,
    #[error("{0}")]
    MacMutationNotSupported(&'static str),
    #[error("source_missing")]
    SourceMissing,
    #[error("source_identity_changed")]
    SourceIdentityChanged,
    #[error("source_claim_failed: {0}")]
    ClaimFailed(String),
    #[error("source_claim_mismatch")]
    ClaimMismatch,
    #[error("source_claim_rollback_failed: {0}")]
    ClaimRollbackFailed(String),
    #[error("source_claim_recovery_required: {0}")]
    RecoveryRequired(String),
    #[error("mac_claim_namespace_rebound")]
    MacClaimNamespaceRebound,
    #[error("mac_claim_identity_mismatch")]
    MacClaimIdentityMismatch,
    #[error("mac_claim_path_missing")]
    MacClaimPathMissing,
    #[error("mac_claim_path_unreadable")]
    MacClaimPathUnreadable,
    #[error("target_exists")]
    TargetExists,
    #[error("cross_device")]
    CrossDevice,
    #[error("atomic_source_binding_unsupported")]
    AtomicSourceBindingUnsupported,
    #[error("reparse_point")]
    ReparsePoint,
    #[error("unsupported_file_type")]
    UnsupportedFileType,
    #[error("cancelled")]
    Cancelled,
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

pub struct SourceClaim {
    original_path: PathBuf,
    current_path: PathBuf,
    claim_path: PathBuf,
    original_name: OsString,
    current_name: OsString,
    original_parent: VerifiedDirectory,
    current_parent: VerifiedDirectory,
    expected_identity: ExpectedFileIdentity,
    actual_identity: ExpectedFileIdentity,
    kind: ClaimedEntryKind,
    handle: Option<File>,
    #[cfg(target_os = "macos")]
    physical_identity: crate::platform::macos::identity::MacPhysicalIdentity,
    deleted: bool,
}

impl std::fmt::Debug for SourceClaim {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SourceClaim")
            .field("original_path", &self.original_path)
            .field("current_path", &self.current_path)
            .field("claim_path", &self.claim_path)
            .field("expected_identity", &self.expected_identity)
            .field("actual_identity", &self.actual_identity)
            .field("kind", &self.kind)
            .field("deleted", &self.deleted)
            .finish_non_exhaustive()
    }
}

impl SourceClaim {
    pub fn original_path(&self) -> &Path {
        &self.original_path
    }

    pub fn current_path(&self) -> &Path {
        &self.current_path
    }

    pub fn claim_path(&self) -> &Path {
        &self.claim_path
    }

    pub fn expected_identity(&self) -> &ExpectedFileIdentity {
        &self.expected_identity
    }

    pub fn actual_identity(&self) -> &ExpectedFileIdentity {
        &self.actual_identity
    }

    pub fn kind(&self) -> ClaimedEntryKind {
        self.kind
    }

    pub fn original_volume_id(&self) -> &str {
        &self.original_parent.identity().volume_id
    }

    pub fn verify_current_identity(
        &self,
        cancel: Option<&AtomicBool>,
    ) -> Result<ExpectedFileIdentity, SourceClaimError> {
        let actual = self.current_identity(cancel)?;
        if !identity::identity_matches(&self.expected_identity, &actual) {
            #[cfg(target_os = "macos")]
            {
                return Err(SourceClaimError::MacClaimIdentityMismatch);
            }
            #[cfg(not(target_os = "macos"))]
            return Err(SourceClaimError::SourceIdentityChanged);
        }
        Ok(actual)
    }

    pub fn open_read(&self) -> Result<File, SourceClaimError> {
        let mut handle = self
            .handle
            .as_ref()
            .ok_or(SourceClaimError::UnsupportedFileType)?
            .try_clone()
            .map_err(SourceClaimError::Io)?;
        handle
            .seek(SeekFrom::Start(0))
            .map_err(SourceClaimError::Io)?;
        Ok(handle)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn clone_handle(&self) -> Result<Option<File>, SourceClaimError> {
        self.handle
            .as_ref()
            .map(|handle| handle.try_clone().map_err(SourceClaimError::Io))
            .transpose()
    }

    pub fn sync(&self) -> Result<(), SourceClaimError> {
        // The claim handle is opened without write access.  A rename/copy
        // does not mutate the source bytes, so Windows cannot require a
        // write-capable source handle merely to flush unchanged content;
        // parent-directory handles provide the namespace durability barrier.
        #[cfg(windows)]
        {
            Ok(())
        }
        #[cfg(not(windows))]
        {
            self.handle.as_ref().map_or(Ok(()), |handle| {
                handle.sync_all().map_err(SourceClaimError::Io)
            })
        }
    }

    pub fn sync_current_parent(&self) -> Result<(), SourceClaimError> {
        self.current_parent.sync().map_err(SourceClaimError::Io)
    }

    pub fn sync_original_parent(&self) -> Result<(), SourceClaimError> {
        self.original_parent.sync().map_err(SourceClaimError::Io)
    }

    pub fn current_parent_unchanged(&self) -> Result<(), SourceClaimError> {
        self.current_parent
            .ensure_unchanged()
            .map_err(map_directory_error)
    }

    /// Revalidates the current namespace leaf against the retained claim
    /// proof.  On macOS this is deliberately `fstatat` relative to the
    /// retained parent directory; a descriptor-only stat cannot detect that
    /// an attacker replaced the pathname after the claim was verified.
    pub fn verify_current_namespace_binding(&self) -> Result<(), SourceClaimError> {
        #[cfg(target_os = "macos")]
        {
            self.ensure_current_namespace_entry_matches_claimed_handle()
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.current_identity(None).map(|_| ())
        }
    }

    pub fn commit_to(
        &mut self,
        target_parent: VerifiedDirectory,
        target_name: &OsStr,
    ) -> Result<PathBuf, SourceClaimError> {
        self.commit_to_with_cancel(target_parent, target_name, None)
    }

    pub fn commit_to_with_cancel(
        &mut self,
        target_parent: VerifiedDirectory,
        target_name: &OsStr,
        cancel: Option<&AtomicBool>,
    ) -> Result<PathBuf, SourceClaimError> {
        if self.deleted {
            return Err(SourceClaimError::RecoveryRequired(
                "claimed source was already deleted".to_string(),
            ));
        }
        target_parent
            .ensure_unchanged()
            .map_err(map_directory_error)?;
        #[cfg(any(test, feature = "native-qa"))]
        run_claim_test_hook(
            ClaimTestPoint::AfterTargetParentVerifiedBeforeCommit,
            self.current_path(),
            &target_parent.path().join(target_name),
        );
        if is_cancelled(cancel) {
            return Err(SourceClaimError::Cancelled);
        }
        #[cfg(target_os = "macos")]
        self.ensure_current_namespace_entry_matches_claimed_handle()?;
        #[cfg(any(test, feature = "native-qa"))]
        run_claim_test_hook(
            ClaimTestPoint::AfterClaimVerifiedBeforeCommit,
            self.original_path(),
            self.current_path(),
        );
        #[cfg(target_os = "macos")]
        self.ensure_current_namespace_entry_matches_claimed_handle()?;
        #[cfg(target_os = "macos")]
        self.ensure_original_namespace_is_unoccupied()?;
        rename_claim_handle(
            self.handle.as_ref(),
            &self.current_parent,
            &self.current_name,
            &target_parent,
            target_name,
            #[cfg(target_os = "macos")]
            Some(self.physical_identity),
        )?;
        self.current_name = target_name.to_os_string();
        self.current_parent = target_parent;
        self.current_path = self.current_parent.path().join(&self.current_name);
        Ok(self.current_path.clone())
    }

    pub fn rollback_to_original(&mut self) -> Result<(), SourceClaimError> {
        if self.deleted {
            return Err(SourceClaimError::RecoveryRequired(
                "claimed source was already deleted".to_string(),
            ));
        }
        #[cfg(target_os = "macos")]
        self.ensure_current_namespace_entry_matches_claimed_handle()?;
        #[cfg(any(test, feature = "native-qa"))]
        run_claim_test_hook(
            ClaimTestPoint::AfterClaimVerifiedBeforeRollback,
            self.original_path(),
            self.current_path(),
        );
        #[cfg(target_os = "macos")]
        self.ensure_current_namespace_entry_matches_claimed_handle()?;
        self.original_parent
            .ensure_unchanged()
            .map_err(map_directory_error)?;
        rename_claim_handle(
            self.handle.as_ref(),
            &self.current_parent,
            &self.current_name,
            &self.original_parent,
            &self.original_name,
            #[cfg(target_os = "macos")]
            Some(self.physical_identity),
        )?;
        self.current_name = self.original_name.clone();
        self.current_parent = reopen_directory(&self.original_parent)?;
        self.current_path = self.original_path.clone();
        Ok(())
    }

    pub fn delete_claim(&mut self) -> Result<(), SourceClaimError> {
        #[cfg(any(test, feature = "native-qa"))]
        {
            self.delete_claim_with_test_hook(current_claim_test_hook())
        }
        #[cfg(not(any(test, feature = "native-qa")))]
        {
            self.delete_claim_with_callback(|_, _| {})
        }
    }

    #[cfg(any(test, feature = "native-qa"))]
    pub(crate) fn delete_claim_with_test_hook(
        &mut self,
        hook: Option<Hook>,
    ) -> Result<(), SourceClaimError> {
        self.delete_claim_with_callback(|source, claim| {
            run_claim_test_hook_with_override(
                hook,
                ClaimTestPoint::AfterClaimVerifiedBeforeDelete,
                source,
                claim,
            );
        })
    }

    fn delete_claim_with_callback<F>(&mut self, before_delete: F) -> Result<(), SourceClaimError>
    where
        F: FnOnce(&Path, &Path),
    {
        if self.deleted {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        self.ensure_current_namespace_entry_matches_claimed_handle()?;
        before_delete(self.original_path(), self.current_path());
        #[cfg(target_os = "macos")]
        self.ensure_current_namespace_entry_matches_claimed_handle()?;
        self.current_parent
            .ensure_unchanged()
            .map_err(map_directory_error)?;
        delete_claim_handle(
            self.handle.as_ref(),
            &self.current_parent,
            &self.current_name,
            self.kind,
            #[cfg(target_os = "macos")]
            self.physical_identity,
        )?;
        self.deleted = true;
        Ok(())
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    /// Removes a claimed namespace object without ever touching its original
    /// user pathname.  macOS directories and packages are retired only after
    /// destination verification, and children are traversed without following
    /// symlinks.
    #[cfg(target_os = "macos")]
    pub fn delete_claim_tree(&mut self) -> Result<(), SourceClaimError> {
        #[cfg(any(test, feature = "native-qa"))]
        {
            self.delete_claim_tree_with_test_hook(current_claim_test_hook())
        }
        #[cfg(not(any(test, feature = "native-qa")))]
        {
            self.delete_claim_tree_with_callback(|_, _| {})
        }
    }

    #[cfg(all(target_os = "macos", any(test, feature = "native-qa")))]
    pub(crate) fn delete_claim_tree_with_test_hook(
        &mut self,
        hook: Option<Hook>,
    ) -> Result<(), SourceClaimError> {
        self.delete_claim_tree_with_callback(|source, claim| {
            run_claim_test_hook_with_override(
                hook,
                ClaimTestPoint::AfterClaimVerifiedBeforeDelete,
                source,
                claim,
            );
        })
    }

    #[cfg(target_os = "macos")]
    fn delete_claim_tree_with_callback<F>(
        &mut self,
        before_delete: F,
    ) -> Result<(), SourceClaimError>
    where
        F: FnOnce(&Path, &Path),
    {
        if self.deleted {
            return Ok(());
        }
        self.ensure_current_namespace_entry_matches_claimed_handle()?;
        before_delete(self.original_path(), self.current_path());
        self.ensure_current_namespace_entry_matches_claimed_handle()?;
        self.current_parent
            .ensure_unchanged()
            .map_err(map_directory_error)?;
        remove_namespace_tree(
            self.current_parent.raw_fd(),
            &self.current_name,
            self.physical_identity,
        )?;
        self.deleted = true;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn ensure_current_namespace_entry_matches_claimed_handle(
        &self,
    ) -> Result<(), SourceClaimError> {
        self.current_parent
            .ensure_unchanged()
            .map_err(map_directory_error)?;
        let namespace_identity = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
            self.current_parent.raw_fd(),
            &self.current_name,
        )
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                SourceClaimError::MacClaimPathMissing
            } else {
                SourceClaimError::MacClaimPathUnreadable
            }
        })?;
        let matches = match self.kind {
            ClaimedEntryKind::Directory => self.physical_identity.matches(namespace_identity),
            ClaimedEntryKind::File | ClaimedEntryKind::Symlink => {
                self.physical_identity.matches_strict(namespace_identity)
            }
        };
        if !matches {
            return Err(SourceClaimError::MacClaimNamespaceRebound);
        }
        Ok(())
    }

    fn current_identity(
        &self,
        cancel: Option<&AtomicBool>,
    ) -> Result<ExpectedFileIdentity, SourceClaimError> {
        match self.handle.as_ref() {
            Some(handle) => {
                capture_claim_identity(handle, self.current_path(), &self.expected_identity, cancel)
                    .map_err(map_identity_error)
            }
            None => {
                capture_claim_path_identity(self.current_path(), &self.expected_identity, cancel)
                    .map_err(map_identity_error)
            }
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn verify_content_identity(
        &self,
        cancel: Option<&AtomicBool>,
    ) -> Result<ExpectedFileIdentity, SourceClaimError> {
        let identity = if matches!(self.kind, ClaimedEntryKind::Symlink) {
            identity::capture_namespace_identity(self.current_path(), cancel)
        } else if let Some(handle) = self.handle.as_ref() {
            identity::capture_identity_from_handle(handle, self.current_path(), cancel)
        } else {
            identity::capture_identity(self.current_path(), cancel)
        };
        identity.map_err(map_identity_error)
    }
}

#[cfg(target_os = "macos")]
const MAC_RETIREMENT_ROOT_NAME: &str = ".zen-canvas-retirement";

#[cfg(target_os = "macos")]
fn macos_private_claim_path(parent: &Path, claim_name: &str) -> PathBuf {
    parent
        .join(MAC_RETIREMENT_ROOT_NAME)
        .join(uuid::Uuid::new_v4().to_string())
        .join(claim_name)
}

pub fn planned_claim_path(source: &Path, _operation_id: &str) -> Result<PathBuf, SourceClaimError> {
    // Resolve only the parent. The final component is intentionally kept as a
    // namespace entry so a selected macOS symlink is claimed as the link
    // object rather than being redirected to its target. macOS adds a
    // side-effect-free private retirement/session suffix; the directories are
    // created only at the execution claim boundary.
    let parent = source
        .parent()
        .ok_or(SourceClaimError::SourceMissing)?
        .canonicalize()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                SourceClaimError::SourceMissing
            } else {
                SourceClaimError::Io(error)
            }
        })?;
    let claim_name = format!(".zen-canvas-claim-{}", uuid::Uuid::new_v4());
    #[cfg(target_os = "macos")]
    {
        Ok(macos_private_claim_path(&parent, &claim_name))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(parent.join(claim_name))
    }
}

/// Rebinds a journaled claim name to the canonical parent selected by a
/// native coordination accessor. The old path is only a name reservation from
/// the pre-coordination preview; retaining its parent would make the claim
/// point at the wrong namespace after File Provider/iCloud rebinding. The
/// caller persists the returned private-namespace path before the claim phase
/// advances.
pub fn rebind_claim_path(
    source: &Path,
    planned_claim_path: &Path,
) -> Result<PathBuf, SourceClaimError> {
    let claim_name = planned_claim_path
        .file_name()
        .ok_or_else(|| SourceClaimError::ClaimFailed("claim path has no name".to_string()))?;
    let parent = source
        .parent()
        .ok_or(SourceClaimError::SourceMissing)?
        .canonicalize()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                SourceClaimError::SourceMissing
            } else {
                SourceClaimError::Io(error)
            }
        })?;
    #[cfg(target_os = "macos")]
    {
        let session_name = planned_claim_path
            .parent()
            .and_then(Path::file_name)
            .ok_or_else(|| {
                SourceClaimError::ClaimFailed(
                    "macOS claim path is missing its private session".to_string(),
                )
            })?;
        let retirement_root = planned_claim_path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .ok_or_else(|| {
                SourceClaimError::ClaimFailed(
                    "macOS claim path is missing its private retirement root".to_string(),
                )
            })?;
        if retirement_root != std::ffi::OsStr::new(MAC_RETIREMENT_ROOT_NAME) {
            return Err(SourceClaimError::ClaimFailed(
                "macOS claim path is outside the private retirement namespace".to_string(),
            ));
        }
        Ok(parent
            .join(retirement_root)
            .join(session_name)
            .join(claim_name))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(parent.join(claim_name))
    }
}

#[cfg(target_os = "macos")]
fn private_claim_names(
    original_parent: &VerifiedDirectory,
    claim_path: &Path,
) -> Result<(OsString, OsString, OsString), SourceClaimError> {
    use std::path::Component;

    let relative = claim_path
        .strip_prefix(original_parent.path())
        .map_err(|_| {
            SourceClaimError::ClaimFailed(
                "macOS claim path is not inside the source parent namespace".to_string(),
            )
        })?;
    let mut components = relative.components();
    let Component::Normal(retirement_root) = components.next().ok_or_else(|| {
        SourceClaimError::ClaimFailed("macOS claim path has no retirement root".to_string())
    })?
    else {
        return Err(SourceClaimError::ClaimFailed(
            "macOS claim path has an invalid retirement root".to_string(),
        ));
    };
    let Component::Normal(session_name) = components.next().ok_or_else(|| {
        SourceClaimError::ClaimFailed("macOS claim path has no private session".to_string())
    })?
    else {
        return Err(SourceClaimError::ClaimFailed(
            "macOS claim path has an invalid private session".to_string(),
        ));
    };
    let Component::Normal(claim_name) = components.next().ok_or_else(|| {
        SourceClaimError::ClaimFailed("macOS claim path has no claim entry".to_string())
    })?
    else {
        return Err(SourceClaimError::ClaimFailed(
            "macOS claim path has an invalid claim entry".to_string(),
        ));
    };
    if components.next().is_some()
        || retirement_root != std::ffi::OsStr::new(MAC_RETIREMENT_ROOT_NAME)
        || session_name.is_empty()
        || claim_name.is_empty()
        || (!claim_name
            .as_encoded_bytes()
            .starts_with(b".zen-canvas-claim-")
            && !claim_name
                .as_encoded_bytes()
                .starts_with(b".zen-canvas-replace-"))
    {
        return Err(SourceClaimError::ClaimFailed(
            "macOS claim path is outside the private retirement namespace".to_string(),
        ));
    }
    Ok((
        retirement_root.to_os_string(),
        session_name.to_os_string(),
        claim_name.to_os_string(),
    ))
}

#[cfg(target_os = "macos")]
fn open_or_create_private_directory(
    parent: &VerifiedDirectory,
    name: &OsStr,
    require_new: bool,
) -> Result<VerifiedDirectory, SourceClaimError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let name_c = CString::new(name.as_bytes())
        .map_err(|_| SourceClaimError::ClaimFailed("private directory contains NUL".to_string()))?;
    let created = unsafe { libc::mkdirat(parent.raw_fd(), name_c.as_ptr(), 0o700) } == 0;
    if !created {
        let error = io::Error::last_os_error();
        if require_new || error.raw_os_error() != Some(libc::EEXIST) {
            return Err(if error.raw_os_error() == Some(libc::EEXIST) {
                SourceClaimError::MacClaimNamespaceRebound
            } else {
                SourceClaimError::Io(error)
            });
        }
    }
    let created_identity = if created {
        Some(
            crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent.raw_fd(), name)
                .map_err(SourceClaimError::Io)?,
        )
    } else {
        None
    };
    let path = parent.path().join(name);
    let directory = VerifiedDirectory::open_existing(&path).map_err(map_directory_error)?;
    let physical =
        crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent.raw_fd(), name)
            .map_err(SourceClaimError::Io)?;
    if physical.file_type != libc::S_IFDIR as u32
        || physical.mode & 0o777 != 0o700
        || created_identity.is_some_and(|created| !created.matches(physical))
        || !verified_directory_matches_physical(&directory, physical)
        || directory.identity().volume_id != parent.identity().volume_id
    {
        return Err(SourceClaimError::MacClaimIdentityMismatch);
    }
    if created {
        directory.sync().map_err(SourceClaimError::Io)?;
        parent.sync().map_err(SourceClaimError::Io)?;
    }
    Ok(directory)
}

#[cfg(target_os = "macos")]
fn open_macos_private_claim_parent(
    original_parent: &VerifiedDirectory,
    claim_path: &Path,
) -> Result<VerifiedDirectory, SourceClaimError> {
    let (retirement_root_name, session_name, _claim_name) =
        private_claim_names(original_parent, claim_path)?;
    let retirement_root =
        open_or_create_private_directory(original_parent, &retirement_root_name, false)?;
    // A fresh random session must never be accepted merely because an
    // attacker pre-created its name. Recovery reopens the retained claim
    // directly; the first claim operation requires an actual mkdirat result.
    open_or_create_private_directory(&retirement_root, &session_name, true)
}

pub fn claim_source(
    source: &Path,
    expected: &ExpectedFileIdentity,
    operation_id: &str,
    cancel: Option<&AtomicBool>,
) -> Result<SourceClaim, SourceClaimError> {
    let claim_path = planned_claim_path(source, operation_id)?;
    claim_source_at(source, expected, &claim_path, operation_id, cancel)
}

pub fn claim_source_at(
    source: &Path,
    expected: &ExpectedFileIdentity,
    claim_path: &Path,
    _operation_id: &str,
    cancel: Option<&AtomicBool>,
) -> Result<SourceClaim, SourceClaimError> {
    platform_support::ensure_supported_file_mutation().map_err(|error| match error {
        platform_support::PlatformSupportError::LinuxUnsupported => {
            SourceClaimError::UnsupportedPlatformLinux
        }
        platform_support::PlatformSupportError::MacosFileMutationSourceBindingUnsupported => {
            SourceClaimError::MacosFileMutationSourceBindingUnsupported
        }
    })?;
    if is_cancelled(cancel) {
        return Err(SourceClaimError::Cancelled);
    }
    if !source.is_absolute() || !claim_path.is_absolute() {
        return Err(SourceClaimError::ClaimFailed(
            "absolute paths required".to_string(),
        ));
    }
    let source_metadata = fs::symlink_metadata(source).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SourceClaimError::SourceMissing
        } else {
            SourceClaimError::Io(error)
        }
    })?;
    #[cfg(not(target_os = "macos"))]
    if source_metadata.file_type().is_symlink() || is_reparse_point(&source_metadata) {
        return Err(SourceClaimError::ReparsePoint);
    }
    #[cfg(target_os = "macos")]
    if is_reparse_point(&source_metadata) {
        return Err(SourceClaimError::ReparsePoint);
    }
    let kind = if source_metadata.file_type().is_symlink() {
        ClaimedEntryKind::Symlink
    } else if source_metadata.is_file() {
        ClaimedEntryKind::File
    } else if source_metadata.is_dir() {
        ClaimedEntryKind::Directory
    } else {
        return Err(SourceClaimError::UnsupportedFileType);
    };
    let original_name = source
        .file_name()
        .ok_or(SourceClaimError::SourceMissing)?
        .to_os_string();
    let parent_path = source.parent().ok_or(SourceClaimError::SourceMissing)?;
    #[cfg(target_os = "macos")]
    let parent_path = parent_path.canonicalize().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SourceClaimError::SourceMissing
        } else {
            SourceClaimError::Io(error)
        }
    })?;
    let claim_parent = claim_path
        .parent()
        .ok_or_else(|| SourceClaimError::ClaimFailed("claim path has no parent".to_string()))?;
    #[cfg(target_os = "macos")]
    let original_parent =
        VerifiedDirectory::open_existing(parent_path.as_path()).map_err(map_directory_error)?;
    #[cfg(not(target_os = "macos"))]
    let original_parent =
        VerifiedDirectory::open_existing(parent_path).map_err(map_directory_error)?;
    #[cfg(target_os = "macos")]
    let current_parent = open_macos_private_claim_parent(&original_parent, claim_path)?;
    #[cfg(not(target_os = "macos"))]
    let current_parent =
        VerifiedDirectory::open_existing(claim_parent).map_err(map_directory_error)?;
    #[cfg(not(target_os = "macos"))]
    if current_parent.identity() != original_parent.identity() {
        return Err(SourceClaimError::ClaimFailed(
            "claim path must resolve to the source parent".to_string(),
        ));
    }
    #[cfg(target_os = "macos")]
    crate::platform::macos::mutation::ensure_path_eligible(source, claim_parent)
        .map_err(SourceClaimError::MacMutationNotSupported)?;
    let handle = open_source_handle(source, kind)?;
    #[cfg(target_os = "macos")]
    {
        if let Some(handle) = handle.as_ref() {
            let parent_device = original_parent
                .identity()
                .volume_id
                .parse::<u64>()
                .map_err(|_| {
                    SourceClaimError::MacMutationNotSupported(
                        crate::platform::macos::mutation::MAC_FILESYSTEM_NOT_SUPPORTED,
                    )
                })?;
            crate::platform::macos::mutation::ensure_opened_source_eligible(
                handle,
                parent_device,
                match kind {
                    ClaimedEntryKind::File => {
                        crate::platform::macos::mutation::MacMutationEntryKind::File
                    }
                    ClaimedEntryKind::Directory => {
                        crate::platform::macos::mutation::MacMutationEntryKind::Directory
                    }
                    ClaimedEntryKind::Symlink => {
                        crate::platform::macos::mutation::MacMutationEntryKind::Symlink
                    }
                },
            )
            .map_err(SourceClaimError::MacMutationNotSupported)?;
        }
    }
    let captured_before = match handle.as_ref() {
        Some(handle) => capture_claim_identity(handle, source, expected, cancel),
        None => capture_claim_path_identity(source, expected, cancel),
    }
    .map_err(map_identity_error)?;
    #[cfg(target_os = "macos")]
    let physical_before = match handle.as_ref() {
        Some(handle) => crate::platform::macos::identity::MacPhysicalIdentity::from_fd(handle),
        None => crate::platform::macos::identity::MacPhysicalIdentity::from_path_no_follow(source),
    }
    .map_err(SourceClaimError::Io)?;
    if !identity::identity_matches(expected, &captured_before) {
        return Err(SourceClaimError::SourceIdentityChanged);
    }
    if fs::symlink_metadata(claim_path).is_ok() {
        return Err(SourceClaimError::ClaimFailed(
            "claim path already exists".to_string(),
        ));
    }

    rename_claim_handle(
        handle.as_ref(),
        &original_parent,
        &original_name,
        &current_parent,
        claim_path.file_name().unwrap(),
        #[cfg(target_os = "macos")]
        Some(physical_before),
    )?;
    #[cfg(any(test, feature = "native-qa"))]
    run_claim_test_hook(
        ClaimTestPoint::AfterClaimBeforeIdentityCheck,
        source,
        claim_path,
    );
    let actual = match handle.as_ref() {
        Some(handle) => capture_claim_identity(handle, claim_path, expected, cancel),
        None => capture_claim_path_identity(claim_path, expected, cancel),
    }
    .map_err(map_identity_error)?;
    let claim_path_identity = if expected.full_hash.is_some() || expected.sample_hash.is_some() {
        identity::capture_namespace_identity(claim_path, cancel)
    } else {
        identity::capture_namespace_identity_only(claim_path, cancel)
    }
    .map_err(map_identity_error)?;
    #[cfg(target_os = "macos")]
    let claim_physical_identity = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
        current_parent.raw_fd(),
        claim_path.file_name().unwrap(),
    )
    .map_err(SourceClaimError::Io)?;
    if fs::symlink_metadata(source).is_ok() {
        #[cfg(target_os = "macos")]
        return Err(SourceClaimError::MacClaimNamespaceRebound);
        #[cfg(not(target_os = "macos"))]
        return Err(SourceClaimError::RecoveryRequired(
            "source path was replaced after the source claim".to_string(),
        ));
    }
    let claim_mismatch = !identity::identity_matches(expected, &actual)
        || !identity::identity_matches(&captured_before, &actual)
        || !identity::identity_matches(&captured_before, &claim_path_identity);
    #[cfg(target_os = "macos")]
    let claim_mismatch = claim_mismatch || !physical_before.matches(claim_physical_identity);
    if claim_mismatch {
        let mut partial = SourceClaim {
            original_path: source.to_path_buf(),
            current_path: claim_path.to_path_buf(),
            claim_path: claim_path.to_path_buf(),
            original_name: original_name.clone(),
            current_name: claim_path.file_name().unwrap().to_os_string(),
            original_parent,
            current_parent,
            expected_identity: expected.clone(),
            actual_identity: actual,
            kind,
            handle,
            #[cfg(target_os = "macos")]
            physical_identity: physical_before,
            deleted: false,
        };
        return match partial.rollback_to_original() {
            Ok(()) => Err(SourceClaimError::ClaimMismatch),
            Err(error) => Err(SourceClaimError::ClaimRollbackFailed(error.to_string())),
        };
    }

    let claim_name = claim_path.file_name().unwrap().to_os_string();
    Ok(SourceClaim {
        original_path: source.to_path_buf(),
        current_path: claim_path.to_path_buf(),
        claim_path: claim_path.to_path_buf(),
        original_name,
        current_name: claim_name,
        original_parent,
        current_parent,
        expected_identity: expected.clone(),
        actual_identity: actual,
        kind,
        handle,
        #[cfg(target_os = "macos")]
        physical_identity: physical_before,
        deleted: false,
    })
}

fn capture_claim_identity(
    handle: &File,
    path: &Path,
    expected: &ExpectedFileIdentity,
    cancel: Option<&AtomicBool>,
) -> Result<ExpectedFileIdentity, IdentityError> {
    if expected.full_hash.is_some() || expected.sample_hash.is_some() {
        identity::capture_identity_from_handle(handle, path, cancel)
    } else {
        identity::capture_namespace_identity_from_handle(handle, path, cancel)
    }
}

fn capture_claim_path_identity(
    path: &Path,
    expected: &ExpectedFileIdentity,
    cancel: Option<&AtomicBool>,
) -> Result<ExpectedFileIdentity, IdentityError> {
    if expected.full_hash.is_some() || expected.sample_hash.is_some() {
        identity::capture_identity(path, cancel)
    } else {
        identity::capture_namespace_identity_only(path, cancel)
    }
}

#[cfg(windows)]
pub(crate) fn commit_open_handle_noreplace(
    handle: &File,
    source_parent: &VerifiedDirectory,
    source_name: &OsStr,
    target_parent: &VerifiedDirectory,
    target_name: &OsStr,
) -> Result<(), SourceClaimError> {
    rename_claim_handle(
        Some(handle),
        source_parent,
        source_name,
        target_parent,
        target_name,
    )
}

#[cfg(windows)]
pub(crate) fn delete_open_handle(
    handle: &File,
    parent: &VerifiedDirectory,
    name: &OsStr,
    kind: ClaimedEntryKind,
) -> Result<(), SourceClaimError> {
    delete_claim_handle(Some(handle), parent, name, kind)
}

fn reopen_directory(directory: &VerifiedDirectory) -> Result<VerifiedDirectory, SourceClaimError> {
    VerifiedDirectory::open_existing(directory.path()).map_err(map_directory_error)
}

fn map_directory_error(error: super::PathGuardError) -> SourceClaimError {
    match error {
        super::PathGuardError::UnsupportedPlatformLinux => {
            SourceClaimError::UnsupportedPlatformLinux
        }
        super::PathGuardError::MacosFileMutationSourceBindingUnsupported => {
            SourceClaimError::MacosFileMutationSourceBindingUnsupported
        }
        super::PathGuardError::ReparsePoint => SourceClaimError::ReparsePoint,
        super::PathGuardError::IdentityChanged => {
            SourceClaimError::RecoveryRequired("verified directory identity changed".to_string())
        }
        super::PathGuardError::UnsafePath => {
            SourceClaimError::ClaimFailed("unsafe path".to_string())
        }
        super::PathGuardError::Io(error) => SourceClaimError::Io(error),
    }
}

fn map_identity_error(error: IdentityError) -> SourceClaimError {
    match error {
        IdentityError::SourceMissing => SourceClaimError::SourceMissing,
        IdentityError::Symlink => SourceClaimError::ReparsePoint,
        IdentityError::UnsupportedFileType => SourceClaimError::UnsupportedFileType,
        IdentityError::DirectoryManifestNameEncodingFailed => {
            SourceClaimError::ClaimFailed("directory_manifest_name_encoding_failed".to_string())
        }
        IdentityError::Cancelled => SourceClaimError::Cancelled,
        IdentityError::ContentReadRejected(reason) => map_content_read_rejected(reason),
        IdentityError::Io(error) => SourceClaimError::Io(error),
    }
}

fn map_content_read_rejected(reason: &'static str) -> SourceClaimError {
    #[cfg(target_os = "macos")]
    {
        SourceClaimError::MacMutationNotSupported(reason)
    }
    #[cfg(not(target_os = "macos"))]
    {
        SourceClaimError::Io(io::Error::new(io::ErrorKind::PermissionDenied, reason))
    }
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Acquire))
}

#[cfg(windows)]
pub(crate) fn windows_wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut units = path
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == b'/' as u16 {
                b'\\' as u16
            } else {
                unit
            }
        })
        .collect::<Vec<_>>();
    let slash = b'\\' as u16;
    let extended = [slash, slash, b'?' as u16, slash];
    if units.starts_with(&extended) {
        units.push(0);
        return units;
    }
    if units.starts_with(&[slash, slash]) {
        let mut prefixed = [
            slash,
            slash,
            b'?' as u16,
            slash,
            b'U' as u16,
            b'N' as u16,
            b'C' as u16,
            slash,
        ]
        .to_vec();
        prefixed.extend_from_slice(&units[2..]);
        prefixed.push(0);
        return prefixed;
    }
    if units.get(1) == Some(&(b':' as u16)) {
        let mut prefixed = [slash, slash, b'?' as u16, slash].to_vec();
        prefixed.extend_from_slice(&units);
        prefixed.push(0);
        return prefixed;
    }
    units.push(0);
    units
}

#[cfg(windows)]
pub(crate) fn open_source_handle(
    path: &Path,
    kind: ClaimedEntryKind,
) -> Result<Option<File>, SourceClaimError> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{GetLastError, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING, SYNCHRONIZE,
        },
    };
    let wide = windows_wide_path(path);
    let flags = FILE_FLAG_OPEN_REPARSE_POINT
        | if matches!(kind, ClaimedEntryKind::Directory) {
            FILE_FLAG_BACKUP_SEMANTICS
        } else {
            0
        };
    // Claiming is a namespace operation.  Keep the source handle read-only
    // apart from DELETE, which is required for the handle-relative rename and
    // claim cleanup.  FILE_WRITE_DATA and FILE_WRITE_ATTRIBUTES are not
    // requested: the source bytes are unchanged and namespace durability is
    // proved through the verified parent-directory handles.
    let access = DELETE
        | FILE_READ_ATTRIBUTES
        | if matches!(kind, ClaimedEntryKind::File) {
            FILE_READ_DATA
        } else {
            0
        }
        | SYNCHRONIZE;
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        let error = unsafe { GetLastError() } as i32;
        return if matches!(error, 1 | 50 | 87 | 120) {
            Err(SourceClaimError::AtomicSourceBindingUnsupported)
        } else {
            Err(SourceClaimError::Io(io::Error::from_raw_os_error(error)))
        };
    }
    Ok(Some(unsafe {
        File::from(OwnedHandle::from_raw_handle(raw))
    }))
}

#[cfg(target_os = "macos")]
pub(crate) fn open_source_handle(
    path: &Path,
    kind: ClaimedEntryKind,
) -> Result<Option<File>, SourceClaimError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, os::unix::io::FromRawFd};
    if matches!(kind, ClaimedEntryKind::Symlink) {
        return Ok(None);
    }
    let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        SourceClaimError::ClaimFailed("source path contains an embedded NUL".to_string())
    })?;
    let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let flags = if matches!(kind, ClaimedEntryKind::Directory) {
        flags | libc::O_DIRECTORY
    } else {
        flags
    };
    let fd = unsafe { libc::open(name.as_ptr(), flags) };
    if fd < 0 {
        return Err(SourceClaimError::Io(io::Error::last_os_error()));
    }
    Ok(Some(unsafe { File::from_raw_fd(fd) }))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn open_source_handle(
    _path: &Path,
    _kind: ClaimedEntryKind,
) -> Result<Option<File>, SourceClaimError> {
    Err(SourceClaimError::UnsupportedPlatformLinux)
}

fn rename_claim_handle(
    handle: Option<&File>,
    source_parent: &VerifiedDirectory,
    source_name: &OsStr,
    target_parent: &VerifiedDirectory,
    target_name: &OsStr,
    #[cfg(target_os = "macos")] expected_physical_identity: Option<
        crate::platform::macos::identity::MacPhysicalIdentity,
    >,
) -> Result<(), SourceClaimError> {
    #[cfg(windows)]
    let _ = source_name;

    #[cfg(windows)]
    {
        let _ = source_parent;
        let handle = handle.ok_or(SourceClaimError::UnsupportedFileType)?;
        rename_windows(handle, target_parent, target_name)
    }

    #[cfg(target_os = "macos")]
    {
        // Darwin has no source-FD rename.  The safe product primitive is a
        // recoverable namespace transaction: the destination is always
        // exclusive, the claim name is private/high-entropy, and callers
        // verify the claimed object immediately after this operation.  A
        // mismatch is rolled back or retained for manual recovery; it is
        // never silently committed.
        let _ = handle;
        let expected =
            expected_physical_identity.ok_or(SourceClaimError::MacMutationNotSupported(
                crate::platform::macos::mutation::MAC_FILESYSTEM_NOT_SUPPORTED,
            ))?;
        ensure_namespace_entry_matches(source_parent, source_name, expected)?;
        rename_macos_noreplace(source_parent, source_name, target_parent, target_name)
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (
            handle,
            source_parent,
            source_name,
            target_parent,
            target_name,
        );
        Err(SourceClaimError::UnsupportedPlatformLinux)
    }
}

#[cfg(windows)]
fn rename_windows(
    handle: &File,
    target_parent: &VerifiedDirectory,
    target_name: &OsStr,
) -> Result<(), SourceClaimError> {
    use std::os::windows::ffi::OsStrExt;
    use std::{mem, os::windows::io::AsRawHandle, ptr};
    use windows_sys::{
        Wdk::Storage::FileSystem::{
            FileRenameInformation, NtSetInformationFile, FILE_RENAME_INFORMATION,
        },
        Win32::{
            Foundation::{
                RtlNtStatusToDosError, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS,
                ERROR_NOT_SAME_DEVICE,
            },
            System::IO::IO_STATUS_BLOCK,
        },
    };
    let name = target_name.encode_wide().collect::<Vec<_>>();
    if name.is_empty() || name.contains(&0) {
        return Err(SourceClaimError::ClaimFailed(
            "empty or invalid target name".to_string(),
        ));
    }
    let total_size = mem::size_of::<FILE_RENAME_INFORMATION>()
        + name.len().saturating_sub(1) * mem::size_of::<u16>();
    let mut buffer = vec![0_u8; total_size];
    let info = buffer.as_mut_ptr() as *mut FILE_RENAME_INFORMATION;
    let mut io_status = unsafe { mem::zeroed::<IO_STATUS_BLOCK>() };
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = target_parent.handle().as_raw_handle();
        (*info).FileNameLength = (name.len() * mem::size_of::<u16>()) as u32;
        ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
        let status = NtSetInformationFile(
            handle.as_raw_handle(),
            &mut io_status,
            buffer.as_ptr().cast(),
            total_size as u32,
            FileRenameInformation,
        );
        if status >= 0 {
            return Ok(());
        }
        let code = RtlNtStatusToDosError(status);
        match code {
            ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS => Err(SourceClaimError::TargetExists),
            ERROR_NOT_SAME_DEVICE => Err(SourceClaimError::CrossDevice),
            code => Err(SourceClaimError::Io(io::Error::from_raw_os_error(
                code as i32,
            ))),
        }
    }
}

#[cfg(windows)]
fn delete_claim_handle(
    handle: Option<&File>,
    _parent: &VerifiedDirectory,
    _name: &OsStr,
    _kind: ClaimedEntryKind,
) -> Result<(), SourceClaimError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{
            GetLastError, ERROR_CALL_NOT_IMPLEMENTED, ERROR_INVALID_FUNCTION,
            ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED,
        },
        Storage::FileSystem::{
            FileDispositionInfoEx, SetFileInformationByHandle, FILE_DISPOSITION_FLAG_DELETE,
            FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
            FILE_DISPOSITION_INFO_EX,
        },
    };
    let disposition = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE
            | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE
            | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    };
    let handle = handle.ok_or(SourceClaimError::UnsupportedFileType)?;
    if unsafe {
        SetFileInformationByHandle(
            handle.as_raw_handle(),
            FileDispositionInfoEx,
            (&disposition as *const FILE_DISPOSITION_INFO_EX).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    } != 0
    {
        return Ok(());
    }
    let code = unsafe { GetLastError() };
    if matches!(
        code,
        ERROR_INVALID_FUNCTION
            | ERROR_INVALID_PARAMETER
            | ERROR_NOT_SUPPORTED
            | ERROR_CALL_NOT_IMPLEMENTED
    ) {
        Err(SourceClaimError::AtomicSourceBindingUnsupported)
    } else {
        Err(SourceClaimError::Io(io::Error::from_raw_os_error(
            code as i32,
        )))
    }
}

#[cfg(target_os = "macos")]
fn remove_namespace_tree(
    parent_fd: std::os::fd::RawFd,
    name: &OsStr,
    expected: crate::platform::macos::identity::MacPhysicalIdentity,
) -> Result<(), SourceClaimError> {
    use std::os::unix::{
        ffi::OsStrExt,
        io::{AsRawFd, FromRawFd},
    };

    let actual = crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent_fd, name)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                SourceClaimError::MacClaimPathMissing
            } else {
                SourceClaimError::MacClaimPathUnreadable
            }
        })?;
    let matches = if actual.file_type == libc::S_IFDIR as u32 {
        expected.matches(actual)
    } else {
        expected.matches_strict(actual)
    };
    if !matches {
        return Err(SourceClaimError::MacClaimNamespaceRebound);
    }

    if actual.file_type == libc::S_IFDIR as u32 {
        let name_c = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| SourceClaimError::ClaimFailed("embedded NUL in claim name".to_string()))?;
        let fd = unsafe {
            libc::openat(
                parent_fd,
                name_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(SourceClaimError::Io(io::Error::last_os_error()));
        }
        let directory = unsafe { std::fs::File::from_raw_fd(fd) };
        let child_names = directory_entry_names(directory.as_raw_fd())?;
        for child in child_names {
            remove_namespace_tree(
                directory.as_raw_fd(),
                &child,
                child_identity(directory.as_raw_fd(), &child)?,
            )?;
        }
        drop(directory);

        let current =
            crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent_fd, name)
                .map_err(|error| {
                    if error.kind() == io::ErrorKind::NotFound {
                        SourceClaimError::MacClaimPathMissing
                    } else {
                        SourceClaimError::MacClaimPathUnreadable
                    }
                })?;
        if !expected.matches(current) {
            return Err(SourceClaimError::MacClaimNamespaceRebound);
        }
        if unsafe { libc::unlinkat(parent_fd, name_c.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            return Err(SourceClaimError::Io(io::Error::last_os_error()));
        }
    } else {
        let name_c = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| SourceClaimError::ClaimFailed("embedded NUL in claim name".to_string()))?;
        let current =
            crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent_fd, name)
                .map_err(|error| {
                    if error.kind() == io::ErrorKind::NotFound {
                        SourceClaimError::MacClaimPathMissing
                    } else {
                        SourceClaimError::MacClaimPathUnreadable
                    }
                })?;
        if !expected.matches_strict(current) {
            return Err(SourceClaimError::MacClaimNamespaceRebound);
        }
        if unsafe { libc::unlinkat(parent_fd, name_c.as_ptr(), 0) } != 0 {
            return Err(SourceClaimError::Io(io::Error::last_os_error()));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn child_identity(
    parent_fd: std::os::fd::RawFd,
    name: &OsStr,
) -> Result<crate::platform::macos::identity::MacPhysicalIdentity, SourceClaimError> {
    crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent_fd, name)
        .map_err(SourceClaimError::Io)
}

#[cfg(target_os = "macos")]
fn directory_entry_names(parent_fd: std::os::fd::RawFd) -> Result<Vec<OsString>, SourceClaimError> {
    use std::os::unix::ffi::OsStringExt;

    let scan_fd = unsafe { libc::dup(parent_fd) };
    if scan_fd < 0 {
        return Err(SourceClaimError::Io(io::Error::last_os_error()));
    }
    let directory = unsafe { libc::fdopendir(scan_fd) };
    if directory.is_null() {
        unsafe { libc::close(scan_fd) };
        return Err(SourceClaimError::Io(io::Error::last_os_error()));
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

#[cfg(target_os = "macos")]
fn delete_claim_handle(
    handle: Option<&File>,
    parent: &VerifiedDirectory,
    name: &OsStr,
    kind: ClaimedEntryKind,
    expected_physical_identity: crate::platform::macos::identity::MacPhysicalIdentity,
) -> Result<(), SourceClaimError> {
    // The caller has already revalidated the private claim name against the
    // retained descriptor.  Delete only that claim entry, never the original
    // user pathname, and use AT_REMOVEDIR for a claimed directory.  Files and
    // symlinks use the strict proof here: a same-inode pathname with changed
    // metadata is still not the object that the destructive operation claimed.
    let _ = handle;
    ensure_namespace_entry_matches_for_removal(parent, name, kind, expected_physical_identity)?;
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let name_c = CString::new(name.as_bytes())
        .map_err(|_| SourceClaimError::ClaimFailed("embedded NUL in claim name".to_string()))?;
    let flags = if matches!(kind, ClaimedEntryKind::Directory) {
        libc::AT_REMOVEDIR
    } else {
        0
    };
    ensure_namespace_entry_matches_for_removal(parent, name, kind, expected_physical_identity)?;
    if unsafe { libc::unlinkat(parent.raw_fd(), name_c.as_ptr(), flags) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ENOENT) {
        Err(SourceClaimError::MacClaimPathMissing)
    } else {
        Err(SourceClaimError::Io(error))
    }
}

#[cfg(target_os = "macos")]
fn rename_macos_noreplace(
    source_parent: &VerifiedDirectory,
    source_name: &OsStr,
    target_parent: &VerifiedDirectory,
    target_name: &OsStr,
) -> Result<(), SourceClaimError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let source_name_c = CString::new(source_name.as_bytes())
        .map_err(|_| SourceClaimError::ClaimFailed("embedded NUL in source name".to_string()))?;
    let target_name_c = CString::new(target_name.as_bytes())
        .map_err(|_| SourceClaimError::ClaimFailed("embedded NUL in target name".to_string()))?;
    // Darwin's RENAME_EXCL makes publication fail if the destination entry
    // exists.  It is intentionally not RENAME_SWAP: replacement has a
    // separate backup/quarantine transaction.
    const RENAME_EXCL: u32 = 0x0000_0004;
    let result = unsafe {
        renameatx_np(
            source_parent.raw_fd(),
            source_name_c.as_ptr(),
            target_parent.raw_fd(),
            target_name_c.as_ptr(),
            RENAME_EXCL,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EEXIST) => Err(SourceClaimError::TargetExists),
        Some(libc::EXDEV) => Err(SourceClaimError::CrossDevice),
        Some(libc::EINVAL | libc::ENOTSUP | libc::ENOSYS) => {
            rename_macos_link_unlink(source_parent, source_name, target_parent, target_name)
        }
        _ => Err(SourceClaimError::Io(error)),
    }
}

/// Darwin's `linkat` plus pathname `unlinkat` is not a safe source-retirement
/// primitive: either pathname can be rebound between an identity check and the
/// destructive call.  Do not use it as a portable rename substitute.  The
/// caller routes this explicit capability failure to the verified copy path or
/// leaves the source available for recovery; it must never guess at cleanup.
#[cfg(target_os = "macos")]
fn rename_macos_link_unlink(
    source_parent: &VerifiedDirectory,
    source_name: &OsStr,
    target_parent: &VerifiedDirectory,
    target_name: &OsStr,
) -> Result<(), SourceClaimError> {
    let _ = (source_parent, source_name, target_parent, target_name);
    Err(SourceClaimError::AtomicSourceBindingUnsupported)
}

#[cfg(target_os = "macos")]
fn verified_directory_matches_physical(
    directory: &VerifiedDirectory,
    physical: crate::platform::macos::identity::MacPhysicalIdentity,
) -> bool {
    directory.identity().volume_id == physical.dev.to_string()
        && directory.identity().file_id == physical.ino.to_string()
}

/// Proves the small namespace primitives needed before a target-first move
/// retires a source on a non-APFS local volume. The probe is intentionally
/// private, unique and identity-bound; it never opens or renames the user's
/// source object. A failure leaves the caller with a fail-closed capability
/// result instead of a blanket portable-filesystem claim.
#[cfg(target_os = "macos")]
pub(crate) fn probe_macos_namespace_retirement(
    parent: &Path,
    directory_probe: bool,
) -> Result<(), SourceClaimError> {
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};

    let parent_directory = VerifiedDirectory::open_existing(parent).map_err(map_directory_error)?;
    let retirement_root_name = OsString::from(".zen-canvas-retirement");
    let retirement_root_path = parent_directory.path().join(&retirement_root_name);
    let retirement_root_name_c = std::ffi::CString::new(retirement_root_name.as_bytes())
        .map_err(|_| SourceClaimError::ClaimFailed("retirement root contains NUL".to_string()))?;
    let root_created = unsafe {
        libc::mkdirat(
            parent_directory.raw_fd(),
            retirement_root_name_c.as_ptr(),
            0o700,
        )
    } == 0;
    if !root_created {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EEXIST) {
            return Err(SourceClaimError::Io(error));
        }
    }
    let retirement_root =
        VerifiedDirectory::open_existing(&retirement_root_path).map_err(map_directory_error)?;
    let root_identity = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
        parent_directory.raw_fd(),
        &retirement_root_name,
    )
    .map_err(SourceClaimError::Io)?;
    if !verified_directory_matches_physical(&retirement_root, root_identity)
        || root_identity.mode & 0o777 != 0o700
    {
        return Err(SourceClaimError::MacClaimIdentityMismatch);
    }
    if root_created {
        retirement_root.sync().map_err(SourceClaimError::Io)?;
        parent_directory.sync().map_err(SourceClaimError::Io)?;
    }

    let session_name = OsString::from(format!("{}", uuid::Uuid::new_v4()));
    let session_name_c = std::ffi::CString::new(session_name.as_bytes()).map_err(|_| {
        SourceClaimError::ClaimFailed("retirement session contains NUL".to_string())
    })?;
    if unsafe { libc::mkdirat(retirement_root.raw_fd(), session_name_c.as_ptr(), 0o700) } != 0 {
        return Err(SourceClaimError::Io(io::Error::last_os_error()));
    }
    retirement_root.sync().map_err(SourceClaimError::Io)?;
    let session_path = retirement_root.path().join(&session_name);
    let session_identity = match crate::platform::macos::identity::MacPhysicalIdentity::from_at(
        retirement_root.raw_fd(),
        &session_name,
    ) {
        Ok(identity) => identity,
        Err(error) => {
            // The private session is intentionally retained. Without an
            // identity there is no safe object to remove.
            return Err(SourceClaimError::Io(error));
        }
    };
    let session = match VerifiedDirectory::open_existing(&session_path) {
        Ok(session) => session,
        Err(error) => return Err(map_directory_error(error)),
    };
    let reopened_session_identity = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
        retirement_root.raw_fd(),
        &session_name,
    )
    .map_err(SourceClaimError::Io)?;
    if !verified_directory_matches_physical(&session, session_identity)
        || !session_identity.matches(reopened_session_identity)
    {
        // Do not attempt to clean a session whose identity cannot be proven.
        return Err(SourceClaimError::MacClaimIdentityMismatch);
    }

    let source_name = OsString::from(format!("source-{}", uuid::Uuid::new_v4()));
    let target_name = OsString::from(format!("target-{}", uuid::Uuid::new_v4()));
    let source_identity = if directory_probe {
        let source_name_c = std::ffi::CString::new(source_name.as_bytes())
            .map_err(|_| SourceClaimError::ClaimFailed("probe name contains NUL".to_string()))?;
        if unsafe { libc::mkdirat(session.raw_fd(), source_name_c.as_ptr(), 0o700) } != 0 {
            return Err(SourceClaimError::Io(io::Error::last_os_error()));
        }
        crate::platform::macos::identity::MacPhysicalIdentity::from_at(
            session.raw_fd(),
            &source_name,
        )
        .map_err(SourceClaimError::Io)?
    } else {
        let source_name_c = std::ffi::CString::new(source_name.as_bytes())
            .map_err(|_| SourceClaimError::ClaimFailed("probe name contains NUL".to_string()))?;
        let fd = unsafe {
            libc::openat(
                session.raw_fd(),
                source_name_c.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if fd < 0 {
            return Err(SourceClaimError::Io(io::Error::last_os_error()));
        }
        let mut probe = unsafe { std::fs::File::from_raw_fd(fd) };
        probe
            .write_all(b"zen-canvas-capability-probe")
            .map_err(SourceClaimError::Io)?;
        probe.sync_all().map_err(SourceClaimError::Io)?;
        crate::platform::macos::identity::MacPhysicalIdentity::from_fd(&probe)
            .map_err(SourceClaimError::Io)?
    };
    let mut moved = false;
    let result = (|| {
        rename_macos_noreplace(&session, &source_name, &session, &target_name)?;
        moved = true;
        let target_identity = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
            session.raw_fd(),
            &target_name,
        )
        .map_err(SourceClaimError::Io)?;
        if !source_identity.matches(target_identity) {
            return Err(SourceClaimError::MacClaimIdentityMismatch);
        }
        session.sync().map_err(SourceClaimError::Io)?;
        let target_name_c = std::ffi::CString::new(target_name.as_bytes())
            .map_err(|_| SourceClaimError::ClaimFailed("probe name contains NUL".to_string()))?;
        let remove_flags = if directory_probe {
            libc::AT_REMOVEDIR
        } else {
            0
        };
        if unsafe { libc::unlinkat(session.raw_fd(), target_name_c.as_ptr(), remove_flags) } != 0 {
            return Err(SourceClaimError::Io(io::Error::last_os_error()));
        }
        session.sync().map_err(SourceClaimError::Io)?;
        Ok(())
    })();

    if result.is_err() {
        let cleanup_name = if moved { &target_name } else { &source_name };
        if let Ok(current) = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
            session.raw_fd(),
            cleanup_name,
        ) {
            let cleanup_matches = if directory_probe {
                source_identity.matches(current)
            } else {
                source_identity.matches_strict(current)
            };
            if cleanup_matches {
                if let Ok(cleanup_name_c) = std::ffi::CString::new(cleanup_name.as_bytes()) {
                    let remove_flags = if directory_probe {
                        libc::AT_REMOVEDIR
                    } else {
                        0
                    };
                    let _ = unsafe {
                        libc::unlinkat(session.raw_fd(), cleanup_name_c.as_ptr(), remove_flags)
                    };
                    let _ = session.sync();
                }
            }
        }
        // If any identity check above failed, the private session remains in
        // place for manual/recovery inspection; never widen cleanup to a path
        // guessed from the original parent.
        return result;
    }

    let current_session_identity = crate::platform::macos::identity::MacPhysicalIdentity::from_at(
        retirement_root.raw_fd(),
        &session_name,
    )
    .map_err(SourceClaimError::Io)?;
    if !session_identity.matches(current_session_identity) {
        return Err(SourceClaimError::MacClaimIdentityMismatch);
    }
    if unsafe {
        libc::unlinkat(
            retirement_root.raw_fd(),
            session_name_c.as_ptr(),
            libc::AT_REMOVEDIR,
        )
    } != 0
    {
        return Err(SourceClaimError::Io(io::Error::last_os_error()));
    }
    retirement_root.sync().map_err(SourceClaimError::Io)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_namespace_entry_matches(
    parent: &VerifiedDirectory,
    name: &OsStr,
    expected: crate::platform::macos::identity::MacPhysicalIdentity,
) -> Result<(), SourceClaimError> {
    let actual =
        crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent.raw_fd(), name)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    SourceClaimError::MacClaimPathMissing
                } else {
                    SourceClaimError::MacClaimPathUnreadable
                }
            })?;
    if !expected.matches(actual) {
        return Err(SourceClaimError::MacClaimNamespaceRebound);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_namespace_entry_matches_for_removal(
    parent: &VerifiedDirectory,
    name: &OsStr,
    kind: ClaimedEntryKind,
    expected: crate::platform::macos::identity::MacPhysicalIdentity,
) -> Result<(), SourceClaimError> {
    let actual =
        crate::platform::macos::identity::MacPhysicalIdentity::from_at(parent.raw_fd(), name)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    SourceClaimError::MacClaimPathMissing
                } else {
                    SourceClaimError::MacClaimPathUnreadable
                }
            })?;
    let matches = match kind {
        ClaimedEntryKind::Directory => expected.matches(actual),
        ClaimedEntryKind::File | ClaimedEntryKind::Symlink => expected.matches_strict(actual),
    };
    if !matches {
        return Err(SourceClaimError::MacClaimNamespaceRebound);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
impl SourceClaim {
    /// A descriptor-bound claim is not enough for a move: if the original
    /// pathname was recreated after claiming, committing the claim would
    /// silently turn one move into "move the old object and leave a new
    /// object behind". Treat that namespace replacement as a manual-review
    /// race and never overwrite or delete the replacement.
    fn ensure_original_namespace_is_unoccupied(&self) -> Result<(), SourceClaimError> {
        match fs::symlink_metadata(&self.original_path) {
            Ok(_) => Err(SourceClaimError::MacClaimNamespaceRebound),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(SourceClaimError::MacClaimPathUnreadable),
        }
    }
}

#[cfg(target_os = "macos")]
extern "C" {
    fn renameatx_np(
        fromfd: libc::c_int,
        from: *const libc::c_char,
        tofd: libc::c_int,
        to: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
}

#[cfg(not(any(windows, target_os = "macos")))]
fn delete_claim_handle(
    _handle: Option<&File>,
    _parent: &VerifiedDirectory,
    _name: &OsStr,
    _kind: ClaimedEntryKind,
) -> Result<(), SourceClaimError> {
    Err(SourceClaimError::UnsupportedPlatformLinux)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(any(test, feature = "native-qa"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimTestPoint {
    AfterJournalPreparedBeforeClaim,
    AfterClaimBeforeIdentityCheck,
    AfterClaimVerifiedBeforeTargetCommit,
    AfterClaimVerifiedBeforeCommit,
    AfterClaimVerifiedBeforeDelete,
    AfterClaimVerifiedBeforeRollback,
    AfterClaimVerifiedBeforeTrashCommit,
    AfterClaimVerifiedBeforeRestoreCommit,
    AfterReplacementBackupClaimed,
    AfterTargetParentVerifiedBeforeCommit,
    AfterStagingVerifiedBeforeCommit,
    AfterCopyProofVerifiedBeforePublish,
    AfterTargetCommitBeforeSourceCleanup,
    AfterSourceCleanupBeforeJournalComplete,
}

#[cfg(any(test, feature = "native-qa"))]
pub use test_hooks::{
    current_claim_test_hook, run_claim_test_hook, run_claim_test_hook_with_override, Hook,
};
#[cfg(all(any(test, feature = "native-qa"), any(windows, target_os = "macos")))]
pub use test_hooks::{lock_claim_test_hooks, set_claim_test_hook};

#[cfg(any(test, feature = "native-qa"))]
mod test_hooks {
    use super::ClaimTestPoint;
    #[cfg(any(windows, target_os = "macos"))]
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::{cell::RefCell, path::Path};

    pub type Hook = fn(ClaimTestPoint, &Path, &Path);
    #[cfg(any(windows, target_os = "macos"))]
    static CLAIM_TEST_SERIAL: OnceLock<Mutex<()>> = OnceLock::new();
    thread_local! {
        static CLAIM_TEST_HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    #[cfg(any(windows, target_os = "macos"))]
    pub fn lock_claim_test_hooks() -> MutexGuard<'static, ()> {
        CLAIM_TEST_SERIAL
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(any(windows, target_os = "macos"))]
    pub fn set_claim_test_hook(hook: Option<Hook>) {
        CLAIM_TEST_HOOK.with(|current| {
            *current.borrow_mut() = hook;
        });
    }

    pub fn current_claim_test_hook() -> Option<Hook> {
        CLAIM_TEST_HOOK.with(|current| *current.borrow())
    }

    pub fn run_claim_test_hook(point: ClaimTestPoint, source: &Path, claim: &Path) {
        run_claim_test_hook_with_override(current_claim_test_hook(), point, source, claim);
    }

    pub fn run_claim_test_hook_with_override(
        hook: Option<Hook>,
        point: ClaimTestPoint,
        source: &Path,
        claim: &Path,
    ) {
        if let Some(hook) = hook {
            hook(point, source, claim);
        }
    }
}

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;
    use crate::fs_safety::atomic_move::{atomic_move_noreplace, AtomicMoveError};

    fn fixture(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-source-claim-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("fixture");
        path
    }

    fn replace_source_after_claim(point: ClaimTestPoint, source: &Path, _claim: &Path) {
        if point == ClaimTestPoint::AfterClaimBeforeIdentityCheck {
            fs::write(source, b"replacement").expect("replacement source");
        }
    }

    fn create_source_replacement_before_commit(
        point: ClaimTestPoint,
        source: &Path,
        _claim: &Path,
    ) {
        if point == ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit {
            fs::write(source, b"new source at original path").expect("replacement source");
        }
    }

    fn create_target_conflict_before_commit(point: ClaimTestPoint, source: &Path, _claim: &Path) {
        if point == ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit {
            fs::write(
                source.parent().expect("source parent").join("target"),
                b"competitor",
            )
            .expect("competitor target");
        }
    }

    fn replace_target_parent_before_commit(point: ClaimTestPoint, source: &Path, _claim: &Path) {
        if point != ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit {
            return;
        }
        let root = source
            .parent()
            .and_then(Path::parent)
            .expect("fixture root");
        let target_parent = root.join("target");
        let displaced = root.join("target-displaced");
        fs::rename(&target_parent, &displaced).expect("displace target parent");
        fs::create_dir(&target_parent).expect("replace target parent");
    }

    fn replace_target_parent_after_verification(
        point: ClaimTestPoint,
        _source: &Path,
        target: &Path,
    ) {
        if point != ClaimTestPoint::AfterTargetParentVerifiedBeforeCommit {
            return;
        }
        let parent = target.parent().expect("target parent");
        let displaced = parent.with_file_name("target-verified-displaced");
        fs::rename(parent, &displaced).expect("displace verified target parent");
        fs::create_dir(parent).expect("replacement target parent");
    }

    #[cfg(windows)]
    fn replace_claim_before_identity_check(point: ClaimTestPoint, _source: &Path, claim: &Path) {
        if point == ClaimTestPoint::AfterClaimBeforeIdentityCheck {
            fs::remove_file(claim).expect("remove claim");
            fs::write(claim, b"replacement claim").expect("replacement claim");
        }
    }

    #[test]
    fn source_replacement_after_claim_is_recovery_required_and_keeps_both_objects() {
        let _serial = lock_claim_test_hooks();
        let root = fixture("source-replacement");
        let source = root.join("source.txt");
        fs::write(&source, b"original").expect("source");
        let expected = identity::capture_identity(&source, None).expect("identity");
        let claim_path = planned_claim_path(&source, "replacement").expect("claim path");
        set_claim_test_hook(Some(replace_source_after_claim));
        let result = claim_source_at(&source, &expected, &claim_path, "replacement", None);
        set_claim_test_hook(None);

        assert!(matches!(result, Err(SourceClaimError::RecoveryRequired(_))));
        assert_eq!(
            fs::read(&source).expect("replacement bytes"),
            b"replacement"
        );
        assert_eq!(fs::read(&claim_path).expect("claimed bytes"), b"original");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn original_path_replacement_after_claim_is_not_deleted_by_commit() {
        let _serial = lock_claim_test_hooks();
        let root = fixture("original-path-replacement");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, b"original").expect("source");
        set_claim_test_hook(Some(create_source_replacement_before_commit));
        let result = atomic_move_noreplace(&source, &target, None, None);
        set_claim_test_hook(None);

        assert!(result.is_ok(), "move result: {result:?}");
        assert_eq!(fs::read(&target).expect("target bytes"), b"original");
        assert_eq!(
            fs::read(&source).expect("replacement bytes"),
            b"new source at original path"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn target_race_returns_target_exists_and_rolls_back_claim() {
        let _serial = lock_claim_test_hooks();
        let root = fixture("target-race");
        let source = root.join("source.txt");
        let target = root.join("target");
        fs::write(&source, b"original").expect("source");
        set_claim_test_hook(Some(create_target_conflict_before_commit));
        let result = atomic_move_noreplace(&source, &target, None, None);
        set_claim_test_hook(None);

        assert!(matches!(result, Err(AtomicMoveError::TargetExists)));
        assert_eq!(fs::read(&source).expect("rolled back source"), b"original");
        assert_eq!(fs::read(&target).expect("competitor target"), b"competitor");
        assert!(!fs::read_dir(&root)
            .expect("root entries")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".zen-canvas-claim-")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn target_parent_replacement_is_rejected_without_redirecting_the_target() {
        let _serial = lock_claim_test_hooks();
        let root = fixture("target-parent-race");
        let source_parent = root.join("source");
        let target_parent = root.join("target");
        fs::create_dir(&source_parent).expect("source parent");
        fs::create_dir(&target_parent).expect("target parent");
        let source = source_parent.join("source.txt");
        let target = target_parent.join("source.txt");
        fs::write(&source, b"original").expect("source");
        set_claim_test_hook(Some(replace_target_parent_before_commit));
        let result = atomic_move_noreplace(&source, &target, None, None);
        set_claim_test_hook(None);

        assert!(result.is_err());
        assert_eq!(
            fs::read(&source).expect("source after rollback"),
            b"original"
        );
        assert!(!target.exists());
        assert!(!root.join("target-displaced").join("source.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn target_parent_replacement_after_verification_cannot_redirect_commit() {
        let _serial = lock_claim_test_hooks();
        let root = fixture("target-parent-after-verified");
        let source_parent = root.join("source");
        let target_parent = root.join("target");
        fs::create_dir(&source_parent).expect("source parent");
        fs::create_dir(&target_parent).expect("target parent");
        let source = source_parent.join("source.txt");
        let target = target_parent.join("source.txt");
        fs::write(&source, b"original").expect("source");
        set_claim_test_hook(Some(replace_target_parent_after_verification));
        let result = atomic_move_noreplace(&source, &target, None, None);
        set_claim_test_hook(None);

        assert!(matches!(
            result,
            Err(AtomicMoveError::TargetCommittedIdentityMismatch)
        ));
        assert!(!target.exists());
        assert_eq!(
            fs::read(root.join("target-verified-displaced").join("source.txt"))
                .expect("bound target"),
            b"original"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unicode_and_long_paths_commit_through_bound_handles() {
        let _serial = lock_claim_test_hooks();
        let root = fixture("unicode-long-path");
        let mut parent = root.clone();
        for index in 0..6 {
            parent.push(format!(
                "segment-{index}-abcdefghijklmnopqrstuvwxyz0123456789"
            ));
        }
        crate::fs_safety::create_directory_chain_no_links(&parent).expect("long parent");
        let source = parent.join("源-данные-α.txt");
        let target = parent.join("目标-результат-β.txt");
        fs::write(&source, b"unicode long path").expect("source");
        atomic_move_noreplace(&source, &target, None, None).expect("bound long-path move");
        assert_eq!(fs::read(&target).expect("target"), b"unicode long path");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unc_paths_are_encoded_with_extended_unc_prefix() {
        let wide = windows_wide_path(Path::new(r"\\server\share\目录\file.txt"));
        let prefix = "\\\\?\\UNC\\".encode_utf16().collect::<Vec<_>>();
        assert!(wide.starts_with(&prefix));
        assert_eq!(wide.last(), Some(&0));
    }

    #[cfg(windows)]
    #[test]
    fn claim_identity_mismatch_does_not_move_replacement_claim_to_original() {
        let _serial = lock_claim_test_hooks();
        let root = fixture("claim-mismatch");
        let source = root.join("source.txt");
        fs::write(&source, b"original").expect("source");
        let expected = identity::capture_identity(&source, None).expect("identity");
        let claim_path = planned_claim_path(&source, "mismatch").expect("claim path");
        set_claim_test_hook(Some(replace_claim_before_identity_check));
        let result = claim_source_at(&source, &expected, &claim_path, "mismatch", None);
        set_claim_test_hook(None);

        assert!(matches!(
            result,
            Err(SourceClaimError::ClaimRollbackFailed(_))
        ));
        assert!(!source.exists());
        assert_eq!(
            fs::read(&claim_path).expect("replacement claim"),
            b"replacement claim"
        );
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod mac_tests {
    use super::*;
    use crate::fs_safety::atomic_move::AtomicMoveError;
    use std::fs;

    fn rebind_claim_after_verified(point: ClaimTestPoint, _source: &Path, claim: &Path) {
        if !matches!(
            point,
            ClaimTestPoint::AfterClaimVerifiedBeforeCommit
                | ClaimTestPoint::AfterClaimVerifiedBeforeDelete
                | ClaimTestPoint::AfterClaimVerifiedBeforeRollback
        ) {
            return;
        }
        let metadata = fs::symlink_metadata(claim).expect("claim metadata");
        let saved = claim.with_file_name(".zen-canvas-attacker-save");
        fs::rename(claim, &saved).expect("attacker saves the claimed object");
        if metadata.is_dir() {
            fs::create_dir(claim).expect("attacker replacement directory");
        } else if metadata.file_type().is_symlink() {
            std::os::unix::fs::symlink("attacker-target", claim)
                .expect("attacker replacement symlink");
        } else {
            fs::write(claim, b"attacker replacement").expect("attacker replacement file");
        }
    }

    #[test]
    fn macos_source_claim_can_rollback_by_identity() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-source-claim-macos-parity-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        let source = root.join("source.txt");
        let claim_path = planned_claim_path(&source, "parity").expect("claim path");
        fs::write(&source, b"source").expect("source");
        let expected = identity::capture_identity(&source, None).expect("identity");

        let mut claim = claim_source_at(&source, &expected, &claim_path, "parity", None)
            .expect("macOS parity claim should bind the source");

        assert_eq!(claim.current_path(), claim_path.as_path());
        assert!(!source.exists());
        assert_eq!(fs::read(&claim_path).expect("claim bytes"), b"source");

        claim
            .rollback_to_original()
            .expect("claim should roll back to the original path");

        assert_eq!(fs::read(&source).expect("source bytes"), b"source");
        assert!(!claim_path.exists());
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn macos_planned_claim_uses_private_mode_0700_namespace() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "zen-canvas-source-claim-macos-private-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        let source = root.join("source.txt");
        fs::write(&source, b"source").expect("source");
        let claim_path = planned_claim_path(&source, "private").expect("claim path");
        let canonical_root = root.canonicalize().expect("canonical fixture root");
        let relative = claim_path
            .strip_prefix(&canonical_root)
            .expect("claim stays below source parent");
        let components = relative
            .components()
            .map(|component| component.as_os_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(components.len(), 3);
        assert_eq!(
            components[0],
            std::ffi::OsStr::new(MAC_RETIREMENT_ROOT_NAME)
        );
        assert!(!components[1].is_empty());
        assert!(components[2]
            .as_encoded_bytes()
            .starts_with(b".zen-canvas-claim-"));
        assert!(!claim_path.exists());

        let expected = identity::capture_identity(&source, None).expect("identity");
        let mut claim = claim_source_at(&source, &expected, &claim_path, "private", None)
            .expect("private claim");
        let retirement_root = canonical_root.join(MAC_RETIREMENT_ROOT_NAME);
        let session = retirement_root.join(&components[1]);
        assert_eq!(
            fs::symlink_metadata(&retirement_root)
                .expect("retirement root metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::symlink_metadata(&session)
                .expect("session metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        claim.rollback_to_original().expect("rollback claim");
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn macos_claim_rebinding_before_commit_is_manual_and_never_moves_replacement() {
        let _serial = lock_claim_test_hooks();
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-source-claim-macos-rebind-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        let claim_path = planned_claim_path(&source, "rebind").expect("claim path");
        fs::write(&source, b"source").expect("source");
        let expected =
            identity::capture_namespace_identity_only(&source, None).expect("namespace identity");
        let mut claim =
            claim_source_at(&source, &expected, &claim_path, "rebind", None).expect("claim");
        set_claim_test_hook(Some(rebind_claim_after_verified));
        let target_parent = VerifiedDirectory::open_existing(&root).expect("target parent");
        let result = claim.commit_to_with_cancel(
            target_parent,
            target.file_name().expect("target name"),
            None,
        );
        set_claim_test_hook(None);

        assert!(matches!(
            result,
            Err(SourceClaimError::MacClaimNamespaceRebound)
        ));
        assert!(!target.exists(), "replacement must not be committed");
        assert_eq!(
            fs::read(claim_path.with_file_name(".zen-canvas-attacker-save"))
                .expect("saved original"),
            b"source"
        );
        assert_eq!(
            fs::read(&claim_path).expect("replacement remains"),
            b"attacker replacement"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn macos_permanent_delete_rebinding_never_deletes_attacker_object() {
        let _serial = lock_claim_test_hooks();
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-source-claim-macos-delete-rebind-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        let source = root.join("source.txt");
        let claim_path = planned_claim_path(&source, "delete").expect("claim path");
        fs::write(&source, b"source").expect("source");
        let expected =
            identity::capture_namespace_identity_only(&source, None).expect("namespace identity");
        let mut claim =
            claim_source_at(&source, &expected, &claim_path, "delete", None).expect("claim");
        set_claim_test_hook(Some(rebind_claim_after_verified));
        let result = claim.delete_claim();
        set_claim_test_hook(None);

        assert!(matches!(
            result,
            Err(SourceClaimError::MacClaimNamespaceRebound)
        ));
        assert_eq!(
            fs::read(claim_path.with_file_name(".zen-canvas-attacker-save"))
                .expect("saved original"),
            b"source"
        );
        assert_eq!(
            fs::read(&claim_path).expect("replacement remains"),
            b"attacker replacement"
        );
        let mapped = result.map_err(crate::fs_safety::atomic_move::map_claim_error);
        assert!(matches!(
            mapped,
            Err(AtomicMoveError::MacClaimNamespaceRebound)
        ));
        let _ = fs::remove_dir_all(root);
    }
}
