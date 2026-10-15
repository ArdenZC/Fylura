//! Native macOS global-index provider.
//!
//! Spotlight owns the initial inventory and its metadata-query notifications
//! provide low-latency updates. FSEvents is deliberately used as a reconcile
//! signal: it is authoritative about filesystem change pressure, while
//! Spotlight remains the source of file metadata.

mod fsevents;
mod spotlight;

use super::coordinator::{GlobalIndexError, GlobalIndexProvider, GlobalIndexSink};
use super::models::{
    normalize_path, GlobalEntryInput, GlobalSourceDescriptor, GlobalVolume,
    INDEX_STATUS_FSEVENTS_UNAVAILABLE, INDEX_STATUS_PERMISSION_REQUIRED, INDEX_STATUS_READY,
    INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED, INDEX_STATUS_SPOTLIGHT_NOT_INDEXED,
    INDEX_STATUS_SPOTLIGHT_UNAVAILABLE, INDEX_STATUS_UNAVAILABLE,
    PROVIDER_MACOS_FSEVENTS_RECONCILE, PROVIDER_MACOS_SPOTLIGHT,
};
use super::wake::{GlobalIndexWakeReason, GlobalIndexWakeSlot};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub(crate) const MAX_PENDING_SPOTLIGHT_ENTRIES: usize = 4096;

#[derive(Default)]
pub(crate) struct PendingUpdates {
    pub upserts: HashMap<String, GlobalEntryInput>,
    pub stale_entry_ids: HashSet<String>,
    pub full_reconcile: bool,
    pub last_event_id: Option<u64>,
    pub last_error: Option<String>,
}

pub(super) fn record_async_watcher_error(
    pending: &Mutex<PendingUpdates>,
    wake: &GlobalIndexWakeSlot,
    error: impl Into<String>,
) {
    let mut pending = pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pending.last_error = Some(error.into());
    drop(pending);
    #[cfg(feature = "native-qa")]
    super::qa_trace::record("provider_change_source\tasync_watcher_error\tcount=1");
    wake.notify(GlobalIndexWakeReason::ProviderChange);
}

impl PendingUpdates {
    pub(crate) fn append_incremental(
        &mut self,
        entries: Vec<GlobalEntryInput>,
        stale_entry_ids: Vec<String>,
        full_reconcile: bool,
    ) {
        if full_reconcile {
            self.full_reconcile = true;
        }
        if self.full_reconcile {
            self.upserts.clear();
            self.stale_entry_ids.clear();
            return;
        }
        for entry in entries {
            let entry_id = entry.entry_id();
            if !self.upserts.contains_key(&entry_id)
                && !self.stale_entry_ids.contains(&entry_id)
                && self.unique_pending_identities() >= MAX_PENDING_SPOTLIGHT_ENTRIES
            {
                self.clear_incremental_and_reconcile();
                return;
            }
            self.stale_entry_ids.remove(&entry_id);
            self.upserts.insert(entry_id, entry);
        }
        for entry_id in stale_entry_ids {
            if !self.upserts.contains_key(&entry_id)
                && !self.stale_entry_ids.contains(&entry_id)
                && self.unique_pending_identities() >= MAX_PENDING_SPOTLIGHT_ENTRIES
            {
                self.clear_incremental_and_reconcile();
                return;
            }
            self.upserts.remove(&entry_id);
            self.stale_entry_ids.insert(entry_id);
        }
    }

    fn unique_pending_identities(&self) -> usize {
        self.upserts
            .len()
            .saturating_add(self.stale_entry_ids.len())
    }

    fn clear_incremental_and_reconcile(&mut self) {
        self.upserts.clear();
        self.stale_entry_ids.clear();
        self.full_reconcile = true;
    }

    fn take_upserts(&mut self) -> Vec<GlobalEntryInput> {
        let mut entries = std::mem::take(&mut self.upserts)
            .into_values()
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.entry_id());
        entries
    }
}

#[derive(Debug, Default)]
pub(crate) struct KnownEntries {
    entry_id_by_path: HashMap<String, String>,
    path_by_entry_id: HashMap<String, String>,
}

impl KnownEntries {
    fn upsert(&mut self, entry: &GlobalEntryInput) {
        let path = normalize_path(&entry.path);
        let entry_id = entry.entry_id();
        if let Some(previous_id) = self.entry_id_by_path.insert(path.clone(), entry_id.clone()) {
            if previous_id != entry_id && self.path_by_entry_id.get(&previous_id) == Some(&path) {
                self.path_by_entry_id.remove(&previous_id);
            }
        }
        if let Some(previous_path) = self.path_by_entry_id.insert(entry_id.clone(), path.clone()) {
            if previous_path != path && self.entry_id_by_path.get(&previous_path) == Some(&entry_id)
            {
                self.entry_id_by_path.remove(&previous_path);
            }
        }
    }

    fn current_entry_id_for_path(&self, path: &str) -> Option<String> {
        self.entry_id_by_path.get(&normalize_path(path)).cloned()
    }

    fn forget_entry_id(&mut self, entry_id: &str) {
        let Some(path) = self.path_by_entry_id.remove(entry_id) else {
            return;
        };
        if self
            .entry_id_by_path
            .get(&path)
            .is_some_and(|value| value == entry_id)
        {
            self.entry_id_by_path.remove(&path);
        }
    }

    fn clear(&mut self) {
        self.entry_id_by_path.clear();
        self.path_by_entry_id.clear();
    }

    #[cfg(test)]
    fn path_for_entry_id(&self, entry_id: &str) -> Option<&str> {
        self.path_by_entry_id.get(entry_id).map(String::as_str)
    }
}

pub struct MacosSpotlightProvider {
    stopped: Arc<AtomicBool>,
    pending: Arc<Mutex<PendingUpdates>>,
    known_entries: Arc<Mutex<KnownEntries>>,
    spotlight_watcher: Mutex<Option<spotlight::SpotlightWatcherHandle>>,
    fsevents_watcher: Mutex<Option<fsevents::FseventsHandle>>,
    baseline_established: AtomicBool,
    wake: Arc<GlobalIndexWakeSlot>,
}

impl MacosSpotlightProvider {
    pub fn new() -> Self {
        Self::with_wake(Arc::new(GlobalIndexWakeSlot::default()))
    }

    pub(crate) fn with_wake(wake: Arc<GlobalIndexWakeSlot>) -> Self {
        Self {
            stopped: Arc::new(AtomicBool::new(false)),
            pending: Arc::new(Mutex::new(PendingUpdates::default())),
            known_entries: Arc::new(Mutex::new(KnownEntries::default())),
            spotlight_watcher: Mutex::new(None),
            fsevents_watcher: Mutex::new(None),
            baseline_established: AtomicBool::new(false),
            wake,
        }
    }

    fn start_watchers(
        &self,
        volume_id: &str,
        since_event_id: Option<u64>,
    ) -> Result<(), GlobalIndexError> {
        let mut spotlight_slot = self.spotlight_watcher.lock().map_err(|_| {
            GlobalIndexError::Provider("macOS Spotlight watcher lock poisoned".to_string())
        })?;
        if spotlight_slot.is_none() {
            *spotlight_slot = Some(
                spotlight::spawn_update_watcher(
                    volume_id.to_string(),
                    self.pending.clone(),
                    self.known_entries.clone(),
                    self.stopped.clone(),
                    self.wake.clone(),
                )
                .map_err(GlobalIndexError::Provider)?,
            );
        }
        drop(spotlight_slot);

        let mut fsevents_slot = self.fsevents_watcher.lock().map_err(|_| {
            GlobalIndexError::Provider("macOS FSEvents watcher lock poisoned".to_string())
        })?;
        if fsevents_slot.is_none() {
            *fsevents_slot = Some(
                fsevents::start_reconcile_watcher(
                    Path::new("/"),
                    self.pending.clone(),
                    self.stopped.clone(),
                    self.wake.clone(),
                    since_event_id,
                )
                .map_err(GlobalIndexError::Provider)?,
            );
        }
        Ok(())
    }

    fn stop_watchers(&self) {
        if let Ok(mut watcher) = self.spotlight_watcher.lock() {
            if let Some(handle) = watcher.take() {
                handle.stop();
            }
        }
        if let Ok(mut watcher) = self.fsevents_watcher.lock() {
            if let Some(handle) = watcher.take() {
                handle.stop();
            }
        }
    }

    fn take_pending(&self) -> PendingUpdates {
        self.pending
            .lock()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_else(|_| PendingUpdates {
                last_error: Some("macOS native index update lock poisoned".to_string()),
                ..PendingUpdates::default()
            })
    }

    fn take_pending_or_report_error(
        &self,
        volume_id: &str,
        sink: &mut dyn GlobalIndexSink,
    ) -> Result<PendingUpdates, GlobalIndexError> {
        let pending = self.take_pending();
        if let Some(error) = pending.last_error.clone() {
            return Err(Self::report_native_error(sink, volume_id, &error)?);
        }
        Ok(pending)
    }

    fn remember_entries(&self, entries: &[GlobalEntryInput]) {
        if let Ok(mut known) = self.known_entries.lock() {
            for entry in entries {
                known.upsert(entry);
            }
        }
    }

    fn forget_entry_id(&self, entry_id: &str) {
        if let Ok(mut known) = self.known_entries.lock() {
            known.forget_entry_id(entry_id);
        }
    }

    fn clear_known_entries(&self) {
        if let Ok(mut known) = self.known_entries.lock() {
            known.clear();
        }
    }

    fn write_entries(
        &self,
        sink: &mut dyn GlobalIndexSink,
        entries: &[GlobalEntryInput],
    ) -> Result<(), GlobalIndexError> {
        for batch in entries.chunks(512) {
            sink.write_batch(batch)?;
            self.remember_entries(batch);
        }
        Ok(())
    }

    fn stream_spotlight_entries(
        &self,
        sink: &mut dyn GlobalIndexSink,
        volume_id: &str,
        cancel: &AtomicBool,
    ) -> Result<spotlight::SpotlightCollectionSummary, GlobalIndexError> {
        match spotlight::stream_local_computer_entries(volume_id, cancel, |batch| {
            sink.write_batch(batch).map_err(|error| error.to_string())?;
            self.remember_entries(batch);
            Ok(())
        }) {
            Ok(summary) => Ok(summary),
            Err(error) if error == "macos_spotlight_query_paused" => Err(GlobalIndexError::Paused),
            Err(error) if error.starts_with("callback:") => Err(GlobalIndexError::Provider(
                error.trim_start_matches("callback:").to_string(),
            )),
            Err(error) => Err(Self::report_native_error(sink, volume_id, &error)?),
        }
    }

    fn record_collection_state(
        sink: &mut dyn GlobalIndexSink,
        volume_id: &str,
        summary: spotlight::SpotlightCollectionSummary,
    ) -> Result<(), GlobalIndexError> {
        if summary.full_disk_access_required {
            sink.set_source_state(
                volume_id,
                INDEX_STATUS_PERMISSION_REQUIRED,
                Some("macos_spotlight_full_disk_access_required"),
            )?;
        } else if summary.external_volume_not_indexed {
            sink.set_source_state(
                volume_id,
                INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED,
                Some("macos_spotlight_external_volume_not_indexed"),
            )?;
        } else if summary.processed == 0 {
            sink.set_source_state(
                volume_id,
                INDEX_STATUS_SPOTLIGHT_NOT_INDEXED,
                Some("macos_spotlight_no_indexed_local_results"),
            )?;
        } else if summary.path_fallbacks > 0 {
            sink.set_source_state(
                volume_id,
                INDEX_STATUS_PERMISSION_REQUIRED,
                Some("macos_spotlight_protected_directories"),
            )?;
        } else if summary.skipped > 0 {
            sink.set_source_state(
                volume_id,
                INDEX_STATUS_PERMISSION_REQUIRED,
                Some("macos_spotlight_incomplete_results"),
            )?;
        } else {
            sink.set_source_state(volume_id, INDEX_STATUS_READY, None)?;
        }
        Ok(())
    }

    fn report_native_error(
        sink: &mut dyn GlobalIndexSink,
        volume_id: &str,
        error: &str,
    ) -> Result<GlobalIndexError, GlobalIndexError> {
        let status = if error.contains("macos_spotlight") {
            if error.contains("full_disk_access")
                || error.contains("permission")
                || error.contains("protected")
            {
                super::models::INDEX_STATUS_PERMISSION_REQUIRED
            } else if error.contains("external_volume_not_indexed") {
                INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED
            } else {
                INDEX_STATUS_SPOTLIGHT_UNAVAILABLE
            }
        } else if error.contains("macos_fsevents") {
            INDEX_STATUS_FSEVENTS_UNAVAILABLE
        } else {
            INDEX_STATUS_UNAVAILABLE
        };
        sink.set_source_state(volume_id, status, Some(error))?;
        Ok(GlobalIndexError::Provider(error.to_string()))
    }
}

impl Default for MacosSpotlightProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalIndexProvider for MacosSpotlightProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(vec![GlobalSourceDescriptor {
            volume: GlobalVolume::macos_spotlight_local_computer(),
        }])
    }

    fn start_initial_index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.stopped.store(false, Ordering::Release);
        sink.mark_volume_entries_stale(&source.volume.id)?;
        self.clear_known_entries();
        if let Err(error) = self.start_watchers(
            &source.volume.id,
            source
                .volume
                .journal_cursor
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok()),
        ) {
            let error = error.to_string();
            return Err(Self::report_native_error(sink, &source.volume.id, &error)?);
        }
        #[cfg(feature = "native-qa")]
        super::qa_trace::record("spotlight_initial_query_started");
        let collection = self.stream_spotlight_entries(sink, &source.volume.id, cancel);
        #[cfg(feature = "native-qa")]
        match &collection {
            Ok(_) => super::qa_trace::record("spotlight_initial_query_completed"),
            Err(error) => {
                super::qa_trace::record_error("spotlight_initial_query", &error.to_string())
            }
        }
        let summary = collection?;
        Self::record_collection_state(sink, &source.volume.id, summary)?;
        self.baseline_established.store(true, Ordering::Release);
        Ok(())
    }

    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.stopped.store(false, Ordering::Release);
        if let Err(error) = self.start_watchers(
            &source.volume.id,
            source
                .volume
                .journal_cursor
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok()),
        ) {
            let error = error.to_string();
            return Err(Self::report_native_error(sink, &source.volume.id, &error)?);
        }
        let mut pending = self.take_pending_or_report_error(&source.volume.id, sink)?;
        let needs_baseline_reconcile = !self.baseline_established.load(Ordering::Acquire);
        if pending.full_reconcile || needs_baseline_reconcile {
            sink.mark_volume_entries_stale(&source.volume.id)?;
            self.clear_known_entries();
            let summary = self.stream_spotlight_entries(sink, &source.volume.id, cancel)?;
            Self::record_collection_state(sink, &source.volume.id, summary)?;
            self.baseline_established.store(true, Ordering::Release);
        } else {
            let stale_entry_ids = std::mem::take(&mut pending.stale_entry_ids);
            for entry_id in stale_entry_ids {
                sink.mark_entry_stale(&entry_id)?;
                self.forget_entry_id(&entry_id);
            }
            self.write_entries(sink, &pending.take_upserts())?;
        }
        if let Some(event_id) = pending.last_event_id {
            let event_id = event_id.to_string();
            sink.checkpoint(&source.volume.id, None, Some(&event_id))?;
        }
        Ok(())
    }

    fn requires_background_admission(&self, _source: &GlobalSourceDescriptor) -> bool {
        !self.baseline_established.load(Ordering::Acquire)
            || self
                .pending
                .lock()
                .map(|pending| pending.full_reconcile)
                .unwrap_or(true)
    }

    fn pause(&self) -> Result<(), GlobalIndexError> {
        self.stopped.store(true, Ordering::Release);
        self.baseline_established.store(false, Ordering::Release);
        self.stop_watchers();
        Ok(())
    }

    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok(format!(
            "{PROVIDER_MACOS_SPOTLIGHT}+{PROVIDER_MACOS_FSEVENTS_RECONCILE}"
        ))
    }

    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        self.stopped.store(true, Ordering::Release);
        self.stop_watchers();
        if let Ok(mut pending) = self.pending.lock() {
            pending.full_reconcile = false;
            pending.upserts.clear();
            pending.stale_entry_ids.clear();
            pending.last_event_id = None;
        }
        self.clear_known_entries();
        self.baseline_established.store(false, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_index::coordinator::GlobalIndexError;
    use crate::global_index::models::{
        GlobalEntry, INDEX_STATUS_FSEVENTS_UNAVAILABLE, INDEX_STATUS_PERMISSION_REQUIRED,
        INDEX_STATUS_READY, INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED,
        INDEX_STATUS_SPOTLIGHT_NOT_INDEXED, INDEX_STATUS_SPOTLIGHT_UNAVAILABLE,
    };

    #[derive(Default)]
    struct RecordingSink {
        statuses: Vec<(String, String, Option<String>)>,
        batch_lengths: Vec<usize>,
    }

    impl GlobalIndexSink for RecordingSink {
        fn write_batch(&mut self, entries: &[GlobalEntryInput]) -> Result<usize, GlobalIndexError> {
            self.batch_lengths.push(entries.len());
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
            volume_id: &str,
            status: &str,
            error: Option<&str>,
        ) -> Result<(), GlobalIndexError> {
            self.statuses.push((
                volume_id.to_string(),
                status.to_string(),
                error.map(ToString::to_string),
            ));
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

    fn input(index: usize) -> GlobalEntryInput {
        GlobalEntryInput {
            volume_id: "volume".to_string(),
            platform_file_id: format!("mac:dev:1:ino:{index}"),
            parent_platform_file_id: "mac:dev:1:ino:1".to_string(),
            name: format!("file-{index}.txt"),
            path: format!("/tmp/file-{index}.txt"),
            extension: "txt".to_string(),
            is_directory: false,
            size: 1,
            created_at_fs: None,
            modified_at_fs: None,
            file_attributes: 0,
            is_hidden: false,
            is_system: false,
            source_provider: PROVIDER_MACOS_SPOTLIGHT.to_string(),
            last_seen_at: index as i64,
        }
    }

    #[test]
    fn macos_spotlight_batches_use_the_shared_sink_contract() {
        let entries = (0..1025).map(input).collect::<Vec<_>>();
        let mut sink = RecordingSink::default();
        let provider = MacosSpotlightProvider::new();
        provider
            .write_entries(&mut sink, &entries)
            .expect("write batches");
        assert_eq!(sink.batch_lengths, vec![512, 512, 1]);
    }

    #[test]
    fn macos_collection_states_distinguish_unavailable_permission_and_partial_results() {
        let cases = [
            (
                spotlight::SpotlightCollectionSummary::default(),
                INDEX_STATUS_SPOTLIGHT_NOT_INDEXED,
            ),
            (
                spotlight::SpotlightCollectionSummary {
                    processed: 1,
                    ..Default::default()
                },
                INDEX_STATUS_READY,
            ),
            (
                spotlight::SpotlightCollectionSummary {
                    processed: 1,
                    full_disk_access_required: true,
                    ..Default::default()
                },
                INDEX_STATUS_PERMISSION_REQUIRED,
            ),
            (
                spotlight::SpotlightCollectionSummary {
                    processed: 1,
                    external_volume_not_indexed: true,
                    ..Default::default()
                },
                INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED,
            ),
            (
                spotlight::SpotlightCollectionSummary {
                    processed: 1,
                    path_fallbacks: 1,
                    ..Default::default()
                },
                INDEX_STATUS_PERMISSION_REQUIRED,
            ),
            (
                spotlight::SpotlightCollectionSummary {
                    processed: 1,
                    skipped: 1,
                    ..Default::default()
                },
                INDEX_STATUS_PERMISSION_REQUIRED,
            ),
        ];
        for (summary, expected_status) in cases {
            let mut sink = RecordingSink::default();
            MacosSpotlightProvider::record_collection_state(&mut sink, "volume", summary)
                .expect("record collection state");
            assert_eq!(
                sink.statuses.last().map(|value| value.1.as_str()),
                Some(expected_status)
            );
        }
    }

    #[test]
    fn macos_native_error_mapping_keeps_permission_and_spotlight_states_distinct() {
        let cases = [
            (
                "macos_spotlight_query_unavailable",
                INDEX_STATUS_SPOTLIGHT_UNAVAILABLE,
            ),
            (
                "macos_spotlight_full_disk_access_required",
                INDEX_STATUS_PERMISSION_REQUIRED,
            ),
            (
                "macos_spotlight_external_volume_not_indexed",
                INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED,
            ),
            (
                "macos_fsevents_stream_unavailable",
                INDEX_STATUS_FSEVENTS_UNAVAILABLE,
            ),
        ];
        for (error, expected_status) in cases {
            let mut sink = RecordingSink::default();
            let returned = MacosSpotlightProvider::report_native_error(&mut sink, "volume", error)
                .expect("native error mapping");
            assert!(returned.to_string().contains(error));
            assert_eq!(
                sink.statuses.last().map(|value| value.1.as_str()),
                Some(expected_status)
            );
        }
    }

    #[test]
    fn async_watcher_error_wakes_then_incremental_drain_records_degraded_status() {
        let cases = [
            (
                "macos_spotlight_realtime_updates_unavailable",
                INDEX_STATUS_SPOTLIGHT_UNAVAILABLE,
            ),
            (
                "macos_fsevents_stream_start_failed",
                INDEX_STATUS_FSEVENTS_UNAVAILABLE,
            ),
        ];
        for (error_code, expected_status) in cases {
            let wake = Arc::new(GlobalIndexWakeSlot::default());
            let provider = MacosSpotlightProvider::with_wake(wake.clone());
            record_async_watcher_error(&provider.pending, &wake, error_code);

            assert_eq!(wake.snapshot().notifications, 1);
            assert!(
                provider.pending.try_lock().is_ok(),
                "wake ran while lock held"
            );

            let mut sink = RecordingSink::default();
            let error = match provider.take_pending_or_report_error("volume", &mut sink) {
                Err(error) => error,
                Ok(_) => panic!("pending watcher error must be reported"),
            };
            assert!(error.to_string().contains(error_code));
            assert_eq!(
                sink.statuses.last().map(|value| value.1.as_str()),
                Some(expected_status)
            );
            assert!(provider
                .pending
                .lock()
                .expect("pending updates")
                .last_error
                .is_none());
        }
    }

    #[test]
    fn macos_bridge_lifecycle_is_idempotent_and_shutdown_clears_native_state() {
        let provider = MacosSpotlightProvider::new();
        assert!(provider.status().is_ok());
        provider.pause().expect("pause provider");
        provider.pause().expect("pause provider twice");
        provider.shutdown().expect("shutdown provider");
        provider.shutdown().expect("shutdown provider twice");
    }

    #[test]
    fn spotlight_pending_overflow_discards_incremental_items_and_requests_reconcile() {
        let mut pending = PendingUpdates::default();
        pending.append_incremental(
            (0..=MAX_PENDING_SPOTLIGHT_ENTRIES).map(input).collect(),
            Vec::new(),
            false,
        );
        assert!(pending.full_reconcile);
        assert!(pending.upserts.is_empty());
        assert!(pending.stale_entry_ids.is_empty());
    }

    #[test]
    fn spotlight_pending_keeps_incremental_updates_bounded() {
        let mut pending = PendingUpdates::default();
        pending.append_incremental(vec![input(1)], vec!["stale-1".to_string()], false);
        assert_eq!(pending.upserts.len(), 1);
        assert!(pending.stale_entry_ids.contains("stale-1"));
        pending.append_incremental(Vec::new(), Vec::new(), true);
        assert!(pending.full_reconcile);
        assert!(pending.upserts.is_empty());
        assert!(pending.stale_entry_ids.is_empty());
    }

    #[test]
    fn spotlight_pending_coalesces_repeated_identity_and_resolves_conflicts() {
        let mut pending = PendingUpdates::default();
        pending.append_incremental(vec![input(7), input(7)], Vec::new(), false);
        assert_eq!(pending.unique_pending_identities(), 1);
        pending.append_incremental(Vec::new(), vec![input(7).entry_id()], false);
        assert!(pending.upserts.is_empty());
        assert!(pending.stale_entry_ids.contains(&input(7).entry_id()));
        pending.append_incremental(vec![input(7)], Vec::new(), false);
        assert!(pending.stale_entry_ids.is_empty());
        assert_eq!(pending.upserts.len(), 1);
    }

    #[test]
    fn known_entries_is_bidirectional_and_delayed_old_remove_is_ignored() {
        let mut known = KnownEntries::default();
        let old = input(11);
        let mut renamed = old.clone();
        renamed.path = "/tmp/renamed-file.txt".to_string();

        known.upsert(&old);
        assert_eq!(
            known.current_entry_id_for_path(&old.path),
            Some(old.entry_id())
        );
        assert_eq!(
            known.path_for_entry_id(&old.entry_id()),
            Some(old.path.as_str())
        );

        known.upsert(&renamed);
        assert_eq!(known.current_entry_id_for_path(&old.path), None);
        assert_eq!(
            known.current_entry_id_for_path(&renamed.path),
            Some(renamed.entry_id())
        );
        assert_eq!(
            known.path_for_entry_id(&renamed.entry_id()),
            Some(renamed.path.as_str())
        );

        // A delayed remove for the old path must resolve through the old path
        // first. The physical identity is still current at the renamed path,
        // so forgetting that identity directly would incorrectly delete the
        // live entry.
        if let Some(entry_id) = known.current_entry_id_for_path(&old.path) {
            known.forget_entry_id(&entry_id);
        }
        assert_eq!(
            known.current_entry_id_for_path(&renamed.path),
            Some(renamed.entry_id())
        );
        known.forget_entry_id(&renamed.entry_id());
        assert_eq!(known.current_entry_id_for_path(&renamed.path), None);
    }

    #[test]
    fn known_entries_removes_previous_identity_when_a_path_is_reused() {
        let mut known = KnownEntries::default();
        let first = input(21);
        let mut second = input(22);
        second.path = first.path.clone();

        known.upsert(&first);
        known.upsert(&second);

        assert_eq!(
            known.current_entry_id_for_path(&first.path),
            Some(second.entry_id())
        );
        assert_eq!(known.path_for_entry_id(&first.entry_id()), None);
        assert_eq!(
            known.path_for_entry_id(&second.entry_id()),
            Some(second.path.as_str())
        );
    }

    #[test]
    fn known_entries_multiple_renames_leave_only_the_current_path() {
        let mut known = KnownEntries::default();
        let mut entry = input(31);
        let paths = [
            "/tmp/rename-a.txt",
            "/tmp/rename-b.txt",
            "/tmp/rename-c.txt",
            "/tmp/rename-d.txt",
        ];
        let mut previous = entry.path.clone();
        for path in paths {
            entry.path = path.to_string();
            known.upsert(&entry);
            assert_eq!(known.current_entry_id_for_path(&previous), None);
            assert_eq!(
                known.current_entry_id_for_path(&entry.path),
                Some(entry.entry_id())
            );
            previous = entry.path.clone();
        }
        known.forget_entry_id(&entry.entry_id());
        for path in paths {
            assert_eq!(known.current_entry_id_for_path(path), None);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_native_bookkeeping_benchmark_is_bounded_by_unique_identity() {
        use std::time::Instant;

        let operations = if std::env::var_os("ZC_MACOS_NATIVE_FULL_PROFILE").is_some() {
            1_000_000usize
        } else {
            100_000usize
        };
        let active_identities = 1_024usize;
        let started = Instant::now();
        let mut known = KnownEntries::default();
        let mut pending = PendingUpdates::default();

        for index in 0..operations {
            let slot = index % active_identities;
            let mut entry = input(slot);
            let old_path = format!("/tmp/zen-canvas-bookkeeping/{slot}/old");
            entry.path = old_path.clone();
            known.upsert(&entry);

            entry.path = format!("/tmp/zen-canvas-bookkeeping/{slot}/new-{index}");
            known.upsert(&entry);
            assert_eq!(known.current_entry_id_for_path(&old_path), None);

            pending.append_incremental(vec![entry.clone()], Vec::new(), false);
            if index % 3 == 0 {
                pending.append_incremental(Vec::new(), vec![entry.entry_id()], false);
            } else if index % 3 == 1 {
                pending.append_incremental(vec![entry], Vec::new(), false);
            }
            if index % 17 == 0 {
                known.forget_entry_id(&format!("mac:dev:1:ino:{slot}"));
            }
        }

        let elapsed = started.elapsed();
        println!(
            "macos_spotlight_bookkeeping operations={} active_identities={} known_paths={} pending_unique={} elapsed_ms={}",
            operations,
            active_identities,
            known.entry_id_by_path.len(),
            pending.unique_pending_identities(),
            elapsed.as_secs_f64() * 1000.0
        );
        assert!(known.entry_id_by_path.len() <= active_identities);
        assert!(known.path_by_entry_id.len() <= active_identities);
        assert!(pending.unique_pending_identities() <= active_identities);
    }
}
