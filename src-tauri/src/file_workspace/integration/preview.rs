use super::{
    runtime::{FileWorkspaceRuntime, MAX_PREVIEW_SESSIONS},
    types::{
        PreviewAssetArtifactDto, PreviewAssetRequestDto, PreviewCreateRequest,
        PreviewSessionRequest, PreviewSnapshotDto, PreviewSwitchSourceRequest,
    },
};
#[cfg(target_os = "macos")]
use crate::file_workspace::{
    native_preview::access::NativePreviewAccessResolveRequest, NativePreviewFailureIdentity,
    PreviewRepresentation,
};
use crate::{
    db::Database,
    file_workspace::{
        browse::{BrowseEntryKind, BrowseService},
        contracts::{ContentReadEligibility, MaterializationState, PreviewSourceRef},
        preview::{
            BoundedContentReadRequest, PreviewContextError, PreviewEntryKind,
            PreviewFolderEnumerationError, PreviewHost, PreviewOperationContext,
            PreviewProviderEnvironmentHandle, PreviewRequest, PreviewResolveRequest,
            PreviewSession, PreviewSessionConfig, PreviewSourceSnapshot, SourceResolveError,
            SourceResolver,
        },
        preview_policy::{
            activated_host_capabilities, project_source_capabilities, PreviewSourceEntryKind,
        },
        read_gate::{MaterializationReadGate, ReadGateError},
    },
    fs_safety::capture_namespace_identity_only,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

/// W1 resolver adapter. It resolves managed sources through the existing
/// File Library detail authority and ephemeral sources through the same
/// BrowseService that issued their refs. It only publishes metadata snapshots.
pub(crate) struct WorkspacePreviewResolver {
    database: Database,
    browse: Arc<BrowseService>,
    read_gate: Arc<MaterializationReadGate>,
}

impl WorkspacePreviewResolver {
    pub(crate) fn new(
        database: Database,
        browse: Arc<BrowseService>,
        read_gate: Arc<MaterializationReadGate>,
    ) -> Self {
        Self {
            database,
            browse,
            read_gate,
        }
    }
}

impl SourceResolver for WorkspacePreviewResolver {
    fn resolve(
        &self,
        request: &PreviewResolveRequest,
        context: &PreviewOperationContext,
    ) -> Result<PreviewSourceSnapshot, SourceResolveError> {
        context.ensure_active().map_err(map_context_error)?;
        let source = request.source.clone();
        let resolved = self.resolve_metadata(&source)?;
        context.ensure_active().map_err(map_context_error)?;

        let eligibility = self.read_gate.content_read_eligibility(&source);
        let source_version = source_version_for_metadata(&self.read_gate, &source, &resolved.path)?;
        let materialization = materialization_for_eligibility(eligibility);
        let metadata = crate::file_workspace::PreviewMetadata {
            display_name: resolved.display_name,
            media_type: None,
            extension: resolved.extension,
            size_bytes: Some(resolved.size_bytes),
            modified_at_epoch_ms: resolved.modified_at_epoch_ms,
            materialization,
            read_eligibility: eligibility,
        };
        let capabilities = project_source_capabilities(
            &source,
            resolved.entry_kind,
            eligibility,
            materialization,
            true,
        );
        Ok(
            PreviewSourceSnapshot::new(source, source_version, metadata, capabilities)
                .with_entry_kind(match resolved.entry_kind {
                    PreviewSourceEntryKind::Directory => PreviewEntryKind::Directory,
                    PreviewSourceEntryKind::File => PreviewEntryKind::File,
                }),
        )
    }
}

struct ResolvedPreviewMetadata {
    path: PathBuf,
    display_name: String,
    extension: Option<String>,
    size_bytes: u64,
    modified_at_epoch_ms: Option<i64>,
    entry_kind: PreviewSourceEntryKind,
}

impl WorkspacePreviewResolver {
    pub(crate) fn resolve_folder_directory(
        &self,
        source: &PreviewSourceRef,
        expected_source_version: &str,
        context: &PreviewOperationContext,
    ) -> Result<
        crate::file_workspace::browse::BackendResolvedDirectory,
        PreviewFolderEnumerationError,
    > {
        context.ensure_active().map_err(map_folder_context_error)?;
        let resolved = self
            .resolve_metadata(source)
            .map_err(map_folder_source_error)?;
        if resolved.entry_kind != PreviewSourceEntryKind::Directory {
            return Err(PreviewFolderEnumerationError::Unsupported);
        }
        let current_source_version =
            source_version_for_metadata(&self.read_gate, source, &resolved.path)
                .map_err(map_folder_source_error)?;
        if current_source_version != expected_source_version {
            return Err(PreviewFolderEnumerationError::IdentityChanged);
        }
        let directory = crate::file_workspace::browse::BackendResolvedDirectory::from_backend_path(
            resolved.path,
        )
        .map_err(map_folder_browse_error)?;
        context.ensure_active().map_err(map_folder_context_error)?;
        Ok(directory)
    }

    fn resolve_metadata(
        &self,
        source: &PreviewSourceRef,
    ) -> Result<ResolvedPreviewMetadata, SourceResolveError> {
        match source {
            PreviewSourceRef::Managed { file_id } => {
                let detail = self
                    .database
                    .get_file_library_detail(file_id)
                    .map_err(|_| SourceResolveError::SourceUnavailable)?;
                let path = PathBuf::from(&detail.path);
                let metadata = fs::symlink_metadata(&path).map_err(map_metadata_error)?;
                Ok(ResolvedPreviewMetadata {
                    path,
                    display_name: detail.name,
                    extension: non_empty(detail.extension),
                    size_bytes: detail.size.max(0) as u64,
                    modified_at_epoch_ms: seconds_to_millis(detail.modified_at),
                    entry_kind: if metadata.is_dir() {
                        PreviewSourceEntryKind::Directory
                    } else {
                        PreviewSourceEntryKind::File
                    },
                })
                .and_then(|resolved| {
                    if metadata.is_file() || metadata.is_dir() {
                        Ok(resolved)
                    } else {
                        Err(SourceResolveError::SourceUnavailable)
                    }
                })
            }
            PreviewSourceRef::Ephemeral {
                browse_session_id,
                entry_id,
            } => {
                let entry_ref = crate::file_workspace::BrowseEntryRef::Ephemeral {
                    browse_session_id: browse_session_id.clone(),
                    entry_id: entry_id.clone(),
                };
                let entry = self
                    .browse
                    .resolve_entry(&entry_ref)
                    .map_err(|_| SourceResolveError::SourceUnavailable)?;
                let metadata = fs::symlink_metadata(&entry.path).map_err(map_metadata_error)?;
                if !matches!(
                    entry.kind,
                    BrowseEntryKind::File | BrowseEntryKind::Directory
                ) {
                    return Err(SourceResolveError::SourceUnavailable);
                }
                let display_name = entry
                    .path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Browse entry")
                    .to_string();
                let extension = entry
                    .path
                    .extension()
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                Ok(ResolvedPreviewMetadata {
                    path: entry.path,
                    display_name,
                    extension,
                    size_bytes: metadata.len(),
                    modified_at_epoch_ms: metadata.modified().ok().and_then(system_time_millis),
                    entry_kind: match entry.kind {
                        BrowseEntryKind::File => PreviewSourceEntryKind::File,
                        BrowseEntryKind::Directory => PreviewSourceEntryKind::Directory,
                    },
                })
            }
            PreviewSourceRef::HostProvided { .. } => Err(SourceResolveError::SourceUnavailable),
        }
    }
}

fn map_folder_context_error(error: PreviewContextError) -> PreviewFolderEnumerationError {
    match error {
        PreviewContextError::Cancelled | PreviewContextError::StalePublication => {
            PreviewFolderEnumerationError::Cancelled
        }
        PreviewContextError::TimedOut => PreviewFolderEnumerationError::Deadline,
    }
}

fn map_folder_source_error(error: SourceResolveError) -> PreviewFolderEnumerationError {
    match error {
        SourceResolveError::SourceUnavailable => PreviewFolderEnumerationError::SourceUnavailable,
        SourceResolveError::PermissionDenied => PreviewFolderEnumerationError::PermissionDenied,
        SourceResolveError::IdentityChanged => PreviewFolderEnumerationError::IdentityChanged,
        SourceResolveError::Cancelled => PreviewFolderEnumerationError::Cancelled,
        SourceResolveError::Timeout => PreviewFolderEnumerationError::Deadline,
        SourceResolveError::MaterializationRequired
        | SourceResolveError::SourceMismatch
        | SourceResolveError::Failed => PreviewFolderEnumerationError::Failed,
    }
}

fn map_folder_browse_error(
    error: crate::file_workspace::browse::BrowseError,
) -> PreviewFolderEnumerationError {
    match error {
        crate::file_workspace::browse::BrowseError::DirectoryPermissionDenied
        | crate::file_workspace::browse::BrowseError::EntryPermissionDenied => {
            PreviewFolderEnumerationError::PermissionDenied
        }
        crate::file_workspace::browse::BrowseError::DirectoryNotFound
        | crate::file_workspace::browse::BrowseError::DirectoryUnavailable
        | crate::file_workspace::browse::BrowseError::SessionNotFound
        | crate::file_workspace::browse::BrowseError::InvalidLocationRef
        | crate::file_workspace::browse::BrowseError::InvalidPathRef
        | crate::file_workspace::browse::BrowseError::InvalidEntryRef
        | crate::file_workspace::browse::BrowseError::TargetNotDirectory => {
            PreviewFolderEnumerationError::SourceUnavailable
        }
        crate::file_workspace::browse::BrowseError::Cancelled => {
            PreviewFolderEnumerationError::Cancelled
        }
        crate::file_workspace::browse::BrowseError::StaleEnumeration
        | crate::file_workspace::browse::BrowseError::StalePublication => {
            PreviewFolderEnumerationError::IdentityChanged
        }
        crate::file_workspace::browse::BrowseError::UnsupportedEntry
        | crate::file_workspace::browse::BrowseError::EntryNotFound
        | crate::file_workspace::browse::BrowseError::EntryUnavailable
        | crate::file_workspace::browse::BrowseError::InvalidCursor
        | crate::file_workspace::browse::BrowseError::InvalidRequest
        | crate::file_workspace::browse::BrowseError::SortCapacityExceeded
        | crate::file_workspace::browse::BrowseError::InvalidPageSize
        | crate::file_workspace::browse::BrowseError::InvalidLimits
        | crate::file_workspace::browse::BrowseError::StateUnavailable
        | crate::file_workspace::browse::BrowseError::SessionCapacityExceeded
        | crate::file_workspace::browse::BrowseError::TemporaryStateCapacityExceeded => {
            PreviewFolderEnumerationError::Failed
        }
    }
}

impl FileWorkspaceRuntime {
    pub(crate) fn create_preview(
        &self,
        request: PreviewCreateRequest,
    ) -> Result<PreviewSnapshotDto, String> {
        self.ensure_live()?;
        let mut previews = self
            .inner
            .preview_sessions
            .lock()
            .map_err(|_| "workspace_preview_state_unavailable".to_string())?;
        if previews.len() >= MAX_PREVIEW_SESSIONS {
            return Err("preview_session_capacity_exceeded".to_string());
        }
        let host_capabilities =
            activated_host_capabilities(request.host_kind).map_err(str::to_string)?;
        let preview_id = FileWorkspaceRuntime::next_id("preview");
        let session = PreviewSession::new(PreviewSessionConfig::new(
            preview_id.clone(),
            request.request_id,
            request.source,
            PreviewHost::new(request.host_kind, host_capabilities),
        ));
        let snapshot = PreviewSnapshotDto::from_internal(preview_id.clone(), session.snapshot());
        previews.insert(preview_id, session);
        Ok(snapshot)
    }

    pub(crate) fn snapshot_preview(
        &self,
        request: PreviewSessionRequest,
    ) -> Result<PreviewSnapshotDto, String> {
        self.ensure_live()?;
        let session = self
            .inner
            .preview_sessions
            .lock()
            .map_err(|_| "workspace_preview_state_unavailable".to_string())?
            .get(&request.preview_id)
            .cloned()
            .ok_or_else(|| "preview_session_not_found".to_string())?;
        Ok(PreviewSnapshotDto::from_internal(
            request.preview_id,
            session.snapshot(),
        ))
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn native_preview_attach_failed(
        &self,
        snapshot: &PreviewSnapshotDto,
        presentation: &super::types::PreviewNativePresentation,
        error: String,
    ) -> Result<PreviewSnapshotDto, String> {
        let Some(source_version) = snapshot.source_version.as_deref() else {
            return Err(error);
        };
        let Some(representation) = snapshot
            .representation
            .as_ref()
            .map(|envelope| &envelope.representation)
        else {
            return Err(error);
        };
        if !matches!(
            representation,
            PreviewRepresentation::NativeOpaque { host, token }
                if *host == presentation.host
                    && token == &presentation.token
                    && source_version == presentation.source_version
                    && snapshot.host_kind == presentation.host
        ) {
            // A stale geometry failure must not tear down a newer native
            // representation that now owns the same preview ID.
            return Err(error);
        }

        let session = self
            .inner
            .preview_sessions
            .lock()
            .map_err(|_| "workspace_preview_state_unavailable".to_string())?
            .get(&snapshot.preview_id)
            .cloned()
            .ok_or_else(|| "preview_session_not_found".to_string())?;
        let expected_identity = NativePreviewFailureIdentity {
            request_id: snapshot.request_id.clone(),
            source_version: source_version.to_string(),
            host: presentation.host,
            token: presentation.token.clone(),
        };
        let fallback_applied =
            session.fallback_from_native_failure("native.macos.quick-look", &expected_identity);

        // Publication is invalidated before view detach. The access record
        // stays claimed until the main-thread owner clears Quick Look and
        // removes the view.
        let detach_error = self
            .inner
            .native_preview_host
            .detach(&snapshot.preview_id, Some(snapshot))
            .err()
            .map(|detach_error| format!("macos_quick_look_detach_failed:{detach_error}"));
        self.inner
            .native_preview_access
            .revoke_token_for_native_failure(&NativePreviewAccessResolveRequest {
                token: presentation.token.clone(),
                session_id: snapshot.session_id.clone(),
                request_id: snapshot.request_id.clone(),
                source_version: source_version.to_string(),
                host: presentation.host,
            });
        self.inner.preview_assets.revoke_request(
            &snapshot.preview_id,
            &snapshot.request_id,
            Some(source_version),
        );
        if !fallback_applied {
            return Err(detach_error.map_or(error.clone(), |detach_error| {
                format!("{error};{detach_error}")
            }));
        }
        if let Some(detach_error) = detach_error {
            return Err(format!("{error};{detach_error}"));
        }
        Ok(PreviewSnapshotDto::from_internal(
            snapshot.preview_id.clone(),
            session.snapshot(),
        ))
    }

    pub(crate) fn start_preview(
        &self,
        request: PreviewSessionRequest,
    ) -> Result<PreviewSnapshotDto, String> {
        self.ensure_live()?;
        let session = self
            .inner
            .preview_sessions
            .lock()
            .map_err(|_| "workspace_preview_state_unavailable".to_string())?
            .get(&request.preview_id)
            .cloned()
            .ok_or_else(|| "preview_session_not_found".to_string())?;
        let registry = Arc::clone(&self.inner.preview_registry);
        let preview_read = Arc::clone(&self.inner.preview_read);
        let asset_publisher: Arc<dyn crate::file_workspace::PreviewAssetPublisher> =
            self.inner.preview_assets.clone();
        let decoder_admission = Arc::new(
            crate::scheduler::adapters::PreviewDecoderResourceLeaseAdapter::new(Arc::clone(
                &self.inner.scheduler,
            )),
        );
        let archive_admission = Arc::new(
            crate::scheduler::adapters::PreviewArchiveResourceLeaseAdapter::new(Arc::clone(
                &self.inner.scheduler,
            )),
        );
        let task = session
            .start_with_environment(
                Arc::clone(&self.inner.preview_resolver) as Arc<dyn SourceResolver>,
                registry,
                PreviewProviderEnvironmentHandle {
                    content_read: None,
                    preview_read: Some(preview_read),
                    folder_enumeration: Some(self.inner.folder_enumeration.clone()),
                    asset_publisher: Some(asset_publisher),
                    decoder_admission: Some(decoder_admission),
                    archive_admission: Some(archive_admission),
                },
            )
            .map_err(map_preview_session_error)?;
        task.join().map_err(map_preview_run_error)?;
        Ok(PreviewSnapshotDto::from_internal(
            request.preview_id,
            session.snapshot(),
        ))
    }

    pub(crate) fn cancel_preview(&self, request: PreviewSessionRequest) -> Result<bool, String> {
        self.ensure_live()?;
        let session = self
            .inner
            .preview_sessions
            .lock()
            .map_err(|_| "workspace_preview_state_unavailable".to_string())?
            .get(&request.preview_id)
            .cloned()
            .ok_or_else(|| "preview_session_not_found".to_string())?;
        #[cfg(target_os = "macos")]
        let superseded =
            PreviewSnapshotDto::from_internal(request.preview_id.clone(), session.snapshot());
        let cancelled = session.cancel();
        #[cfg(target_os = "macos")]
        let detach_error = self
            .inner
            .native_preview_host
            .detach(&request.preview_id, Some(&superseded))
            .err();
        #[cfg(not(target_os = "macos"))]
        let detach_error: Option<String> = None;
        self.inner
            .native_preview_access
            .revoke_session(&request.preview_id);
        self.inner
            .preview_assets
            .revoke_session(&request.preview_id);
        if let Some(detach_error) = detach_error {
            return Err(detach_error);
        }
        Ok(cancelled)
    }

    pub(crate) fn dispose_preview(&self, request: PreviewSessionRequest) -> Result<bool, String> {
        self.ensure_live()?;
        let session = self
            .inner
            .preview_sessions
            .lock()
            .map_err(|_| "workspace_preview_state_unavailable".to_string())?
            .get(&request.preview_id)
            .cloned()
            .ok_or_else(|| "preview_session_not_found".to_string())?;
        #[cfg(target_os = "macos")]
        let superseded =
            PreviewSnapshotDto::from_internal(request.preview_id.clone(), session.snapshot());
        let disposed = session.dispose();
        #[cfg(target_os = "macos")]
        let detach_error = self
            .inner
            .native_preview_host
            .detach(&request.preview_id, Some(&superseded))
            .err();
        #[cfg(not(target_os = "macos"))]
        let detach_error: Option<String> = None;
        let remove_error = self
            .inner
            .preview_sessions
            .lock()
            .map(|mut sessions| {
                sessions.remove(&request.preview_id);
            })
            .map_err(|_| "workspace_preview_state_unavailable".to_string())
            .err();
        self.inner
            .native_preview_access
            .revoke_session(&request.preview_id);
        self.inner
            .preview_assets
            .revoke_session(&request.preview_id);
        if let Some(error) = detach_error.or(remove_error) {
            return Err(error);
        }
        Ok(disposed)
    }

    pub(crate) fn switch_preview_source(
        &self,
        request: PreviewSwitchSourceRequest,
    ) -> Result<PreviewSnapshotDto, String> {
        self.ensure_live()?;
        let session = self
            .inner
            .preview_sessions
            .lock()
            .map_err(|_| "workspace_preview_state_unavailable".to_string())?
            .get(&request.preview_id)
            .cloned()
            .ok_or_else(|| "preview_session_not_found".to_string())?;
        let superseded_session = session.snapshot();
        #[cfg(target_os = "macos")]
        let superseded = PreviewSnapshotDto::from_internal(
            request.preview_id.clone(),
            superseded_session.clone(),
        );
        session
            .switch_source(PreviewRequest {
                request_id: request.request_id,
                source: request.source,
            })
            .map_err(map_preview_session_error)?;
        #[cfg(target_os = "macos")]
        let detach_error = self
            .inner
            .native_preview_host
            .detach(&request.preview_id, Some(&superseded))
            .err();
        #[cfg(not(target_os = "macos"))]
        let detach_error: Option<String> = None;
        self.inner.native_preview_access.revoke_request(
            &request.preview_id,
            &superseded_session.request_id,
            superseded_session.source_version.as_deref(),
        );
        self.inner.preview_assets.revoke_request(
            &request.preview_id,
            &superseded_session.request_id,
            superseded_session.source_version.as_deref(),
        );
        if let Some(detach_error) = detach_error {
            return Err(detach_error);
        }
        Ok(PreviewSnapshotDto::from_internal(
            request.preview_id,
            session.snapshot(),
        ))
    }

    pub(crate) fn request_preview_asset(
        &self,
        request: PreviewAssetRequestDto,
    ) -> Result<PreviewAssetArtifactDto, String> {
        self.ensure_live()?;
        let asset_request = crate::file_workspace::preview_asset::PreviewAssetRequest {
            session_id: request.preview_id,
            request_id: request.request_id,
            source_version: request.source_version,
            asset_token: request.asset_token,
        };
        let artifact = match (request.offset_bytes, request.max_bytes) {
            (None, None) => self
                .inner
                .preview_assets
                .read(&asset_request)
                .map_err(|error| format!("preview_asset_{error}"))?,
            (Some(offset_bytes), Some(max_bytes)) => self
                .inner
                .preview_assets
                .read_range(
                    &asset_request,
                    BoundedContentReadRequest {
                        offset_bytes,
                        max_bytes,
                    },
                )
                .map_err(|error| format!("preview_asset_{error}"))?,
            _ => return Err("preview_asset_invalid_range_request".to_string()),
        };
        Ok(PreviewAssetArtifactDto {
            media_type: artifact.media_type,
            bytes: artifact.bytes,
        })
    }
}

fn map_context_error(error: PreviewContextError) -> SourceResolveError {
    match error {
        PreviewContextError::Cancelled => SourceResolveError::Cancelled,
        PreviewContextError::TimedOut => SourceResolveError::Timeout,
        PreviewContextError::StalePublication => SourceResolveError::Cancelled,
    }
}

fn map_metadata_error(error: std::io::Error) -> SourceResolveError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => SourceResolveError::PermissionDenied,
        std::io::ErrorKind::NotFound => SourceResolveError::SourceUnavailable,
        _ => SourceResolveError::Failed,
    }
}

fn namespace_source_version(path: &Path) -> Result<String, SourceResolveError> {
    let identity = capture_namespace_identity_only(path, None)
        .map_err(|_| SourceResolveError::SourceUnavailable)?;
    let encoded = serde_json::to_vec(&identity).map_err(|_| SourceResolveError::Failed)?;
    Ok(format!(
        "metadata-source-v1:{}",
        blake3::hash(&encoded).to_hex()
    ))
}

/// Metadata Preview may use a namespace-only version when W1-07 deliberately
/// declines byte-read eligibility for a directory/package or a
/// materialization-only source. Identity/availability failures never take
/// this fallback: swallowing those errors would publish a fresh-looking
/// version for a source whose identity could not be revalidated.
fn source_version_for_metadata(
    read_gate: &MaterializationReadGate,
    source: &PreviewSourceRef,
    path: &Path,
) -> Result<String, SourceResolveError> {
    match read_gate.current_source_version(source) {
        Ok(version) => Ok(version),
        Err(error) if metadata_source_version_fallback_allowed(error) => {
            namespace_source_version(path)
        }
        Err(error) => Err(map_read_gate_source_version_error(error)),
    }
}

fn metadata_source_version_fallback_allowed(error: ReadGateError) -> bool {
    matches!(
        error,
        ReadGateError::MaterializationRequired
            | ReadGateError::Downloading
            | ReadGateError::MetadataOnly
            | ReadGateError::SourceNotSupported
            | ReadGateError::PackageUnsupported
    )
}

fn map_read_gate_source_version_error(error: ReadGateError) -> SourceResolveError {
    match error {
        ReadGateError::SourceUnavailable => SourceResolveError::SourceUnavailable,
        ReadGateError::PermissionDenied => SourceResolveError::PermissionDenied,
        ReadGateError::IdentityChanged => SourceResolveError::IdentityChanged,
        ReadGateError::MaterializationRequired => SourceResolveError::MaterializationRequired,
        ReadGateError::AvailabilityUnknown => SourceResolveError::SourceUnavailable,
        ReadGateError::Downloading
        | ReadGateError::MetadataOnly
        | ReadGateError::SourceNotSupported
        | ReadGateError::PackageUnsupported
        | ReadGateError::Symlink
        | ReadGateError::LeaseInvalid
        | ReadGateError::InvalidRequest
        | ReadGateError::LeaseCapacityExceeded
        | ReadGateError::Disposed => SourceResolveError::Failed,
    }
}

fn materialization_for_eligibility(value: ContentReadEligibility) -> MaterializationState {
    match value {
        ContentReadEligibility::Eligible => MaterializationState::BoundaryReadable,
        ContentReadEligibility::MaterializationRequired => MaterializationState::RemotePlaceholder,
        ContentReadEligibility::Downloading => MaterializationState::Hydrating,
        ContentReadEligibility::MetadataOnly => MaterializationState::MetadataOnly,
        ContentReadEligibility::PermissionRequired
        | ContentReadEligibility::SourceUnavailable
        | ContentReadEligibility::SourceNotSupported
        | ContentReadEligibility::PackageUnsupported
        | ContentReadEligibility::Symlink
        | ContentReadEligibility::IdentityChanged => MaterializationState::Unavailable,
        ContentReadEligibility::AvailabilityUnknown => MaterializationState::Unknown,
    }
}

fn seconds_to_millis(value: i64) -> Option<i64> {
    value.checked_mul(1_000)
}

fn system_time_millis(value: std::time::SystemTime) -> Option<i64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn map_preview_session_error(error: crate::file_workspace::PreviewSessionError) -> String {
    match error {
        crate::file_workspace::PreviewSessionError::Disposed => {
            "preview_session_disposed".to_string()
        }
        crate::file_workspace::PreviewSessionError::InvalidRequest => {
            "preview_request_invalid".to_string()
        }
        crate::file_workspace::PreviewSessionError::AlreadyRunning => {
            "preview_session_already_running".to_string()
        }
        crate::file_workspace::PreviewSessionError::InvalidState(_) => {
            "preview_session_invalid_state".to_string()
        }
        crate::file_workspace::PreviewSessionError::ExecutionUnavailable(_) => {
            "preview_execution_unavailable".to_string()
        }
    }
}

fn map_preview_run_error(error: crate::file_workspace::PreviewRunError) -> String {
    match error {
        crate::file_workspace::PreviewRunError::Session(error) => map_preview_session_error(error),
        crate::file_workspace::PreviewRunError::SourceResolver(error) => match error {
            SourceResolveError::SourceUnavailable => "preview_source_unavailable".to_string(),
            SourceResolveError::MaterializationRequired => {
                "preview_materialization_required".to_string()
            }
            SourceResolveError::PermissionDenied => "preview_permission_denied".to_string(),
            SourceResolveError::IdentityChanged => "preview_source_identity_changed".to_string(),
            SourceResolveError::Timeout => "preview_source_timeout".to_string(),
            SourceResolveError::Cancelled => "preview_cancelled".to_string(),
            SourceResolveError::SourceMismatch => "preview_source_mismatch".to_string(),
            SourceResolveError::Failed => "preview_source_resolution_failed".to_string(),
        },
        crate::file_workspace::PreviewRunError::ProviderTerminal { condition, .. } => {
            match condition {
                crate::file_workspace::PreviewTerminalCondition::SourceUnavailable => {
                    "preview_source_unavailable".to_string()
                }
                crate::file_workspace::PreviewTerminalCondition::MaterializationRequired => {
                    "preview_materialization_required".to_string()
                }
                crate::file_workspace::PreviewTerminalCondition::PermissionDenied => {
                    "preview_permission_denied".to_string()
                }
                crate::file_workspace::PreviewTerminalCondition::IdentityChanged => {
                    "preview_source_identity_changed".to_string()
                }
                crate::file_workspace::PreviewTerminalCondition::Cancelled => {
                    "preview_cancelled".to_string()
                }
            }
        }
        crate::file_workspace::PreviewRunError::Cancelled => "preview_cancelled".to_string(),
        crate::file_workspace::PreviewRunError::StalePublication => {
            "preview_stale_publication".to_string()
        }
        crate::file_workspace::PreviewRunError::ExecutionUnavailable(_) => {
            "preview_execution_unavailable".to_string()
        }
        crate::file_workspace::PreviewRunError::WorkerPanicked => {
            "preview_worker_failed".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        map_preview_run_error, map_read_gate_source_version_error,
        metadata_source_version_fallback_allowed, ReadGateError, SourceResolveError,
    };
    use crate::file_workspace::{PreviewRunError, PreviewTerminalCondition};

    #[test]
    fn metadata_version_fallback_does_not_swallow_identity_or_availability_failures() {
        assert!(metadata_source_version_fallback_allowed(
            ReadGateError::SourceNotSupported
        ));
        assert!(metadata_source_version_fallback_allowed(
            ReadGateError::MaterializationRequired
        ));
        assert!(!metadata_source_version_fallback_allowed(
            ReadGateError::IdentityChanged
        ));
        assert!(!metadata_source_version_fallback_allowed(
            ReadGateError::SourceUnavailable
        ));
        assert!(!metadata_source_version_fallback_allowed(
            ReadGateError::AvailabilityUnknown
        ));
    }

    #[test]
    fn metadata_source_version_preserves_availability_unknown_as_source_unavailable() {
        assert_eq!(
            map_read_gate_source_version_error(ReadGateError::SourceUnavailable),
            SourceResolveError::SourceUnavailable
        );
        assert_eq!(
            map_read_gate_source_version_error(ReadGateError::AvailabilityUnknown),
            SourceResolveError::SourceUnavailable
        );
    }

    #[test]
    fn provider_terminal_errors_preserve_exact_wire_condition() {
        let cases = [
            (
                PreviewTerminalCondition::SourceUnavailable,
                "preview_source_unavailable",
            ),
            (
                PreviewTerminalCondition::MaterializationRequired,
                "preview_materialization_required",
            ),
            (
                PreviewTerminalCondition::PermissionDenied,
                "preview_permission_denied",
            ),
            (
                PreviewTerminalCondition::IdentityChanged,
                "preview_source_identity_changed",
            ),
            (PreviewTerminalCondition::Cancelled, "preview_cancelled"),
        ];

        for (condition, expected) in cases {
            assert_eq!(
                map_preview_run_error(PreviewRunError::ProviderTerminal {
                    provider_id: "builtin.text".to_string(),
                    condition,
                }),
                expected
            );
        }
    }
}
