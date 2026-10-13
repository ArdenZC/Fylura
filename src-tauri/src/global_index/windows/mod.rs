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

    fn service_failed(&self) {
        self.service_available.store(false, Ordering::Release);
    }

    fn service_connected(&self) {
        self.service_available.store(true, Ordering::Release);
    }

    fn service_response_error(response: &IndexServiceResponse) -> GlobalIndexError {
        service_errors::decode(response)
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
        let mut admission = service::IndexServiceAdmission::new(request);
        let response =
            service::call_index_service_admitted(request, |event| admission.apply(sink, event))?;
        admission.finish(&response)?;
        self.service_connected();
        if response.ok {
            super::qa_trace::record("windows_service_route_ok");
            Ok(Ok(()))
        } else {
            Ok(Err(Self::service_response_error(&response)))
        }
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
            return direct(&self.direct, sink);
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
                direct(&self.direct, sink)
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
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        #[cfg(test)]
        if self.force_direct {
            return self.direct.discover_sources();
        }
        let request = IndexServiceRequest::new(IndexServiceCommand::DiscoverSources, None);
        let mut sources = None;
        match service::call_index_service_stream(&request, |_client, event| {
            if let IndexServiceEvent::Sources {
                sources: discovered,
            } = event
            {
                sources = Some(discovered);
            }
            Ok(())
        }) {
            Ok(response) if response.ok => {
                self.service_connected();
                Ok(sources
                    .unwrap_or_default()
                    .into_iter()
                    .map(|volume| GlobalSourceDescriptor { volume })
                    .collect())
            }
            Ok(response) => Err(Self::service_response_error(&response)),
            Err(_) => {
                self.service_failed();
                self.direct.discover_sources()
            }
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

    fn pause(&self) -> Result<(), GlobalIndexError> {
        self.clear_native_change_watchers();
        let request = IndexServiceRequest::new(IndexServiceCommand::Pause, None);
        match service::call_index_service(&request) {
            Ok(response) if response.ok => {
                self.service_connected();
                Ok(())
            }
            Ok(response) => Err(Self::service_response_error(&response)),
            Err(_) => {
                self.service_failed();
                self.direct.pause()
            }
        }
    }

    fn status(&self) -> Result<String, GlobalIndexError> {
        let request = IndexServiceRequest::new(IndexServiceCommand::Status, None);
        match service::call_index_service(&request) {
            Ok(response) if response.ok => {
                self.service_connected();
                Ok(format!(
                    "windows_index_service:{}",
                    response.status.unwrap_or_else(|| "ready".to_string())
                ))
            }
            Ok(response) => Err(Self::service_response_error(&response)),
            Err(_) => {
                self.service_failed();
                Ok(format!("{}:service_unavailable", self.direct.status()?))
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_index::models::{GlobalEntry, GlobalEntryInput, GlobalVolume};

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
