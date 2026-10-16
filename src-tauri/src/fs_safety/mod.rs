pub mod atomic_move;
#[cfg(windows)]
pub mod copy_commit;
pub mod identity;
pub mod path_guard;
pub mod physical;
pub mod platform_support;
pub mod source_claim;
pub mod verified_directory;

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

pub(crate) type PhaseObserver<'a> = dyn FnMut(&str) -> Result<(), AtomicMoveError> + 'a;

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
