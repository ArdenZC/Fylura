//! App adapter for the one shared HostProvided implementation.
//!
//! The lifecycle, bounds, cancellation and post-read revalidation live in
//! `fylura-native-host`, which is also consumed by the Windows handler.
//! This adapter only translates the app's existing `PreviewHostKind` enum so
//! the app-side Preview contracts remain source-compatible.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the shared HostProvided adapter is activated by native preview bridge tracks"
    )
)]

use crate::file_workspace::{contracts::PreviewHostKind, preview::BoundedContentRead};
use std::sync::Arc;

#[allow(unused_imports)]
pub(crate) use fylura_native_host::{
    HostProvidedHandle, HostProvidedReadContext, HostProvidedReadSource, HostProvidedSourceError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostProvidedConfig {
    pub(crate) max_records: usize,
    pub(crate) max_read_bytes: u32,
    pub(crate) ttl: std::time::Duration,
}

impl Default for HostProvidedConfig {
    fn default() -> Self {
        let shared = fylura_native_host::HostProvidedConfig::default();
        Self {
            max_records: shared.max_records,
            max_read_bytes: shared.max_read_bytes,
            ttl: shared.ttl,
        }
    }
}

#[derive(Clone)]
pub(crate) struct HostProvidedRegistration {
    pub(crate) host: PreviewHostKind,
    pub(crate) generation_id: String,
    pub(crate) source: Arc<dyn HostProvidedReadSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostProvidedReadRequest {
    pub(crate) host_token: String,
    pub(crate) host: PreviewHostKind,
    pub(crate) generation_id: String,
    pub(crate) offset_bytes: u64,
    pub(crate) max_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum HostProvidedError {
    #[error("host-provided request is invalid")]
    InvalidRequest,
    #[error("native shell host is not activated for HostProvided registration")]
    UnsupportedHost,
    #[error("host-provided registry is at capacity")]
    CapacityExceeded,
    #[error("host-provided token is invalid or stale")]
    InvalidOrStale,
    #[error("host-provided registry is disposed")]
    Disposed,
    #[error("host-provided source is unavailable")]
    SourceUnavailable,
    #[error("host-provided source permission was denied")]
    PermissionDenied,
    #[error("host-provided source request was cancelled")]
    Cancelled,
    #[error("host-provided source read failed")]
    Failed,
}

pub(crate) struct HostProvidedRegistry {
    inner: Arc<fylura_native_host::HostProvidedRegistry>,
}

impl HostProvidedRegistry {
    pub(crate) fn new(config: HostProvidedConfig) -> Result<Arc<Self>, HostProvidedError> {
        let shared = fylura_native_host::HostProvidedConfig {
            max_records: config.max_records,
            max_read_bytes: config.max_read_bytes,
            ttl: config.ttl,
        };
        let inner = fylura_native_host::HostProvidedRegistry::new(shared).map_err(map_error)?;
        Ok(Arc::new(Self { inner }))
    }

    pub(crate) fn register(
        &self,
        registration: HostProvidedRegistration,
    ) -> Result<HostProvidedHandle, HostProvidedError> {
        self.inner
            .register(fylura_native_host::HostProvidedRegistration {
                host: map_host(registration.host),
                generation_id: registration.generation_id,
                source: registration.source,
            })
            .map_err(map_error)
    }

    pub(crate) fn read(
        &self,
        request: &HostProvidedReadRequest,
    ) -> Result<BoundedContentRead, HostProvidedError> {
        self.inner
            .read(&fylura_native_host::HostProvidedReadRequest {
                host_token: request.host_token.clone(),
                host: map_host(request.host),
                generation_id: request.generation_id.clone(),
                offset_bytes: request.offset_bytes,
                max_bytes: request.max_bytes,
            })
            .map_err(map_error)
    }

    pub(crate) fn revoke(
        &self,
        host_token: &str,
        host: PreviewHostKind,
        generation_id: &str,
    ) -> bool {
        self.inner.revoke(host_token, map_host(host), generation_id)
    }

    pub(crate) fn revoke_generation(&self, host: PreviewHostKind, generation_id: &str) -> usize {
        self.inner.revoke_generation(map_host(host), generation_id)
    }

    pub(crate) fn dispose(&self) {
        self.inner.dispose();
    }

    #[cfg(test)]
    fn state_lock_is_available_for_test(&self) -> bool {
        self.inner.state_lock_is_available_for_test()
    }

    #[cfg(test)]
    fn force_expire_for_test(&self, host_token: &str) {
        self.inner.force_expire_for_test(host_token);
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.inner.count()
    }
}

fn map_host(host: PreviewHostKind) -> fylura_native_host::HostProvidedHost {
    match host {
        PreviewHostKind::ZenFloating => fylura_native_host::HostProvidedHost::ZenFloating,
        PreviewHostKind::ZenPinned => fylura_native_host::HostProvidedHost::ZenPinned,
        PreviewHostKind::MacQuickLookExtension => {
            fylura_native_host::HostProvidedHost::MacQuickLookExtension
        }
        PreviewHostKind::WindowsQuickPreview => {
            fylura_native_host::HostProvidedHost::WindowsQuickPreview
        }
        PreviewHostKind::WindowsPreviewHandler => {
            fylura_native_host::HostProvidedHost::WindowsPreviewHandler
        }
    }
}

fn map_error(error: fylura_native_host::HostProvidedError) -> HostProvidedError {
    match error {
        fylura_native_host::HostProvidedError::InvalidRequest => HostProvidedError::InvalidRequest,
        fylura_native_host::HostProvidedError::UnsupportedHost => {
            HostProvidedError::UnsupportedHost
        }
        fylura_native_host::HostProvidedError::CapacityExceeded => {
            HostProvidedError::CapacityExceeded
        }
        fylura_native_host::HostProvidedError::InvalidOrStale => HostProvidedError::InvalidOrStale,
        fylura_native_host::HostProvidedError::Disposed => HostProvidedError::Disposed,
        fylura_native_host::HostProvidedError::SourceUnavailable => {
            HostProvidedError::SourceUnavailable
        }
        fylura_native_host::HostProvidedError::PermissionDenied => {
            HostProvidedError::PermissionDenied
        }
        fylura_native_host::HostProvidedError::Cancelled => HostProvidedError::Cancelled,
        fylura_native_host::HostProvidedError::Failed => HostProvidedError::Failed,
    }
}

#[cfg(test)]
#[path = "tests/host_provided_lifecycle.rs"]
mod host_provided_lifecycle_tests;
