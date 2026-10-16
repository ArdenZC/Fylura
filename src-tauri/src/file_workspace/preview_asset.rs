//! Bounded Preview-specific asset publication and retrieval.
//!
//! Asset bytes are an ephemeral projection owned by one Preview runtime. The
//! registry is not a file server: providers submit already-authorized bounded
//! bytes through the Preview environment and consumers must present the exact
//! session/request/sourceVersion/token tuple to retrieve them. Large local
//! PDFs use a range-backed record instead of storing or copying the whole
//! source. Each range still goes through the existing one-megabyte Read Gate.

use super::contracts::PreviewSourceRef;
use super::preview::{
    BoundedContentReadRequest, PreviewAssetError, PreviewAssetPublisher, PreviewCancellation,
    PreviewContentReadAccess, PreviewContextError, PreviewOperationContext, PreviewReadAccessError,
};
#[cfg(test)]
use std::sync::Condvar;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use thiserror::Error;
use uuid::Uuid;

pub(crate) const MAX_PREVIEW_ASSETS: usize = 64;
pub(crate) const MAX_PREVIEW_ASSET_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_PREVIEW_ASSET_TOTAL_BYTES: usize = 32 * 1024 * 1024;
/// Maximum logical source length for one range-backed Preview asset. This is a
/// resource policy, not a read size: no source bytes are reserved or copied by
/// publication, and every request remains bounded by the Read Gate ceiling.
pub(crate) const MAX_PREVIEW_RANGE_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const PREVIEW_ASSET_TTL: Duration = Duration::from_secs(30);
const MAX_MEDIA_TYPE_BYTES: usize = 256;

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreviewAssetArtifact {
    pub(crate) media_type: String,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreviewAssetRequest {
    pub(crate) session_id: String,
    pub(crate) request_id: String,
    pub(crate) source_version: String,
    pub(crate) asset_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum PreviewAssetReadError {
    #[error("preview asset token is invalid or stale")]
    InvalidOrStale,
    #[error("preview asset registry is disposed")]
    Disposed,
    #[error("preview asset range request is invalid")]
    InvalidRange,
    #[error("preview asset range backing is unavailable")]
    RangeUnsupported,
    #[error("preview asset range read failed")]
    ReadFailed,
}

#[derive(Debug)]
enum AssetBacking {
    Bytes(Vec<u8>),
    Range {
        source: PreviewSourceRef,
        cancellation: PreviewCancellation,
        length_bytes: u64,
    },
}

#[derive(Debug)]
struct AssetRecord {
    session_id: String,
    request_id: String,
    source_version: String,
    media_type: String,
    backing: AssetBacking,
    expires_at: Instant,
}

#[derive(Debug, Default)]
struct AssetState {
    records: HashMap<String, AssetRecord>,
    total_bytes: usize,
    disposed: bool,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct RevokeGateState {
    entered: bool,
    released: bool,
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct PreviewAssetRevokeGate {
    state: Mutex<RevokeGateState>,
    wake: Condvar,
}

#[cfg(test)]
impl PreviewAssetRevokeGate {
    pub(crate) fn wait_until_entered(&self) {
        let mut state = lock(&self.state);
        while !state.entered {
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub(crate) fn release(&self) {
        let mut state = lock(&self.state);
        state.released = true;
        self.wake.notify_all();
    }

    fn pause(&self) {
        let mut state = lock(&self.state);
        state.entered = true;
        self.wake.notify_all();
        while !state.released {
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct PreviewAssetPublishGate {
    state: Mutex<RevokeGateState>,
    wake: Condvar,
}

#[cfg(test)]
impl PreviewAssetPublishGate {
    pub(crate) fn wait_until_entered(&self) {
        let mut state = lock(&self.state);
        while !state.entered {
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub(crate) fn release(&self) {
        let mut state = lock(&self.state);
        state.released = true;
        self.wake.notify_all();
    }

    fn pause(&self) {
        let mut state = lock(&self.state);
        state.entered = true;
        self.wake.notify_all();
        while !state.released {
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

/// Process-local, bounded and disposable Preview asset owner.
pub(crate) struct PreviewAssetRegistry {
    state: Mutex<AssetState>,
    range_reader: Option<Arc<dyn PreviewContentReadAccess>>,
    #[cfg(test)]
    revoke_gate: Mutex<Option<Arc<PreviewAssetRevokeGate>>>,
    #[cfg(test)]
    cleanup_gate: Mutex<Option<Arc<PreviewAssetRevokeGate>>>,
    #[cfg(test)]
    publish_gate: Mutex<Option<Arc<PreviewAssetPublishGate>>>,
}

impl PreviewAssetRegistry {
    #[cfg(test)]
    pub(crate) fn new() -> Arc<Self> {
        Self::new_with_optional_range_reader(None)
    }

    pub(crate) fn new_with_range_reader(
        range_reader: Arc<dyn PreviewContentReadAccess>,
    ) -> Arc<Self> {
        Self::new_with_optional_range_reader(Some(range_reader))
    }

    fn new_with_optional_range_reader(
        range_reader: Option<Arc<dyn PreviewContentReadAccess>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(AssetState::default()),
            range_reader,
            #[cfg(test)]
            revoke_gate: Mutex::new(None),
            #[cfg(test)]
            cleanup_gate: Mutex::new(None),
            #[cfg(test)]
            publish_gate: Mutex::new(None),
        })
    }

    pub(crate) fn read(
        &self,
        request: &PreviewAssetRequest,
    ) -> Result<PreviewAssetArtifact, PreviewAssetReadError> {
        validate_request(request)?;
        let mut state = lock(&self.state);
        if state.disposed {
            return Err(PreviewAssetReadError::Disposed);
        }
        prune_expired(&mut state);
        let record = state
            .records
            .get(request.asset_token.as_str())
            .ok_or(PreviewAssetReadError::InvalidOrStale)?;
        if record.session_id != request.session_id
            || record.request_id != request.request_id
            || record.source_version != request.source_version
        {
            return Err(PreviewAssetReadError::InvalidOrStale);
        }
        match &record.backing {
            AssetBacking::Bytes(bytes) => Ok(PreviewAssetArtifact {
                media_type: record.media_type.clone(),
                bytes: bytes.clone(),
            }),
            AssetBacking::Range { .. } => Err(PreviewAssetReadError::InvalidOrStale),
        }
    }

    /// Read one exact, bounded range from an opaque range-backed asset. The
    /// registry deliberately drops its state lock while the existing Read Gate
    /// re-resolves permissions, materialization and source identity.
    pub(crate) fn read_range(
        &self,
        request: &PreviewAssetRequest,
        range: BoundedContentReadRequest,
    ) -> Result<PreviewAssetArtifact, PreviewAssetReadError> {
        validate_request(request)?;
        if range.max_bytes == 0 || range.max_bytes > 1024 * 1024 {
            return Err(PreviewAssetReadError::InvalidRange);
        }

        let (media_type, source, source_version, cancellation, length_bytes, reader) = {
            let mut state = lock(&self.state);
            if state.disposed {
                return Err(PreviewAssetReadError::Disposed);
            }
            prune_expired(&mut state);
            let record = state
                .records
                .get(request.asset_token.as_str())
                .ok_or(PreviewAssetReadError::InvalidOrStale)?;
            if record.session_id != request.session_id
                || record.request_id != request.request_id
                || record.source_version != request.source_version
            {
                return Err(PreviewAssetReadError::InvalidOrStale);
            }
            let AssetBacking::Range {
                source,
                cancellation,
                length_bytes,
            } = &record.backing
            else {
                return Err(PreviewAssetReadError::InvalidRange);
            };
            let end = range
                .offset_bytes
                .checked_add(u64::from(range.max_bytes))
                .ok_or(PreviewAssetReadError::InvalidRange)?;
            if range.offset_bytes > *length_bytes || end > *length_bytes {
                return Err(PreviewAssetReadError::InvalidRange);
            }
            let reader = self
                .range_reader
                .clone()
                .ok_or(PreviewAssetReadError::RangeUnsupported)?;
            (
                record.media_type.clone(),
                source.clone(),
                record.source_version.clone(),
                cancellation.clone(),
                *length_bytes,
                reader,
            )
        };

        if cancellation.is_cancelled() {
            return Err(PreviewAssetReadError::InvalidOrStale);
        }
        let context = PreviewOperationContext::for_backend_content_read(
            request.session_id.clone(),
            request.request_id.clone(),
            source_version.clone(),
            cancellation.clone(),
            Instant::now() + Duration::from_secs(10),
        );
        let read = reader
            .read_source_bounded(&source, &source_version, range, &context)
            .map_err(map_range_read_error)?;
        context
            .ensure_active()
            .map_err(|_| PreviewAssetReadError::InvalidOrStale)?;
        let requested_bytes = usize::try_from(range.max_bytes).unwrap_or(usize::MAX);
        let range_end = range
            .offset_bytes
            .checked_add(u64::from(range.max_bytes))
            .ok_or(PreviewAssetReadError::InvalidRange)?;
        if read.bytes.len() != requested_bytes || (range_end == length_bytes && !read.complete) {
            return Err(PreviewAssetReadError::ReadFailed);
        }

        let mut state = lock(&self.state);
        if state.disposed {
            return Err(PreviewAssetReadError::Disposed);
        }
        prune_expired(&mut state);
        let record = state
            .records
            .get(request.asset_token.as_str())
            .ok_or(PreviewAssetReadError::InvalidOrStale)?;
        if record.session_id != request.session_id
            || record.request_id != request.request_id
            || record.source_version != request.source_version
            || cancellation.is_cancelled()
            || !matches!(
                &record.backing,
                AssetBacking::Range {
                    length_bytes: current_length,
                    ..
                } if *current_length == length_bytes
            )
        {
            return Err(PreviewAssetReadError::InvalidOrStale);
        }
        Ok(PreviewAssetArtifact {
            media_type,
            bytes: read.bytes,
        })
    }

    pub(crate) fn revoke_session(&self, session_id: &str) {
        #[cfg(test)]
        self.pause_before_cleanup_for_test();
        let mut state = lock(&self.state);
        remove_where(&mut state, |record| record.session_id == session_id);
        drop(state);
        #[cfg(test)]
        self.pause_after_revoke_for_test();
    }

    #[cfg(test)]
    pub(crate) fn set_revoke_gate_for_test(&self, gate: Option<Arc<PreviewAssetRevokeGate>>) {
        *lock(&self.revoke_gate) = gate;
    }

    #[cfg(test)]
    pub(crate) fn set_cleanup_gate_for_test(&self, gate: Option<Arc<PreviewAssetRevokeGate>>) {
        *lock(&self.cleanup_gate) = gate;
    }

    #[cfg(test)]
    pub(crate) fn set_publish_gate_for_test(&self, gate: Option<Arc<PreviewAssetPublishGate>>) {
        *lock(&self.publish_gate) = gate;
    }

    #[cfg(test)]
    fn pause_before_cleanup_for_test(&self) {
        let gate = lock(&self.cleanup_gate).clone();
        if let Some(gate) = gate {
            gate.pause();
        }
    }

    #[cfg(test)]
    fn pause_after_revoke_for_test(&self) {
        let gate = lock(&self.revoke_gate).clone();
        if let Some(gate) = gate {
            gate.pause();
        }
    }

    #[cfg(test)]
    fn pause_before_registry_lock_for_test(&self) {
        let gate = lock(&self.publish_gate).clone();
        if let Some(gate) = gate {
            gate.pause();
        }
    }

    pub(crate) fn revoke_request(
        &self,
        session_id: &str,
        request_id: &str,
        source_version: Option<&str>,
    ) {
        #[cfg(test)]
        self.pause_before_cleanup_for_test();
        let Some(source_version) = source_version else {
            return;
        };
        let mut state = lock(&self.state);
        remove_where(&mut state, |record| {
            record.session_id == session_id
                && record.request_id == request_id
                && record.source_version == source_version
        });
        drop(state);
        #[cfg(test)]
        self.pause_after_revoke_for_test();
    }

    pub(crate) fn dispose(&self) {
        let mut state = lock(&self.state);
        state.records.clear();
        state.total_bytes = 0;
        state.disposed = true;
    }

    #[cfg(test)]
    pub(crate) fn retained_counts_for_test(&self) -> (usize, usize) {
        let state = lock(&self.state);
        (state.records.len(), state.total_bytes)
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize) {
        let mut state = lock(&self.state);
        prune_expired(&mut state);
        (state.records.len(), state.total_bytes)
    }
}

impl PreviewAssetPublisher for PreviewAssetRegistry {
    fn publish_asset(
        &self,
        context: &PreviewOperationContext,
        media_type: &str,
        bytes: Vec<u8>,
    ) -> Result<String, PreviewAssetError> {
        context.ensure_active().map_err(map_context_error)?;
        #[cfg(test)]
        self.pause_before_registry_lock_for_test();
        validate_media_type(media_type)?;
        if bytes.len() > MAX_PREVIEW_ASSET_BYTES {
            return Err(PreviewAssetError::OutputTooLarge);
        }

        let mut state = lock(&self.state);
        context.ensure_active().map_err(map_context_error)?;
        if state.disposed {
            return Err(PreviewAssetError::Disposed);
        }
        prune_expired(&mut state);
        let new_total = state
            .total_bytes
            .checked_add(bytes.len())
            .ok_or(PreviewAssetError::CapacityExceeded)?;
        if state.records.len() >= MAX_PREVIEW_ASSETS || new_total > MAX_PREVIEW_ASSET_TOTAL_BYTES {
            return Err(PreviewAssetError::CapacityExceeded);
        }
        let source_version = context
            .source_version()
            .ok_or(PreviewAssetError::StalePublication)?
            .to_string();
        let token = format!("preview-asset-{}", Uuid::new_v4());
        state.total_bytes = new_total;
        state.records.insert(
            token.clone(),
            AssetRecord {
                session_id: context.session_id().to_string(),
                request_id: context.request_id().to_string(),
                source_version,
                media_type: media_type.to_string(),
                backing: AssetBacking::Bytes(bytes),
                expires_at: Instant::now() + PREVIEW_ASSET_TTL,
            },
        );
        Ok(token)
    }

    fn publish_range_asset(
        &self,
        context: &PreviewOperationContext,
        media_type: &str,
        source: &PreviewSourceRef,
        source_version: &str,
        length_bytes: u64,
    ) -> Result<String, PreviewAssetError> {
        context.ensure_active().map_err(map_context_error)?;
        if self.range_reader.is_none() {
            return Err(PreviewAssetError::RangeUnsupported);
        }
        validate_media_type(media_type)?;
        if source_version.is_empty() || context.source_version() != Some(source_version) {
            return Err(PreviewAssetError::StalePublication);
        }
        if length_bytes > MAX_PREVIEW_RANGE_SOURCE_BYTES {
            return Err(PreviewAssetError::OutputTooLarge);
        }

        let mut state = lock(&self.state);
        context.ensure_active().map_err(map_context_error)?;
        if state.disposed {
            return Err(PreviewAssetError::Disposed);
        }
        prune_expired(&mut state);
        if state.records.len() >= MAX_PREVIEW_ASSETS {
            return Err(PreviewAssetError::CapacityExceeded);
        }
        let token = format!("preview-asset-{}", Uuid::new_v4());
        state.records.insert(
            token.clone(),
            AssetRecord {
                session_id: context.session_id().to_string(),
                request_id: context.request_id().to_string(),
                source_version: source_version.to_string(),
                media_type: media_type.to_string(),
                backing: AssetBacking::Range {
                    source: source.clone(),
                    cancellation: context.cancellation(),
                    length_bytes,
                },
                expires_at: Instant::now() + PREVIEW_ASSET_TTL,
            },
        );
        Ok(token)
    }
}

fn map_context_error(error: PreviewContextError) -> PreviewAssetError {
    match error {
        PreviewContextError::Cancelled => PreviewAssetError::Cancelled,
        PreviewContextError::TimedOut | PreviewContextError::StalePublication => {
            PreviewAssetError::StalePublication
        }
    }
}

fn validate_request(request: &PreviewAssetRequest) -> Result<(), PreviewAssetReadError> {
    if !valid_opaque(&request.session_id)
        || !valid_opaque(&request.request_id)
        || !valid_opaque(&request.source_version)
        || !valid_opaque(&request.asset_token)
        || request.asset_token.contains('/')
        || request.asset_token.contains('\\')
    {
        return Err(PreviewAssetReadError::InvalidOrStale);
    }
    Ok(())
}

fn validate_media_type(media_type: &str) -> Result<(), PreviewAssetError> {
    if media_type.is_empty()
        || media_type.len() > MAX_MEDIA_TYPE_BYTES
        || media_type.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(PreviewAssetError::InvalidMediaType);
    }
    Ok(())
}

fn map_range_read_error(error: PreviewReadAccessError) -> PreviewAssetReadError {
    match error {
        PreviewReadAccessError::Cancelled
        | PreviewReadAccessError::TimedOut
        | PreviewReadAccessError::LeaseInvalid
        | PreviewReadAccessError::SourceVersionMismatch => PreviewAssetReadError::InvalidOrStale,
        PreviewReadAccessError::PermissionDenied
        | PreviewReadAccessError::SourceUnavailable
        | PreviewReadAccessError::MaterializationRequired
        | PreviewReadAccessError::MetadataOnly
        | PreviewReadAccessError::Failed => PreviewAssetReadError::ReadFailed,
    }
}

fn prune_expired(state: &mut AssetState) {
    let now = Instant::now();
    remove_where(state, |record| record.expires_at <= now);
}

fn remove_where<F>(state: &mut AssetState, mut predicate: F)
where
    F: FnMut(&AssetRecord) -> bool,
{
    let tokens: Vec<String> = state
        .records
        .iter()
        .filter(|(_, record)| predicate(record))
        .map(|(token, _)| token.clone())
        .collect();
    for token in tokens {
        if let Some(record) = state.records.remove(&token) {
            let bytes = match record.backing {
                AssetBacking::Bytes(bytes) => bytes.len(),
                AssetBacking::Range { .. } => 0,
            };
            state.total_bytes = state.total_bytes.saturating_sub(bytes);
        }
    }
}

fn valid_opaque(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_workspace::preview::{PreviewCancellation, PreviewOperationContext};
    use std::{sync::Mutex, time::Instant};

    fn context(
        session_id: &str,
        request_id: &str,
        source_version: &str,
    ) -> PreviewOperationContext {
        PreviewOperationContext::for_backend_content_read(
            session_id,
            request_id,
            source_version,
            PreviewCancellation::default(),
            Instant::now() + Duration::from_secs(5),
        )
    }

    #[test]
    fn asset_is_opaque_bound_and_revocable() {
        let registry = PreviewAssetRegistry::new();
        let context = context("preview-1", "request-1", "version-1");
        let token = registry
            .publish_asset(&context, "image/png", vec![1, 2, 3])
            .expect("publish asset");
        assert!(token.starts_with("preview-asset-"));
        assert!(!token.contains('/') && !token.contains('\\'));
        let artifact = registry
            .read(&PreviewAssetRequest {
                session_id: "preview-1".to_string(),
                request_id: "request-1".to_string(),
                source_version: "version-1".to_string(),
                asset_token: token.clone(),
            })
            .expect("read asset");
        assert_eq!(artifact.media_type, "image/png");
        assert_eq!(artifact.bytes, vec![1, 2, 3]);
        assert_eq!(
            registry.read(&PreviewAssetRequest {
                session_id: "preview-2".to_string(),
                request_id: "request-1".to_string(),
                source_version: "version-1".to_string(),
                asset_token: token.clone(),
            }),
            Err(PreviewAssetReadError::InvalidOrStale)
        );
        registry.revoke_request("preview-1", "request-1", Some("version-1"));
        assert_eq!(registry.counts(), (0, 0));
        assert_eq!(
            registry.read(&PreviewAssetRequest {
                session_id: "preview-1".to_string(),
                request_id: "request-1".to_string(),
                source_version: "version-1".to_string(),
                asset_token: token,
            }),
            Err(PreviewAssetReadError::InvalidOrStale)
        );
    }

    #[test]
    fn capacity_and_size_are_bounded() {
        let registry = PreviewAssetRegistry::new();
        let context = context("preview-1", "request-1", "version-1");
        assert_eq!(
            registry.publish_asset(&context, "image/png", vec![0; MAX_PREVIEW_ASSET_BYTES + 1]),
            Err(PreviewAssetError::OutputTooLarge)
        );
        assert_eq!(
            registry.publish_asset(&context, "", vec![1]),
            Err(PreviewAssetError::InvalidMediaType)
        );
        registry.dispose();
        assert_eq!(
            registry.publish_asset(&context, "image/png", vec![1]),
            Err(PreviewAssetError::Disposed)
        );
    }

    struct FakeRangeReader {
        calls: Mutex<Vec<BoundedContentReadRequest>>,
    }

    impl PreviewContentReadAccess for FakeRangeReader {
        fn read_source_bounded(
            &self,
            source: &PreviewSourceRef,
            source_version: &str,
            request: BoundedContentReadRequest,
            context: &PreviewOperationContext,
        ) -> Result<crate::file_workspace::preview::BoundedContentRead, PreviewReadAccessError>
        {
            context
                .ensure_active()
                .map_err(|_| PreviewReadAccessError::Cancelled)?;
            assert!(matches!(
                source,
                PreviewSourceRef::Managed { file_id } if file_id == "file-1"
            ));
            assert_eq!(source_version, "version-1");
            self.calls.lock().expect("range call lock").push(request);
            Ok(crate::file_workspace::preview::BoundedContentRead {
                bytes: vec![0x5a; request.max_bytes as usize],
                complete: true,
            })
        }
    }

    #[test]
    fn range_asset_keeps_logical_sizes_outside_byte_asset_ceiling_bounded() {
        let reader = Arc::new(FakeRangeReader {
            calls: Mutex::new(Vec::new()),
        });
        let registry = PreviewAssetRegistry::new_with_range_reader(reader.clone());
        let context = context("preview-1", "request-1", "version-1");
        let source = PreviewSourceRef::Managed {
            file_id: "file-1".to_string(),
        };

        for length_bytes in [2 * 1024 * 1024, 10 * 1024 * 1024, 20 * 1024 * 1024] {
            let token = registry
                .publish_range_asset(
                    &context,
                    "application/pdf",
                    &source,
                    "version-1",
                    length_bytes,
                )
                .expect("publish bounded range asset");
            let artifact = registry
                .read_range(
                    &PreviewAssetRequest {
                        session_id: "preview-1".to_string(),
                        request_id: "request-1".to_string(),
                        source_version: "version-1".to_string(),
                        asset_token: token,
                    },
                    BoundedContentReadRequest {
                        offset_bytes: length_bytes - 1024,
                        max_bytes: 1024,
                    },
                )
                .expect("read bounded range");
            assert_eq!(artifact.media_type, "application/pdf");
            assert_eq!(artifact.bytes.len(), 1024);
        }
        assert_eq!(
            reader.calls.lock().expect("range call lock").len(),
            3,
            "a range read is one gate-bounded request regardless of logical PDF length"
        );
        assert_eq!(
            registry.publish_range_asset(
                &context,
                "application/pdf",
                &source,
                "version-1",
                MAX_PREVIEW_RANGE_SOURCE_BYTES + 1,
            ),
            Err(PreviewAssetError::OutputTooLarge)
        );
    }

    #[test]
    fn range_asset_rejects_invalid_identity_ranges_and_cancelled_reads() {
        let reader = Arc::new(FakeRangeReader {
            calls: Mutex::new(Vec::new()),
        });
        let registry = PreviewAssetRegistry::new_with_range_reader(reader);
        let context = context("preview-1", "request-1", "version-1");
        let source = PreviewSourceRef::Managed {
            file_id: "file-1".to_string(),
        };
        let token = registry
            .publish_range_asset(
                &context,
                "application/pdf",
                &source,
                "version-1",
                2 * 1024 * 1024,
            )
            .expect("publish range asset");
        let request = PreviewAssetRequest {
            session_id: "preview-1".to_string(),
            request_id: "request-1".to_string(),
            source_version: "version-1".to_string(),
            asset_token: token,
        };
        assert_eq!(
            registry.read_range(
                &request,
                BoundedContentReadRequest {
                    offset_bytes: 2 * 1024 * 1024 - 512,
                    max_bytes: 1024,
                }
            ),
            Err(PreviewAssetReadError::InvalidRange)
        );
        let mut wrong_request = request.clone();
        wrong_request.request_id = "other-request".to_string();
        assert_eq!(
            registry.read_range(
                &wrong_request,
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1024,
                }
            ),
            Err(PreviewAssetReadError::InvalidOrStale)
        );
        context.cancellation().cancel();
        assert_eq!(
            registry.read_range(
                &request,
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 1024,
                }
            ),
            Err(PreviewAssetReadError::InvalidOrStale)
        );
    }
}
