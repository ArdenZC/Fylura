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
    GlobalSourceDescriptor, INDEX_STATUS_INDEXING, INDEX_STATUS_PERMISSION_REQUIRED,
    INDEX_STATUS_REBUILD_REQUIRED, INDEX_STATUS_SYNCING, INDEX_STATUS_UNAVAILABLE,
    PROVIDER_WINDOWS_MFT_USN,
};
use crate::global_index::wake::{GlobalIndexWakeReason, GlobalIndexWakeSlot};
use service::{
    IndexServiceCallError, IndexServiceClientIo, IndexServiceCommand, IndexServiceEvent,
    IndexServiceEventError, IndexServiceLookupResponse, IndexServiceRequest, IndexServiceResponse,
};
use std::collections::{HashMap, HashSet};
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
                INDEX_STATUS_PERMISSION_REQUIRED,
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
            sink.set_source_state(volume_id, INDEX_STATUS_PERMISSION_REQUIRED, Some(&message))?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexEventDisposition {
    Admission,
    Apply,
}

struct IndexServiceAdmission {
    expected: crate::global_index::models::GlobalVolume,
    active_status: &'static str,
    requires_stale_before_entries: bool,
    admitted: bool,
    source_provider_seen: bool,
    operation_payload_seen: bool,
    volume_stale_seen: bool,
    terminal_source_state_seen: bool,
}

fn discovered_sources_are_well_formed(
    sources: &[crate::global_index::models::GlobalVolume],
) -> bool {
    let mut source_ids = HashSet::with_capacity(sources.len());
    sources.iter().all(|source| {
        !source.id.trim().is_empty()
            && !source.stable_volume_id.trim().is_empty()
            && !source.mount_path.trim().is_empty()
            && !source.filesystem_type.trim().is_empty()
            && !source.provider.trim().is_empty()
            && source_ids.insert(source.id.as_str())
    })
}

impl IndexServiceAdmission {
    fn new(request: &IndexServiceRequest) -> Result<Self, String> {
        let requires_stale_before_entries = matches!(
            &request.command,
            IndexServiceCommand::StartInitialIndex { .. } | IndexServiceCommand::Rebuild { .. }
        );
        let source_id = match &request.command {
            IndexServiceCommand::StartInitialIndex { source_id }
            | IndexServiceCommand::ResumeIncrementalSync { source_id }
            | IndexServiceCommand::Rebuild { source_id } => source_id,
            _ => return Err("index_service_admission_requires_index_command".to_string()),
        };
        let expected = request
            .source
            .as_ref()
            .ok_or_else(|| "index_service_source_snapshot_required".to_string())?;
        if expected.id != *source_id {
            return Err("index_service_source_id_mismatch".to_string());
        }
        let source_reappeared = expected.index_status == INDEX_STATUS_UNAVAILABLE
            && expected.last_error.as_deref() == Some("global_index_source_unavailable");
        let active_status = if expected.last_full_index_at.is_some()
            && !source_reappeared
            && expected.index_status != INDEX_STATUS_REBUILD_REQUIRED
        {
            INDEX_STATUS_SYNCING
        } else {
            INDEX_STATUS_INDEXING
        };
        Ok(Self {
            expected: expected.clone(),
            active_status,
            requires_stale_before_entries,
            admitted: false,
            source_provider_seen: false,
            operation_payload_seen: false,
            volume_stale_seen: false,
            terminal_source_state_seen: false,
        })
    }

    fn validate(&mut self, event: &IndexServiceEvent) -> Result<IndexEventDisposition, String> {
        if let IndexServiceEvent::Sources { sources } = event {
            if self.admitted {
                return Err("index_service_duplicate_admission".to_string());
            }
            if sources.len() != 1
                || !self.expected.enabled
                || !sources[0].enabled
                || !service::source_identity_matches(&self.expected, &sources[0])
            {
                return Err("index_service_admission_source_mismatch".to_string());
            }
            self.admitted = true;
            return Ok(IndexEventDisposition::Admission);
        }

        if !self.admitted {
            return Err("index_service_event_before_admission".to_string());
        }
        if self.terminal_source_state_seen {
            return Err("index_service_event_after_terminal_source_state".to_string());
        }
        let expected_id = self.expected.id.as_str();
        let volume_matches = match event {
            IndexServiceEvent::Entries { entries } => entries.iter().all(|entry| {
                entry.volume_id == expected_id
                    && service::provider_snapshot_is_compatible(
                        &entry.source_provider,
                        &self.expected.provider,
                    )
            }),
            IndexServiceEvent::EntryStale { volume_id, .. }
            | IndexServiceEvent::VolumeEntriesStale { volume_id }
            | IndexServiceEvent::Checkpoint { volume_id, .. }
            | IndexServiceEvent::SourceState { volume_id, .. }
            | IndexServiceEvent::SourceProvider { volume_id, .. }
            | IndexServiceEvent::ResolveParentPath { volume_id, .. }
            | IndexServiceEvent::FindEntryByIdentity { volume_id, .. } => volume_id == expected_id,
            IndexServiceEvent::Sources { .. } => unreachable!("handled above"),
        };
        if !volume_matches {
            return Err("index_service_event_source_mismatch".to_string());
        }
        if self.requires_stale_before_entries
            && matches!(event, IndexServiceEvent::Entries { .. })
            && !self.volume_stale_seen
        {
            return Err("index_service_entries_before_volume_stale".to_string());
        }
        match event {
            IndexServiceEvent::SourceProvider { provider, .. } => {
                if self.source_provider_seen || self.operation_payload_seen {
                    return Err("index_service_source_provider_event_out_of_order".to_string());
                }
                if !service::provider_snapshot_is_compatible(provider, &self.expected.provider) {
                    return Err("index_service_event_provider_mismatch".to_string());
                }
                self.source_provider_seen = true;
            }
            IndexServiceEvent::SourceState { .. } => {
                self.terminal_source_state_seen = true;
            }
            IndexServiceEvent::VolumeEntriesStale { .. } => {
                if !self.source_provider_seen || self.volume_stale_seen {
                    return Err("index_service_volume_stale_event_out_of_order".to_string());
                }
                self.volume_stale_seen = true;
                self.operation_payload_seen = true;
            }
            _ => {
                if !self.source_provider_seen {
                    return Err("index_service_operation_event_before_source_provider".to_string());
                }
                self.operation_payload_seen = true;
            }
        }
        if let IndexServiceEvent::Entries { entries } = event {
            if !entries.iter().all(|entry| {
                service::provider_snapshot_is_compatible(
                    &entry.source_provider,
                    &self.expected.provider,
                )
            }) {
                return Err("index_service_event_provider_mismatch".to_string());
            }
        }
        Ok(IndexEventDisposition::Apply)
    }
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
        cancel: &AtomicBool,
        direct: impl FnOnce(
            &DirectWindowsGlobalIndexProvider,
            &mut dyn GlobalIndexSink,
        ) -> Result<(), GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        self.ensure_native_change_watcher(source)?;
        self.run_with_fallback(request, sink, cancel, direct)
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

    fn apply_event<C: IndexServiceClientIo>(
        sink: &mut dyn GlobalIndexSink,
        client: &mut C,
        event: IndexServiceEvent,
    ) -> Result<(), IndexServiceEventError> {
        match event {
            IndexServiceEvent::Entries { entries } => sink
                .write_batch(&entries)
                .map(|_| ())
                .map_err(IndexServiceEventError::Sink),
            IndexServiceEvent::EntryStale { entry_id, .. } => sink
                .mark_entry_stale(&entry_id)
                .map_err(IndexServiceEventError::Sink),
            IndexServiceEvent::VolumeEntriesStale { volume_id } => sink
                .mark_volume_entries_stale(&volume_id)
                .map_err(IndexServiceEventError::Sink),
            IndexServiceEvent::Checkpoint {
                volume_id,
                journal_id,
                journal_cursor,
            } => sink
                .checkpoint(&volume_id, journal_id.as_deref(), journal_cursor.as_deref())
                .map_err(IndexServiceEventError::Sink),
            IndexServiceEvent::SourceState {
                volume_id,
                status,
                error,
            } => sink
                .set_source_state(&volume_id, &status, error.as_deref())
                .map_err(IndexServiceEventError::Sink),
            IndexServiceEvent::SourceProvider {
                volume_id,
                provider,
            } => sink
                .set_source_provider(&volume_id, &provider)
                .map_err(IndexServiceEventError::Sink),
            IndexServiceEvent::ResolveParentPath {
                lookup_id,
                volume_id,
                parent_platform_file_id,
            } => {
                let path = sink
                    .resolve_parent_path(&volume_id, &parent_platform_file_id)
                    .map_err(IndexServiceEventError::Sink)?;
                client
                    .send_lookup_response(IndexServiceLookupResponse::ParentPath {
                        lookup_id,
                        path,
                    })
                    .map_err(IndexServiceEventError::Transport)
            }
            IndexServiceEvent::FindEntryByIdentity {
                lookup_id,
                volume_id,
                platform_file_id,
                parent_platform_file_id,
                name,
            } => {
                let entry = sink
                    .find_entry_by_identity(
                        &volume_id,
                        &platform_file_id,
                        &parent_platform_file_id,
                        &name,
                    )
                    .map_err(IndexServiceEventError::Sink)?;
                client
                    .send_lookup_response(IndexServiceLookupResponse::Entry {
                        lookup_id,
                        entry: Box::new(entry),
                    })
                    .map_err(IndexServiceEventError::Transport)
            }
            IndexServiceEvent::Sources { .. } => Ok(()),
        }
    }

    fn run_service_stream(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<Result<(), GlobalIndexError>, IndexServiceCallError> {
        service::validate_request(request).map_err(IndexServiceCallError::InvalidRequest)?;
        if cancel.load(Ordering::Acquire) {
            return Ok(Err(GlobalIndexError::Paused));
        }
        let mut admission =
            IndexServiceAdmission::new(request).map_err(IndexServiceCallError::InvalidRequest)?;
        let response = service::call_index_service_stream(request, |client, event| {
            Self::apply_validated_index_event(&mut admission, sink, cancel, client, event)
        })?;
        self.finish_service_stream(request, admission, response)
    }

    #[cfg(test)]
    fn run_service_stream_with_client<C: IndexServiceClientIo>(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
        client: &mut C,
    ) -> Result<Result<(), GlobalIndexError>, IndexServiceCallError> {
        service::validate_request(request).map_err(IndexServiceCallError::InvalidRequest)?;
        if cancel.load(Ordering::Acquire) {
            return Ok(Err(GlobalIndexError::Paused));
        }
        let mut admission =
            IndexServiceAdmission::new(request).map_err(IndexServiceCallError::InvalidRequest)?;
        let response = service::call_stream_with_client(request, client, |client, event| {
            Self::apply_validated_index_event(&mut admission, sink, cancel, client, event)
        })?;
        self.finish_service_stream(request, admission, response)
    }

    fn apply_validated_index_event<C: IndexServiceClientIo>(
        admission: &mut IndexServiceAdmission,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
        client: &mut C,
        event: IndexServiceEvent,
    ) -> Result<(), IndexServiceEventError> {
        if cancel.load(Ordering::Acquire) {
            return Err(IndexServiceEventError::Rejected(
                "index_service_request_cancelled_after_dispatch".to_string(),
            ));
        }
        match admission
            .validate(&event)
            .map_err(IndexServiceEventError::Rejected)?
        {
            IndexEventDisposition::Admission => sink
                .set_source_state(
                    &admission.expected.id,
                    admission.active_status,
                    (admission.expected.index_status == INDEX_STATUS_PERMISSION_REQUIRED)
                        .then_some(admission.expected.last_error.as_deref())
                        .flatten(),
                )
                .map_err(IndexServiceEventError::Sink),
            IndexEventDisposition::Apply => Self::apply_event(sink, client, event),
        }
    }

    fn finish_service_stream(
        &self,
        _request: &IndexServiceRequest,
        admission: IndexServiceAdmission,
        response: IndexServiceResponse,
    ) -> Result<Result<(), GlobalIndexError>, IndexServiceCallError> {
        if response.ok {
            if !admission.admitted {
                return Err(IndexServiceCallError::UnknownOutcome(
                    "index_service_success_before_admission".to_string(),
                ));
            }
            super::qa_trace::record("windows_service_route_ok");
        } else if response.error_code.as_deref() == Some("index_service_busy") && admission.admitted
        {
            return Err(IndexServiceCallError::UnknownOutcome(
                "index_service_busy_after_admission".to_string(),
            ));
        }
        self.service_connected();
        if response.ok {
            Ok(Ok(()))
        } else {
            Ok(Err(Self::service_response_error(&response)))
        }
    }

    fn run_with_fallback(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
        direct: impl FnOnce(
            &DirectWindowsGlobalIndexProvider,
            &mut dyn GlobalIndexSink,
        ) -> Result<(), GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        self.run_with_fallback_using(
            request,
            sink,
            cancel,
            direct,
            |provider, request, sink, cancel| provider.run_service_stream(request, sink, cancel),
        )
    }

    fn run_with_fallback_using<F>(
        &self,
        request: &IndexServiceRequest,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
        direct: impl FnOnce(
            &DirectWindowsGlobalIndexProvider,
            &mut dyn GlobalIndexSink,
        ) -> Result<(), GlobalIndexError>,
        service_call: F,
    ) -> Result<(), GlobalIndexError>
    where
        F: FnOnce(
            &Self,
            &IndexServiceRequest,
            &mut dyn GlobalIndexSink,
            &AtomicBool,
        ) -> Result<Result<(), GlobalIndexError>, IndexServiceCallError>,
    {
        #[cfg(test)]
        if self.force_direct {
            return direct(&self.direct, sink);
        }
        match service_call(self, request, sink, cancel) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(IndexServiceCallError::NotSent(error)) => {
                self.service_failed();
                let _ = sink.set_source_state(
                    request
                        .source
                        .as_ref()
                        .map(|source| source.id.as_str())
                        .unwrap_or_default(),
                    crate::global_index::models::INDEX_STATUS_UNAVAILABLE,
                    Some(&error),
                );
                direct(&self.direct, sink)
            }
            Err(IndexServiceCallError::InvalidRequest(error)) => {
                Err(GlobalIndexError::Provider(error))
            }
            Err(IndexServiceCallError::SinkFailure(error)) => Err(error),
            Err(IndexServiceCallError::UnknownOutcome(error)) => {
                self.service_failed();
                Err(GlobalIndexError::ServiceOutcomeUnknown(error))
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
        match service::call_index_service_stream(&request, |_client, event| match event {
            IndexServiceEvent::Sources {
                sources: discovered,
            } => {
                if sources.is_some() || !discovered_sources_are_well_formed(&discovered) {
                    return Err(IndexServiceEventError::Rejected(
                        "index_service_invalid_sources_event".to_string(),
                    ));
                }
                sources = Some(discovered);
                Ok(())
            }
            _ => Err(IndexServiceEventError::Rejected(
                "index_service_unexpected_discovery_event".to_string(),
            )),
        }) {
            Ok(response) if response.ok => {
                self.service_connected();
                let sources = sources.ok_or_else(|| {
                    GlobalIndexError::ServiceOutcomeUnknown(
                        "index_service_discovery_missing_sources_event".to_string(),
                    )
                })?;
                Ok(sources
                    .into_iter()
                    .map(|volume| GlobalSourceDescriptor { volume })
                    .collect())
            }
            Ok(response) => Err(Self::service_response_error(&response)),
            Err(IndexServiceCallError::NotSent(_)) => {
                self.service_failed();
                self.direct.discover_sources()
            }
            Err(IndexServiceCallError::InvalidRequest(error)) => {
                Err(GlobalIndexError::Provider(error))
            }
            Err(IndexServiceCallError::SinkFailure(error)) => Err(error),
            Err(IndexServiceCallError::UnknownOutcome(error)) => {
                self.service_failed();
                Err(GlobalIndexError::ServiceOutcomeUnknown(error))
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
        self.run_source_request(&request, source, sink, cancel, |direct, sink| {
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
        self.run_source_request(&request, source, sink, cancel, |direct, sink| {
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
            Err(IndexServiceCallError::NotSent(_)) => {
                self.service_failed();
                self.direct.pause()
            }
            Err(IndexServiceCallError::InvalidRequest(error)) => {
                Err(GlobalIndexError::Provider(error))
            }
            Err(IndexServiceCallError::SinkFailure(error)) => Err(error),
            Err(IndexServiceCallError::UnknownOutcome(error)) => {
                self.service_failed();
                Err(GlobalIndexError::ServiceOutcomeUnknown(error))
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
            Err(IndexServiceCallError::NotSent(_)) => {
                self.service_failed();
                Ok(format!("{}:service_unavailable", self.direct.status()?))
            }
            Err(IndexServiceCallError::InvalidRequest(error)) => {
                Err(GlobalIndexError::Provider(error))
            }
            Err(IndexServiceCallError::SinkFailure(error)) => Err(error),
            Err(IndexServiceCallError::UnknownOutcome(error)) => {
                self.service_failed();
                Err(GlobalIndexError::ServiceOutcomeUnknown(error))
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

    fn defers_active_status_until_admission(&self) -> bool {
        true
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
            .run_source_request(
                &request,
                &source,
                &mut sink,
                &AtomicBool::new(false),
                |_direct, _sink| {
                    reached_direct_provider = true;
                    Ok(())
                },
            )
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
