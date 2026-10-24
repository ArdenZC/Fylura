pub mod fallback;
pub mod mft;
#[cfg(feature = "native-qa")]
pub mod recovery_qa;
pub mod service;
mod service_errors;
pub mod service_host;
pub mod usn;
pub mod volumes;

use crate::global_index::coordinator::{GlobalIndexError, GlobalIndexProvider, GlobalIndexSink};
use crate::global_index::models::{
    GlobalSourceDescriptor, INDEX_STATUS_REBUILD_REQUIRED, INDEX_STATUS_UNAVAILABLE,
    PROVIDER_WINDOWS_MFT_USN,
};
use crate::global_index::wake::{GlobalIndexWakeReason, GlobalIndexWakeSlot};
use crate::global_index::{SourceOperationKind, SourceOperationRoute};
use service::{IndexServiceCommand, IndexServiceEvent, IndexServiceRequest, IndexServiceResponse};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

/// The provider used by the installed Windows service. It never calls the
/// service pipe, which prevents the service process from recursively routing
/// its own MFT/USN work back through the desktop transport.
pub struct DirectWindowsGlobalIndexProvider {}

impl DirectWindowsGlobalIndexProvider {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for DirectWindowsGlobalIndexProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalIndexProvider for DirectWindowsGlobalIndexProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        volumes::discover_windows_volumes()
    }

    fn start_initial_index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        if source.volume.provider != PROVIDER_WINDOWS_MFT_USN {
            let message = format!(
                "windows_global_index_unsupported_filesystem:{}",
                source.volume.filesystem_type
            );
            sink.set_source_state(&source.volume.id, INDEX_STATUS_UNAVAILABLE, Some(&message))?;
            return Ok(());
        }
        sink.set_source_provider(&source.volume.id, PROVIDER_WINDOWS_MFT_USN)?;
        sink.mark_volume_entries_stale(&source.volume.id)?;
        let _qos = crate::resource_governor::scope_thread_qos(
            crate::file_workspace::WorkClass::Background,
        );
        let result = mft::enumerate_volume(source, sink, cancel);
        finish_initial_mft_result(&source.volume.id, sink, result)
    }

    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        if source.volume.provider != PROVIDER_WINDOWS_MFT_USN {
            let message = format!(
                "windows_global_index_unsupported_filesystem:{}",
                source.volume.filesystem_type
            );
            sink.set_source_state(&source.volume.id, INDEX_STATUS_UNAVAILABLE, Some(&message))?;
            return Ok(());
        }
        sink.set_source_provider(&source.volume.id, PROVIDER_WINDOWS_MFT_USN)?;
        let result = usn::sync_volume(source, sink, cancel);
        finish_incremental_result(&source.volume.id, sink, result)
    }

    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }

    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("windows_mft_usn".to_string())
    }

    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
}

fn finish_incremental_result(
    volume_id: &str,
    sink: &mut dyn GlobalIndexSink,
    result: Result<usn::UsnSyncResult, GlobalIndexError>,
) -> Result<(), GlobalIndexError> {
    match result {
        Ok(result) if result.directory_path_changed => {
            sink.mark_volume_entries_stale(volume_id)?;
            usn::require_rebuild(
                volume_id,
                sink,
                "USN directory rename requires an MFT reconciliation",
            )
        }
        Ok(_) => Ok(()),
        Err(GlobalIndexError::Paused) => Err(GlobalIndexError::Paused),
        Err(error @ GlobalIndexError::RebuildRequired(_)) => Err(error),
        Err(error) => {
            sink.set_source_state(
                volume_id,
                service_errors::classify(&error).1,
                Some(&error.to_string()),
            )?;
            Err(error)
        }
    }
}

fn finish_initial_mft_result(
    volume_id: &str,
    sink: &mut dyn GlobalIndexSink,
    result: Result<mft::MftJournalState, GlobalIndexError>,
) -> Result<(), GlobalIndexError> {
    match result {
        Ok(_) => Ok(()),
        Err(GlobalIndexError::Paused) => Err(GlobalIndexError::Paused),
        Err(error) if mft::is_integrity_error(&error) => {
            let message = error.to_string();
            sink.set_source_state(volume_id, INDEX_STATUS_REBUILD_REQUIRED, Some(&message))?;
            Err(error)
        }
        Err(error) => {
            let message = error.to_string();
            sink.set_source_state(
                volume_id,
                service_errors::classify(&error).1,
                Some(&message),
            )?;
            Err(error)
        }
    }
}

/// Desktop-side provider. The normal route is the installed service over a
/// versioned named pipe. If the service is not installed, stopped, or cannot
/// be reached, the desktop keeps the existing direct provider as a bounded
/// least-privilege fallback so search remains usable in development and
/// recovery scenarios.
pub struct WindowsGlobalIndexProvider {
    direct: DirectWindowsGlobalIndexProvider,
    service_available: AtomicBool,
    direct_blocked_sources: Mutex<std::collections::HashSet<String>>,
    wake: std::sync::Arc<GlobalIndexWakeSlot>,
    database_path: PathBuf,
    change_watchers: Mutex<HashMap<String, fallback::ChangeSignalWatcher>>,
    #[cfg(test)]
    force_direct: bool,
    #[cfg(test)]
    smoke_ignored_wake_root: Option<PathBuf>,
    #[cfg(test)]
    fake_change_watchers: bool,
    #[cfg(test)]
    test_watcher_start_count: std::sync::atomic::AtomicUsize,
}

#[cfg(feature = "native-qa")]
pub(crate) enum PauseObservation<'a> {
    Request {
        protocol_version: u16,
        request_id: &'a str,
    },
    Response(&'a IndexServiceResponse),
    TransportError {
        kind: &'static str,
    },
}

impl WindowsGlobalIndexProvider {
    pub fn new() -> Self {
        Self::with_wake(
            std::sync::Arc::new(GlobalIndexWakeSlot::default()),
            PathBuf::new(),
        )
    }

    pub(crate) fn with_wake(
        wake: std::sync::Arc<GlobalIndexWakeSlot>,
        database_path: PathBuf,
    ) -> Self {
        Self {
            direct: DirectWindowsGlobalIndexProvider::new(),
            service_available: AtomicBool::new(false),
            direct_blocked_sources: Mutex::new(Default::default()),
            wake,
            database_path,
            change_watchers: Mutex::new(HashMap::new()),
            #[cfg(test)]
            force_direct: false,
            #[cfg(test)]
            smoke_ignored_wake_root: None,
            #[cfg(test)]
            fake_change_watchers: false,
            #[cfg(test)]
            test_watcher_start_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_direct_native_smoke(
        wake: std::sync::Arc<GlobalIndexWakeSlot>,
        database_path: PathBuf,
        ignored_wake_root: PathBuf,
    ) -> Self {
        Self {
            direct: DirectWindowsGlobalIndexProvider::new(),
            service_available: AtomicBool::new(false),
            direct_blocked_sources: Mutex::new(Default::default()),
            wake,
            database_path,
            change_watchers: Mutex::new(HashMap::new()),
            force_direct: true,
            smoke_ignored_wake_root: Some(ignored_wake_root),
            fake_change_watchers: false,
            test_watcher_start_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    fn with_fake_change_watchers() -> Self {
        let mut provider = Self::with_wake(
            std::sync::Arc::new(GlobalIndexWakeSlot::default()),
            PathBuf::new(),
        );
        provider.force_direct = true;
        provider.fake_change_watchers = true;
        provider
    }

    fn ensure_native_change_watcher(
        &self,
        source: &GlobalSourceDescriptor,
    ) -> Result<(), GlobalIndexError> {
        if source.volume.provider != PROVIDER_WINDOWS_MFT_USN {
            return Ok(());
        }
        let mut watchers = self.change_watchers.lock().map_err(|_| {
            GlobalIndexError::Provider("Windows Global Index watcher lock poisoned".to_string())
        })?;
        if watchers
            .get(&source.volume.id)
            .is_some_and(fallback::ChangeSignalWatcher::has_failed)
        {
            // A watcher error is only a wake hint failure. Drop its native
            // handle so this request can install a fresh hint source and
            // continue through the existing service/direct USN authority.
            watchers.remove(&source.volume.id);
        }
        if watchers.contains_key(&source.volume.id) {
            return Ok(());
        }
        let ignored_database_path = self
            .database_path
            .is_absolute()
            .then(|| self.database_path.clone());
        #[cfg(test)]
        let watcher = if self.fake_change_watchers {
            self.test_watcher_start_count.fetch_add(1, Ordering::AcqRel);
            fallback::ChangeSignalWatcher::new_for_test()
        } else if let Some(ignored_root) = &self.smoke_ignored_wake_root {
            fallback::ChangeSignalWatcher::start_ignoring_test_root(
                Path::new(&source.volume.mount_path),
                self.wake.clone(),
                ignored_database_path,
                ignored_root.clone(),
            )?
        } else {
            fallback::ChangeSignalWatcher::start(
                Path::new(&source.volume.mount_path),
                self.wake.clone(),
                ignored_database_path,
            )?
        };
        #[cfg(not(test))]
        let watcher = fallback::ChangeSignalWatcher::start(
            Path::new(&source.volume.mount_path),
            self.wake.clone(),
            ignored_database_path,
        )?;
        watchers.insert(source.volume.id.clone(), watcher);
        Ok(())
    }

    fn clear_native_change_watcher(&self, source_id: &str) {
        if let Ok(mut watchers) = self.change_watchers.lock() {
            watchers.remove(source_id);
        }
    }

    fn clear_native_change_watchers(&self) {
        if let Ok(mut watchers) = self.change_watchers.lock() {
            watchers.clear();
        }
    }

    fn run_source_request(
        &self,
        request: &IndexServiceRequest,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        direct: impl FnOnce(
            &DirectWindowsGlobalIndexProvider,
            &mut dyn GlobalIndexSink,
        ) -> Result<(), GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        self.ensure_native_change_watcher(source)?;
        self.run_with_fallback(request, sink, direct)
    }

    fn run_direct_admitted(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        direct: impl FnOnce(
            &DirectWindowsGlobalIndexProvider,
            &mut dyn GlobalIndexSink,
        ) -> Result<(), GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        let source = request.source.as_ref().ok_or_else(|| {
            GlobalIndexError::ServiceProtocol("Direct operation source absent".into())
        })?;
        let kind = match &request.command {
            IndexServiceCommand::StartInitialIndex { .. } => SourceOperationKind::Initial,
            IndexServiceCommand::ResumeIncrementalSync { .. } => SourceOperationKind::Incremental,
            IndexServiceCommand::Rebuild { .. } => SourceOperationKind::Rebuild,
            _ => {
                return Err(GlobalIndexError::ServiceProtocol(
                    "Direct fallback for a non-index command".into(),
                ))
            }
        };
        sink.admit_source_operation(
            source,
            &request.request_id,
            kind,
            SourceOperationRoute::Direct,
        )?;
        direct(&self.direct, sink)
    }

    fn service_failed(&self) {
        self.service_available.store(false, Ordering::Release);
    }

    fn service_connected(&self) {
        self.service_available.store(true, Ordering::Release);
    }

    fn service_response_error(response: &IndexServiceResponse) -> GlobalIndexError {
        service_errors::decode(response)
    }

    fn pause_with_observer(
        &self,
        #[cfg(feature = "native-qa")] mut observe: impl FnMut(PauseObservation<'_>),
    ) -> Result<(), GlobalIndexError> {
        self.clear_native_change_watchers();
        let request = IndexServiceRequest::new(IndexServiceCommand::Pause, None);
        #[cfg(feature = "native-qa")]
        observe(PauseObservation::Request {
            protocol_version: request.protocol_version,
            request_id: &request.request_id,
        });
        match service::call_index_service_admitted(&request, |_| {
            Err(service::IndexServiceCallError::Protocol(
                "unexpected control event".into(),
            ))
        }) {
            Ok(response) => {
                #[cfg(feature = "native-qa")]
                observe(PauseObservation::Response(&response));
                if response.ok {
                    self.service_connected();
                    Ok(())
                } else {
                    Err(Self::service_response_error(&response))
                }
            }
            Err(error) => {
                #[cfg(feature = "native-qa")]
                let kind = match &error {
                    service::IndexServiceCallError::NotSent(_) => "not_sent",
                    service::IndexServiceCallError::Protocol(_) => "protocol",
                    service::IndexServiceCallError::UnknownOutcome(_) => "unknown_outcome",
                    service::IndexServiceCallError::Sink(_) => "sink",
                };
                #[cfg(feature = "native-qa")]
                observe(PauseObservation::TransportError { kind });
                match error {
                    service::IndexServiceCallError::NotSent(_) => {
                        self.service_failed();
                        self.direct.pause()
                    }
                    error => Err(error.into_global()),
                }
            }
        }
    }

    #[cfg(feature = "native-qa")]
    pub(crate) fn pause_for_native_qa(
        &self,
        observe: impl FnMut(PauseObservation<'_>),
    ) -> Result<(), GlobalIndexError> {
        self.pause_with_observer(observe)
    }

    fn source_request(
        command: IndexServiceCommand,
        source: &GlobalSourceDescriptor,
    ) -> IndexServiceRequest {
        IndexServiceRequest::new(command, Some(source.volume.clone()))
    }

    fn run_service_stream(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
    ) -> Result<Result<(), GlobalIndexError>, service::IndexServiceCallError> {
        self.run_service_stream_using(request, sink, |request, event| {
            service::call_index_service_admitted(request, event)
        })
    }

    fn run_service_stream_using(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        call: impl FnOnce(
            &IndexServiceRequest,
            &mut dyn FnMut(
                IndexServiceEvent,
            ) -> Result<
                Option<service::IndexServiceLookupResponse>,
                service::IndexServiceCallError,
            >,
        ) -> Result<IndexServiceResponse, service::IndexServiceCallError>,
    ) -> Result<Result<(), GlobalIndexError>, service::IndexServiceCallError> {
        let mut admission = service::IndexServiceAdmission::new(request);
        let mut admitted = false;
        let response = {
            let mut apply_event = |event| {
                let is_admission = matches!(&event, IndexServiceEvent::Sources { .. });
                let result = admission.apply(sink, event)?;
                admitted |= is_admission;
                Ok(result)
            };
            call(request, &mut apply_event)?
        };
        admission.finish(&response)?;
        if admitted && !response.ok {
            // Sink writes are not transactional; do not replay this source via
            // Direct after a valid failed terminal response.
            let source_id = request
                .source
                .as_ref()
                .map(|source| source.id.clone())
                .unwrap_or_default();
            self.direct_blocked_sources
                .lock()
                .map_err(|_| {
                    service::IndexServiceCallError::Protocol("Direct safety fence poisoned".into())
                })?
                .insert(source_id);
        }
        self.service_connected();
        if response.ok {
            super::qa_trace::record("windows_service_route_ok");
            Ok(Ok(()))
        } else {
            Ok(Err(Self::service_response_error(&response)))
        }
    }

    #[cfg(test)]
    fn run_service_stream_with_io(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        io: &mut impl service::IndexServiceIo,
    ) -> Result<Result<(), GlobalIndexError>, service::IndexServiceCallError> {
        self.run_service_stream_using(request, sink, |request, event| {
            service::exchange(request, io, event)
        })
    }

    #[cfg(test)]
    pub(crate) fn run_with_test_io_and_fallback(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        io: &mut impl service::IndexServiceIo,
        direct: impl FnOnce(
            &DirectWindowsGlobalIndexProvider,
            &mut dyn GlobalIndexSink,
        ) -> Result<(), GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        let result = self.run_service_stream_with_io(request, sink, io);
        self.resolve_service_route(request, sink, result, |_, sink| {
            self.run_direct_admitted(request, sink, direct)
        })
    }

    fn run_with_fallback(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        direct: impl FnOnce(
            &DirectWindowsGlobalIndexProvider,
            &mut dyn GlobalIndexSink,
        ) -> Result<(), GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        #[cfg(test)]
        if self.force_direct {
            return self.run_direct_admitted(request, sink, direct);
        }
        let result = self.run_service_stream(request, sink);
        self.resolve_service_route(request, sink, result, direct)
    }

    fn resolve_service_route(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        result: Result<Result<(), GlobalIndexError>, service::IndexServiceCallError>,
        direct: impl FnOnce(
            &DirectWindowsGlobalIndexProvider,
            &mut dyn GlobalIndexSink,
        ) -> Result<(), GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        let source_id = request
            .source
            .as_ref()
            .map(|source| source.id.clone())
            .unwrap_or_default();
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(service::IndexServiceCallError::NotSent(_)) => {
                self.service_failed();
                let blocked = self.direct_blocked_sources.lock().map_err(|_| {
                    GlobalIndexError::ServiceProtocol("Direct safety fence poisoned".into())
                })?;
                if blocked.contains(&source_id) {
                    return Err(GlobalIndexError::ServiceProtocol("Direct denied after an unverified previous Service request; reconciliation required".into()));
                }
                drop(blocked);
                self.run_direct_admitted(request, sink, direct)
            }
            Err(error) => {
                // Fail closed for this provider lifetime. P2 owns durable intent
                // and reconciliation across desktop/Service restart.
                if let Ok(mut blocked) = self.direct_blocked_sources.lock() {
                    blocked.insert(source_id);
                }
                Err(error.into_global())
            }
        }
    }
}

impl Default for WindowsGlobalIndexProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalIndexProvider for WindowsGlobalIndexProvider {
    fn has_external_source_operation_admission(&self) -> bool {
        true
    }

    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        #[cfg(test)]
        if self.force_direct {
            return self.direct.discover_sources();
        }
        let request = IndexServiceRequest::new(IndexServiceCommand::DiscoverSources, None);
        let mut sources = None;
        match service::call_index_service_admitted(&request, |event| {
            let IndexServiceEvent::Sources {
                sources: discovered,
            } = event
            else {
                return Err(service::IndexServiceCallError::Protocol(
                    "unexpected discovery event".into(),
                ));
            };
            if sources.is_some() {
                return Err(service::IndexServiceCallError::Protocol(
                    "duplicate discovery result".into(),
                ));
            }
            let mut ids = std::collections::HashSet::new();
            for source in &discovered {
                service::validate_source_snapshot(source)
                    .map_err(service::IndexServiceCallError::Protocol)?;
                if !ids.insert(source.id.clone()) {
                    return Err(service::IndexServiceCallError::Protocol(
                        "duplicate discovery source identity".into(),
                    ));
                }
            }
            sources = Some(discovered);
            Ok(None)
        }) {
            Ok(response) if response.ok => {
                self.service_connected();
                Ok(sources
                    .ok_or_else(|| {
                        GlobalIndexError::ServiceProtocol(
                            "discovery success without Sources".into(),
                        )
                    })?
                    .into_iter()
                    .map(|volume| GlobalSourceDescriptor { volume })
                    .collect())
            }
            Ok(response) => Err(Self::service_response_error(&response)),
            Err(service::IndexServiceCallError::NotSent(_)) => {
                self.service_failed();
                self.direct.discover_sources()
            }
            Err(error) => Err(error.into_global()),
        }
    }

    fn start_initial_index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        let request = Self::source_request(
            IndexServiceCommand::StartInitialIndex {
                source_id: source.volume.id.clone(),
            },
            source,
        );
        self.run_source_request(&request, source, sink, |direct, sink| {
            direct.start_initial_index(source, sink, cancel)
        })
    }

    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        let request = Self::source_request(
            IndexServiceCommand::ResumeIncrementalSync {
                source_id: source.volume.id.clone(),
            },
            source,
        );
        self.run_source_request(&request, source, sink, |direct, sink| {
            direct.resume_incremental_sync(source, sink, cancel)
        })
    }

    fn rebuild(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        let request = Self::source_request(
            IndexServiceCommand::Rebuild {
                source_id: source.volume.id.clone(),
            },
            source,
        );
        self.run_source_request(&request, source, sink, |direct, sink| {
            direct.rebuild(source, sink, cancel)
        })
    }

    fn pause(&self) -> Result<(), GlobalIndexError> {
        #[cfg(feature = "native-qa")]
        {
            self.pause_with_observer(|_| {})
        }
        #[cfg(not(feature = "native-qa"))]
        {
            self.pause_with_observer()
        }
    }

    fn status(&self) -> Result<String, GlobalIndexError> {
        let request = IndexServiceRequest::new(IndexServiceCommand::Status, None);
        match service::call_index_service_admitted(&request, |_| {
            Err(service::IndexServiceCallError::Protocol(
                "unexpected control event".into(),
            ))
        }) {
            Ok(response) if response.ok => {
                self.service_connected();
                Ok(format!(
                    "windows_index_service:{}",
                    response.status.unwrap_or_else(|| "ready".to_string())
                ))
            }
            Ok(response) => Err(Self::service_response_error(&response)),
            Err(service::IndexServiceCallError::NotSent(_)) => {
                self.service_failed();
                Ok(format!("{}:service_unavailable", self.direct.status()?))
            }
            Err(error) => Err(error.into_global()),
        }
    }

    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        self.clear_native_change_watchers();
        // The installed service is independent of the desktop lifetime. Stop
        // only its active operation when the UI exits; the SCM owns service
        // process lifetime and installer/uninstaller owns registration.
        let _ = self.pause();
        self.direct.shutdown()
    }

    fn topology_audit_interval(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(5 * 60))
    }

    fn audit_source_topology(
        &self,
    ) -> Result<Option<Vec<GlobalSourceDescriptor>>, GlobalIndexError> {
        volumes::discover_windows_volumes().map(Some)
    }

    fn source_enabled_changed(&self, source_id: &str, enabled: bool) {
        self.clear_native_change_watcher(source_id);
        if enabled {
            self.wake.notify(GlobalIndexWakeReason::ExplicitCommand);
        }
    }
}

// Test-only bridge: exercise the unchanged production v3 decoder with the
// coordinator ownership diagnostic without expanding production visibility.
#[cfg(test)]
pub(crate) fn decode_response_for_ownership_test(
    response: &IndexServiceResponse,
) -> GlobalIndexError {
    service_errors::decode(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_index::models::{
        GlobalEntry, GlobalEntryInput, GlobalVolume, INDEX_STATUS_PERMISSION_REQUIRED,
        INDEX_STATUS_READY, INDEX_STATUS_REBUILD_REQUIRED, INDEX_STATUS_SYNCING,
    };
    use std::collections::VecDeque;
    use std::sync::mpsc::{self, Sender};
    use std::thread::JoinHandle;

    struct ScriptedServiceIo {
        frames: VecDeque<service::IndexServiceFrame>,
    }

    impl service::IndexServiceIo for ScriptedServiceIo {
        fn send_request(
            &mut self,
            _request: &IndexServiceRequest,
        ) -> Result<(), service::IndexServiceCallError> {
            Ok(())
        }

        fn read_frame(
            &mut self,
        ) -> Result<service::IndexServiceFrame, service::IndexServiceCallError> {
            self.frames.pop_front().ok_or_else(|| {
                service::IndexServiceCallError::UnknownOutcome("scripted stream ended".into())
            })
        }

        fn send_lookup(
            &mut self,
            _response: service::IndexServiceLookupResponse,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    fn windows_source() -> GlobalSourceDescriptor {
        GlobalSourceDescriptor {
            volume: GlobalVolume {
                id: "volume".to_string(),
                platform: "windows".to_string(),
                stable_volume_id: "stable".to_string(),
                display_name: "C".to_string(),
                mount_path: "C:\\".to_string(),
                filesystem_type: "ntfs".to_string(),
                drive_kind: "fixed".to_string(),
                enabled: true,
                provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
                index_status: "ready".to_string(),
                last_error: None,
                journal_id: Some("journal".to_string()),
                journal_cursor: Some("42".to_string()),
                last_full_index_at: Some(1),
                last_incremental_sync_at: Some(2),
                entry_count: 3,
                created_at: 0,
                updated_at: 0,
            },
        }
    }

    #[derive(Clone)]
    enum CoordinatorReply {
        ServiceBusy,
        Complete,
        CompleteWithEntries,
        PermissionFailure,
        UnknownAfterAdmission,
        NotSent,
        SqliteBusy(PathBuf),
        SqliteBusyAfterAdmission(PathBuf),
    }

    type CoordinatorReplyMap = std::collections::HashMap<String, VecDeque<CoordinatorReply>>;

    #[derive(Default)]
    struct CoordinatorCycleCounters {
        requests: std::sync::atomic::AtomicUsize,
        direct_calls: std::sync::atomic::AtomicUsize,
        service_busy: AtomicBool,
        sqlite_busy: AtomicBool,
    }

    struct WindowsCoordinatorTestProvider {
        windows: WindowsGlobalIndexProvider,
        sources: Vec<GlobalSourceDescriptor>,
        replies: std::sync::Arc<Mutex<CoordinatorReplyMap>>,
        counters: std::sync::Arc<CoordinatorCycleCounters>,
    }

    impl WindowsCoordinatorTestProvider {
        fn new(
            sources: Vec<GlobalSourceDescriptor>,
            replies: std::sync::Arc<Mutex<CoordinatorReplyMap>>,
            counters: std::sync::Arc<CoordinatorCycleCounters>,
        ) -> Self {
            Self {
                windows: WindowsGlobalIndexProvider::new(),
                sources,
                replies,
                counters,
            }
        }

        fn run_operation(
            &self,
            source: &GlobalSourceDescriptor,
            sink: &mut dyn GlobalIndexSink,
            command: IndexServiceCommand,
        ) -> Result<(), GlobalIndexError> {
            self.counters.requests.fetch_add(1, Ordering::AcqRel);
            let reply = self
                .replies
                .lock()
                .expect("coordinator reply scripts")
                .get_mut(&source.volume.id)
                .and_then(VecDeque::pop_front)
                .unwrap_or(CoordinatorReply::Complete);
            let request = IndexServiceRequest::new(command, Some(source.volume.clone()));
            let request_id = request.request_id.clone();
            let mut frames = VecDeque::new();
            let frame_event = |event| service::IndexServiceFrame::Event {
                request_id: request_id.clone(),
                event,
            };
            let source_admission = || IndexServiceEvent::Sources {
                sources: vec![request.source.clone().expect("index source")],
            };
            let full = !matches!(
                &request.command,
                IndexServiceCommand::ResumeIncrementalSync { .. }
            );
            let terminal =
                |ok, error_code: Option<&str>, message: Option<&str>, status: Option<&str>| {
                    service::IndexServiceFrame::Response {
                        response: IndexServiceResponse {
                            protocol_version: service::IPC_PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            ok,
                            error_code: error_code.map(str::to_string),
                            message: message.map(str::to_string),
                            status: status.map(str::to_string),
                        },
                    }
                };
            match &reply {
                CoordinatorReply::ServiceBusy => frames.push_back(terminal(
                    false,
                    Some("index_service_busy"),
                    Some("another request owns the service slot"),
                    Some("indexing"),
                )),
                CoordinatorReply::Complete | CoordinatorReply::SqliteBusy(_) => {
                    frames.push_back(frame_event(source_admission()));
                    frames.push_back(frame_event(IndexServiceEvent::SourceProvider {
                        volume_id: source.volume.id.clone(),
                        provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
                    }));
                    frames.push_back(frame_event(IndexServiceEvent::Checkpoint {
                        volume_id: source.volume.id.clone(),
                        journal_id: Some("42".into()),
                        journal_cursor: Some("43".into()),
                    }));
                    frames.push_back(terminal(true, None, None, Some("ready")));
                }
                CoordinatorReply::CompleteWithEntries
                | CoordinatorReply::SqliteBusyAfterAdmission(_) => {
                    frames.push_back(frame_event(source_admission()));
                    frames.push_back(frame_event(IndexServiceEvent::SourceProvider {
                        volume_id: source.volume.id.clone(),
                        provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
                    }));
                    if full {
                        frames.push_back(frame_event(IndexServiceEvent::VolumeEntriesStale {
                            volume_id: source.volume.id.clone(),
                        }));
                    }
                    frames.push_back(frame_event(IndexServiceEvent::Entries {
                        entries: vec![coordinator_test_entry(&source.volume.id)],
                    }));
                    frames.push_back(frame_event(IndexServiceEvent::Checkpoint {
                        volume_id: source.volume.id.clone(),
                        journal_id: Some("42".into()),
                        journal_cursor: Some("43".into()),
                    }));
                    frames.push_back(terminal(true, None, None, Some("ready")));
                }
                CoordinatorReply::PermissionFailure => {
                    frames.push_back(frame_event(source_admission()));
                    frames.push_back(terminal(
                        false,
                        Some("index_permission_required"),
                        Some("source_access_denied"),
                        Some("permission_required"),
                    ));
                }
                CoordinatorReply::UnknownAfterAdmission => {
                    frames.push_back(frame_event(source_admission()));
                }
                CoordinatorReply::NotSent => {}
            }
            let mut io = CoordinatorCycleIo {
                frames,
                request_not_sent: matches!(&reply, CoordinatorReply::NotSent),
                busy_database_path: match &reply {
                    CoordinatorReply::SqliteBusy(path) => Some(path.clone()),
                    _ => None,
                },
                busy_after_admission_path: match &reply {
                    CoordinatorReply::SqliteBusyAfterAdmission(path) => Some(path.clone()),
                    _ => None,
                },
                lock_holder: None,
                lock_release: None,
                lock_thread: None,
            };
            let result =
                self.windows
                    .run_with_test_io_and_fallback(&request, sink, &mut io, |_, _| {
                        self.counters.direct_calls.fetch_add(1, Ordering::AcqRel);
                        Ok(())
                    });
            io.release_writer_lock();
            match &result {
                Err(GlobalIndexError::ServiceBusy(_)) => {
                    self.counters.service_busy.store(true, Ordering::Release)
                }
                Err(GlobalIndexError::Database(crate::db::DbError::Sqlite(
                    rusqlite::Error::SqliteFailure(code, _),
                ))) if code.extended_code == rusqlite::ffi::SQLITE_BUSY => {
                    self.counters.sqlite_busy.store(true, Ordering::Release)
                }
                _ => {}
            }
            result
        }
    }

    struct CoordinatorCycleIo {
        frames: VecDeque<service::IndexServiceFrame>,
        request_not_sent: bool,
        busy_database_path: Option<PathBuf>,
        busy_after_admission_path: Option<PathBuf>,
        lock_holder: Option<rusqlite::Connection>,
        lock_release: Option<Sender<()>>,
        lock_thread: Option<JoinHandle<Result<(), String>>>,
    }

    impl CoordinatorCycleIo {
        fn start_writer_lock(&mut self, path: PathBuf) {
            let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
            let (release_tx, release_rx) = mpsc::channel();
            let lock_thread = std::thread::spawn(move || {
                let connection = match rusqlite::Connection::open(path) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let message = error.to_string();
                        let _ = ready_tx.send(Err(message.clone()));
                        return Err(message);
                    }
                };
                if let Err(error) = connection.execute_batch("BEGIN IMMEDIATE") {
                    let message = error.to_string();
                    let _ = ready_tx.send(Err(message.clone()));
                    return Err(message);
                }
                ready_tx.send(Ok(())).map_err(|error| error.to_string())?;
                release_rx.recv().map_err(|error| error.to_string())?;
                connection
                    .execute_batch("ROLLBACK")
                    .map_err(|error| error.to_string())
            });
            match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(Ok(())) => {
                    self.lock_release = Some(release_tx);
                    self.lock_thread = Some(lock_thread);
                }
                Ok(Err(error)) => panic!("failed to hold post-admission writer lock: {error}"),
                Err(error) => panic!("post-admission writer did not reach barrier: {error}"),
            }
        }

        fn release_writer_lock(&mut self) {
            if let Some(release) = self.lock_release.take() {
                let _ = release.send(());
            }
            if let Some(lock_thread) = self.lock_thread.take() {
                lock_thread
                    .join()
                    .expect("post-admission SQLite lock holder should not panic")
                    .expect("post-admission SQLite lock holder should roll back cleanly");
            }
            if let Some(connection) = self.lock_holder.take() {
                let _ = connection.execute_batch("ROLLBACK;");
            }
        }
    }

    impl service::IndexServiceIo for CoordinatorCycleIo {
        fn send_request(
            &mut self,
            _request: &IndexServiceRequest,
        ) -> Result<(), service::IndexServiceCallError> {
            if self.request_not_sent {
                return Err(service::IndexServiceCallError::NotSent(
                    "scripted failure before request dispatch".into(),
                ));
            }
            if let Some(path) = self.busy_database_path.take() {
                let connection = rusqlite::Connection::open(path).expect("open SQLite lock holder");
                connection
                    .execute_batch("BEGIN IMMEDIATE;")
                    .expect("hold SQLite writer reservation before admission callback");
                self.lock_holder = Some(connection);
            }
            Ok(())
        }

        fn read_frame(
            &mut self,
        ) -> Result<service::IndexServiceFrame, service::IndexServiceCallError> {
            if matches!(
                self.frames.front(),
                Some(service::IndexServiceFrame::Event {
                    event: IndexServiceEvent::Entries { .. },
                    ..
                })
            ) {
                if let Some(path) = self.busy_after_admission_path.take() {
                    self.start_writer_lock(path);
                }
            }
            self.frames.pop_front().ok_or_else(|| {
                service::IndexServiceCallError::UnknownOutcome("scripted stream ended".into())
            })
        }

        fn send_lookup(
            &mut self,
            _response: service::IndexServiceLookupResponse,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    impl Drop for CoordinatorCycleIo {
        fn drop(&mut self) {
            self.release_writer_lock();
        }
    }

    impl GlobalIndexProvider for WindowsCoordinatorTestProvider {
        fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
            Ok(self.sources.clone())
        }

        fn start_initial_index(
            &self,
            source: &GlobalSourceDescriptor,
            sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            self.run_operation(
                source,
                sink,
                IndexServiceCommand::StartInitialIndex {
                    source_id: source.volume.id.clone(),
                },
            )
        }

        fn resume_incremental_sync(
            &self,
            source: &GlobalSourceDescriptor,
            sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            self.run_operation(
                source,
                sink,
                IndexServiceCommand::ResumeIncrementalSync {
                    source_id: source.volume.id.clone(),
                },
            )
        }

        fn rebuild(
            &self,
            source: &GlobalSourceDescriptor,
            sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            self.run_operation(
                source,
                sink,
                IndexServiceCommand::Rebuild {
                    source_id: source.volume.id.clone(),
                },
            )
        }

        fn has_external_source_operation_admission(&self) -> bool {
            true
        }

        fn pause(&self) -> Result<(), GlobalIndexError> {
            Ok(())
        }

        fn status(&self) -> Result<String, GlobalIndexError> {
            Ok("test-service".into())
        }

        fn shutdown(&self) -> Result<(), GlobalIndexError> {
            Ok(())
        }
    }

    struct CoordinatorDatabasePath(PathBuf);

    impl CoordinatorDatabasePath {
        fn new(label: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "zen-canvas-issue380-{label}-{}.sqlite3",
                uuid::Uuid::new_v4()
            )))
        }
    }

    impl Drop for CoordinatorDatabasePath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("sqlite3-wal"));
            let _ = std::fs::remove_file(self.0.with_extension("sqlite3-shm"));
        }
    }

    fn coordinator_source(id: &str) -> GlobalSourceDescriptor {
        let mut source = windows_source();
        source.volume.id = id.into();
        source.volume.stable_volume_id = format!("stable-{id}");
        source.volume.display_name = id.into();
        source.volume.mount_path = format!("{id}:\\");
        // Keep scripted checkpoints valid for the real v3 admission validator,
        // which accepts canonical decimal USN values only.
        source.volume.journal_id = Some("42".into());
        source.volume.journal_cursor = Some("42".into());
        source
    }

    fn coordinator_test_entry(volume_id: &str) -> GlobalEntryInput {
        GlobalEntryInput {
            volume_id: volume_id.into(),
            platform_file_id: format!("file-{volume_id}"),
            parent_platform_file_id: "root".into(),
            name: "preserved.txt".into(),
            path: format!("{volume_id}:\\preserved.txt"),
            extension: "txt".into(),
            is_directory: false,
            size: 42,
            created_at_fs: Some(3),
            modified_at_fs: Some(5),
            file_attributes: 0,
            is_hidden: false,
            is_system: false,
            source_provider: PROVIDER_WINDOWS_MFT_USN.into(),
            last_seen_at: 7,
        }
    }

    fn coordinator_reply_script(
        sources: &[GlobalSourceDescriptor],
        replies: Vec<(String, Vec<CoordinatorReply>)>,
    ) -> std::sync::Arc<Mutex<CoordinatorReplyMap>> {
        let mut scripts = CoordinatorReplyMap::new();
        for source in sources {
            scripts.insert(source.volume.id.clone(), VecDeque::new());
        }
        for (id, responses) in replies {
            scripts.insert(id, VecDeque::from(responses));
        }
        std::sync::Arc::new(Mutex::new(scripts))
    }

    fn seed_coordinator_source(db: &crate::db::Database, source: &GlobalSourceDescriptor) {
        db.upsert_global_volume(&source.volume)
            .expect("seed integration source");
        db.upsert_global_entries_batch(&[coordinator_test_entry(&source.volume.id)])
            .expect("seed active integration entry");
    }

    fn entry_stale_flag(db: &crate::db::Database, volume_id: &str) -> i64 {
        db.conn()
            .expect("database connection")
            .query_row(
                "SELECT is_stale FROM global_entries WHERE volume_id = ?1",
                [volume_id],
                |row| row.get(0),
            )
            .expect("read Global Search entry state")
    }

    #[test]
    fn issue380_v3_busy_preserves_source_and_entry_then_explicit_wake_recovers() {
        let path = CoordinatorDatabasePath::new("busy-recovery");
        let db = crate::db::Database::open(&path.0).expect("open coordinator test database");
        let source = coordinator_source("volume-a");
        seed_coordinator_source(&db, &source);
        let before = db.get_global_volume(&source.volume.id).unwrap().unwrap();
        let counters = std::sync::Arc::new(CoordinatorCycleCounters::default());
        let replies = coordinator_reply_script(
            std::slice::from_ref(&source),
            vec![(
                source.volume.id.clone(),
                vec![CoordinatorReply::ServiceBusy, CoordinatorReply::Complete],
            )],
        );
        let provider =
            WindowsCoordinatorTestProvider::new(vec![source.clone()], replies, counters.clone());
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let wake = GlobalIndexWakeSlot::default();

        crate::global_index::coordinator::run_index_cycle_for_test(&provider, &db, &cancel, &wake)
            .expect("typed Service busy is isolated to this request");

        let busy = db.get_global_volume(&source.volume.id).unwrap().unwrap();
        let busy_operation = db
            .global_index_operation_state(&source.volume.id)
            .unwrap()
            .unwrap();
        assert!(counters.service_busy.load(Ordering::Acquire));
        assert_eq!(counters.requests.load(Ordering::Acquire), 1);
        assert_eq!(counters.direct_calls.load(Ordering::Acquire), 0);
        assert_eq!(wake.snapshot().notifications, 0);
        assert_eq!(busy.index_status, before.index_status);
        assert_eq!(busy.last_error, before.last_error);
        assert_eq!(busy.journal_id, before.journal_id);
        assert_eq!(busy.journal_cursor, before.journal_cursor);
        assert_eq!(busy.last_full_index_at, before.last_full_index_at);
        assert_eq!(
            busy.last_incremental_sync_at,
            before.last_incremental_sync_at
        );
        assert_eq!(busy.entry_count, before.entry_count);
        assert_eq!(busy_operation.revision, 0);
        assert_eq!(busy_operation.owner_id, None);
        assert_eq!(entry_stale_flag(&db, &source.volume.id), 0);

        wake.notify(GlobalIndexWakeReason::ExplicitCommand);
        crate::global_index::coordinator::run_index_cycle_for_test(&provider, &db, &cancel, &wake)
            .expect("one later explicit wake admits and completes the request");
        let recovered = db.get_global_volume(&source.volume.id).unwrap().unwrap();
        assert_eq!(counters.requests.load(Ordering::Acquire), 2);
        assert_eq!(counters.direct_calls.load(Ordering::Acquire), 0);
        assert_eq!(recovered.index_status, INDEX_STATUS_READY);
        assert_eq!(recovered.journal_id.as_deref(), Some("42"));
        assert_eq!(recovered.journal_cursor.as_deref(), Some("43"));
        assert_eq!(entry_stale_flag(&db, &source.volume.id), 0);
        assert!(db
            .global_index_operation_state(&source.volume.id)
            .unwrap()
            .unwrap()
            .owner_id
            .is_none());
    }

    #[test]
    fn issue380_v3_admitted_permission_failure_and_success_remain_source_scoped() {
        let path = CoordinatorDatabasePath::new("source-isolation");
        let db = crate::db::Database::open(&path.0).expect("open coordinator test database");
        let denied = coordinator_source("volume-a");
        let healthy = coordinator_source("volume-b");
        seed_coordinator_source(&db, &denied);
        seed_coordinator_source(&db, &healthy);
        let counters = std::sync::Arc::new(CoordinatorCycleCounters::default());
        let provider = WindowsCoordinatorTestProvider::new(
            vec![denied.clone(), healthy.clone()],
            coordinator_reply_script(
                &[denied.clone(), healthy.clone()],
                vec![
                    (
                        denied.volume.id.clone(),
                        vec![CoordinatorReply::PermissionFailure],
                    ),
                    (healthy.volume.id.clone(), vec![CoordinatorReply::Complete]),
                ],
            ),
            counters.clone(),
        );
        let cancel = std::sync::Arc::new(AtomicBool::new(false));

        crate::global_index::coordinator::run_index_cycle_for_test(
            &provider,
            &db,
            &cancel,
            &GlobalIndexWakeSlot::default(),
        )
        .expect("source-local failure does not abort the other source");

        let denied_after = db.get_global_volume(&denied.volume.id).unwrap().unwrap();
        let healthy_after = db.get_global_volume(&healthy.volume.id).unwrap().unwrap();
        assert_eq!(denied_after.index_status, INDEX_STATUS_PERMISSION_REQUIRED);
        assert_eq!(
            denied_after.last_error.as_deref(),
            Some("permission required: source_access_denied")
        );
        assert_eq!(denied_after.journal_cursor, denied.volume.journal_cursor);
        assert_eq!(healthy_after.index_status, INDEX_STATUS_READY);
        assert_eq!(healthy_after.journal_id.as_deref(), Some("42"));
        assert_eq!(healthy_after.journal_cursor.as_deref(), Some("43"));
        assert_eq!(entry_stale_flag(&db, &denied.volume.id), 0);
        assert_eq!(entry_stale_flag(&db, &healthy.volume.id), 0);
        assert_eq!(counters.requests.load(Ordering::Acquire), 2);
        assert_eq!(counters.direct_calls.load(Ordering::Acquire), 0);
        for source in [&denied, &healthy] {
            let operation = db
                .global_index_operation_state(&source.volume.id)
                .unwrap()
                .unwrap();
            assert_eq!(operation.owner_id, None);
            assert_eq!(operation.operation_state, None);
        }
    }

    #[test]
    fn issue380_unknown_service_owner_blocks_direct_after_client_restart_then_reconciles() {
        let path = CoordinatorDatabasePath::new("unknown-restart");
        let db = crate::db::Database::open(&path.0).expect("open coordinator test database");
        let source = coordinator_source("volume-a");
        seed_coordinator_source(&db, &source);
        let counters = std::sync::Arc::new(CoordinatorCycleCounters::default());
        let replies = coordinator_reply_script(
            std::slice::from_ref(&source),
            vec![(
                source.volume.id.clone(),
                vec![
                    CoordinatorReply::UnknownAfterAdmission,
                    CoordinatorReply::NotSent,
                    CoordinatorReply::Complete,
                ],
            )],
        );
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let wake = GlobalIndexWakeSlot::default();
        let first_client = WindowsCoordinatorTestProvider::new(
            vec![source.clone()],
            replies.clone(),
            counters.clone(),
        );
        crate::global_index::coordinator::run_index_cycle_for_test(
            &first_client,
            &db,
            &cancel,
            &wake,
        )
        .expect("admitted transport loss is durably marked unknown");
        let unknown = db
            .global_index_operation_state(&source.volume.id)
            .unwrap()
            .unwrap();
        assert_eq!(unknown.operation_state.as_deref(), Some("unknown"));
        let unknown_owner = unknown.owner_id.clone().expect("persisted Service owner");
        let source_after_unknown = db.get_global_volume(&source.volume.id).unwrap().unwrap();
        assert_eq!(source_after_unknown.index_status, INDEX_STATUS_SYNCING);
        assert_eq!(
            source_after_unknown.journal_cursor,
            source.volume.journal_cursor
        );
        assert_eq!(entry_stale_flag(&db, &source.volume.id), 0);

        // A new Windows provider instance has no in-memory Direct replay fence.
        // The durable Service owner must still reject a not-sent Direct route.
        let restarted_client =
            WindowsCoordinatorTestProvider::new(vec![source.clone()], replies, counters.clone());
        crate::global_index::coordinator::run_index_cycle_for_test(
            &restarted_client,
            &db,
            &cancel,
            &wake,
        )
        .expect("Direct cannot replay an unresolved Service operation");
        let after_direct_rejection = db
            .global_index_operation_state(&source.volume.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            after_direct_rejection.owner_id.as_deref(),
            Some(unknown_owner.as_str())
        );
        assert_eq!(
            after_direct_rejection.operation_state.as_deref(),
            Some("unknown")
        );
        assert_eq!(counters.direct_calls.load(Ordering::Acquire), 0);
        assert_eq!(counters.requests.load(Ordering::Acquire), 2);

        wake.notify(GlobalIndexWakeReason::ExplicitCommand);
        crate::global_index::coordinator::run_index_cycle_for_test(
            &restarted_client,
            &db,
            &cancel,
            &wake,
        )
        .expect("a newly admitted Service request reconciles the source");
        let recovered = db.get_global_volume(&source.volume.id).unwrap().unwrap();
        assert_eq!(recovered.index_status, INDEX_STATUS_READY);
        assert_eq!(recovered.journal_cursor.as_deref(), Some("43"));
        assert_eq!(counters.requests.load(Ordering::Acquire), 3);
        assert!(db
            .global_index_operation_state(&source.volume.id)
            .unwrap()
            .unwrap()
            .owner_id
            .is_none());
    }

    #[test]
    fn issue380_real_sqlite_busy_at_v3_admission_is_not_service_busy_and_recovers() {
        let path = CoordinatorDatabasePath::new("sqlite-busy");
        let db = crate::db::Database::open(&path.0).expect("open coordinator test database");
        let source = coordinator_source("volume-a");
        seed_coordinator_source(&db, &source);
        let counters = std::sync::Arc::new(CoordinatorCycleCounters::default());
        let provider = WindowsCoordinatorTestProvider::new(
            vec![source.clone()],
            coordinator_reply_script(
                std::slice::from_ref(&source),
                vec![(
                    source.volume.id.clone(),
                    vec![
                        CoordinatorReply::SqliteBusy(path.0.clone()),
                        CoordinatorReply::Complete,
                    ],
                )],
            ),
            counters.clone(),
        );
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let wake = GlobalIndexWakeSlot::default();
        let before = db.get_global_volume(&source.volume.id).unwrap().unwrap();

        crate::global_index::coordinator::run_index_cycle_for_test(&provider, &db, &cancel, &wake)
            .expect("request-level SQLite contention leaves source health committed");

        let after_busy = db.get_global_volume(&source.volume.id).unwrap().unwrap();
        let operation = db
            .global_index_operation_state(&source.volume.id)
            .unwrap()
            .unwrap();
        assert!(counters.sqlite_busy.load(Ordering::Acquire));
        assert!(!counters.service_busy.load(Ordering::Acquire));
        assert_eq!(counters.direct_calls.load(Ordering::Acquire), 0);
        assert_eq!(after_busy.index_status, before.index_status);
        assert_eq!(after_busy.last_error, before.last_error);
        assert_eq!(after_busy.journal_cursor, before.journal_cursor);
        assert_eq!(operation.revision, 0);
        assert_eq!(operation.owner_id, None);
        assert_eq!(entry_stale_flag(&db, &source.volume.id), 0);
        assert_eq!(wake.snapshot().notifications, 0);

        wake.notify(GlobalIndexWakeReason::ExplicitCommand);
        crate::global_index::coordinator::run_index_cycle_for_test(&provider, &db, &cancel, &wake)
            .expect("one later admitted operation succeeds after SQLite releases");
        let recovered = db.get_global_volume(&source.volume.id).unwrap().unwrap();
        assert_eq!(recovered.index_status, INDEX_STATUS_READY);
        assert_eq!(recovered.journal_cursor.as_deref(), Some("43"));
        assert_eq!(counters.requests.load(Ordering::Acquire), 2);
    }

    #[test]
    fn issue328_admitted_sqlite_busy_fails_closed_and_rebuilds_on_a_new_service_event() {
        let path = CoordinatorDatabasePath::new("admitted-sqlite-busy");
        let db = crate::db::Database::open(&path.0).expect("open coordinator test database");
        db.set_test_busy_timeout_for_all_connections(5_000, std::time::Duration::from_millis(250))
            .expect("use a bounded test-only lock wait");
        let failed_source = coordinator_source("volume-a");
        let healthy_source = coordinator_source("volume-b");
        seed_coordinator_source(&db, &failed_source);
        seed_coordinator_source(&db, &healthy_source);
        assert!(db
            .request_global_index_rebuild(&failed_source.volume.id)
            .expect("persist rebuild intent before starting the operation"));
        let failed_before = db
            .get_global_volume(&failed_source.volume.id)
            .unwrap()
            .unwrap();
        let counters = std::sync::Arc::new(CoordinatorCycleCounters::default());
        let provider = WindowsCoordinatorTestProvider::new(
            vec![failed_source.clone(), healthy_source.clone()],
            coordinator_reply_script(
                &[failed_source.clone(), healthy_source.clone()],
                vec![
                    (
                        failed_source.volume.id.clone(),
                        vec![
                            CoordinatorReply::SqliteBusyAfterAdmission(path.0.clone()),
                            CoordinatorReply::CompleteWithEntries,
                        ],
                    ),
                    (
                        healthy_source.volume.id.clone(),
                        vec![CoordinatorReply::Complete, CoordinatorReply::Complete],
                    ),
                ],
            ),
            counters.clone(),
        );
        provider.windows.service_connected();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let wake = GlobalIndexWakeSlot::default();

        crate::global_index::coordinator::run_index_cycle_for_test(&provider, &db, &cancel, &wake)
            .expect("the admitted SQLite failure is committed as source-local error state");

        let failed_after = db
            .get_global_volume(&failed_source.volume.id)
            .unwrap()
            .unwrap();
        let failed_operation = db
            .global_index_operation_state(&failed_source.volume.id)
            .unwrap()
            .unwrap();
        let healthy_after = db
            .get_global_volume(&healthy_source.volume.id)
            .unwrap()
            .unwrap();
        let healthy_operation = db
            .global_index_operation_state(&healthy_source.volume.id)
            .unwrap()
            .unwrap();
        assert!(counters.sqlite_busy.load(Ordering::Acquire));
        assert!(!counters.service_busy.load(Ordering::Acquire));
        assert_eq!(counters.direct_calls.load(Ordering::Acquire), 0);
        assert!(provider.windows.service_available.load(Ordering::Acquire));
        assert_eq!(counters.requests.load(Ordering::Acquire), 2);
        assert_eq!(
            failed_after.index_status,
            crate::global_index::models::INDEX_STATUS_ERROR
        );
        assert!(failed_after.last_error.is_some());
        assert_eq!(failed_after.journal_id, failed_before.journal_id);
        assert_eq!(failed_after.journal_cursor, failed_before.journal_cursor);
        assert_eq!(
            failed_after.last_full_index_at,
            failed_before.last_full_index_at
        );
        assert_eq!(
            failed_after.last_incremental_sync_at,
            failed_before.last_incremental_sync_at
        );
        assert_eq!(entry_stale_flag(&db, &failed_source.volume.id), 1);
        assert_eq!(failed_operation.revision, 1);
        assert_eq!(failed_operation.owner_id, None);
        assert_eq!(failed_operation.operation_state, None);
        assert!(failed_operation.rebuild_pending());
        assert_eq!(healthy_after.index_status, INDEX_STATUS_READY);
        assert_eq!(healthy_after.journal_cursor.as_deref(), Some("43"));
        assert_eq!(entry_stale_flag(&db, &healthy_source.volume.id), 0);
        assert_eq!(healthy_operation.revision, 1);
        assert_eq!(healthy_operation.owner_id, None);

        wake.notify(GlobalIndexWakeReason::ExplicitCommand);
        crate::global_index::coordinator::run_index_cycle_for_test(&provider, &db, &cancel, &wake)
            .expect("a fresh Service admission safely reconciles the failed rebuild");
        let recovered = db
            .get_global_volume(&failed_source.volume.id)
            .unwrap()
            .unwrap();
        let recovered_operation = db
            .global_index_operation_state(&failed_source.volume.id)
            .unwrap()
            .unwrap();
        assert_eq!(counters.requests.load(Ordering::Acquire), 4);
        assert_eq!(counters.direct_calls.load(Ordering::Acquire), 0);
        assert_eq!(recovered.index_status, INDEX_STATUS_READY);
        assert_eq!(recovered.journal_cursor.as_deref(), Some("43"));
        assert_eq!(entry_stale_flag(&db, &failed_source.volume.id), 0);
        assert_eq!(recovered_operation.revision, 2);
        assert_eq!(recovered_operation.owner_id, None);
        assert!(!recovered_operation.rebuild_pending());
    }

    #[derive(Default)]
    struct SourceStateSink {
        database: Option<crate::db::Database>,
        status: Option<String>,
        error: Option<String>,
    }

    impl GlobalIndexSink for SourceStateSink {
        fn write_batch(&mut self, entries: &[GlobalEntryInput]) -> Result<usize, GlobalIndexError> {
            Ok(entries.len())
        }

        fn mark_entry_stale(&mut self, _entry_id: &str) -> Result<(), GlobalIndexError> {
            Ok(())
        }

        fn checkpoint(
            &mut self,
            _volume_id: &str,
            _journal_id: Option<&str>,
            _journal_cursor: Option<&str>,
        ) -> Result<(), GlobalIndexError> {
            Ok(())
        }

        fn set_source_state(
            &mut self,
            _volume_id: &str,
            status: &str,
            error: Option<&str>,
        ) -> Result<(), GlobalIndexError> {
            if let Some(db) = &self.database {
                db.update_global_volume_state(_volume_id, status, error, None, None, None, None)?;
            }
            self.status = Some(status.to_string());
            self.error = error.map(str::to_string);
            Ok(())
        }

        fn set_source_provider(
            &mut self,
            _volume_id: &str,
            _provider: &str,
        ) -> Result<(), GlobalIndexError> {
            Ok(())
        }

        fn resolve_parent_path(
            &mut self,
            _volume_id: &str,
            _parent_platform_file_id: &str,
        ) -> Result<Option<String>, GlobalIndexError> {
            Ok(None)
        }

        fn find_entry_by_identity(
            &mut self,
            _volume_id: &str,
            _platform_file_id: &str,
            _parent_platform_file_id: &str,
            _name: &str,
        ) -> Result<Option<GlobalEntry>, GlobalIndexError> {
            Ok(None)
        }

        fn mark_volume_entries_stale(&mut self, _volume_id: &str) -> Result<(), GlobalIndexError> {
            Ok(())
        }
    }

    #[test]
    fn p1_direct_fallback_only_for_proven_not_sent_and_never_after_uncertain_history() {
        let provider = WindowsGlobalIndexProvider::new();
        let request = WindowsGlobalIndexProvider::source_request(
            IndexServiceCommand::Rebuild {
                source_id: "volume".into(),
            },
            &windows_source(),
        );
        let mut sink = SourceStateSink::default();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let direct = |_: &DirectWindowsGlobalIndexProvider, _: &mut dyn GlobalIndexSink| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        };
        provider
            .resolve_service_route(
                &request,
                &mut sink,
                Err(service::IndexServiceCallError::NotSent(
                    "connect failed".into(),
                )),
                direct,
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        for result in [
            Ok(Err(GlobalIndexError::ServiceBusy("busy".into()))),
            Err(service::IndexServiceCallError::Protocol(
                "bad identity".into(),
            )),
            Err(service::IndexServiceCallError::UnknownOutcome(
                "partial send".into(),
            )),
            Err(service::IndexServiceCallError::Sink(
                GlobalIndexError::Provider("sink failure".into()),
            )),
        ] {
            assert!(provider
                .resolve_service_route(&request, &mut sink, result, direct)
                .is_err());
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        }
        assert!(provider
            .resolve_service_route(
                &request,
                &mut sink,
                Err(service::IndexServiceCallError::NotSent(
                    "later disconnect".into()
                )),
                direct
            )
            .is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn p1_admitted_failed_terminal_fences_later_not_sent_direct_replay() {
        let provider = WindowsGlobalIndexProvider::new();
        let volume = service::p1_tests::source();
        let request = IndexServiceRequest::new(
            IndexServiceCommand::Rebuild {
                source_id: volume.id.clone(),
            },
            Some(volume.clone()),
        );
        let event = |event| service::IndexServiceFrame::Event {
            request_id: request.request_id.clone(),
            event,
        };
        let terminal = IndexServiceResponse {
            protocol_version: service::IPC_PROTOCOL_VERSION,
            request_id: request.request_id.clone(),
            ok: false,
            error_code: Some("index_provider_error".into()),
            message: Some("provider failed after partial scan".into()),
            status: Some("error".into()),
        };
        let mut io = ScriptedServiceIo {
            frames: VecDeque::from([
                event(IndexServiceEvent::Sources {
                    sources: vec![volume.clone()],
                }),
                event(IndexServiceEvent::SourceProvider {
                    volume_id: volume.id.clone(),
                    provider: PROVIDER_WINDOWS_MFT_USN.into(),
                }),
                event(IndexServiceEvent::VolumeEntriesStale {
                    volume_id: volume.id.clone(),
                }),
                event(IndexServiceEvent::Entries {
                    entries: vec![GlobalEntryInput {
                        volume_id: volume.id.clone(),
                        platform_file_id: "file-1".into(),
                        parent_platform_file_id: "parent-1".into(),
                        name: "partial.txt".into(),
                        path: "Z:\\partial.txt".into(),
                        extension: "txt".into(),
                        is_directory: false,
                        size: 17,
                        created_at_fs: None,
                        modified_at_fs: None,
                        file_attributes: 0,
                        is_hidden: false,
                        is_system: false,
                        source_provider: PROVIDER_WINDOWS_MFT_USN.into(),
                        last_seen_at: 1,
                    }],
                }),
                service::IndexServiceFrame::Response { response: terminal },
            ]),
        };
        let mut sink = service::p1_tests::Sink::default();
        let stream = provider.run_service_stream_with_io(&request, &mut sink, &mut io);
        assert!(matches!(
            &stream,
            Ok(Err(GlobalIndexError::Provider(message)))
                if message == "provider failed after partial scan"
        ));
        assert_eq!(sink.provider_writes, 1);
        assert_eq!(sink.volume_stale_writes, 1);
        assert_eq!(sink.batch_writes, 1);
        assert_eq!(sink.writes, 3);

        let calls = std::sync::atomic::AtomicUsize::new(0);
        let error = provider
            .resolve_service_route(&request, &mut sink, stream, |_, _| {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(error, GlobalIndexError::Provider(_)));
        let next_request = IndexServiceRequest::new(
            IndexServiceCommand::Rebuild {
                source_id: volume.id.clone(),
            },
            Some(volume),
        );
        let next = provider.resolve_service_route(
            &next_request,
            &mut sink,
            Err(service::IndexServiceCallError::NotSent(
                "next connection failed before send".into(),
            )),
            |_, _| {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        );
        assert!(matches!(next, Err(GlobalIndexError::ServiceProtocol(_))));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn p1_pre_admission_busy_stays_typed_and_does_not_block_a_later_not_sent_request() {
        let provider = WindowsGlobalIndexProvider::new();
        let volume = service::p1_tests::source();
        let request = IndexServiceRequest::new(
            IndexServiceCommand::Rebuild {
                source_id: volume.id.clone(),
            },
            Some(volume),
        );
        let busy = IndexServiceResponse {
            protocol_version: service::IPC_PROTOCOL_VERSION,
            request_id: request.request_id.clone(),
            ok: false,
            error_code: Some("index_service_busy".into()),
            message: Some("another request owns the service slot".into()),
            status: Some("indexing".into()),
        };
        let mut io = ScriptedServiceIo {
            frames: VecDeque::from([service::IndexServiceFrame::Response { response: busy }]),
        };
        let mut sink = service::p1_tests::Sink::default();
        let stream = provider.run_service_stream_with_io(&request, &mut sink, &mut io);
        assert!(matches!(
            &stream,
            Ok(Err(GlobalIndexError::ServiceBusy(message)))
                if message == "another request owns the service slot"
        ));
        assert_eq!(sink.writes, 0);

        let calls = std::sync::atomic::AtomicUsize::new(0);
        let error = provider
            .resolve_service_route(&request, &mut sink, stream, |_, _| {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(error, GlobalIndexError::ServiceBusy(_)));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        let next_request = IndexServiceRequest::new(
            IndexServiceCommand::Rebuild {
                source_id: request.source.as_ref().unwrap().id.clone(),
            },
            request.source.clone(),
        );
        provider
            .resolve_service_route(
                &next_request,
                &mut sink,
                Err(service::IndexServiceCallError::NotSent(
                    "new request was not sent".into(),
                )),
                |_, _| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn empty_initial_mft_baseline_marks_direct_provider_rebuild_required() {
        let mut sink = SourceStateSink::default();
        let error = GlobalIndexError::RebuildRequired(
            "windows_mft_baseline_empty: staged_records=0 resolved_entries=0 emitted_entries=0 checkpoint_usn=0"
                .to_string(),
        );

        let returned = finish_initial_mft_result("volume", &mut sink, Err(error));

        assert!(returned.is_err());
        assert_eq!(sink.status.as_deref(), Some(INDEX_STATUS_REBUILD_REQUIRED));
        assert!(sink.error.as_deref().is_some_and(
            |error| error.starts_with("rebuild required: windows_mft_baseline_empty:")
        ));
    }

    #[test]
    fn direct_provider_status_does_not_claim_service_transport() {
        assert_eq!(
            DirectWindowsGlobalIndexProvider::new()
                .status()
                .expect("direct status"),
            "windows_mft_usn"
        );
    }

    #[test]
    fn failed_change_watcher_is_replaced_and_source_request_continues() {
        let provider = WindowsGlobalIndexProvider::with_fake_change_watchers();
        let source = windows_source();
        provider
            .ensure_native_change_watcher(&source)
            .expect("create initial watcher");
        assert_eq!(provider.test_watcher_start_count.load(Ordering::Acquire), 1);

        provider
            .ensure_native_change_watcher(&source)
            .expect("reuse healthy watcher");
        assert_eq!(provider.test_watcher_start_count.load(Ordering::Acquire), 1);

        provider
            .change_watchers
            .lock()
            .expect("watcher map")
            .get(&source.volume.id)
            .expect("current watcher")
            .mark_failed_for_test();

        let request = WindowsGlobalIndexProvider::source_request(
            IndexServiceCommand::ResumeIncrementalSync {
                source_id: source.volume.id.clone(),
            },
            &source,
        );
        let mut sink = SourceStateSink::default();
        let mut reached_direct_provider = false;
        provider
            .run_source_request(&request, &source, &mut sink, |_direct, _sink| {
                reached_direct_provider = true;
                Ok(())
            })
            .expect("request continues after watcher replacement");

        assert!(reached_direct_provider, "USN provider path was not reached");
        assert_eq!(provider.test_watcher_start_count.load(Ordering::Acquire), 2);
        assert!(provider
            .change_watchers
            .lock()
            .expect("watcher map")
            .get(&source.volume.id)
            .is_some_and(|watcher| !watcher.has_failed()));
    }

    #[test]
    fn source_disable_and_watcher_cleanup_helpers_drop_native_handles() {
        let provider = WindowsGlobalIndexProvider::with_fake_change_watchers();
        let source = windows_source();
        provider
            .ensure_native_change_watcher(&source)
            .expect("create watcher");

        provider.source_enabled_changed(&source.volume.id, false);
        assert!(provider
            .change_watchers
            .lock()
            .expect("watcher map")
            .is_empty());

        provider
            .ensure_native_change_watcher(&source)
            .expect("recreate watcher");
        provider.clear_native_change_watchers();
        assert!(provider
            .change_watchers
            .lock()
            .expect("watcher map")
            .is_empty());
    }

    #[test]
    fn service_request_keeps_persisted_source_checkpoint() {
        let source = GlobalSourceDescriptor {
            volume: GlobalVolume {
                id: "volume".to_string(),
                platform: "windows".to_string(),
                stable_volume_id: "stable".to_string(),
                display_name: "C".to_string(),
                mount_path: "C:\\".to_string(),
                filesystem_type: "ntfs".to_string(),
                drive_kind: "fixed".to_string(),
                enabled: true,
                provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
                index_status: "ready".to_string(),
                last_error: None,
                journal_id: Some("journal".to_string()),
                journal_cursor: Some("42".to_string()),
                last_full_index_at: Some(1),
                last_incremental_sync_at: Some(2),
                entry_count: 3,
                created_at: 0,
                updated_at: 0,
            },
        };
        let request = WindowsGlobalIndexProvider::source_request(
            IndexServiceCommand::ResumeIncrementalSync {
                source_id: "volume".to_string(),
            },
            &source,
        );
        assert_eq!(request.source, Some(source.volume));
    }
    include!("recovery_tests.rs");
}
