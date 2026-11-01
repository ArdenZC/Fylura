pub mod atomic_move;
#[cfg(windows)]
pub mod copy_commit;
pub mod identity;
pub mod path_guard;
pub mod physical;
pub mod platform_support;
pub mod source_claim;
pub mod verified_directory;

use std::time::{Duration, Instant};

pub use atomic_move::{
    atomic_move_noreplace, atomic_move_noreplace_with_claim_path, AtomicMoveCommitState,
    AtomicMoveError, AtomicMoveMethod, AtomicMoveOutcome,
};
#[cfg(any(test, feature = "native-qa"))]
pub use atomic_move::{
    atomic_move_noreplace_for_test_operation, atomic_permanent_delete_for_test,
    atomic_permanent_delete_for_test_with_hook, atomic_replace_for_test, AtomicMoveTestOperation,
};
#[cfg(target_os = "macos")]
pub(crate) use identity::capture_identity_from_handle;
pub use identity::{
    capture_identity, capture_namespace_identity, capture_namespace_identity_only,
    identity_matches, recovery_identity_matches, ContentVerificationIdentity, ExpectedFileIdentity,
    IdentityError, NamespaceIdentity,
};
pub use path_guard::{create_directory_chain_no_links, PathGuardError};
pub use physical::{
    capture_physical_identity, capture_source_object_identity, PhysicalFileIdentity,
    PhysicalIdentityError, PhysicalPlatform,
};
pub use platform_support::{
    ensure_supported_cleanup_mutation, ensure_supported_file_mutation, PlatformSupportError,
    MACOS_FILE_MUTATION_SOURCE_BINDING_UNSUPPORTED, UNSUPPORTED_PLATFORM_LINUX,
};
pub use source_claim::{
    claim_source, claim_source_at, planned_claim_path, ClaimedEntryKind, SourceClaim,
    SourceClaimError,
};
pub use verified_directory::{DirectoryIdentity, VerifiedDirectory};

#[derive(Debug, Clone)]
pub(crate) enum FileOperationEvent {
    Phase(String),
    Progress(FileOperationProgress),
}

#[derive(Debug, Clone)]
pub(crate) struct FileOperationProgress {
    pub(crate) stage: String,
    pub(crate) current_path: String,
    pub(crate) processed_items: u64,
    pub(crate) total_items: Option<u64>,
    pub(crate) processed_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
}

pub(crate) type PhaseObserver<'a> =
    dyn FnMut(FileOperationEvent) -> Result<(), AtomicMoveError> + 'a;

/// Throttled counters for the active phase of a recursive transfer. A total
/// stays absent until a bounded source pass has actually established it.
pub(crate) struct DirectoryTransferProgress {
    stage: String,
    processed_items: u64,
    total_items: Option<u64>,
    processed_bytes: u64,
    total_bytes: Option<u64>,
    current_path: String,
    last_emit_at: Instant,
    last_emitted_items: u64,
    last_emitted_bytes: u64,
}

impl DirectoryTransferProgress {
    pub(crate) fn new(stage: &str) -> Self {
        Self {
            stage: stage.to_string(),
            processed_items: 0,
            total_items: None,
            processed_bytes: 0,
            total_bytes: None,
            current_path: String::new(),
            last_emit_at: Instant::now(),
            last_emitted_items: 0,
            last_emitted_bytes: 0,
        }
    }

    pub(crate) fn reset(
        &mut self,
        stage: &str,
        total_items: Option<u64>,
        total_bytes: Option<u64>,
        observer: &mut Option<&mut PhaseObserver<'_>>,
    ) -> Result<(), AtomicMoveError> {
        self.stage = stage.to_string();
        self.processed_items = 0;
        self.total_items = total_items;
        self.processed_bytes = 0;
        self.total_bytes = total_bytes;
        self.current_path.clear();
        self.last_emitted_items = 0;
        self.last_emitted_bytes = 0;
        self.last_emit_at = Instant::now();
        self.emit(observer)
    }

    pub(crate) fn record_item(
        &mut self,
        path: &std::path::Path,
        observer: &mut Option<&mut PhaseObserver<'_>>,
    ) -> Result<(), AtomicMoveError> {
        self.processed_items = self.processed_items.saturating_add(1);
        self.current_path = path.to_string_lossy().into_owned();
        self.emit_if_due(observer)
    }

    pub(crate) fn record_bytes(
        &mut self,
        path: &std::path::Path,
        bytes: u64,
        observer: &mut Option<&mut PhaseObserver<'_>>,
    ) -> Result<(), AtomicMoveError> {
        self.processed_bytes = self.processed_bytes.saturating_add(bytes);
        self.current_path = path.to_string_lossy().into_owned();
        self.emit_if_due(observer)
    }

    pub(crate) fn finish(
        &mut self,
        observer: &mut Option<&mut PhaseObserver<'_>>,
    ) -> Result<(), AtomicMoveError> {
        self.emit(observer)
    }

    pub(crate) fn set_total_items(&mut self, total_items: Option<u64>) {
        self.total_items = total_items;
    }

    pub(crate) fn total_items(&self) -> Option<u64> {
        self.total_items
    }

    pub(crate) fn processed_items(&self) -> u64 {
        self.processed_items
    }

    fn emit_if_due(
        &mut self,
        observer: &mut Option<&mut PhaseObserver<'_>>,
    ) -> Result<(), AtomicMoveError> {
        if self.processed_items.saturating_sub(self.last_emitted_items) >= 10
            || self.processed_bytes.saturating_sub(self.last_emitted_bytes) >= 8 * 1024 * 1024
            || self.last_emit_at.elapsed() >= Duration::from_millis(200)
        {
            self.emit(observer)?;
        }
        Ok(())
    }

    fn emit(
        &mut self,
        observer: &mut Option<&mut PhaseObserver<'_>>,
    ) -> Result<(), AtomicMoveError> {
        if let Some(observer) = observer.as_deref_mut() {
            observer(FileOperationEvent::Progress(FileOperationProgress {
                stage: self.stage.clone(),
                current_path: self.current_path.clone(),
                processed_items: self.processed_items,
                total_items: self.total_items,
                processed_bytes: self.processed_bytes,
                total_bytes: self.total_bytes,
            }))?;
        }
        self.last_emit_at = Instant::now();
        self.last_emitted_items = self.processed_items;
        self.last_emitted_bytes = self.processed_bytes;
        Ok(())
    }
}

/// Receives the URLs selected by a native coordination accessor before the
/// filesystem claim starts.  The optional claim path is the path that the
/// uncoordinated primitive will actually use after rebinding it to the
/// accessor-supplied source parent.  Returning an error is deliberate: the
/// durable journal must accept those paths before the namespace transaction
/// advances past its prepared boundary.
pub(crate) type ActualPathObserver<'a> = dyn FnMut(
        &std::path::Path,
        &std::path::Path,
        Option<&std::path::Path>,
    ) -> Result<(), AtomicMoveError>
    + 'a;
