use super::folder::FolderPreviewEnumerationAdapter;
use super::preview::WorkspacePreviewResolver;
use crate::{
    db::Database,
    file_workspace::{
        browse::{BrowseLimits, BrowseService},
        change::EphemeralChangeMonitor,
        native_preview::{
            access::{NativePreviewAccessConfig, NativePreviewAccessRegistry},
            host_provided::{HostProvidedConfig, HostProvidedRegistry},
        },
        preview_asset::PreviewAssetRegistry,
        preview_policy::production_preview_provider_registry,
        preview_policy::production_preview_provider_registry_with_native_access,
        read_gate::{MaterializationReadGate, ReadGateConfig},
        thumbnail::{
            MacQuickLookThumbnailRenderer, ThumbnailRenderer, ThumbnailService,
            ThumbnailServiceConfig, ThumbnailTask,
        },
    },
    platform::macos::native_preview::MacQuickLookPreviewHost,
    platform::macos::quick_look::MacThumbnailService,
    scheduler::WorkScheduler,
};
#[cfg(test)]
use std::sync::Condvar;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use uuid::Uuid;

pub(crate) const MAX_BROWSE_SESSIONS: usize = 32;
pub(crate) const MAX_CHANGE_MONITORS: usize = 32;
pub(crate) const MAX_PREVIEW_SESSIONS: usize = 32;
pub(crate) const MAX_THUMBNAIL_TASKS: usize = 128;

pub(crate) struct BrowseRecord {
    pub(crate) info: super::super::browse::BrowseSessionInfo,
    pub(crate) display_name: String,
}

pub(crate) struct MonitorRecord {
    pub(crate) session_id: String,
    pub(crate) monitor: Arc<EphemeralChangeMonitor>,
}

pub(crate) enum ThumbnailRegistration {
    Reserved {
        cancel_requested: bool,
    },
    Running {
        task: Arc<ThumbnailTask>,
        cancel_requested: bool,
    },
}

impl ThumbnailRegistration {
    pub(crate) fn cancel_requested(&self) -> bool {
        match self {
            Self::Reserved { cancel_requested }
            | Self::Running {
                cancel_requested, ..
            } => *cancel_requested,
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct ThumbnailReservationGate {
    state: Mutex<(bool, bool)>,
    wake: Condvar,
}

#[cfg(test)]
impl ThumbnailReservationGate {
    pub(crate) fn wait_until_reached(&self) {
        let mut state = self.state.lock().expect("thumbnail reservation gate lock");
        while !state.0 {
            state = self
                .wake
                .wait(state)
                .expect("thumbnail reservation gate wait");
        }
    }

    pub(crate) fn pause(&self) {
        let mut state = self.state.lock().expect("thumbnail reservation gate lock");
        state.0 = true;
        self.wake.notify_all();
        while !state.1 {
            state = self
                .wake
                .wait(state)
                .expect("thumbnail reservation gate wait");
        }
    }

    pub(crate) fn release(&self) {
        let mut state = self.state.lock().expect("thumbnail reservation gate lock");
        state.1 = true;
        self.wake.notify_all();
    }
}

pub(crate) struct RuntimeInner {
    pub(crate) database: Database,
    pub(crate) transfers: Mutex<crate::file_ops::manual_transfer::TransferState>,
    pub(crate) browse: Arc<BrowseService>,
    pub(crate) read_gate: Arc<MaterializationReadGate>,
    // Retain the exact global scheduler reference as part of the integration
    // ownership record; ThumbnailService and Preview adapters receive this Arc.
    #[allow(dead_code)]
    pub(crate) scheduler: Arc<WorkScheduler>,
    pub(crate) thumbnail: Arc<ThumbnailService>,
    pub(crate) preview_resolver: Arc<WorkspacePreviewResolver>,
    pub(crate) folder_enumeration: Arc<FolderPreviewEnumerationAdapter>,
    pub(crate) preview_read: Arc<crate::file_workspace::PreviewReadGateAdapter>,
    pub(crate) preview_registry: Arc<crate::file_workspace::PreviewProviderRegistry>,
    pub(crate) preview_assets: Arc<PreviewAssetRegistry>,
    pub(crate) native_preview_access: Arc<NativePreviewAccessRegistry>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) native_preview_host: MacQuickLookPreviewHost,
    pub(crate) host_provided: Arc<HostProvidedRegistry>,
    pub(crate) sessions: Mutex<HashMap<String, BrowseRecord>>,
    pub(crate) monitors: Mutex<HashMap<String, MonitorRecord>>,
    pub(crate) thumbnail_tasks: Mutex<HashMap<String, ThumbnailRegistration>>,
    pub(crate) preview_sessions: Mutex<HashMap<String, crate::file_workspace::PreviewSession>>,
    #[cfg(test)]
    pub(crate) thumbnail_reservation_gate: Mutex<Option<Arc<ThumbnailReservationGate>>>,
    disposed: AtomicBool,
}

/// Process-local ownership for the W1-10 adapters and W4 native request seams.
/// The services referenced by this object remain the authorities for their
/// domains; these fields only keep bounded lifecycle owners alive.
#[derive(Clone)]
pub struct FileWorkspaceRuntime {
    pub(crate) inner: Arc<RuntimeInner>,
}

impl FileWorkspaceRuntime {
    /// Test-only convenience keeps historical unit/performance fixtures small.
    /// Production composition must pass an explicit app-data native root.
    #[cfg(test)]
    pub fn new(
        database: Database,
        legacy_thumbnail_service: MacThumbnailService,
        thumbnail_cache_dir: PathBuf,
    ) -> Result<Self, String> {
        let native_preview_root = thumbnail_cache_dir.with_file_name("native-preview");
        Self::new_with_native_preview_root(
            database,
            legacy_thumbnail_service,
            thumbnail_cache_dir,
            native_preview_root,
        )
    }

    pub fn new_with_native_preview_root(
        database: Database,
        legacy_thumbnail_service: MacThumbnailService,
        thumbnail_cache_dir: PathBuf,
        native_preview_root: PathBuf,
    ) -> Result<Self, String> {
        let renderer: Arc<dyn ThumbnailRenderer> =
            Arc::new(MacQuickLookThumbnailRenderer::new(legacy_thumbnail_service));
        Self::new_with_renderer(
            database,
            renderer,
            thumbnail_cache_dir,
            native_preview_root,
            BrowseLimits::default(),
        )
    }

    fn new_with_renderer(
        database: Database,
        renderer: Arc<dyn ThumbnailRenderer>,
        thumbnail_cache_dir: PathBuf,
        native_preview_root: PathBuf,
        browse_limits: BrowseLimits,
    ) -> Result<Self, String> {
        Self::new_with_renderer_using_scheduler(
            database,
            renderer,
            thumbnail_cache_dir,
            native_preview_root,
            WorkScheduler::global(),
            browse_limits,
        )
    }

    fn new_with_renderer_using_scheduler(
        database: Database,
        renderer: Arc<dyn ThumbnailRenderer>,
        thumbnail_cache_dir: PathBuf,
        native_preview_root: PathBuf,
        scheduler: Arc<WorkScheduler>,
        browse_limits: BrowseLimits,
    ) -> Result<Self, String> {
        let browse = Arc::new(
            BrowseService::new(browse_limits)
                .map_err(|error| format!("workspace_browse_{error}"))?,
        );
        let read_gate = Arc::new(
            MaterializationReadGate::from_workspace_sources(
                database.clone(),
                Arc::clone(&browse),
                ReadGateConfig::default(),
            )
            .map_err(|error| format!("workspace_read_gate_{error}"))?,
        );
        let native_preview_access = NativePreviewAccessRegistry::new(
            native_preview_root,
            Arc::clone(&read_gate),
            Arc::clone(&scheduler),
            NativePreviewAccessConfig::default(),
        )
        .map_err(|error| format!("workspace_native_preview_access_{error}"))?;
        let host_provided = HostProvidedRegistry::new(HostProvidedConfig::default())
            .map_err(|error| format!("workspace_host_provided_{error}"))?;
        let thumbnail_read_gate: Arc<dyn crate::file_workspace::ThumbnailReadGate> =
            read_gate.clone();
        let thumbnail = Arc::new(
            ThumbnailService::new(
                thumbnail_read_gate,
                Arc::clone(&scheduler),
                renderer,
                Some(thumbnail_cache_dir),
                ThumbnailServiceConfig::default(),
            )
            .map_err(|error| format!("workspace_thumbnail_{error}"))?,
        );
        let preview_resolver = Arc::new(WorkspacePreviewResolver::new(
            database.clone(),
            Arc::clone(&browse),
            Arc::clone(&read_gate),
        ));
        let folder_enumeration = Arc::new(FolderPreviewEnumerationAdapter::new(
            Arc::clone(&preview_resolver),
            Arc::clone(&browse),
            Arc::clone(&scheduler),
        ));
        let preview_read = Arc::new(crate::file_workspace::PreviewReadGateAdapter::new(
            Arc::clone(&read_gate),
        ));
        let native_preview_host = MacQuickLookPreviewHost::new();
        let preview_registry = if crate::platform::macos::native_preview::available() {
            production_preview_provider_registry_with_native_access(Some(Arc::clone(
                &native_preview_access,
            )))
        } else {
            production_preview_provider_registry()
        }
        .map_err(|error| format!("workspace_preview_registry_{error}"))?;
        let preview_read_for_assets: Arc<dyn crate::file_workspace::PreviewContentReadAccess> =
            preview_read.clone();
        let preview_assets = PreviewAssetRegistry::new_with_range_reader(preview_read_for_assets);

        Ok(Self {
            inner: Arc::new(RuntimeInner {
                transfers: Mutex::new(Default::default()),
                database,
                browse,
                read_gate,
                scheduler,
                thumbnail,
                preview_resolver,
                folder_enumeration,
                preview_read,
                preview_registry,
                preview_assets,
                native_preview_access,
                native_preview_host,
                host_provided,
                sessions: Mutex::new(HashMap::new()),
                monitors: Mutex::new(HashMap::new()),
                thumbnail_tasks: Mutex::new(HashMap::new()),
                preview_sessions: Mutex::new(HashMap::new()),
                #[cfg(test)]
                thumbnail_reservation_gate: Mutex::new(None),
                disposed: AtomicBool::new(false),
            }),
        })
    }

    #[cfg(test)]
    pub(crate) fn new_with_thumbnail_renderer_for_test(
        database: Database,
        renderer: Arc<dyn ThumbnailRenderer>,
        thumbnail_cache_dir: PathBuf,
    ) -> Result<Self, String> {
        let native_preview_root = thumbnail_cache_dir.with_file_name("native-preview");
        Self::new_with_renderer(
            database,
            renderer,
            thumbnail_cache_dir,
            native_preview_root,
            BrowseLimits::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_scheduler_for_test(
        database: Database,
        legacy_thumbnail_service: MacThumbnailService,
        thumbnail_cache_dir: PathBuf,
        scheduler: Arc<WorkScheduler>,
    ) -> Result<Self, String> {
        let renderer: Arc<dyn ThumbnailRenderer> =
            Arc::new(MacQuickLookThumbnailRenderer::new(legacy_thumbnail_service));
        let native_preview_root = thumbnail_cache_dir.with_file_name("native-preview");
        Self::new_with_renderer_using_scheduler(
            database,
            renderer,
            thumbnail_cache_dir,
            native_preview_root,
            scheduler,
            BrowseLimits::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_browse_limits_for_test(
        database: Database,
        legacy_thumbnail_service: MacThumbnailService,
        thumbnail_cache_dir: PathBuf,
        browse_limits: BrowseLimits,
    ) -> Result<Self, String> {
        let renderer: Arc<dyn ThumbnailRenderer> =
            Arc::new(MacQuickLookThumbnailRenderer::new(legacy_thumbnail_service));
        let native_preview_root = thumbnail_cache_dir.with_file_name("native-preview");
        Self::new_with_renderer(
            database,
            renderer,
            thumbnail_cache_dir,
            native_preview_root,
            browse_limits,
        )
    }

    pub(crate) fn ensure_live(&self) -> Result<(), String> {
        if self.inner.disposed.load(Ordering::Acquire) {
            Err("file_workspace_runtime_disposed".to_string())
        } else {
            Ok(())
        }
    }

    pub(crate) fn next_id(prefix: &str) -> String {
        format!("{prefix}-{}", Uuid::new_v4())
    }

    pub fn dispose(&self) -> bool {
        dispose_inner(&self.inner)
    }

    /// Disposes this Main-window-owned runtime and reports whether every
    /// native owner could be released. The owner may retry this method after
    /// a native dispatcher temporarily fails; the disposed flag prevents new
    /// work while the retained native host keeps failed releases retryable.
    pub fn dispose_for_main_teardown(&self) -> Result<(), String> {
        self.inner.disposed.store(true, Ordering::Release);
        dispose_inner_fields(&self.inner)
    }

    #[cfg(test)]
    pub(crate) fn set_thumbnail_reservation_gate(&self, gate: Arc<ThumbnailReservationGate>) {
        *self
            .inner
            .thumbnail_reservation_gate
            .lock()
            .expect("thumbnail reservation gate registry") = Some(gate);
    }

    #[cfg(test)]
    pub(crate) fn pause_after_thumbnail_reservation(&self) {
        let gate = self
            .inner
            .thumbnail_reservation_gate
            .lock()
            .expect("thumbnail reservation gate registry")
            .take();
        if let Some(gate) = gate {
            gate.pause();
        }
    }

    #[cfg(test)]
    pub(crate) fn resource_counts(&self) -> ResourceCounts {
        let browse_counts = self.inner.browse.resource_counts();
        let (native_preview_records, native_preview_inflight, native_preview_bytes) =
            self.inner.native_preview_access.counts();
        ResourceCounts {
            browse_sessions: self
                .inner
                .sessions
                .lock()
                .map(|records| records.len())
                .unwrap_or_default(),
            change_monitors: self
                .inner
                .monitors
                .lock()
                .map(|records| records.len())
                .unwrap_or_default(),
            thumbnail_requests: self
                .inner
                .thumbnail_tasks
                .lock()
                .map(|records| records.len())
                .unwrap_or_default(),
            preview_sessions: self
                .inner
                .preview_sessions
                .lock()
                .map(|records| records.len())
                .unwrap_or_default(),
            native_preview_records,
            native_preview_inflight,
            native_preview_bytes,
            host_provided_records: self.inner.host_provided.count(),
            browse_service_sessions: browse_counts.sessions,
            browse_entry_refs: browse_counts.entry_refs,
            browse_path_refs: browse_counts.path_refs,
            browse_active_enumerations: browse_counts.active_enumerations,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResourceCounts {
    pub(crate) browse_sessions: usize,
    pub(crate) change_monitors: usize,
    pub(crate) thumbnail_requests: usize,
    pub(crate) preview_sessions: usize,
    pub(crate) native_preview_records: usize,
    pub(crate) native_preview_inflight: usize,
    pub(crate) native_preview_bytes: u64,
    pub(crate) host_provided_records: usize,
    pub(crate) browse_service_sessions: usize,
    pub(crate) browse_entry_refs: usize,
    pub(crate) browse_path_refs: usize,
    pub(crate) browse_active_enumerations: usize,
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        if let Err(error) = dispose_inner_fields(self) {
            eprintln!("file_workspace_runtime_drop_dispose_failed:{error}");
        }
    }
}

fn dispose_inner(inner: &Arc<RuntimeInner>) -> bool {
    if inner.disposed.swap(true, Ordering::AcqRel) {
        return false;
    }
    if let Err(error) = dispose_inner_fields(inner) {
        eprintln!("file_workspace_runtime_dispose_failed:{error}");
    }
    true
}

fn dispose_inner_fields(inner: &RuntimeInner) -> Result<(), String> {
    let monitors = take_map(&inner.monitors);
    for record in monitors.into_values() {
        record.monitor.dispose();
    }

    // PreviewSession remains the publication/cancellation authority. Invalidate
    // every publication first, then detach/release the main-thread native
    // owner, and only then revoke Native Preview Access/staging.
    let previews = take_map(&inner.preview_sessions);
    for session in previews.into_values() {
        session.dispose();
    }
    let native_preview_host_release = inner.native_preview_host.dispose();
    if native_preview_host_release.is_ok() {
        inner.native_preview_access.dispose();
    }
    inner.host_provided.dispose();

    let thumbnail_tasks = take_map(&inner.thumbnail_tasks);
    for registration in thumbnail_tasks.into_values() {
        if let ThumbnailRegistration::Running { task, .. } = registration {
            let _ = task.cancel();
        }
    }

    let sessions = take_map(&inner.sessions);
    for record in sessions.into_values() {
        let _ = inner.browse.dispose_session(&record.info.session_id);
    }

    inner.thumbnail.dispose();
    inner.read_gate.dispose();
    inner.preview_assets.dispose();
    native_preview_host_release
        .map_err(|error| format!("native_preview_host_dispose_failed:{error}"))
}

fn take_map<T>(map: &Mutex<HashMap<String, T>>) -> HashMap<String, T> {
    map.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .drain()
        .collect()
}
