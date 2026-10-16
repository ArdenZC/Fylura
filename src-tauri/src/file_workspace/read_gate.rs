//! Bounded W1-07 materialization/read gate.
//!
//! This module adapts the existing platform byte-read authority to the
//! opaque content-read lease contract from Preview Core.  It owns only
//! process-local, bounded lease state.  It does not persist leases, resolve
//! renderer paths, materialize provider content, or perform filesystem
//! mutation.

use super::{
    browse::{BrowseError, BrowseService},
    contracts::{BrowseEntryRef, ContentReadEligibility, ContentReadLeaseRef, PreviewSourceRef},
    preview::{
        BoundedContentRead, BoundedContentReadRequest, ContentReadAccessError,
        ContentReadLeaseConsumer, PreviewContentReadAccess, PreviewContextError,
        PreviewOperationContext, PreviewReadAccessError,
    },
};
use crate::{
    db::Database,
    fs_safety::{capture_physical_identity, PhysicalFileIdentity, PhysicalIdentityError},
};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use thiserror::Error;

#[cfg(not(target_os = "macos"))]
use std::fs::OpenOptions;

const MAX_TOKEN_LENGTH: usize = 256;
const DEFAULT_MAX_ACTIVE_LEASES: usize = 128;
const DEFAULT_MAX_READ_BYTES: u32 = 1024 * 1024;
const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(30);

/// The reason a byte consumer wants access to a source.
///
/// `MetadataOnly` is intentionally not a byte-read intent.  It is retained
/// as an explicit rejected value so a caller cannot accidentally turn a
/// metadata probe into a lease request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadIntent {
    Preview,
    Thumbnail,
    ContentAnalysis,
    Hashing,
    MetadataOnly,
}

impl ReadIntent {
    const fn requires_bytes(self) -> bool {
        !matches!(self, Self::MetadataOnly)
    }
}

/// W1 policy: content is never materialized implicitly.  A future explicit
/// user action may cross this boundary through the existing materialization
/// authority; this gate does not execute that action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializationPolicy {
    NeverImplicit,
    UserInitiatedOnly,
}

/// Bounds for the process-local lease registry and every individual read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadGateConfig {
    pub max_active_leases: usize,
    pub max_read_bytes: u32,
    pub lease_ttl: Duration,
    pub materialization_policy: MaterializationPolicy,
}

impl Default for ReadGateConfig {
    fn default() -> Self {
        Self {
            max_active_leases: DEFAULT_MAX_ACTIVE_LEASES,
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
            lease_ttl: DEFAULT_LEASE_TTL,
            materialization_policy: MaterializationPolicy::UserInitiatedOnly,
        }
    }
}

impl ReadGateConfig {
    #[allow(dead_code)]
    fn validate(self) -> Result<Self, ReadGateConfigError> {
        if self.max_active_leases == 0 {
            return Err(ReadGateConfigError::ZeroLeaseCapacity);
        }
        if self.max_read_bytes == 0 {
            return Err(ReadGateConfigError::ZeroReadLimit);
        }
        if self.lease_ttl.is_zero() || Instant::now().checked_add(self.lease_ttl).is_none() {
            return Err(ReadGateConfigError::InvalidLeaseLifetime);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReadGateConfigError {
    #[error("content read lease capacity must be non-zero")]
    ZeroLeaseCapacity,
    #[error("content read limit must be non-zero")]
    ZeroReadLimit,
    #[error("content read lease lifetime is invalid")]
    InvalidLeaseLifetime,
}

/// Errors returned while issuing or disposing a bounded lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReadGateError {
    #[error("content source is unavailable")]
    SourceUnavailable,
    #[error("content source permission was denied")]
    PermissionDenied,
    #[error("content materialization is required")]
    MaterializationRequired,
    #[error("content source is downloading")]
    Downloading,
    #[error("content source is metadata-only")]
    MetadataOnly,
    #[error("content source is not supported")]
    SourceNotSupported,
    #[error("content source package cannot be read")]
    PackageUnsupported,
    #[error("content source is a symlink or reparse point")]
    Symlink,
    #[error("content source availability is unknown")]
    AvailabilityUnknown,
    #[error("content source identity changed")]
    IdentityChanged,
    #[error("content read lease is invalid")]
    LeaseInvalid,
    #[error("content read request is invalid")]
    InvalidRequest,
    #[error("content read lease registry is at capacity")]
    LeaseCapacityExceeded,
    #[error("content read gate is disposed")]
    Disposed,
}

/// A backend-only source resolution result.  The path never crosses the
/// public lease or Preview provider boundary.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedContentSource {
    path: PathBuf,
}

impl ResolvedContentSource {
    #[allow(dead_code)]
    pub(crate) fn from_backend_path(path: PathBuf) -> Self {
        Self { path }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum SourceResolutionError {
    Unavailable,
    PermissionDenied,
    NotSupported,
    Unknown,
}

/// Backend source resolution is deliberately an internal seam.  Production
/// callers provide an EntryRef-derived source; only this backend trait may
/// turn it into a private path for the authoritative opener.
pub(crate) trait ReadGateSourceResolver: Send + Sync {
    fn resolve_source(
        &self,
        source: &PreviewSourceRef,
    ) -> Result<ResolvedContentSource, SourceResolutionError>;
}

/// The W1-10 wiring adapter for existing managed File Library and Browse
/// authorities.  It does not persist or own either authority.
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct WorkspaceContentSourceResolver {
    database: Database,
    browse: Arc<BrowseService>,
}

impl WorkspaceContentSourceResolver {
    #[allow(dead_code)]
    pub(crate) fn new(database: Database, browse: Arc<BrowseService>) -> Self {
        Self { database, browse }
    }
}

impl ReadGateSourceResolver for WorkspaceContentSourceResolver {
    fn resolve_source(
        &self,
        source: &PreviewSourceRef,
    ) -> Result<ResolvedContentSource, SourceResolutionError> {
        match source {
            PreviewSourceRef::Managed { file_id } => self
                .database
                .resolve_file_library_path(file_id)
                .map(|path| ResolvedContentSource::from_backend_path(PathBuf::from(path)))
                .map_err(|_| SourceResolutionError::Unavailable),
            PreviewSourceRef::Ephemeral {
                browse_session_id,
                entry_id,
            } => {
                let entry_ref = BrowseEntryRef::Ephemeral {
                    browse_session_id: browse_session_id.clone(),
                    entry_id: entry_id.clone(),
                };
                let resolved = self
                    .browse
                    .resolve_entry(&entry_ref)
                    .map_err(map_browse_resolution_error)?;
                Ok(ResolvedContentSource::from_backend_path(resolved.path))
            }
            // Host tokens are owned by a native host integration that is not
            // part of W1-07.  Never interpret them as paths.
            PreviewSourceRef::HostProvided { .. } => Err(SourceResolutionError::NotSupported),
        }
    }
}

#[derive(Debug, Clone)]
struct LeaseRecord {
    request_id: String,
    source: PreviewSourceRef,
    source_version: String,
    intent: ReadIntent,
    expires_at: Instant,
}

#[derive(Debug, Default)]
struct LeaseRegistry {
    disposed: bool,
    leases: HashMap<String, LeaseRecord>,
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The bounded W1-07 read gate.
pub struct MaterializationReadGate {
    resolver: Arc<dyn ReadGateSourceResolver>,
    config: ReadGateConfig,
    leases: Mutex<LeaseRegistry>,
    #[cfg(test)]
    test_eligibility: Mutex<Option<ContentReadEligibility>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VerifiedCopyBounds {
    pub(crate) max_total_bytes: u64,
    pub(crate) chunk_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifiedCopyError {
    Access(PreviewReadAccessError),
    InvalidRequest,
    SourceTooLarge,
}

/// Preview-only adapter over the existing MaterializationReadGate. A provider
/// receives bounded source reads, never the gate's lease issuer or a path. Each
/// call issues one request/sourceVersion-bound Preview lease and releases it
/// before returning; the guard also releases on an early failure or unwind.
pub(crate) struct PreviewReadGateAdapter {
    gate: Arc<MaterializationReadGate>,
    #[cfg(test)]
    before_issue: Option<Arc<PreviewReadGateTestBarrier>>,
    #[cfg(test)]
    after_issue: Option<Arc<PreviewReadGateTestBarrier>>,
}

impl PreviewReadGateAdapter {
    pub(crate) fn new(gate: Arc<MaterializationReadGate>) -> Self {
        Self {
            gate,
            #[cfg(test)]
            before_issue: None,
            #[cfg(test)]
            after_issue: None,
        }
    }

    #[cfg(test)]
    fn new_with_test_controls(
        gate: Arc<MaterializationReadGate>,
        before_issue: Option<Arc<PreviewReadGateTestBarrier>>,
        after_issue: Option<Arc<PreviewReadGateTestBarrier>>,
    ) -> Self {
        Self {
            gate,
            before_issue,
            after_issue,
        }
    }
}

/// Test-owned barriers expose the two lifecycle edges that matter for R0:
/// immediately before fresh lease issue and immediately after the lease is
/// stored, before the bounded read starts. They never exist in production.
#[cfg(test)]
struct PreviewReadGateTestBarrier {
    entered: std::sync::Barrier,
    released: std::sync::Barrier,
}

#[cfg(test)]
impl PreviewReadGateTestBarrier {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: std::sync::Barrier::new(2),
            released: std::sync::Barrier::new(2),
        })
    }

    fn worker_wait(&self) {
        self.entered.wait();
        self.released.wait();
    }

    fn wait_for_entry(&self) {
        self.entered.wait();
    }

    fn release(&self) {
        self.released.wait();
    }
}

struct PreviewReadLeaseGuard {
    gate: Arc<MaterializationReadGate>,
    lease: Option<ContentReadLeaseRef>,
}

impl PreviewReadLeaseGuard {
    fn lease(&self) -> &ContentReadLeaseRef {
        self.lease
            .as_ref()
            .expect("preview read lease guard must own a lease")
    }

    fn release(&mut self) -> Result<(), ReadGateError> {
        let Some(lease) = self.lease.take() else {
            return Ok(());
        };
        self.gate.release_lease(&lease)
    }
}

impl Drop for PreviewReadLeaseGuard {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

struct BorrowedPreviewReadLeaseGuard<'a> {
    gate: &'a MaterializationReadGate,
    lease: Option<ContentReadLeaseRef>,
}

impl BorrowedPreviewReadLeaseGuard<'_> {
    fn lease(&self) -> &ContentReadLeaseRef {
        self.lease
            .as_ref()
            .expect("borrowed preview read lease guard must own a lease")
    }

    fn release(&mut self) -> Result<(), ReadGateError> {
        let Some(lease) = self.lease.take() else {
            return Ok(());
        };
        self.gate.release_lease(&lease)
    }
}

impl Drop for BorrowedPreviewReadLeaseGuard<'_> {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

impl PreviewContentReadAccess for PreviewReadGateAdapter {
    fn read_source_bounded(
        &self,
        source: &PreviewSourceRef,
        source_version: &str,
        request: BoundedContentReadRequest,
        context: &PreviewOperationContext,
    ) -> Result<BoundedContentRead, PreviewReadAccessError> {
        context
            .ensure_active()
            .map_err(map_context_error_to_preview_access)?;
        if source_version.is_empty() || context.source_version() != Some(source_version) {
            return Err(PreviewReadAccessError::SourceVersionMismatch);
        }
        #[cfg(test)]
        if let Some(control) = self.before_issue.as_ref() {
            control.worker_wait();
        }
        let lease = self
            .gate
            .issue_lease(
                context.request_id(),
                source.clone(),
                source_version,
                ReadIntent::Preview,
            )
            .map_err(map_gate_error_to_preview_access)?;
        let mut guard = PreviewReadLeaseGuard {
            gate: Arc::clone(&self.gate),
            lease: Some(lease),
        };
        #[cfg(test)]
        if let Some(control) = self.after_issue.as_ref() {
            control.worker_wait();
        }
        let read = self
            .gate
            .read_bounded_for_preview(guard.lease(), request, context);
        let release = guard.release();
        match read {
            Ok(read) => {
                release.map_err(map_gate_error_to_preview_access)?;
                Ok(read)
            }
            Err(error) => {
                let _ = release;
                Err(error)
            }
        }
    }
}

impl MaterializationReadGate {
    #[allow(dead_code)]
    pub(crate) fn new<R>(
        resolver: Arc<R>,
        config: ReadGateConfig,
    ) -> Result<Self, ReadGateConfigError>
    where
        R: ReadGateSourceResolver + 'static,
    {
        Ok(Self {
            resolver,
            config: config.validate()?,
            leases: Mutex::new(LeaseRegistry::default()),
            #[cfg(test)]
            test_eligibility: Mutex::new(None),
        })
    }

    #[allow(dead_code)]
    pub(crate) fn from_workspace_sources(
        database: Database,
        browse: Arc<BrowseService>,
        config: ReadGateConfig,
    ) -> Result<Self, ReadGateConfigError> {
        Self::new(
            Arc::new(WorkspaceContentSourceResolver::new(database, browse)),
            config,
        )
    }

    /// Return the authoritative lifetime used for every process-local read
    /// lease. Consumers may validate a shorter operation budget against this
    /// fact without taking ownership of Read Gate policy configuration.
    pub(crate) fn lease_ttl(&self) -> Duration {
        self.config.lease_ttl
    }

    /// Project the current authoritative platform result without issuing a
    /// lease.  Errors in opaque source resolution are descriptive fail-closed
    /// states, never a path-based authorization.
    pub fn content_read_eligibility(&self, source: &PreviewSourceRef) -> ContentReadEligibility {
        if self.is_disposed() || validate_source_ref(source).is_err() {
            return ContentReadEligibility::SourceUnavailable;
        }
        #[cfg(test)]
        if let Some(eligibility) = *lock(&self.test_eligibility) {
            return eligibility;
        }
        let resolved = match self.resolver.resolve_source(source) {
            Ok(resolved) => resolved,
            Err(error) => return map_resolution_to_eligibility(error),
        };
        self.classify_source(&resolved.path)
    }

    pub fn eligibility(&self, source: &PreviewSourceRef) -> ContentReadEligibility {
        self.content_read_eligibility(source)
    }

    /// Return the current opaque source version without issuing byte access.
    /// The value contains physical identity facts only; it never contains a
    /// filesystem path.
    pub fn current_source_version(
        &self,
        source: &PreviewSourceRef,
    ) -> Result<String, ReadGateError> {
        self.ensure_usable_source(source)?;
        Ok(self.resolve_eligible(source)?.source_version)
    }

    /// Return only a backend-derived leaf name for adapters that need to keep
    /// a safe extension while staging already-authorized bytes.  The path is
    /// resolved and consumed entirely inside the read-gate authority; no path
    /// crosses the public source/lease contract.
    pub(crate) fn source_file_name(&self, source: &PreviewSourceRef) -> Option<String> {
        self.ensure_usable_source(source).ok()?;
        let resolved = self.resolver.resolve_source(source).ok()?;
        let name = resolved.path.file_name()?.to_str()?.trim();
        (!name.is_empty() && !name.contains('/') && !name.contains('\\')).then(|| name.to_string())
    }

    /// Issue a lease bound to a caller's already-resolved source version.
    /// The version is checked against a fresh backend resolution before the
    /// lease is stored.
    pub fn issue_lease(
        &self,
        request_id: impl Into<String>,
        source: PreviewSourceRef,
        source_version: impl Into<String>,
        intent: ReadIntent,
    ) -> Result<ContentReadLeaseRef, ReadGateError> {
        let request_id = request_id.into();
        let source_version = source_version.into();
        self.validate_request_id(&request_id)?;
        self.ensure_usable_source(&source)?;
        if !intent.requires_bytes() {
            return Err(ReadGateError::InvalidRequest);
        }
        if source_version.is_empty() || source_version.len() > MAX_TOKEN_LENGTH {
            return Err(ReadGateError::InvalidRequest);
        }

        let current = self.resolve_eligible(&source)?;
        if current.source_version != source_version {
            return Err(ReadGateError::IdentityChanged);
        }
        self.store_lease(request_id, source, source_version, intent)
    }

    /// Convenience for backend callers that have not yet published a
    /// PreviewSourceSnapshot.  It still resolves and validates the source
    /// before storing the lease.
    pub fn issue_lease_for_current(
        &self,
        request_id: impl Into<String>,
        source: PreviewSourceRef,
        intent: ReadIntent,
    ) -> Result<ContentReadLeaseRef, ReadGateError> {
        let request_id = request_id.into();
        self.validate_request_id(&request_id)?;
        self.ensure_usable_source(&source)?;
        if !intent.requires_bytes() {
            return Err(ReadGateError::InvalidRequest);
        }
        let current = self.resolve_eligible(&source)?;
        self.store_lease(request_id, source, current.source_version, intent)
    }

    pub fn release_lease(&self, lease: &ContentReadLeaseRef) -> Result<(), ReadGateError> {
        let mut registry = lock(&self.leases);
        if registry.disposed {
            return Err(ReadGateError::Disposed);
        }
        prune_expired(&mut registry.leases);
        let Some(record) = registry.leases.get(&lease.lease_id) else {
            return Err(ReadGateError::LeaseInvalid);
        };
        if record.request_id != lease.request_id {
            return Err(ReadGateError::LeaseInvalid);
        }
        if record.source_version != lease.source_version {
            return Err(ReadGateError::IdentityChanged);
        }
        registry.leases.remove(&lease.lease_id);
        Ok(())
    }

    /// Revoke all process-local leases.  No durable state is created.
    pub fn dispose(&self) -> bool {
        let mut registry = lock(&self.leases);
        if registry.disposed {
            return false;
        }
        registry.disposed = true;
        registry.leases.clear();
        true
    }

    /// Diagnostic count of retained records, including expired leases. Unlike
    /// active_lease_count, sampling must not clean up the owner being measured.
    #[cfg(test)]
    pub(crate) fn retained_lease_count_for_test(&self) -> usize {
        lock(&self.leases).leases.len()
    }

    pub fn active_lease_count(&self) -> usize {
        let mut registry = lock(&self.leases);
        prune_expired(&mut registry.leases);
        registry.leases.len()
    }

    fn ensure_usable_source(&self, source: &PreviewSourceRef) -> Result<(), ReadGateError> {
        if self.is_disposed() {
            return Err(ReadGateError::Disposed);
        }
        validate_source_ref(source)
    }

    fn validate_request_id(&self, request_id: &str) -> Result<(), ReadGateError> {
        if request_id.is_empty() || request_id.len() > MAX_TOKEN_LENGTH {
            return Err(ReadGateError::InvalidRequest);
        }
        if self.is_disposed() {
            return Err(ReadGateError::Disposed);
        }
        Ok(())
    }

    fn is_disposed(&self) -> bool {
        lock(&self.leases).disposed
    }

    fn resolve_eligible(
        &self,
        source: &PreviewSourceRef,
    ) -> Result<EvaluatedSource, ReadGateError> {
        let resolved = self
            .resolver
            .resolve_source(source)
            .map_err(map_resolution_to_error)?;
        let eligibility = self.classify_source(&resolved.path);
        if eligibility != ContentReadEligibility::Eligible {
            return Err(map_eligibility_to_error(eligibility));
        }
        let identity = capture_physical_identity(&resolved.path).map_err(map_identity_error)?;
        let source_version = source_version_for_identity(&identity)?;
        Ok(EvaluatedSource {
            path: resolved.path,
            identity,
            source_version,
        })
    }

    fn classify_source(&self, path: &Path) -> ContentReadEligibility {
        #[cfg(test)]
        if let Some(eligibility) = *lock(&self.test_eligibility) {
            return eligibility;
        }
        classify_path(path)
    }

    #[cfg(test)]
    pub(crate) fn set_test_eligibility(&self, eligibility: Option<ContentReadEligibility>) {
        *lock(&self.test_eligibility) = eligibility;
    }

    fn store_lease(
        &self,
        request_id: String,
        source: PreviewSourceRef,
        source_version: String,
        intent: ReadIntent,
    ) -> Result<ContentReadLeaseRef, ReadGateError> {
        let mut registry = lock(&self.leases);
        if registry.disposed {
            return Err(ReadGateError::Disposed);
        }
        prune_expired(&mut registry.leases);
        if registry.leases.len() >= self.config.max_active_leases {
            return Err(ReadGateError::LeaseCapacityExceeded);
        }
        let lease_id = uuid::Uuid::new_v4().to_string();
        let expires_at = Instant::now()
            .checked_add(self.config.lease_ttl)
            .ok_or(ReadGateError::InvalidRequest)?;
        registry.leases.insert(
            lease_id.clone(),
            LeaseRecord {
                request_id: request_id.clone(),
                source,
                source_version: source_version.clone(),
                intent,
                expires_at,
            },
        );
        Ok(ContentReadLeaseRef {
            lease_id,
            request_id,
            source_version,
        })
    }

    fn lease_is_active(&self, lease_id: &str) -> bool {
        let mut registry = lock(&self.leases);
        if registry.disposed {
            return false;
        }
        prune_expired(&mut registry.leases);
        registry.leases.contains_key(lease_id)
    }

    /// Run the authoritative bounded-read path with a caller-selected public
    /// error mapping. The source is still resolved and opened exactly once
    /// here, after the lease has been issued, so Preview can preserve fresh
    /// eligibility truth without creating a second read implementation.
    fn read_bounded_with_mapping<E>(
        &self,
        lease: &ContentReadLeaseRef,
        request: BoundedContentReadRequest,
        context: &PreviewOperationContext,
        map_context_error: fn(PreviewContextError) -> E,
        map_gate_error: fn(ReadGateError) -> E,
        map_content_error: fn(ContentReadAccessError) -> E,
    ) -> Result<BoundedContentRead, E> {
        context.ensure_active().map_err(map_context_error)?;
        if request.max_bytes == 0 || request.max_bytes > self.config.max_read_bytes {
            return Err(map_content_error(ContentReadAccessError::Failed));
        }
        let max_bytes = u64::from(request.max_bytes);
        let read_limit = max_bytes
            .checked_add(1)
            .ok_or_else(|| map_content_error(ContentReadAccessError::Failed))?;
        request
            .offset_bytes
            .checked_add(read_limit)
            .ok_or_else(|| map_content_error(ContentReadAccessError::Failed))?;

        let record = {
            let mut registry = lock(&self.leases);
            if registry.disposed {
                return Err(map_content_error(ContentReadAccessError::LeaseInvalid));
            }
            prune_expired(&mut registry.leases);
            registry
                .leases
                .get(&lease.lease_id)
                .cloned()
                .ok_or_else(|| map_content_error(ContentReadAccessError::LeaseInvalid))?
        };
        if record.request_id != lease.request_id || record.request_id != context.request_id() {
            return Err(map_content_error(ContentReadAccessError::LeaseInvalid));
        }
        if record.source_version != lease.source_version
            || context.source_version() != Some(record.source_version.as_str())
        {
            return Err(map_content_error(
                ContentReadAccessError::SourceVersionMismatch,
            ));
        }
        if !record.intent.requires_bytes() {
            return Err(map_content_error(ContentReadAccessError::LeaseInvalid));
        }
        context.ensure_active().map_err(map_context_error)?;

        // Re-resolve the opaque source for every bounded read. The lease is
        // not a cached path or a durable open handle. In particular, the
        // caller-selected gate mapping preserves any fresh terminal state.
        let current = self
            .resolve_eligible(&record.source)
            .map_err(map_gate_error)?;
        if current.source_version != record.source_version {
            return Err(map_content_error(
                ContentReadAccessError::SourceVersionMismatch,
            ));
        }
        context.ensure_active().map_err(map_context_error)?;

        let mut file = open_authoritative_file(&current.path, &current.identity)
            .map_err(map_open_error_to_access)
            .map_err(map_content_error)?;
        context.ensure_active().map_err(map_context_error)?;
        file.seek(SeekFrom::Start(request.offset_bytes))
            .map_err(map_io_error_to_access)
            .map_err(map_content_error)?;
        let mut bytes = Vec::new();
        file.take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(map_io_error_to_access)
            .map_err(map_content_error)?;
        context.ensure_active().map_err(map_context_error)?;

        // Release/dispose/expiry during the read revokes publication. The
        // local read may have finished, but no bytes are returned to a caller.
        if !self.lease_is_active(&lease.lease_id) {
            return Err(map_content_error(ContentReadAccessError::LeaseInvalid));
        }
        let complete = bytes.len() <= usize::try_from(max_bytes).unwrap_or(usize::MAX);
        if !complete {
            bytes.truncate(usize::try_from(max_bytes).unwrap_or(usize::MAX));
        }
        Ok(BoundedContentRead { bytes, complete })
    }

    fn read_bounded_for_preview(
        &self,
        lease: &ContentReadLeaseRef,
        request: BoundedContentReadRequest,
        context: &PreviewOperationContext,
    ) -> Result<BoundedContentRead, PreviewReadAccessError> {
        self.read_bounded_with_mapping(
            lease,
            request,
            context,
            map_context_error_to_preview_access,
            map_gate_error_to_preview_access,
            map_content_read_to_preview_access,
        )
    }

    /// Stream one complete, identity-checked source snapshot through the
    /// existing Read Gate. The source is fresh-resolved and opened exactly
    /// once; the opened file and source path never leave this authority.
    ///
    /// `lease_check` is the caller's request-owned lease boundary. It is
    /// checked alongside the Preview context and the Read Gate lease so a
    /// staging owner can revoke a copy while it is in progress without
    /// creating another read authority.
    pub(crate) fn stream_verified_source_to_writer<W>(
        &self,
        source: &PreviewSourceRef,
        expected_source_version: &str,
        context: &PreviewOperationContext,
        bounds: VerifiedCopyBounds,
        lease_check: impl Fn() -> Result<(), PreviewReadAccessError>,
        writer: &mut W,
    ) -> Result<u64, VerifiedCopyError>
    where
        W: Write + ?Sized,
    {
        context
            .ensure_active()
            .map_err(map_context_error_to_preview_access)
            .map_err(VerifiedCopyError::Access)?;
        if expected_source_version.is_empty()
            || expected_source_version.len() > MAX_TOKEN_LENGTH
            || context.source_version() != Some(expected_source_version)
        {
            return Err(VerifiedCopyError::Access(
                PreviewReadAccessError::SourceVersionMismatch,
            ));
        }
        if bounds.max_total_bytes == 0
            || bounds.chunk_bytes == 0
            || bounds.chunk_bytes > self.config.max_read_bytes
        {
            return Err(VerifiedCopyError::InvalidRequest);
        }
        lease_check().map_err(VerifiedCopyError::Access)?;
        self.ensure_usable_source(source)
            .map_err(map_gate_error_to_preview_access)
            .map_err(VerifiedCopyError::Access)?;

        let current = self
            .resolve_eligible(source)
            .map_err(map_gate_error_to_preview_access)
            .map_err(VerifiedCopyError::Access)?;
        if current.source_version != expected_source_version {
            return Err(VerifiedCopyError::Access(
                PreviewReadAccessError::SourceVersionMismatch,
            ));
        }
        if current.identity.size > bounds.max_total_bytes {
            return Err(VerifiedCopyError::SourceTooLarge);
        }

        let lease = self
            .store_lease(
                context.request_id().to_string(),
                source.clone(),
                expected_source_version.to_string(),
                ReadIntent::Preview,
            )
            .map_err(map_gate_error_to_preview_access)
            .map_err(VerifiedCopyError::Access)?;
        let mut lease_guard = BorrowedPreviewReadLeaseGuard {
            gate: self,
            lease: Some(lease),
        };

        context
            .ensure_active()
            .map_err(map_context_error_to_preview_access)
            .map_err(VerifiedCopyError::Access)?;
        lease_check().map_err(VerifiedCopyError::Access)?;

        let mut file = open_authoritative_file(&current.path, &current.identity)
            .map_err(map_open_error_to_access)
            .map_err(map_content_read_to_preview_access)
            .map_err(VerifiedCopyError::Access)?;
        let mut copied = 0_u64;
        let mut buffer = vec![0_u8; usize::try_from(bounds.chunk_bytes).unwrap_or(usize::MAX)];
        while copied < current.identity.size {
            context
                .ensure_active()
                .map_err(map_context_error_to_preview_access)
                .map_err(VerifiedCopyError::Access)?;
            lease_check().map_err(VerifiedCopyError::Access)?;
            if !self.lease_is_active(lease_guard.lease().lease_id.as_str()) {
                return Err(VerifiedCopyError::Access(
                    PreviewReadAccessError::LeaseInvalid,
                ));
            }

            let remaining = current.identity.size - copied;
            let read_len = remaining.min(u64::from(bounds.chunk_bytes)) as usize;
            let read = file
                .read(&mut buffer[..read_len])
                .map_err(map_io_error_to_access)
                .map_err(map_content_read_to_preview_access)
                .map_err(VerifiedCopyError::Access)?;
            if read == 0 {
                return Err(VerifiedCopyError::Access(
                    PreviewReadAccessError::SourceVersionMismatch,
                ));
            }
            copied = copied
                .checked_add(read as u64)
                .ok_or(VerifiedCopyError::SourceTooLarge)?;
            if copied > bounds.max_total_bytes {
                return Err(VerifiedCopyError::SourceTooLarge);
            }
            writer
                .write_all(&buffer[..read])
                .map_err(|_| VerifiedCopyError::Access(PreviewReadAccessError::Failed))?;

            context
                .ensure_active()
                .map_err(map_context_error_to_preview_access)
                .map_err(VerifiedCopyError::Access)?;
            lease_check().map_err(VerifiedCopyError::Access)?;
            if !self.lease_is_active(lease_guard.lease().lease_id.as_str()) {
                return Err(VerifiedCopyError::Access(
                    PreviewReadAccessError::LeaseInvalid,
                ));
            }
        }

        context
            .ensure_active()
            .map_err(map_context_error_to_preview_access)
            .map_err(VerifiedCopyError::Access)?;
        lease_check().map_err(VerifiedCopyError::Access)?;
        if !self.lease_is_active(lease_guard.lease().lease_id.as_str()) {
            return Err(VerifiedCopyError::Access(
                PreviewReadAccessError::LeaseInvalid,
            ));
        }
        lease_guard
            .release()
            .map_err(map_gate_error_to_preview_access)
            .map_err(VerifiedCopyError::Access)?;
        Ok(copied)
    }
}

impl ContentReadLeaseConsumer for MaterializationReadGate {
    fn read_bounded(
        &self,
        lease: &ContentReadLeaseRef,
        request: BoundedContentReadRequest,
        context: &PreviewOperationContext,
    ) -> Result<BoundedContentRead, ContentReadAccessError> {
        self.read_bounded_with_mapping(
            lease,
            request,
            context,
            map_context_error_to_content_access,
            map_gate_error_to_content_access,
            identity_content_read_error,
        )
    }
}

#[derive(Debug, Clone)]
struct EvaluatedSource {
    path: PathBuf,
    identity: PhysicalFileIdentity,
    source_version: String,
}

fn prune_expired(leases: &mut HashMap<String, LeaseRecord>) {
    let now = Instant::now();
    leases.retain(|_, record| record.expires_at > now);
}

fn validate_source_ref(source: &PreviewSourceRef) -> Result<(), ReadGateError> {
    let valid = match source {
        PreviewSourceRef::Managed { file_id } => valid_token(file_id),
        PreviewSourceRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => valid_token(browse_session_id) && valid_token(entry_id),
        PreviewSourceRef::HostProvided { host_token } => valid_token(host_token),
    };
    if valid {
        Ok(())
    } else {
        Err(ReadGateError::InvalidRequest)
    }
}

fn valid_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TOKEN_LENGTH
}

#[allow(dead_code)]
fn map_browse_resolution_error(error: BrowseError) -> SourceResolutionError {
    match error {
        BrowseError::EntryPermissionDenied | BrowseError::DirectoryPermissionDenied => {
            SourceResolutionError::PermissionDenied
        }
        BrowseError::UnsupportedEntry
        | BrowseError::InvalidEntryRef
        | BrowseError::InvalidPathRef
        | BrowseError::InvalidLocationRef => SourceResolutionError::NotSupported,
        BrowseError::EntryNotFound
        | BrowseError::SessionNotFound
        | BrowseError::EntryUnavailable
        | BrowseError::DirectoryNotFound
        | BrowseError::DirectoryUnavailable
        | BrowseError::StaleEnumeration
        | BrowseError::StalePublication
        | BrowseError::Cancelled => SourceResolutionError::Unavailable,
        BrowseError::StateUnavailable
        | BrowseError::InvalidRequest
        | BrowseError::InvalidCursor
        | BrowseError::InvalidPageSize
        | BrowseError::InvalidLimits
        | BrowseError::SessionCapacityExceeded
        | BrowseError::TemporaryStateCapacityExceeded
        | BrowseError::TargetNotDirectory => SourceResolutionError::Unknown,
    }
}

fn map_resolution_to_eligibility(error: SourceResolutionError) -> ContentReadEligibility {
    match error {
        SourceResolutionError::Unavailable => ContentReadEligibility::SourceUnavailable,
        SourceResolutionError::PermissionDenied => ContentReadEligibility::PermissionRequired,
        SourceResolutionError::NotSupported => ContentReadEligibility::SourceNotSupported,
        SourceResolutionError::Unknown => ContentReadEligibility::AvailabilityUnknown,
    }
}

fn map_resolution_to_error(error: SourceResolutionError) -> ReadGateError {
    match error {
        SourceResolutionError::Unavailable => ReadGateError::SourceUnavailable,
        SourceResolutionError::PermissionDenied => ReadGateError::PermissionDenied,
        SourceResolutionError::NotSupported => ReadGateError::SourceNotSupported,
        SourceResolutionError::Unknown => ReadGateError::AvailabilityUnknown,
    }
}

fn map_eligibility_to_error(eligibility: ContentReadEligibility) -> ReadGateError {
    match eligibility {
        ContentReadEligibility::Eligible => unreachable!("eligible source has no read error"),
        ContentReadEligibility::MaterializationRequired => ReadGateError::MaterializationRequired,
        ContentReadEligibility::Downloading => ReadGateError::Downloading,
        ContentReadEligibility::MetadataOnly => ReadGateError::MetadataOnly,
        ContentReadEligibility::PermissionRequired => ReadGateError::PermissionDenied,
        ContentReadEligibility::SourceUnavailable => ReadGateError::SourceUnavailable,
        ContentReadEligibility::SourceNotSupported => ReadGateError::SourceNotSupported,
        ContentReadEligibility::PackageUnsupported => ReadGateError::PackageUnsupported,
        ContentReadEligibility::Symlink => ReadGateError::Symlink,
        ContentReadEligibility::IdentityChanged => ReadGateError::IdentityChanged,
        ContentReadEligibility::AvailabilityUnknown => ReadGateError::AvailabilityUnknown,
    }
}

fn map_identity_error(error: PhysicalIdentityError) -> ReadGateError {
    match error {
        PhysicalIdentityError::Missing => ReadGateError::SourceUnavailable,
        PhysicalIdentityError::UnsupportedLink => ReadGateError::Symlink,
        PhysicalIdentityError::UnsupportedType => ReadGateError::SourceNotSupported,
        PhysicalIdentityError::Io(error) => match error.kind() {
            io::ErrorKind::PermissionDenied => ReadGateError::PermissionDenied,
            io::ErrorKind::NotFound => ReadGateError::SourceUnavailable,
            _ => ReadGateError::AvailabilityUnknown,
        },
    }
}

fn source_version_for_identity(identity: &PhysicalFileIdentity) -> Result<String, ReadGateError> {
    let physical_key = identity
        .physical_key
        .as_deref()
        .ok_or(ReadGateError::AvailabilityUnknown)?;
    Ok(format!(
        "read-source-v1:{physical_key}:{}:{}",
        identity.size,
        identity.modified_ns.unwrap_or_default()
    ))
}

fn classify_path(path: &Path) -> ContentReadEligibility {
    #[cfg(target_os = "macos")]
    {
        map_macos_eligibility(
            crate::platform::macos::file_semantics::content_read_eligibility(path),
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => return map_metadata_error_to_eligibility(error),
        };
        if metadata.file_type().is_symlink() {
            return ContentReadEligibility::Symlink;
        }
        #[cfg(windows)]
        if is_windows_reparse_point(&metadata) {
            // W1-07 has no independent Windows Cloud Files/provider identity
            // authority.  Reparse-backed sources therefore fail closed.
            return ContentReadEligibility::SourceNotSupported;
        }
        if !metadata.is_file() {
            return ContentReadEligibility::SourceNotSupported;
        }
        match capture_physical_identity(path) {
            Ok(identity) if identity.physical_key.is_some() => ContentReadEligibility::Eligible,
            Ok(_) => ContentReadEligibility::AvailabilityUnknown,
            Err(error) => map_identity_error_to_eligibility(error),
        }
    }
}

#[cfg(target_os = "macos")]
fn map_macos_eligibility(
    eligibility: crate::platform::macos::MacContentReadEligibility,
) -> ContentReadEligibility {
    use crate::platform::macos::MacContentReadEligibility;

    match eligibility {
        MacContentReadEligibility::Eligible => ContentReadEligibility::Eligible,
        MacContentReadEligibility::Symlink => ContentReadEligibility::Symlink,
        MacContentReadEligibility::NonRegular
        | MacContentReadEligibility::ContentSourceNotSupported => {
            ContentReadEligibility::SourceNotSupported
        }
        MacContentReadEligibility::PackageUnsupported => ContentReadEligibility::PackageUnsupported,
        MacContentReadEligibility::ICloudItemNotLocal
        | MacContentReadEligibility::FileProviderItemNotLocal => {
            ContentReadEligibility::MaterializationRequired
        }
        MacContentReadEligibility::ICloudLocalReadDeferred
        | MacContentReadEligibility::MetadataOnly => ContentReadEligibility::MetadataOnly,
        MacContentReadEligibility::CloudDownloading => ContentReadEligibility::Downloading,
        MacContentReadEligibility::PermissionRequired => ContentReadEligibility::PermissionRequired,
        MacContentReadEligibility::ContentAvailabilityUnknown => {
            ContentReadEligibility::AvailabilityUnknown
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn map_metadata_error_to_eligibility(error: io::Error) -> ContentReadEligibility {
    match error.kind() {
        io::ErrorKind::PermissionDenied => ContentReadEligibility::PermissionRequired,
        io::ErrorKind::NotFound => ContentReadEligibility::SourceUnavailable,
        _ => ContentReadEligibility::AvailabilityUnknown,
    }
}

#[cfg(not(target_os = "macos"))]
fn map_identity_error_to_eligibility(error: PhysicalIdentityError) -> ContentReadEligibility {
    match error {
        PhysicalIdentityError::Missing => ContentReadEligibility::SourceUnavailable,
        PhysicalIdentityError::UnsupportedLink => ContentReadEligibility::SourceNotSupported,
        PhysicalIdentityError::UnsupportedType => ContentReadEligibility::SourceNotSupported,
        PhysicalIdentityError::Io(error) => map_metadata_error_to_eligibility(error),
    }
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenReadError {
    PermissionDenied,
    SourceUnavailable,
    IdentityChanged,
    #[cfg(target_os = "macos")]
    Unsupported,
    Failed,
}

fn open_authoritative_file(
    path: &Path,
    expected: &PhysicalFileIdentity,
) -> Result<File, OpenReadError> {
    #[cfg(target_os = "macos")]
    let file = crate::platform::macos::file_semantics::open_content_read(path)
        .map_err(map_macos_open_error)?;

    #[cfg(not(target_os = "macos"))]
    let file = open_local_file(path).map_err(map_io_error_to_open)?;

    if !opened_file_matches(&file, expected)? {
        return Err(OpenReadError::IdentityChanged);
    }
    Ok(file)
}

#[cfg(all(not(target_os = "macos"), unix))]
fn open_local_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(all(not(target_os = "macos"), windows))]
fn open_local_file(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(all(not(target_os = "macos"), not(any(unix, windows))))]
fn open_local_file(path: &Path) -> io::Result<File> {
    File::open(path)
}

#[cfg(target_os = "macos")]
fn map_macos_open_error(reason: &'static str) -> OpenReadError {
    match reason {
        "content_permission_required" => OpenReadError::PermissionDenied,
        "content_source_identity_changed" | "mac_provider_url_changed" => {
            OpenReadError::IdentityChanged
        }
        "icloud_item_not_local"
        | "icloud_local_read_deferred"
        | "file_provider_item_not_local"
        | "cloud_item_downloading"
        | "content_metadata_only" => OpenReadError::SourceUnavailable,
        "content_symlink_traversal_blocked"
        | "content_source_not_supported"
        | "package_not_supported" => OpenReadError::Unsupported,
        _ => OpenReadError::Failed,
    }
}

#[cfg(not(target_os = "macos"))]
fn map_io_error_to_open(error: io::Error) -> OpenReadError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => OpenReadError::PermissionDenied,
        io::ErrorKind::NotFound => OpenReadError::SourceUnavailable,
        _ => OpenReadError::Failed,
    }
}

fn opened_file_matches(
    file: &File,
    expected: &PhysicalFileIdentity,
) -> Result<bool, OpenReadError> {
    let metadata = file.metadata().map_err(map_io_error_to_open_generic)?;
    if !metadata.is_file() || metadata.len() != expected.size {
        return Ok(false);
    }
    if let Some(expected_modified) = expected.modified_ns {
        if modified_ns(&metadata) != Some(expected_modified) {
            return Ok(false);
        }
    }

    opened_file_platform_matches(file, &metadata, expected)
}

#[cfg(target_os = "macos")]
fn opened_file_platform_matches(
    file: &File,
    _metadata: &fs::Metadata,
    expected: &PhysicalFileIdentity,
) -> Result<bool, OpenReadError> {
    let opened = crate::platform::macos::identity::MacPhysicalIdentity::from_fd(file)
        .map_err(map_io_error_to_open_generic)?;
    let volume = opened.dev.to_string();
    let file_id = opened.ino.to_string();
    Ok(
        expected.platform_volume_id.as_deref() == Some(volume.as_str())
            && expected.platform_file_id.as_deref() == Some(file_id.as_str())
            && opened.size == expected.size,
    )
}

#[cfg(all(unix, not(target_os = "macos")))]
fn opened_file_platform_matches(
    _file: &File,
    metadata: &fs::Metadata,
    expected: &PhysicalFileIdentity,
) -> Result<bool, OpenReadError> {
    use std::os::unix::fs::MetadataExt;

    let volume = metadata.dev().to_string();
    let file_id = metadata.ino().to_string();
    Ok(
        expected.platform_volume_id.as_deref() == Some(volume.as_str())
            && expected.platform_file_id.as_deref() == Some(file_id.as_str()),
    )
}

#[cfg(windows)]
fn opened_file_platform_matches(
    file: &File,
    metadata: &fs::Metadata,
    expected: &PhysicalFileIdentity,
) -> Result<bool, OpenReadError> {
    use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Ok(false);
    }
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let success = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
    if success == 0 {
        return Err(OpenReadError::Failed);
    }
    let volume = info.dwVolumeSerialNumber.to_string();
    let file_id =
        ((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)).to_string();
    Ok(
        expected.platform_volume_id.as_deref() == Some(volume.as_str())
            && expected.platform_file_id.as_deref() == Some(file_id.as_str()),
    )
}

#[cfg(not(any(unix, windows)))]
fn opened_file_platform_matches(
    _file: &File,
    _metadata: &fs::Metadata,
    _expected: &PhysicalFileIdentity,
) -> Result<bool, OpenReadError> {
    Ok(false)
}

fn modified_ns(metadata: &fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i128::try_from(duration.as_nanos()).ok())
        .and_then(|value| i64::try_from(value).ok())
}

fn map_io_error_to_access(error: io::Error) -> ContentReadAccessError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => ContentReadAccessError::PermissionDenied,
        io::ErrorKind::NotFound => ContentReadAccessError::SourceUnavailable,
        _ => ContentReadAccessError::Failed,
    }
}

fn identity_content_read_error(error: ContentReadAccessError) -> ContentReadAccessError {
    error
}

fn map_io_error_to_open_generic(error: io::Error) -> OpenReadError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => OpenReadError::PermissionDenied,
        io::ErrorKind::NotFound => OpenReadError::SourceUnavailable,
        _ => OpenReadError::Failed,
    }
}

fn map_open_error_to_access(error: OpenReadError) -> ContentReadAccessError {
    match error {
        OpenReadError::PermissionDenied => ContentReadAccessError::PermissionDenied,
        OpenReadError::SourceUnavailable => ContentReadAccessError::SourceUnavailable,
        OpenReadError::IdentityChanged => ContentReadAccessError::SourceVersionMismatch,
        #[cfg(target_os = "macos")]
        OpenReadError::Unsupported => ContentReadAccessError::Failed,
        OpenReadError::Failed => ContentReadAccessError::Failed,
    }
}

fn map_gate_error_to_content_access(error: ReadGateError) -> ContentReadAccessError {
    match error {
        ReadGateError::LeaseInvalid | ReadGateError::Disposed => {
            ContentReadAccessError::LeaseInvalid
        }
        ReadGateError::IdentityChanged => ContentReadAccessError::SourceVersionMismatch,
        ReadGateError::PermissionDenied => ContentReadAccessError::PermissionDenied,
        ReadGateError::SourceUnavailable
        | ReadGateError::MaterializationRequired
        | ReadGateError::Downloading
        | ReadGateError::MetadataOnly => ContentReadAccessError::SourceUnavailable,
        ReadGateError::SourceNotSupported
        | ReadGateError::PackageUnsupported
        | ReadGateError::Symlink
        | ReadGateError::AvailabilityUnknown
        | ReadGateError::InvalidRequest
        | ReadGateError::LeaseCapacityExceeded => ContentReadAccessError::Failed,
    }
}

fn map_gate_error_to_preview_access(error: ReadGateError) -> PreviewReadAccessError {
    match error {
        ReadGateError::LeaseInvalid | ReadGateError::Disposed => {
            PreviewReadAccessError::LeaseInvalid
        }
        ReadGateError::IdentityChanged => PreviewReadAccessError::SourceVersionMismatch,
        ReadGateError::PermissionDenied => PreviewReadAccessError::PermissionDenied,
        ReadGateError::SourceUnavailable | ReadGateError::AvailabilityUnknown => {
            PreviewReadAccessError::SourceUnavailable
        }
        ReadGateError::MaterializationRequired | ReadGateError::Downloading => {
            PreviewReadAccessError::MaterializationRequired
        }
        ReadGateError::MetadataOnly => PreviewReadAccessError::MetadataOnly,
        ReadGateError::SourceNotSupported
        | ReadGateError::PackageUnsupported
        | ReadGateError::Symlink
        | ReadGateError::InvalidRequest
        | ReadGateError::LeaseCapacityExceeded => PreviewReadAccessError::Failed,
    }
}

fn map_content_read_to_preview_access(error: ContentReadAccessError) -> PreviewReadAccessError {
    match error {
        ContentReadAccessError::LeaseInvalid => PreviewReadAccessError::LeaseInvalid,
        ContentReadAccessError::SourceVersionMismatch => {
            PreviewReadAccessError::SourceVersionMismatch
        }
        ContentReadAccessError::PermissionDenied => PreviewReadAccessError::PermissionDenied,
        ContentReadAccessError::SourceUnavailable => PreviewReadAccessError::SourceUnavailable,
        ContentReadAccessError::Cancelled => PreviewReadAccessError::Cancelled,
        ContentReadAccessError::TimedOut => PreviewReadAccessError::TimedOut,
        ContentReadAccessError::Failed => PreviewReadAccessError::Failed,
    }
}

fn map_context_error_to_content_access(
    error: super::preview::PreviewContextError,
) -> ContentReadAccessError {
    match error {
        super::preview::PreviewContextError::Cancelled => ContentReadAccessError::Cancelled,
        super::preview::PreviewContextError::TimedOut => ContentReadAccessError::TimedOut,
        super::preview::PreviewContextError::StalePublication => {
            ContentReadAccessError::SourceVersionMismatch
        }
    }
}

fn map_context_error_to_preview_access(
    error: super::preview::PreviewContextError,
) -> PreviewReadAccessError {
    match error {
        super::preview::PreviewContextError::Cancelled => PreviewReadAccessError::Cancelled,
        super::preview::PreviewContextError::TimedOut => PreviewReadAccessError::TimedOut,
        super::preview::PreviewContextError::StalePublication => {
            PreviewReadAccessError::SourceVersionMismatch
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_workspace::contracts::PreviewHostKind;
    use crate::file_workspace::preview::{
        PreparedPreview, PreviewCapabilities, PreviewCompleteness, PreviewContentReadAccess,
        PreviewHost, PreviewMetadata, PreviewProvider, PreviewProviderDescriptor,
        PreviewProviderEnvironment, PreviewProviderEnvironmentHandle, PreviewProviderError,
        PreviewProviderErrorCode, PreviewProviderRegistry, PreviewProviderResult,
        PreviewReadAccessError, PreviewRepresentation, PreviewRequest, PreviewRunError,
        PreviewSession, PreviewSessionConfig, PreviewSourceSnapshot, PreviewTask,
        PreviewTerminalCondition, PreviewWarning, PreviewWorkBudget, ProviderProbe,
        SourceResolveError, SourceResolver,
    };
    use crate::file_workspace::preview_asset::{
        PreviewAssetReadError, PreviewAssetRegistry, PreviewAssetRequest,
    };
    use crate::file_workspace::preview_providers::production_preview_providers;
    use crate::scheduler::{
        adapters::{
            PreviewArchiveResourceLeaseAdapter, PreviewDecoderAdmissionTestBarrier,
            PreviewDecoderResourceLeaseAdapter,
        },
        PermissiveResourcePolicy, ResourceCapacities, SchedulerConfig, WorkScheduler,
    };
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture {
        root: PathBuf,
    }

    struct ArchiveRevalidationBarrier {
        entered: std::sync::Barrier,
        released: std::sync::Barrier,
    }

    impl ArchiveRevalidationBarrier {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                entered: std::sync::Barrier::new(2),
                released: std::sync::Barrier::new(2),
            })
        }

        fn worker_wait(&self) {
            self.entered.wait();
            self.released.wait();
        }

        fn wait_for_entry(&self) {
            self.entered.wait();
        }

        fn release(&self) {
            self.released.wait();
        }
    }

    type ArchiveRevalidationPause = (usize, Arc<ArchiveRevalidationBarrier>);

    impl Fixture {
        fn new() -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("src-tauri has repository parent")
                .join(".tmp-tests")
                .join("read-gate")
                .join(uuid::Uuid::new_v4().to_string());
            fs::create_dir_all(&root).expect("create read-gate fixture root");
            Self { root }
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.root.join(name);
            fs::write(&path, bytes).expect("write read-gate fixture");
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.root) {
                eprintln!(
                    "read-gate fixture cleanup failed at {:?}: {error}",
                    self.root
                );
            }
        }
    }

    #[derive(Clone)]
    struct TestResolver {
        path: Arc<Mutex<PathBuf>>,
        resolution_error: Arc<Mutex<Option<SourceResolutionError>>>,
        resolve_count: Arc<AtomicUsize>,
        revalidation_barrier: Arc<Mutex<Option<ArchiveRevalidationPause>>>,
    }

    impl TestResolver {
        fn new(path: PathBuf) -> Self {
            Self {
                path: Arc::new(Mutex::new(path)),
                resolution_error: Arc::new(Mutex::new(None)),
                resolve_count: Arc::new(AtomicUsize::new(0)),
                revalidation_barrier: Arc::new(Mutex::new(None)),
            }
        }

        fn replace_path(&self, path: PathBuf) {
            *lock(&self.path) = path;
        }

        fn set_resolution_error(&self, error: Option<SourceResolutionError>) {
            *lock(&self.resolution_error) = error;
        }

        fn resolve_count(&self) -> usize {
            self.resolve_count.load(Ordering::Acquire)
        }

        fn pause_on_resolve_number(
            &self,
            resolve_number: usize,
            barrier: Arc<ArchiveRevalidationBarrier>,
        ) {
            *lock(&self.revalidation_barrier) = Some((resolve_number, barrier));
        }
    }

    impl ReadGateSourceResolver for TestResolver {
        fn resolve_source(
            &self,
            _source: &PreviewSourceRef,
        ) -> Result<ResolvedContentSource, SourceResolutionError> {
            let resolve_number = self.resolve_count.fetch_add(1, Ordering::AcqRel) + 1;
            if let Some((target, barrier)) = lock(&self.revalidation_barrier).clone() {
                if target == resolve_number {
                    barrier.worker_wait();
                }
            }
            if let Some(error) = *lock(&self.resolution_error) {
                return Err(error);
            }
            Ok(ResolvedContentSource::from_backend_path(
                lock(&self.path).clone(),
            ))
        }
    }

    fn source() -> PreviewSourceRef {
        PreviewSourceRef::Managed {
            file_id: "managed-test-file".to_string(),
        }
    }

    fn gate(resolver: Arc<TestResolver>, config: ReadGateConfig) -> Arc<MaterializationReadGate> {
        Arc::new(MaterializationReadGate::new(resolver, config).expect("valid read-gate config"))
    }

    #[derive(Clone)]
    struct StaticPreviewResolver {
        snapshot: PreviewSourceSnapshot,
    }

    impl SourceResolver for StaticPreviewResolver {
        fn resolve(
            &self,
            request: &super::super::preview::PreviewResolveRequest,
            context: &PreviewOperationContext,
        ) -> Result<PreviewSourceSnapshot, SourceResolveError> {
            context.ensure_active().map_err(|error| match error {
                super::super::preview::PreviewContextError::Cancelled => {
                    SourceResolveError::Cancelled
                }
                super::super::preview::PreviewContextError::TimedOut => SourceResolveError::Timeout,
                super::super::preview::PreviewContextError::StalePublication => {
                    SourceResolveError::Cancelled
                }
            })?;
            if request.source != self.snapshot.source {
                return Err(SourceResolveError::SourceMismatch);
            }
            Ok(self.snapshot.clone())
        }
    }

    fn text_snapshot(
        source: PreviewSourceRef,
        source_version: String,
        display_name: &str,
    ) -> PreviewSourceSnapshot {
        PreviewSourceSnapshot::new(
            source,
            source_version,
            PreviewMetadata {
                display_name: display_name.to_string(),
                media_type: Some("text/plain".to_string()),
                extension: Some("txt".to_string()),
                size_bytes: Some(12),
                modified_at_epoch_ms: None,
                materialization: super::super::contracts::MaterializationState::BoundaryReadable,
                read_eligibility: ContentReadEligibility::Eligible,
            },
            PreviewCapabilities {
                can_select_text: true,
                ..PreviewCapabilities::default()
            },
        )
    }

    fn rich_snapshot(
        source: PreviewSourceRef,
        source_version: String,
        display_name: &str,
        extension: &str,
        media_type: &str,
        size_bytes: u64,
    ) -> PreviewSourceSnapshot {
        PreviewSourceSnapshot::new(
            source,
            source_version,
            PreviewMetadata {
                display_name: display_name.to_string(),
                media_type: Some(media_type.to_string()),
                extension: Some(extension.to_string()),
                size_bytes: Some(size_bytes),
                modified_at_epoch_ms: None,
                materialization: super::super::contracts::MaterializationState::BoundaryReadable,
                read_eligibility: ContentReadEligibility::Eligible,
            },
            PreviewCapabilities {
                can_select_text: true,
                ..PreviewCapabilities::default()
            },
        )
    }

    fn archive_fixture_bytes() -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file("folder/file.txt", zip::write::SimpleFileOptions::default())
            .expect("archive file entry");
        std::io::Write::write_all(&mut writer, b"archive metadata payload")
            .expect("archive file payload");
        writer
            .finish()
            .expect("archive fixture finish")
            .into_inner()
    }

    fn archive_snapshot(
        source: PreviewSourceRef,
        source_version: String,
        size_bytes: u64,
    ) -> PreviewSourceSnapshot {
        rich_snapshot(
            source,
            source_version,
            "fixture.zip",
            "zip",
            "application/zip",
            size_bytes,
        )
    }

    fn production_registry() -> Arc<PreviewProviderRegistry> {
        Arc::new(
            PreviewProviderRegistry::new(production_preview_providers())
                .expect("production providers are unique"),
        )
    }

    fn start_production_preview(
        gate: Arc<MaterializationReadGate>,
        snapshot: PreviewSourceSnapshot,
        before_issue: Option<Arc<PreviewReadGateTestBarrier>>,
        after_issue: Option<Arc<PreviewReadGateTestBarrier>>,
    ) -> (PreviewSession, PreviewTask) {
        let source = snapshot.source.clone();
        let session = PreviewSession::new(PreviewSessionConfig::new(
            "preview-race-session",
            "preview-request",
            source,
            PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
        ));
        let adapter: Arc<dyn PreviewContentReadAccess> = Arc::new(
            PreviewReadGateAdapter::new_with_test_controls(gate, before_issue, after_issue),
        );
        let task = session
            .start_with_environment(
                Arc::new(StaticPreviewResolver { snapshot }),
                production_registry(),
                PreviewProviderEnvironmentHandle::with_preview_read(adapter),
            )
            .expect("start preview race");
        (session, task)
    }

    fn start_production_archive_preview(
        gate: Arc<MaterializationReadGate>,
        snapshot: PreviewSourceSnapshot,
    ) -> (PreviewSession, PreviewTask, Arc<WorkScheduler>) {
        let source = snapshot.source.clone();
        let session = PreviewSession::new(PreviewSessionConfig::new(
            "w308-archive-session",
            "w308-archive-request",
            source,
            PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
        ));
        let scheduler = image_scheduler();
        let archive_admission = Arc::new(PreviewArchiveResourceLeaseAdapter::new(Arc::clone(
            &scheduler,
        )));
        let adapter: Arc<dyn PreviewContentReadAccess> =
            Arc::new(PreviewReadGateAdapter::new(gate));
        let environment = PreviewProviderEnvironmentHandle {
            content_read: None,
            preview_read: Some(adapter),
            folder_enumeration: None,
            asset_publisher: None,
            decoder_admission: None,
            archive_admission: Some(archive_admission),
        };
        let task = session
            .start_with_environment(
                Arc::new(StaticPreviewResolver { snapshot }),
                production_registry(),
                environment,
            )
            .expect("start archive preview");
        (session, task, scheduler)
    }

    fn start_production_image_preview(
        gate: Arc<MaterializationReadGate>,
        snapshot: PreviewSourceSnapshot,
        before_issue: Option<Arc<PreviewReadGateTestBarrier>>,
        after_issue: Option<Arc<PreviewReadGateTestBarrier>>,
        assets: Arc<PreviewAssetRegistry>,
        decoder_admission: Arc<PreviewDecoderResourceLeaseAdapter>,
    ) -> (PreviewSession, PreviewTask) {
        let source = snapshot.source.clone();
        let session = PreviewSession::new(PreviewSessionConfig::new(
            "preview-image-session",
            "preview-image-request",
            source,
            PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
        ));
        let adapter: Arc<dyn PreviewContentReadAccess> = Arc::new(
            PreviewReadGateAdapter::new_with_test_controls(gate, before_issue, after_issue),
        );
        let task = session
            .start_with_environment(
                Arc::new(StaticPreviewResolver { snapshot }),
                production_registry(),
                PreviewProviderEnvironmentHandle::with_preview_read_and_asset_publisher_and_decoder(
                    adapter,
                    assets,
                    decoder_admission,
                ),
            )
            .expect("start image preview");
        (session, task)
    }

    struct ReadProvider {
        descriptor: PreviewProviderDescriptor,
        lease: ContentReadLeaseRef,
        request: BoundedContentReadRequest,
        cancel_before_read: bool,
        result: Arc<Mutex<Option<Result<BoundedContentRead, ContentReadAccessError>>>>,
    }

    impl PreviewProvider for ReadProvider {
        fn descriptor(&self) -> &PreviewProviderDescriptor {
            &self.descriptor
        }

        fn probe(
            &self,
            _snapshot: &PreviewSourceSnapshot,
            _context: &PreviewOperationContext,
        ) -> ProviderProbe {
            ProviderProbe::Compatible
        }

        fn prepare(
            &self,
            _snapshot: &PreviewSourceSnapshot,
            _context: &PreviewOperationContext,
        ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
            Ok(Box::new(ReadPrepared {
                lease: self.lease.clone(),
                request: self.request,
                cancel_before_read: self.cancel_before_read,
                result: Arc::clone(&self.result),
            }))
        }
    }

    struct ReadPrepared {
        lease: ContentReadLeaseRef,
        request: BoundedContentReadRequest,
        cancel_before_read: bool,
        result: Arc<Mutex<Option<Result<BoundedContentRead, ContentReadAccessError>>>>,
    }

    impl PreparedPreview for ReadPrepared {
        fn load(
            &mut self,
            context: &PreviewOperationContext,
            environment: PreviewProviderEnvironment<'_>,
        ) -> Result<PreviewProviderResult, PreviewProviderError> {
            if self.cancel_before_read {
                context.cancellation().cancel();
            }
            let read_result = environment
                .content_read
                .map_or(Err(ContentReadAccessError::Failed), |consumer| {
                    consumer.read_bounded(&self.lease, self.request, context)
                });
            let provider_result = read_result.clone().map(|read| PreviewProviderResult {
                representation: PreviewRepresentation::Text {
                    text: String::from_utf8_lossy(&read.bytes).into_owned(),
                    language: None,
                },
                completeness: if read.complete {
                    super::super::preview::PreviewCompleteness::Complete
                } else {
                    super::super::preview::PreviewCompleteness::Partial
                },
                warnings: Vec::new(),
            });
            *lock(&self.result) = Some(read_result);
            provider_result.map_err(|_| PreviewProviderError::Failed)
        }

        fn cleanup(&mut self) {}
    }

    fn read_through_preview(
        gate: Arc<MaterializationReadGate>,
        source: PreviewSourceRef,
        lease: ContentReadLeaseRef,
        request_id: &str,
        request: BoundedContentReadRequest,
        cancel_before_read: bool,
    ) -> Option<Result<BoundedContentRead, ContentReadAccessError>> {
        let snapshot = PreviewSourceSnapshot::new(
            source.clone(),
            lease.source_version.clone(),
            PreviewMetadata {
                display_name: "fixture.bin".to_string(),
                media_type: Some("application/octet-stream".to_string()),
                extension: Some("bin".to_string()),
                size_bytes: Some(5),
                modified_at_epoch_ms: None,
                materialization: super::super::contracts::MaterializationState::Local,
                read_eligibility: ContentReadEligibility::Eligible,
            },
            PreviewCapabilities::all(),
        );
        let resolver = Arc::new(StaticPreviewResolver { snapshot });
        let result = Arc::new(Mutex::new(None));
        let provider = Arc::new(ReadProvider {
            descriptor: PreviewProviderDescriptor::new(
                "read-gate-test",
                1,
                PreviewCapabilities::metadata_fallback(),
                vec![PreviewHostKind::ZenFloating],
                true,
            ),
            lease,
            request,
            cancel_before_read,
            result: Arc::clone(&result),
        });
        let registry =
            Arc::new(PreviewProviderRegistry::new(vec![provider]).expect("test provider registry"));
        let session = PreviewSession::new(PreviewSessionConfig {
            session_id: "read-gate-session".to_string(),
            request: super::super::preview::PreviewRequest {
                request_id: request_id.to_string(),
                source,
            },
            host: PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
            budget: PreviewWorkBudget::default(),
        });
        let _ = session.run_with_environment(
            resolver,
            registry,
            super::super::preview::PreviewProviderEnvironmentHandle::with_content_read(gate),
        );
        let captured = lock(&result).take();
        captured
    }

    #[test]
    fn opaque_lease_contains_no_path_and_preview_can_inject_gate() {
        let fixture = Fixture::new();
        let path = fixture.file("ordinary.bin", b"hello");
        let gate = gate(
            Arc::new(TestResolver::new(path.clone())),
            ReadGateConfig::default(),
        );
        let source = source();
        let lease = gate
            .issue_lease_for_current("request-1", source.clone(), ReadIntent::Preview)
            .expect("issue lease");
        let wire = serde_json::to_value(&lease).expect("serialize opaque lease");
        assert!(!wire.to_string().contains(path.to_string_lossy().as_ref()));
        assert!(wire.get("path").is_none());
        assert_eq!(gate.active_lease_count(), 1);

        // W1-06's existing environment is the only provider seam; the gate is
        // injected as a consumer and no path is passed to the provider.
        let result = read_through_preview(
            Arc::clone(&gate),
            source,
            lease,
            "request-1",
            BoundedContentReadRequest {
                offset_bytes: 0,
                max_bytes: 5,
            },
            false,
        );
        assert!(result.is_some());
    }

    #[test]
    fn preview_read_adapter_binds_and_releases_a_short_lived_lease() {
        let fixture = Fixture::new();
        let path = fixture.file("adapter.txt", b"adapter");
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current source version");
        let cancellation = crate::file_workspace::PreviewCancellation::default();
        let context = PreviewOperationContext::for_backend_content_read(
            "adapter-session",
            "adapter-request",
            source_version.clone(),
            cancellation.clone(),
            Instant::now() + Duration::from_secs(1),
        );
        let adapter = PreviewReadGateAdapter::new(Arc::clone(&gate));
        assert_eq!(gate.active_lease_count(), 0);
        let read = adapter
            .read_source_bounded(
                &source,
                &source_version,
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 7,
                },
                &context,
            )
            .expect("adapter read");
        assert_eq!(read.bytes, b"adapter");
        assert!(read.complete);
        assert_eq!(gate.active_lease_count(), 0);

        let failed = adapter.read_source_bounded(
            &source,
            &source_version,
            BoundedContentReadRequest {
                offset_bytes: 0,
                max_bytes: DEFAULT_MAX_READ_BYTES + 1,
            },
            &context,
        );
        assert_eq!(failed, Err(PreviewReadAccessError::Failed));
        assert_eq!(gate.active_lease_count(), 0);

        cancellation.cancel();
        assert_eq!(
            adapter.read_source_bounded(
                &source,
                &source_version,
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1,
                },
                &context,
            ),
            Err(PreviewReadAccessError::Cancelled)
        );
        assert_eq!(gate.active_lease_count(), 0);
    }

    fn image_fixture_bytes(with_trailing_chunk: bool) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_fn(2, 2, |x, y| {
            Rgba([x as u8, y as u8, 23, 255])
        }));
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, ImageFormat::Png)
            .expect("encode image fixture");
        let mut bytes = output.into_inner();
        if with_trailing_chunk {
            let insert_at = bytes.len().saturating_sub(12);
            let mut chunks = Vec::new();
            for chunk_index in 0..5_u8 {
                let mut data = b"Comment\0".to_vec();
                data.extend(std::iter::repeat_n(chunk_index, 256 * 1024));
                chunks.extend((data.len() as u32).to_be_bytes());
                chunks.extend_from_slice(b"tEXt");
                chunks.extend_from_slice(&data);
                let checksum = png_crc32(&chunks[chunks.len() - data.len() - 4..]);
                chunks.extend(checksum.to_be_bytes());
            }
            bytes.splice(insert_at..insert_at, chunks);
        }
        bytes
    }

    fn png_crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffff_u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0_u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xedb8_8320_u32 & mask);
            }
        }
        !crc
    }

    fn image_scheduler() -> Arc<WorkScheduler> {
        Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ))
    }

    #[test]
    fn w309_pdf_range_asset_uses_real_read_gate_for_large_source() {
        let fixture = Fixture::new();
        let length_bytes = 20 * 1024 * 1024;
        let bytes: Vec<u8> = (0..length_bytes)
            .map(|offset| (offset % 251) as u8)
            .collect();
        let path = fixture.file("large.pdf", &bytes);
        let resolver = Arc::new(TestResolver::new(path));
        let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current large PDF source version");
        let snapshot = rich_snapshot(
            source.clone(),
            source_version.clone(),
            "large.pdf",
            "pdf",
            "application/pdf",
            length_bytes as u64,
        );
        let session = PreviewSession::new(PreviewSessionConfig::new(
            "w309-pdf-session",
            "w309-pdf-request",
            source,
            PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
        ));
        let read_adapter: Arc<dyn PreviewContentReadAccess> =
            Arc::new(PreviewReadGateAdapter::new(Arc::clone(&gate)));
        let assets = PreviewAssetRegistry::new_with_range_reader(read_adapter.clone());
        let task = session
            .start_with_environment(
                Arc::new(StaticPreviewResolver { snapshot }),
                production_registry(),
                PreviewProviderEnvironmentHandle::with_preview_read_and_asset_publisher(
                    read_adapter,
                    assets.clone(),
                ),
            )
            .expect("start large PDF preview");
        let outcome = task.join().expect("large PDF preview publishes");
        assert_eq!(outcome.provider_id.as_deref(), Some("builtin.pdf"));
        let (asset_token, published_length) = match outcome.envelope.representation {
            PreviewRepresentation::Pdf {
                asset_token,
                media_type,
                length_bytes,
            } => {
                assert_eq!(media_type, "application/pdf");
                (asset_token, length_bytes)
            }
            representation => panic!("unexpected large PDF representation: {representation:?}"),
        };
        assert_eq!(published_length, length_bytes as u64);
        assert_eq!(assets.counts(), (1, 0));
        assert_eq!(gate.active_lease_count(), 0);

        let request = PreviewAssetRequest {
            session_id: "w309-pdf-session".to_string(),
            request_id: "w309-pdf-request".to_string(),
            source_version: source_version.clone(),
            asset_token,
        };
        let offset_bytes = 17_usize * 1024 * 1024;
        let max_bytes = 1024_u32 * 1024;
        let artifact = assets
            .read_range(
                &request,
                BoundedContentReadRequest {
                    offset_bytes: offset_bytes as u64,
                    max_bytes,
                },
            )
            .expect("large PDF range goes through the Read Gate");
        assert_eq!(artifact.media_type, "application/pdf");
        assert_eq!(
            artifact.bytes,
            bytes[offset_bytes..offset_bytes + max_bytes as usize]
        );
        assert_eq!(gate.active_lease_count(), 0);
        assert!(
            resolver.resolve_count() >= 3,
            "current version, lease issue, and range read each re-resolve the source"
        );

        resolver.set_resolution_error(Some(SourceResolutionError::PermissionDenied));
        assert_eq!(
            assets.read_range(
                &request,
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1024,
                },
            ),
            Err(PreviewAssetReadError::ReadFailed)
        );
        assert_eq!(gate.active_lease_count(), 0);
        resolver.set_resolution_error(None);

        assets.revoke_session("w309-pdf-session");
        assert_eq!(assets.counts(), (0, 0));
    }

    #[test]
    fn w306_image_provider_uses_real_read_gate_and_decoder_slot() {
        let fixture = Fixture::new();
        let bytes = image_fixture_bytes(false);
        let path = fixture.file("sample.png", &bytes);
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current image source version");
        let snapshot = rich_snapshot(
            source.clone(),
            source_version.clone(),
            "sample.png",
            "png",
            "image/png",
            bytes.len() as u64,
        );
        let session = PreviewSession::new(PreviewSessionConfig::new(
            "w306-image-session",
            "w306-image-request",
            source,
            PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
        ));
        let read_barrier = PreviewReadGateTestBarrier::new();
        let read_adapter: Arc<dyn PreviewContentReadAccess> =
            Arc::new(PreviewReadGateAdapter::new_with_test_controls(
                Arc::clone(&gate),
                None,
                Some(Arc::clone(&read_barrier)),
            ));
        let scheduler = image_scheduler();
        let decoder_barrier = Arc::new(PreviewDecoderAdmissionTestBarrier::new());
        let decoder_adapter = Arc::new(PreviewDecoderResourceLeaseAdapter::new_with_test_barrier(
            Arc::clone(&scheduler),
            Arc::clone(&decoder_barrier),
        ));
        let assets = PreviewAssetRegistry::new();
        let baseline_read_leases = gate.active_lease_count();
        let task = session
            .start_with_environment(
                Arc::new(StaticPreviewResolver { snapshot }),
                production_registry(),
                PreviewProviderEnvironmentHandle::with_preview_read_and_asset_publisher_and_decoder(
                    read_adapter,
                    assets.clone(),
                    decoder_adapter,
                ),
            )
            .expect("start image preview");

        read_barrier.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline_read_leases + 1);
        read_barrier.release();

        decoder_barrier.wait_for_entry();
        assert_eq!(scheduler.snapshot().granted.decoder, 1);
        assert_eq!(gate.active_lease_count(), baseline_read_leases);
        decoder_barrier.release();

        let outcome = task.join().expect("image preview succeeds");
        assert_eq!(outcome.provider_id.as_deref(), Some("builtin.image"));
        assert_eq!(outcome.envelope.completeness, PreviewCompleteness::Complete);
        let asset_token = match outcome.envelope.representation {
            PreviewRepresentation::Image {
                asset_token,
                media_type,
            } => {
                assert_eq!(media_type, "image/png");
                asset_token
            }
            representation => panic!("unexpected image representation: {representation:?}"),
        };
        let artifact = assets
            .read(&PreviewAssetRequest {
                session_id: "w306-image-session".to_string(),
                request_id: "w306-image-request".to_string(),
                source_version: source_version.clone(),
                asset_token: asset_token.clone(),
            })
            .expect("exact image asset tuple reads");
        assert_eq!(artifact.media_type, "image/png");
        assert!(!artifact.bytes.is_empty());
        for (request_id, requested_source_version, requested_token) in [
            ("wrong-request", source_version.clone(), asset_token.clone()),
            (
                "w306-image-request",
                "wrong-source-version".to_string(),
                asset_token.clone(),
            ),
            (
                "w306-image-request",
                source_version.clone(),
                "wrong-token".to_string(),
            ),
        ] {
            assert_eq!(
                assets.read(&PreviewAssetRequest {
                    session_id: "w306-image-session".to_string(),
                    request_id: request_id.to_string(),
                    source_version: requested_source_version,
                    asset_token: requested_token,
                }),
                Err(PreviewAssetReadError::InvalidOrStale)
            );
        }
        assert_eq!(scheduler.snapshot().granted.decoder, 0);
        assert_eq!(gate.active_lease_count(), baseline_read_leases);

        assets.revoke_session("w306-image-session");
        assert_eq!(assets.counts(), (0, 0));
        assert_eq!(
            assets.read(&PreviewAssetRequest {
                session_id: "w306-image-session".to_string(),
                request_id: "w306-image-request".to_string(),
                source_version: "stale".to_string(),
                asset_token: "stale".to_string(),
            }),
            Err(PreviewAssetReadError::InvalidOrStale)
        );
    }

    #[test]
    fn w306_image_provider_reads_a_bounded_multi_chunk_source() {
        let fixture = Fixture::new();
        let bytes = image_fixture_bytes(true);
        assert!(bytes.len() > DEFAULT_MAX_READ_BYTES as usize);
        let path = fixture.file("chunked.png", &bytes);
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current chunked image source version");
        let snapshot = rich_snapshot(
            source,
            source_version,
            "chunked.png",
            "png",
            "image/png",
            bytes.len() as u64,
        );
        let assets = PreviewAssetRegistry::new();
        let scheduler = image_scheduler();
        let decoder_admission = Arc::new(PreviewDecoderResourceLeaseAdapter::new(Arc::clone(
            &scheduler,
        )));
        let (_session, task) = start_production_image_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            None,
            Arc::clone(&assets),
            decoder_admission,
        );

        let outcome = task.join().expect("chunked image preview succeeds");
        assert_eq!(outcome.provider_id.as_deref(), Some("builtin.image"));
        assert_eq!(outcome.envelope.completeness, PreviewCompleteness::Complete);
        assert_eq!(gate.active_lease_count(), 0);
        assert_eq!(scheduler.snapshot().granted.decoder, 0);
        let (records, bytes) = assets.counts();
        assert_eq!(records, 1);
        assert!(bytes > 0);
        assets.revoke_session("preview-image-session");
        assert_eq!(assets.counts(), (0, 0));
    }

    #[test]
    fn w306_image_provider_corrupt_source_falls_back_without_leaks() {
        let fixture = Fixture::new();
        let valid = image_fixture_bytes(false);
        let bytes = valid[..valid.len().min(16)].to_vec();
        let path = fixture.file("corrupt.png", &bytes);
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current corrupt image source version");
        let snapshot = rich_snapshot(
            source,
            source_version,
            "corrupt.png",
            "png",
            "image/png",
            bytes.len() as u64,
        );
        let assets = PreviewAssetRegistry::new();
        let scheduler = image_scheduler();
        let decoder_admission = Arc::new(PreviewDecoderResourceLeaseAdapter::new(Arc::clone(
            &scheduler,
        )));
        let (_session, task) = start_production_image_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            None,
            Arc::clone(&assets),
            decoder_admission,
        );

        let outcome = task.join().expect("corrupt image falls back locally");
        assert!(outcome.provider_id.is_none());
        assert!(outcome.envelope.warnings.iter().any(|warning| matches!(
            warning,
            PreviewWarning::ProviderFallback {
                provider_id,
                reason: PreviewProviderErrorCode::CorruptSource
                    | PreviewProviderErrorCode::Failed
            } if provider_id == "builtin.image"
        )));
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert_eq!(gate.active_lease_count(), 0);
        assert_eq!(scheduler.snapshot().granted.decoder, 0);
        assert_eq!(assets.counts(), (0, 0));
    }

    #[test]
    fn w306_image_stale_switch_releases_real_read_and_decoder_leases() {
        let fixture = Fixture::new();
        let bytes = image_fixture_bytes(false);
        let path = fixture.file("stale.png", &bytes);
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current image source version");
        let snapshot = rich_snapshot(
            source.clone(),
            source_version,
            "stale.png",
            "png",
            "image/png",
            bytes.len() as u64,
        );
        let session = PreviewSession::new(PreviewSessionConfig::new(
            "w306-stale-image-session",
            "w306-stale-image-request",
            source,
            PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
        ));
        let read_barrier = PreviewReadGateTestBarrier::new();
        let read_adapter: Arc<dyn PreviewContentReadAccess> =
            Arc::new(PreviewReadGateAdapter::new_with_test_controls(
                Arc::clone(&gate),
                None,
                Some(Arc::clone(&read_barrier)),
            ));
        let scheduler = image_scheduler();
        let decoder_barrier = Arc::new(PreviewDecoderAdmissionTestBarrier::new());
        let decoder_adapter = Arc::new(PreviewDecoderResourceLeaseAdapter::new_with_test_barrier(
            Arc::clone(&scheduler),
            Arc::clone(&decoder_barrier),
        ));
        let assets = PreviewAssetRegistry::new();
        let baseline_read_leases = gate.active_lease_count();
        let task = session
            .start_with_environment(
                Arc::new(StaticPreviewResolver { snapshot }),
                production_registry(),
                PreviewProviderEnvironmentHandle::with_preview_read_and_asset_publisher_and_decoder(
                    read_adapter,
                    assets.clone(),
                    decoder_adapter,
                ),
            )
            .expect("start stale image preview");

        read_barrier.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline_read_leases + 1);
        read_barrier.release();
        decoder_barrier.wait_for_entry();
        assert_eq!(scheduler.snapshot().granted.decoder, 1);

        session
            .switch_source(PreviewRequest {
                request_id: "w306-stale-image-b".to_string(),
                source: PreviewSourceRef::Managed {
                    file_id: "w306-image-b".to_string(),
                },
            })
            .expect("switch stale image source");
        decoder_barrier.release();

        assert!(matches!(
            task.join(),
            Err(PreviewRunError::StalePublication)
        ));
        assert_eq!(gate.active_lease_count(), baseline_read_leases);
        assert_eq!(scheduler.snapshot().granted.decoder, 0);
        assert_eq!(assets.counts(), (0, 0));
    }

    #[test]
    fn production_text_provider_reads_through_real_gate_and_releases_lease() {
        let fixture = Fixture::new();
        let path = fixture.file("provider.txt", "hello provider\n世界".as_bytes());
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current source version");
        let snapshot = PreviewSourceSnapshot::new(
            source.clone(),
            source_version,
            PreviewMetadata {
                display_name: "provider.txt".to_string(),
                media_type: Some("text/plain".to_string()),
                extension: Some("txt".to_string()),
                size_bytes: Some(20),
                modified_at_epoch_ms: None,
                materialization: super::super::contracts::MaterializationState::BoundaryReadable,
                read_eligibility: ContentReadEligibility::Eligible,
            },
            PreviewCapabilities {
                can_select_text: true,
                ..PreviewCapabilities::default()
            },
        );
        let resolver = Arc::new(StaticPreviewResolver { snapshot });
        let registry = Arc::new(
            PreviewProviderRegistry::new(production_preview_providers())
                .expect("production providers are unique"),
        );
        let session = PreviewSession::new(PreviewSessionConfig::new(
            "provider-session",
            "provider-request",
            source,
            PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
        ));
        let adapter: Arc<dyn PreviewContentReadAccess> =
            Arc::new(PreviewReadGateAdapter::new(Arc::clone(&gate)));
        let outcome = session
            .run_with_environment(
                resolver,
                registry,
                super::super::preview::PreviewProviderEnvironmentHandle::with_preview_read(adapter),
            )
            .expect("real provider run");

        assert_eq!(outcome.provider_id.as_deref(), Some("builtin.text"));
        assert_eq!(gate.active_lease_count(), 0);
        assert_eq!(
            outcome.envelope.representation,
            PreviewRepresentation::Text {
                text: "hello provider\n世界".to_string(),
                language: None,
            }
        );
        assert_eq!(
            outcome.envelope.completeness,
            super::super::preview::PreviewCompleteness::Complete
        );
    }

    #[test]
    fn w308_archive_provider_uses_real_read_gate_and_scheduler_and_restores_baselines() {
        let fixture = Fixture::new();
        let bytes = archive_fixture_bytes();
        let path = fixture.file("w308-success.zip", &bytes);
        let resolver = Arc::new(TestResolver::new(path));
        let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current archive source version");
        let snapshot = archive_snapshot(source, source_version, bytes.len() as u64);
        let baseline_read_leases = gate.active_lease_count();
        let baseline_resolves = resolver.resolve_count();
        let revalidation_barrier = ArchiveRevalidationBarrier::new();
        resolver.pause_on_resolve_number(baseline_resolves + 2, Arc::clone(&revalidation_barrier));
        let (_session, task, scheduler) =
            start_production_archive_preview(Arc::clone(&gate), snapshot);

        revalidation_barrier.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline_read_leases + 1);
        assert_eq!(resolver.resolve_count(), baseline_resolves + 2);
        assert_eq!(scheduler.snapshot().granted.cpu, 1);
        assert_eq!(scheduler.snapshot().granted.io, 1);
        revalidation_barrier.release();

        let outcome = task.join().expect("real archive preview succeeds");
        assert_eq!(outcome.provider_id.as_deref(), Some("builtin.archive-zip"));
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::ArchiveTree { .. }
        ));
        assert!(
            resolver.resolve_count() >= baseline_resolves + 2,
            "read must perform a fresh source revalidation after lease issue"
        );
        assert_eq!(gate.active_lease_count(), baseline_read_leases);
        assert_eq!(scheduler.snapshot().granted.cpu, 0);
        assert_eq!(scheduler.snapshot().granted.io, 0);
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn w308_archive_post_lease_drift_preserves_real_terminal_truth() {
        #[derive(Clone, Copy)]
        enum Drift {
            MaterializationRequired,
            Downloading,
            Permission,
            IdentityChanged,
            SourceUnavailable,
            AvailabilityUnknown,
            MetadataOnly,
        }

        for (index, drift) in [
            Drift::MaterializationRequired,
            Drift::Downloading,
            Drift::Permission,
            Drift::IdentityChanged,
            Drift::SourceUnavailable,
            Drift::AvailabilityUnknown,
            Drift::MetadataOnly,
        ]
        .into_iter()
        .enumerate()
        {
            let fixture = Fixture::new();
            let bytes = archive_fixture_bytes();
            let path = fixture.file(&format!("w308-drift-{index}.zip"), &bytes);
            let replacement = fixture.file(&format!("w308-drift-{index}-replacement.zip"), &bytes);
            let resolver = Arc::new(TestResolver::new(path));
            let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
            let source = source();
            let source_version = gate
                .current_source_version(&source)
                .expect("current archive drift source version");
            let snapshot = archive_snapshot(source.clone(), source_version, bytes.len() as u64);
            let baseline_read_leases = gate.active_lease_count();
            let baseline_resolves = resolver.resolve_count();
            let revalidation_barrier = ArchiveRevalidationBarrier::new();
            resolver
                .pause_on_resolve_number(baseline_resolves + 2, Arc::clone(&revalidation_barrier));
            let (session, task, scheduler) =
                start_production_archive_preview(Arc::clone(&gate), snapshot);

            revalidation_barrier.wait_for_entry();
            assert_eq!(gate.active_lease_count(), baseline_read_leases + 1);
            match drift {
                Drift::MaterializationRequired => {
                    gate.set_test_eligibility(Some(ContentReadEligibility::MaterializationRequired))
                }
                Drift::Downloading => {
                    gate.set_test_eligibility(Some(ContentReadEligibility::Downloading))
                }
                Drift::Permission => {
                    gate.set_test_eligibility(Some(ContentReadEligibility::PermissionRequired))
                }
                Drift::IdentityChanged => resolver.replace_path(replacement),
                Drift::SourceUnavailable => {
                    resolver.set_resolution_error(Some(SourceResolutionError::Unavailable))
                }
                Drift::AvailabilityUnknown => {
                    resolver.set_resolution_error(Some(SourceResolutionError::Unknown))
                }
                Drift::MetadataOnly => {
                    gate.set_test_eligibility(Some(ContentReadEligibility::MetadataOnly))
                }
            }
            revalidation_barrier.release();

            let expected_terminal = match drift {
                Drift::MaterializationRequired | Drift::Downloading => {
                    Some(PreviewTerminalCondition::MaterializationRequired)
                }
                Drift::Permission => Some(PreviewTerminalCondition::PermissionDenied),
                Drift::IdentityChanged => Some(PreviewTerminalCondition::IdentityChanged),
                Drift::SourceUnavailable | Drift::AvailabilityUnknown => {
                    Some(PreviewTerminalCondition::SourceUnavailable)
                }
                Drift::MetadataOnly => None,
            };
            match expected_terminal {
                Some(expected) => {
                    match task.join() {
                        Err(PreviewRunError::ProviderTerminal { condition, .. }) => {
                            assert_eq!(condition, expected)
                        }
                        other => panic!("unexpected archive terminal result: {other:?}"),
                    }
                    let envelope = session
                        .representation()
                        .expect("terminal archive metadata fallback");
                    assert!(matches!(
                        envelope.representation,
                        PreviewRepresentation::Metadata { .. }
                    ));
                    assert!(envelope.warnings.iter().any(|warning| matches!(
                        warning,
                        PreviewWarning::TerminalCondition { condition } if *condition == expected
                    )));
                }
                None => {
                    let outcome = task.join().expect("metadata-only archive fallback");
                    assert!(outcome.provider_id.is_none());
                    assert!(matches!(
                        outcome.envelope.representation,
                        PreviewRepresentation::Metadata { .. }
                    ));
                    assert!(outcome.envelope.warnings.contains(
                        &PreviewWarning::ProviderFallback {
                            provider_id: "builtin.archive-zip".to_string(),
                            reason: PreviewProviderErrorCode::Unsupported,
                        }
                    ));
                    assert!(!outcome.envelope.warnings.iter().any(|warning| matches!(
                        warning,
                        PreviewWarning::TerminalCondition { .. }
                    )));
                }
            }
            assert_eq!(gate.active_lease_count(), baseline_read_leases);
            assert_eq!(scheduler.snapshot().granted.cpu, 0);
            assert_eq!(scheduler.snapshot().granted.io, 0);
        }
    }

    #[test]
    fn w308_archive_cancel_stale_switch_and_dispose_release_real_resources() {
        #[derive(Clone, Copy)]
        enum Action {
            Cancel,
            Switch,
            Dispose,
        }

        for (index, action) in [Action::Cancel, Action::Switch, Action::Dispose]
            .into_iter()
            .enumerate()
        {
            let fixture = Fixture::new();
            let bytes = archive_fixture_bytes();
            let path = fixture.file(&format!("w308-lifecycle-{index}.zip"), &bytes);
            let resolver = Arc::new(TestResolver::new(path));
            let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
            let source = source();
            let source_version = gate
                .current_source_version(&source)
                .expect("current lifecycle source version");
            let snapshot = archive_snapshot(source.clone(), source_version, bytes.len() as u64);
            let baseline_read_leases = gate.active_lease_count();
            let baseline_resolves = resolver.resolve_count();
            let revalidation_barrier = ArchiveRevalidationBarrier::new();
            resolver
                .pause_on_resolve_number(baseline_resolves + 2, Arc::clone(&revalidation_barrier));
            let (session, task, scheduler) =
                start_production_archive_preview(Arc::clone(&gate), snapshot);

            revalidation_barrier.wait_for_entry();
            assert_eq!(gate.active_lease_count(), baseline_read_leases + 1);
            assert_eq!(scheduler.snapshot().granted.cpu, 1);
            match action {
                Action::Cancel => assert!(session.cancel()),
                Action::Switch => session
                    .switch_source(PreviewRequest {
                        request_id: format!("w308-lifecycle-b-{index}"),
                        source: PreviewSourceRef::Managed {
                            file_id: format!("w308-lifecycle-b-{index}"),
                        },
                    })
                    .expect("switch archive source"),
                Action::Dispose => assert!(session.dispose()),
            }
            revalidation_barrier.release();

            assert!(matches!(
                task.join(),
                Err(PreviewRunError::StalePublication) | Err(PreviewRunError::Cancelled)
            ));
            assert!(!session.representation().is_some_and(|envelope| matches!(
                envelope.representation,
                PreviewRepresentation::ArchiveTree { .. }
            )));
            if matches!(action, Action::Switch) {
                assert_eq!(
                    session.snapshot().source,
                    PreviewSourceRef::Managed {
                        file_id: format!("w308-lifecycle-b-{index}")
                    }
                );
            }
            assert_eq!(gate.active_lease_count(), baseline_read_leases);
            assert_eq!(scheduler.snapshot().granted.cpu, 0);
            assert_eq!(scheduler.snapshot().granted.io, 0);
            assert_eq!(scheduler.snapshot().running, 0);
        }
    }

    #[test]
    fn w305_structured_provider_issues_real_lease_and_returns_to_baseline() {
        let fixture = Fixture::new();
        let bytes = br#"{"name":"Zen"}"#;
        let path = fixture.file("provider.json", bytes);
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current source version");
        let snapshot = rich_snapshot(
            source.clone(),
            source_version,
            "provider.json",
            "json",
            "application/json",
            bytes.len() as u64,
        );
        let baseline = gate.active_lease_count();
        let after_issue = PreviewReadGateTestBarrier::new();
        let (session, task) = start_production_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            Some(Arc::clone(&after_issue)),
        );

        after_issue.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline + 1);
        after_issue.release();

        let outcome = task.join().expect("structured provider succeeds");
        assert_eq!(
            outcome.provider_id.as_deref(),
            Some("builtin.structured-json")
        );
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::StructuredTree { .. }
        ));
        assert_eq!(gate.active_lease_count(), baseline);
        assert!(session.representation().is_some());
    }

    #[test]
    fn w305_table_provider_failure_after_read_returns_to_baseline() {
        let fixture = Fixture::new();
        let bytes = b"header,value\n\"unterminated";
        let path = fixture.file("provider.csv", bytes);
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current source version");
        let snapshot = rich_snapshot(
            source,
            source_version,
            "provider.csv",
            "csv",
            "text/csv",
            bytes.len() as u64,
        );
        let baseline = gate.active_lease_count();
        let after_issue = PreviewReadGateTestBarrier::new();
        let (_session, task) = start_production_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            Some(Arc::clone(&after_issue)),
        );

        after_issue.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline + 1);
        after_issue.release();

        let outcome = task
            .join()
            .expect("provider-local table failure falls back");
        assert!(outcome.provider_id.is_none());
        assert!(outcome
            .envelope
            .warnings
            .contains(&PreviewWarning::ProviderFallback {
                provider_id: "builtin.table-csv".to_string(),
                reason: PreviewProviderErrorCode::CorruptSource,
            }));
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert_eq!(gate.active_lease_count(), baseline);
    }

    #[test]
    fn w305_structured_stale_switch_cannot_publish_after_lease_issue() {
        let fixture = Fixture::new();
        let bytes = br#"{"source":"A"}"#;
        let path = fixture.file("stale.json", bytes);
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current source version");
        let snapshot = rich_snapshot(
            source.clone(),
            source_version,
            "stale.json",
            "json",
            "application/json",
            bytes.len() as u64,
        );
        let baseline = gate.active_lease_count();
        let after_issue = PreviewReadGateTestBarrier::new();
        let (session, task) = start_production_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            Some(Arc::clone(&after_issue)),
        );

        after_issue.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline + 1);
        session
            .switch_source(PreviewRequest {
                request_id: "w305-source-b".to_string(),
                source: PreviewSourceRef::Managed {
                    file_id: "w305-source-b".to_string(),
                },
            })
            .expect("source switch revokes A publication");
        after_issue.release();

        assert!(matches!(
            task.join(),
            Err(PreviewRunError::StalePublication)
        ));
        assert_eq!(gate.active_lease_count(), baseline);
        assert_eq!(
            session.snapshot().source,
            PreviewSourceRef::Managed {
                file_id: "w305-source-b".to_string()
            }
        );
        assert!(session.representation().is_none());
    }

    #[test]
    fn w305_table_cancel_after_lease_issue_returns_to_baseline() {
        let fixture = Fixture::new();
        let bytes = b"name,value\nA,1\n";
        let path = fixture.file("cancel.tsv", bytes);
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("current source version");
        let snapshot = rich_snapshot(
            source,
            source_version,
            "cancel.tsv",
            "tsv",
            "text/tab-separated-values",
            bytes.len() as u64,
        );
        let baseline = gate.active_lease_count();
        let after_issue = PreviewReadGateTestBarrier::new();
        let (session, task) = start_production_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            Some(Arc::clone(&after_issue)),
        );

        after_issue.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline + 1);
        assert!(session.cancel());
        after_issue.release();

        assert!(matches!(
            task.join(),
            Err(PreviewRunError::StalePublication) | Err(PreviewRunError::Cancelled)
        ));
        assert_eq!(gate.active_lease_count(), baseline);
    }

    #[test]
    fn preview_read_gate_error_mapping_preserves_terminal_truth() {
        assert_eq!(
            map_gate_error_to_preview_access(ReadGateError::MaterializationRequired),
            PreviewReadAccessError::MaterializationRequired
        );
        assert_eq!(
            map_gate_error_to_preview_access(ReadGateError::Downloading),
            PreviewReadAccessError::MaterializationRequired
        );
        assert_eq!(
            map_gate_error_to_preview_access(ReadGateError::AvailabilityUnknown),
            PreviewReadAccessError::SourceUnavailable
        );
        assert_eq!(
            map_gate_error_to_preview_access(ReadGateError::SourceUnavailable),
            PreviewReadAccessError::SourceUnavailable
        );
        assert_eq!(
            map_gate_error_to_preview_access(ReadGateError::MetadataOnly),
            PreviewReadAccessError::MetadataOnly
        );
    }

    #[test]
    fn fresh_materialization_drift_remains_a_preview_terminal_condition() {
        let fixture = Fixture::new();
        let path = fixture.file("materialization-drift.txt", b"fresh state");
        let resolver = Arc::new(TestResolver::new(path));
        let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
        let baseline = gate.active_lease_count();
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("eligible snapshot source version");
        let snapshot = text_snapshot(source.clone(), source_version, "materialization-drift.txt");
        let after_issue = PreviewReadGateTestBarrier::new();
        let (session, task) = start_production_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            Some(Arc::clone(&after_issue)),
        );

        after_issue.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline + 1);
        assert_eq!(
            session
                .source_snapshot()
                .expect("snapshot published before post-lease read")
                .metadata
                .read_eligibility,
            ContentReadEligibility::Eligible
        );
        gate.set_test_eligibility(Some(ContentReadEligibility::MaterializationRequired));
        assert_eq!(
            gate.content_read_eligibility(&source),
            ContentReadEligibility::MaterializationRequired
        );
        after_issue.release();

        let result = task.join();
        assert!(matches!(
            result,
            Err(PreviewRunError::ProviderTerminal {
                condition: PreviewTerminalCondition::MaterializationRequired,
                ..
            })
        ));
        let envelope = session
            .representation()
            .expect("terminal metadata fallback is published");
        assert!(matches!(
            envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert!(envelope
            .warnings
            .contains(&PreviewWarning::TerminalCondition {
                condition: PreviewTerminalCondition::MaterializationRequired,
            }));
        assert!(!envelope
            .warnings
            .contains(&PreviewWarning::TerminalCondition {
                condition: PreviewTerminalCondition::SourceUnavailable,
            }));
        assert_eq!(gate.active_lease_count(), baseline);
    }

    #[test]
    fn fresh_availability_unknown_drift_remains_source_unavailable_terminal() {
        let fixture = Fixture::new();
        let path = fixture.file("availability-drift.txt", b"fresh state");
        let resolver = Arc::new(TestResolver::new(path));
        let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
        let baseline = gate.active_lease_count();
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("eligible snapshot source version");
        let snapshot = text_snapshot(source.clone(), source_version, "availability-drift.txt");
        let after_issue = PreviewReadGateTestBarrier::new();
        let (session, task) = start_production_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            Some(Arc::clone(&after_issue)),
        );

        after_issue.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline + 1);
        resolver.set_resolution_error(Some(SourceResolutionError::Unknown));
        assert_eq!(
            gate.content_read_eligibility(&source),
            ContentReadEligibility::AvailabilityUnknown
        );
        after_issue.release();

        let result = task.join();
        assert!(matches!(
            result,
            Err(PreviewRunError::ProviderTerminal {
                condition: PreviewTerminalCondition::SourceUnavailable,
                ..
            })
        ));
        let envelope = session
            .representation()
            .expect("terminal metadata fallback is published");
        assert!(matches!(
            envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert!(envelope
            .warnings
            .contains(&PreviewWarning::TerminalCondition {
                condition: PreviewTerminalCondition::SourceUnavailable,
            }));
        assert_eq!(gate.active_lease_count(), baseline);
    }

    #[test]
    fn fresh_metadata_only_drift_falls_back_to_metadata_without_terminal_warning() {
        let fixture = Fixture::new();
        let path = fixture.file("metadata-drift.txt", b"fresh state");
        let resolver = Arc::new(TestResolver::new(path));
        let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
        let baseline = gate.active_lease_count();
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("eligible snapshot source version");
        let snapshot = text_snapshot(source.clone(), source_version, "metadata-drift.txt");
        let after_issue = PreviewReadGateTestBarrier::new();
        let (_session, task) = start_production_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            Some(Arc::clone(&after_issue)),
        );

        after_issue.wait_for_entry();
        assert_eq!(gate.active_lease_count(), baseline + 1);
        gate.set_test_eligibility(Some(ContentReadEligibility::MetadataOnly));
        assert_eq!(
            gate.content_read_eligibility(&source),
            ContentReadEligibility::MetadataOnly
        );
        after_issue.release();

        let outcome = task.join().expect("metadata fallback is not terminal");
        assert!(outcome.provider_id.is_none());
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert!(outcome
            .envelope
            .warnings
            .contains(&PreviewWarning::ProviderFallback {
                provider_id: "builtin.text".to_string(),
                reason: PreviewProviderErrorCode::Unsupported,
            }));
        assert!(outcome
            .envelope
            .warnings
            .contains(&PreviewWarning::MetadataFallback));
        assert!(!outcome
            .envelope
            .warnings
            .iter()
            .any(|warning| matches!(warning, PreviewWarning::TerminalCondition { .. })));
        assert_eq!(gate.active_lease_count(), baseline);
    }

    #[test]
    fn w305_structured_post_lease_drift_preserves_terminal_and_metadata_semantics() {
        #[derive(Clone, Copy)]
        enum Drift {
            MaterializationRequired,
            AvailabilityUnknown,
            MetadataOnly,
        }

        for drift in [
            Drift::MaterializationRequired,
            Drift::AvailabilityUnknown,
            Drift::MetadataOnly,
        ] {
            let fixture = Fixture::new();
            let bytes = br#"{"source":"drift"}"#;
            let path = fixture.file("w305-drift.json", bytes);
            let resolver = Arc::new(TestResolver::new(path));
            let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
            let source = source();
            let source_version = gate
                .current_source_version(&source)
                .expect("current source version");
            let snapshot = rich_snapshot(
                source.clone(),
                source_version,
                "w305-drift.json",
                "json",
                "application/json",
                bytes.len() as u64,
            );
            let baseline = gate.active_lease_count();
            let after_issue = PreviewReadGateTestBarrier::new();
            let (session, task) = start_production_preview(
                Arc::clone(&gate),
                snapshot,
                None,
                Some(Arc::clone(&after_issue)),
            );

            after_issue.wait_for_entry();
            assert_eq!(gate.active_lease_count(), baseline + 1);
            match drift {
                Drift::MaterializationRequired => {
                    gate.set_test_eligibility(Some(
                        ContentReadEligibility::MaterializationRequired,
                    ));
                }
                Drift::AvailabilityUnknown => {
                    resolver.set_resolution_error(Some(SourceResolutionError::Unknown));
                }
                Drift::MetadataOnly => {
                    gate.set_test_eligibility(Some(ContentReadEligibility::MetadataOnly));
                }
            }
            after_issue.release();

            match drift {
                Drift::MaterializationRequired => {
                    let result = task.join();
                    assert!(matches!(
                        result,
                        Err(PreviewRunError::ProviderTerminal {
                            condition: PreviewTerminalCondition::MaterializationRequired,
                            ..
                        })
                    ));
                    let envelope = session
                        .representation()
                        .expect("terminal metadata fallback");
                    assert!(matches!(
                        envelope.representation,
                        PreviewRepresentation::Metadata { .. }
                    ));
                    assert!(envelope
                        .warnings
                        .contains(&PreviewWarning::TerminalCondition {
                            condition: PreviewTerminalCondition::MaterializationRequired,
                        }));
                }
                Drift::AvailabilityUnknown => {
                    let result = task.join();
                    assert!(matches!(
                        result,
                        Err(PreviewRunError::ProviderTerminal {
                            condition: PreviewTerminalCondition::SourceUnavailable,
                            ..
                        })
                    ));
                    let envelope = session
                        .representation()
                        .expect("terminal metadata fallback");
                    assert!(matches!(
                        envelope.representation,
                        PreviewRepresentation::Metadata { .. }
                    ));
                    assert!(envelope
                        .warnings
                        .contains(&PreviewWarning::TerminalCondition {
                            condition: PreviewTerminalCondition::SourceUnavailable,
                        }));
                }
                Drift::MetadataOnly => {
                    let outcome = task.join().expect("MetadataOnly remains provider-local");
                    assert!(outcome.provider_id.is_none());
                    assert!(matches!(
                        outcome.envelope.representation,
                        PreviewRepresentation::Metadata { .. }
                    ));
                    assert!(outcome.envelope.warnings.contains(
                        &PreviewWarning::ProviderFallback {
                            provider_id: "builtin.structured-json".to_string(),
                            reason: PreviewProviderErrorCode::Unsupported,
                        }
                    ));
                    assert!(!outcome.envelope.warnings.iter().any(|warning| matches!(
                        warning,
                        PreviewWarning::TerminalCondition { .. }
                    )));
                }
            }
            assert_eq!(gate.active_lease_count(), baseline);
        }
    }

    #[test]
    fn provider_processing_failure_after_successful_read_releases_preview_lease() {
        let fixture = Fixture::new();
        let path = fixture.file("invalid-provider.txt", &[0xff, 0xfe]);
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("eligible source version");
        let snapshot = text_snapshot(source.clone(), source_version, "invalid-provider.txt");
        let session = PreviewSession::new(PreviewSessionConfig::new(
            "provider-failure-session",
            "provider-failure-request",
            source,
            PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
        ));
        let adapter: Arc<dyn PreviewContentReadAccess> =
            Arc::new(PreviewReadGateAdapter::new(Arc::clone(&gate)));
        let outcome = session
            .run_with_environment(
                Arc::new(StaticPreviewResolver { snapshot }),
                production_registry(),
                PreviewProviderEnvironmentHandle::with_preview_read(adapter),
            )
            .expect("provider-local failure falls back to metadata");

        assert!(outcome.provider_id.is_none());
        assert!(outcome
            .envelope
            .warnings
            .contains(&PreviewWarning::ProviderFallback {
                provider_id: "builtin.text".to_string(),
                reason: PreviewProviderErrorCode::CorruptSource,
            }));
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert_eq!(gate.active_lease_count(), 0);
    }

    #[test]
    fn source_switch_after_preview_lease_issue_releases_lease_and_blocks_stale_publish() {
        let fixture = Fixture::new();
        let path = fixture.file("switch-after-lease.txt", b"switch me");
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let source_version = gate
            .current_source_version(&source)
            .expect("eligible source version");
        let snapshot = text_snapshot(source.clone(), source_version, "switch-after-lease.txt");
        let after_issue = PreviewReadGateTestBarrier::new();
        let (session, task) = start_production_preview(
            Arc::clone(&gate),
            snapshot,
            None,
            Some(Arc::clone(&after_issue)),
        );

        after_issue.wait_for_entry();
        assert_eq!(gate.active_lease_count(), 1);
        session
            .switch_source(PreviewRequest {
                request_id: "next-preview-request".to_string(),
                source: PreviewSourceRef::Managed {
                    file_id: "next-preview-source".to_string(),
                },
            })
            .expect("switch source while read is held");
        assert!(session.representation().is_none());
        after_issue.release();

        assert!(matches!(
            task.join(),
            Err(PreviewRunError::StalePublication)
        ));
        assert_eq!(gate.active_lease_count(), 0);
        assert!(session.representation().is_none());
    }

    #[test]
    fn preview_read_adapter_rejects_request_or_source_version_drift_before_lease_issue() {
        let fixture = Fixture::new();
        let path = fixture.file("adapter-drift.txt", b"drift");
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let context = PreviewOperationContext::for_backend_content_read(
            "adapter-session",
            "adapter-request",
            "version-current",
            crate::file_workspace::PreviewCancellation::default(),
            Instant::now() + Duration::from_secs(1),
        );
        let adapter = PreviewReadGateAdapter::new(Arc::clone(&gate));
        assert_eq!(
            adapter.read_source_bounded(
                &source,
                "version-other",
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1,
                },
                &context,
            ),
            Err(PreviewReadAccessError::SourceVersionMismatch)
        );
        assert_eq!(gate.active_lease_count(), 0);
    }

    #[test]
    fn unknown_released_and_expired_leases_fail_closed() {
        let fixture = Fixture::new();
        let path = fixture.file("lease.bin", b"lease");
        let resolver = Arc::new(TestResolver::new(path));
        let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
        let source = source();
        let unknown = ContentReadLeaseRef {
            lease_id: "unknown".to_string(),
            request_id: "request-1".to_string(),
            source_version: "unknown".to_string(),
        };
        assert_eq!(
            read_through_preview(
                Arc::clone(&gate),
                source.clone(),
                unknown,
                "request-1",
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1,
                },
                false,
            ),
            Some(Err(ContentReadAccessError::LeaseInvalid))
        );

        let released = gate
            .issue_lease_for_current("request-1", source.clone(), ReadIntent::Preview)
            .expect("issue released lease");
        gate.release_lease(&released).expect("release lease");
        assert_eq!(
            read_through_preview(
                Arc::clone(&gate),
                source.clone(),
                released,
                "request-1",
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1,
                },
                false,
            ),
            Some(Err(ContentReadAccessError::LeaseInvalid))
        );

        let expiring = MaterializationReadGate::new(
            resolver,
            ReadGateConfig {
                lease_ttl: Duration::from_millis(1),
                ..ReadGateConfig::default()
            },
        )
        .expect("short lease config");
        let expiring = Arc::new(expiring);
        let lease = expiring
            .issue_lease_for_current("request-1", source.clone(), ReadIntent::Preview)
            .expect("issue expiring lease");
        std::thread::sleep(Duration::from_millis(5));
        // Resource diagnostics must reveal retained expired records without
        // making the next memory sample look healthier by collecting them.
        assert_eq!(expiring.retained_lease_count_for_test(), 1);
        assert_eq!(expiring.retained_lease_count_for_test(), 1);
        assert_eq!(
            read_through_preview(
                expiring,
                source,
                lease,
                "request-1",
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1,
                },
                false,
            ),
            Some(Err(ContentReadAccessError::LeaseInvalid))
        );
    }

    #[test]
    fn request_and_source_version_mismatches_are_rejected() {
        let fixture = Fixture::new();
        let first = fixture.file("first.bin", b"first");
        let second = fixture.file("second.bin", b"second");
        let resolver = Arc::new(TestResolver::new(first));
        let gate = gate(Arc::clone(&resolver), ReadGateConfig::default());
        let source = source();
        let lease = gate
            .issue_lease_for_current("request-1", source.clone(), ReadIntent::Preview)
            .expect("issue lease");
        assert_eq!(
            read_through_preview(
                Arc::clone(&gate),
                source.clone(),
                lease.clone(),
                "request-2",
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1,
                },
                false,
            ),
            Some(Err(ContentReadAccessError::LeaseInvalid))
        );
        resolver.replace_path(second);
        assert_eq!(
            read_through_preview(
                gate,
                source,
                lease,
                "request-1",
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1,
                },
                false,
            ),
            Some(Err(ContentReadAccessError::SourceVersionMismatch))
        );
    }

    #[test]
    fn oversized_and_overflowing_reads_are_rejected() {
        let fixture = Fixture::new();
        let path = fixture.file("bounded.bin", b"bounded");
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let lease = gate
            .issue_lease_for_current("request-1", source.clone(), ReadIntent::Preview)
            .expect("issue lease");
        assert_eq!(
            read_through_preview(
                Arc::clone(&gate),
                source.clone(),
                lease.clone(),
                "request-1",
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: DEFAULT_MAX_READ_BYTES + 1,
                },
                false,
            ),
            Some(Err(ContentReadAccessError::Failed))
        );
        assert_eq!(
            read_through_preview(
                gate,
                source,
                lease,
                "request-1",
                BoundedContentReadRequest {
                    offset_bytes: u64::MAX,
                    max_bytes: 1,
                },
                false,
            ),
            Some(Err(ContentReadAccessError::Failed))
        );
    }

    #[test]
    fn cancellation_prevents_publication_after_gate_entry() {
        let fixture = Fixture::new();
        let path = fixture.file("cancel.bin", b"cancel");
        let gate = gate(Arc::new(TestResolver::new(path)), ReadGateConfig::default());
        let source = source();
        let lease = gate
            .issue_lease_for_current("request-1", source.clone(), ReadIntent::Preview)
            .expect("issue lease");
        assert_eq!(
            read_through_preview(
                gate,
                source,
                lease,
                "request-1",
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1,
                },
                true,
            ),
            Some(Err(ContentReadAccessError::Cancelled))
        );
    }

    #[test]
    fn metadata_only_and_non_regular_sources_never_issue_a_lease() {
        let fixture = Fixture::new();
        let directory = fixture.root.join("directory");
        fs::create_dir(&directory).expect("create directory fixture");
        let gate = gate(
            Arc::new(TestResolver::new(directory)),
            ReadGateConfig::default(),
        );
        let source = source();
        assert_eq!(
            gate.content_read_eligibility(&source),
            ContentReadEligibility::SourceNotSupported
        );
        assert_eq!(
            gate.issue_lease_for_current("request-1", source.clone(), ReadIntent::Preview),
            Err(ReadGateError::SourceNotSupported)
        );
        assert_eq!(
            gate.issue_lease_for_current("request-1", source, ReadIntent::MetadataOnly),
            Err(ReadGateError::InvalidRequest)
        );
        assert_eq!(gate.active_lease_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_source_fails_closed_without_following_target() {
        let fixture = Fixture::new();
        let target = fixture.file("target.bin", b"target");
        let link = fixture.root.join("link.bin");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink fixture");
        let gate = gate(Arc::new(TestResolver::new(link)), ReadGateConfig::default());
        let source = source();
        assert_eq!(
            gate.content_read_eligibility(&source),
            ContentReadEligibility::Symlink
        );
        assert_eq!(
            gate.issue_lease_for_current("request-1", source, ReadIntent::Preview),
            Err(ReadGateError::Symlink)
        );
    }

    #[test]
    fn read_errors_are_conservative_and_release_frees_bounded_capacity() {
        let fixture = Fixture::new();
        let path = fixture.file("capacity.bin", b"capacity");
        let resolver = Arc::new(TestResolver::new(path));
        let gate = gate(
            Arc::clone(&resolver),
            ReadGateConfig {
                max_active_leases: 1,
                ..ReadGateConfig::default()
            },
        );
        let source = source();
        let first = gate
            .issue_lease_for_current("request-1", source.clone(), ReadIntent::Preview)
            .expect("issue first lease");
        assert_eq!(
            gate.issue_lease_for_current("request-2", source.clone(), ReadIntent::Preview),
            Err(ReadGateError::LeaseCapacityExceeded)
        );
        assert_eq!(
            read_through_preview(
                Arc::clone(&gate),
                source.clone(),
                first.clone(),
                "request-1",
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 2,
                },
                false,
            )
            .expect("captured read result")
            .expect("bounded read succeeds")
            .bytes,
            b"ca"
        );
        gate.release_lease(&first).expect("release first lease");
        let second = gate
            .issue_lease_for_current("request-2", source.clone(), ReadIntent::Preview)
            .expect("capacity is released");
        assert_eq!(gate.active_lease_count(), 1);
        gate.dispose();
        assert_eq!(gate.active_lease_count(), 0);
        assert_eq!(gate.release_lease(&second), Err(ReadGateError::Disposed));
    }
}
