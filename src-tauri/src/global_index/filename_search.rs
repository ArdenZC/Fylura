//! Default-off, in-process filename candidate projection for Global Search.
//!
//! SQLite remains authoritative. This module never installs persistent
//! triggers or writes an index/feed. Pool connections report commits through
//! SQLite's connection-local hooks; a dedicated read-only connection binds a
//! compact base plus the bounded in-memory overlay to one SQLite snapshot.

use crate::db::DbError;
#[cfg(test)]
use rusqlite::StatementStatus;
use rusqlite::{hooks::Action, Connection, OpenFlags, OptionalExtension, Transaction};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{
    AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering as AtomicOrdering,
};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

#[cfg(test)]
thread_local! {
    static CANDIDATE_SEARCH_STAGE_CAPTURE: std::cell::RefCell<Option<HashMap<&'static str, u64>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn capture_candidate_search_stages<T>(
    operation: impl FnOnce() -> T,
) -> (T, HashMap<&'static str, u64>) {
    let previous =
        CANDIDATE_SEARCH_STAGE_CAPTURE.with(|capture| capture.replace(Some(HashMap::new())));
    let value = operation();
    let captured = CANDIDATE_SEARCH_STAGE_CAPTURE
        .with(|capture| capture.replace(previous))
        .unwrap_or_default();
    (value, captured)
}

#[cfg(test)]
pub(crate) fn record_candidate_search_stage(stage: &'static str, started: Instant) {
    let elapsed_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    CANDIDATE_SEARCH_STAGE_CAPTURE.with(|capture| {
        if let Some(stages) = capture.borrow_mut().as_mut() {
            let total = stages.entry(stage).or_default();
            *total = total.saturating_add(elapsed_ns);
        }
    });
}

const MAX_INDEX_ENTRIES: usize = 2_000_000;
const MAX_INDEX_PRIVATE_BYTES: usize = 112 * 1024 * 1024;
const MAX_EXTENSION_DICTIONARY_BUILD_BYTES: usize = 8 * 1024 * 1024;
const FILENAME_READER_CACHE_LIMIT_KIB: i64 = 4_096;
const MAX_OVERLAY_ROWIDS: usize = 16_384;
const MAX_OVERLAY_BATCHES: usize = 8_192;
const MAX_ROWIDS_PER_TRANSACTION: usize = MAX_OVERLAY_ROWIDS;
const REBUILD_RETRY_DELAY: Duration = Duration::from_secs(3);

const FILENAME_INDEX_BUILD_SQL: &str =
    "SELECT ge.rowid, ge.id, ge.name_normalized, lower(ge.extension), ge.modified_at_fs
    FROM global_entries ge INDEXED BY idx_global_entries_active_name_order
    JOIN global_volumes gv ON gv.id=ge.volume_id
    WHERE gv.enabled=1 AND gv.coverage_pending=0 AND ge.is_stale=0
    ORDER BY ge.name_normalized COLLATE BINARY";

#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct FilenameSearchDiagnostics {
    pub enabled: bool,
    pub ready: bool,
    pub rebuild_in_progress: bool,
    pub rebuilds: u64,
    pub accepted_queries: u64,
    pub fallback_not_ready: u64,
    pub fallback_busy: u64,
    pub fallback_untracked_change: u64,
    pub fallback_source_change: u64,
    pub fallback_overlay_limit: u64,
    pub overlay_rows: usize,
    pub index_entries: usize,
    pub distinct_filename_count: usize,
    pub index_private_bytes: usize,
    pub index_build_peak_bytes: usize,
    pub last_build_micros: u64,
    pub last_error: Option<String>,
}

#[cfg(test)]
#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct FilenameIndexBuildMemoryPhase {
    pub stage: &'static str,
    pub elapsed_micros: u64,
    pub name_bytes_capacity: usize,
    pub name_offsets_capacity_bytes: usize,
    pub extension_bytes_capacity: usize,
    pub extension_offsets_capacity_bytes: usize,
    pub stable_id_bytes_capacity: usize,
    pub stable_id_offsets_capacity_bytes: usize,
    pub rowid_capacity_bytes: usize,
    pub modified_at_capacity_bytes: usize,
    pub modified_at_present_capacity_bytes: usize,
    pub name_ids_capacity_bytes: usize,
    pub extension_ids_capacity_bytes: usize,
    pub rowid_order_capacity_bytes: usize,
    pub index_capacity_bytes: usize,
    pub extension_dictionary_entries: usize,
    pub extension_dictionary_capacity: usize,
    pub extension_dictionary_key_capacity_bytes: usize,
    pub extension_dictionary_estimated_bytes: usize,
    pub sqlite_cache_used_bytes: Option<usize>,
    pub sqlite_schema_used_bytes: Option<usize>,
    pub sqlite_statement_used_bytes: Option<usize>,
    pub sqlite_lookaside_used_bytes: Option<usize>,
    pub sqlite_build_statement_memory_bytes: Option<usize>,
}

#[cfg(test)]
#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct FilenameIndexBuildMemoryProfile {
    pub phases: Vec<FilenameIndexBuildMemoryPhase>,
    pub row_text_string_allocations: u64,
    pub row_text_capacity_bytes: u64,
    pub distinct_name_string_capacity_bytes_total: u64,
    pub current_name_live_capacity_bytes: u64,
    pub retained_extension_key_capacity_bytes: u64,
    pub max_single_row_text_capacity_bytes: usize,
    pub temporary_row_text_capacity_bytes_total: u64,
    pub vector_capacity_growth_buffers: u64,
    pub vector_capacity_growth_bytes: u64,
    pub vector_reallocation_conservative_peak_bytes: usize,
    pub extension_dictionary_growth_events: u64,
    pub extension_dictionary_growth_bytes: u64,
    pub extension_dictionary_reallocation_conservative_peak_bytes: usize,
    pub row_scan_micros: u64,
    pub rowid_order_materialize_micros: u64,
    pub rowid_order_sort_micros: u64,
    pub total_build_micros: u64,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FilenameRankStageProfile {
    pub identity_setup_ns: u64,
    pub lookup_and_classification_ns: u64,
    pub top_k_heap_ns: u64,
    pub heap_sort_ns: u64,
    pub accepted_candidates: usize,
    pub overlay_rows: usize,
}

#[cfg(test)]
fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

#[derive(Default)]
struct DiagnosticCounters {
    rebuilds: AtomicU64,
    accepted_queries: AtomicU64,
    fallback_not_ready: AtomicU64,
    fallback_busy: AtomicU64,
    fallback_untracked_change: AtomicU64,
    fallback_source_change: AtomicU64,
    fallback_overlay_limit: AtomicU64,
    last_overlay_rows: AtomicU64,
    last_build_micros: AtomicU64,
    last_error: Mutex<Option<String>>,
}

#[derive(Default)]
struct ReaderState {
    connection: Option<Arc<Mutex<Connection>>>,
    index: Option<Arc<FilenameSearchIndex>>,
    retry_after: Option<Instant>,
}

pub(crate) struct FilenameSearchManager {
    path: PathBuf,
    state: Mutex<ReaderState>,
    tracker: Arc<MutationTracker>,
    enabled: AtomicBool,
    rebuild_in_progress: AtomicBool,
    counters: DiagnosticCounters,
}

impl FilenameSearchManager {
    pub(crate) fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            state: Mutex::new(ReaderState::default()),
            tracker: Arc::new(MutationTracker::default()),
            enabled: AtomicBool::new(candidate_enabled_from_environment()),
            rebuild_in_progress: AtomicBool::new(false),
            counters: DiagnosticCounters::default(),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(AtomicOrdering::Acquire)
    }

    /// Test builds install hooks even when the runtime candidate is off so a
    /// test can enable it after opening its fixture Database.
    pub(crate) fn should_install_mutation_hooks(&self) -> bool {
        cfg!(test) || self.is_enabled()
    }

    pub(crate) fn install_mutation_hooks(&self, connection: &Connection) -> rusqlite::Result<()> {
        let pending = Arc::new(PendingHookState::default());
        let main_write_pending = Arc::new(AtomicBool::new(false));
        let update_pending = Arc::clone(&pending);
        let update_main_write_pending = Arc::clone(&main_write_pending);
        let update_tracker = Arc::clone(&self.tracker);
        connection.update_hook(Some(
            move |_action: Action, database: &str, table: &str, rowid: i64| {
                if database != "main" {
                    return;
                }
                // Every committed main-database write advances the local
                // generation so it can be compared with PRAGMA data_version.
                // Ordinary library/AI writes take this atomic fast path only;
                // the contended transaction mutex is reserved for the two
                // tables that affect the compact filename projection.
                update_main_write_pending.store(true, AtomicOrdering::Release);
                match table {
                    "global_entries" => {
                        update_pending
                            .entries_changed
                            .store(true, AtomicOrdering::Release);
                        if update_tracker.capture_rowids.load(AtomicOrdering::Acquire) {
                            let slot = update_pending
                                .rowid_count
                                .fetch_add(1, AtomicOrdering::AcqRel);
                            if slot < MAX_ROWIDS_PER_TRANSACTION {
                                update_pending.rowid_slots[slot]
                                    .store(rowid, AtomicOrdering::Release);
                            } else {
                                update_pending
                                    .entry_rowids_complete
                                    .store(false, AtomicOrdering::Release);
                            }
                        } else {
                            update_pending
                                .entry_rowids_complete
                                .store(false, AtomicOrdering::Release);
                        }
                    }
                    "global_volumes" => {
                        update_pending
                            .source_state_changed
                            .store(true, AtomicOrdering::Release);
                    }
                    _ => {}
                }
            },
        ))?;

        let commit_pending = Arc::clone(&pending);
        let commit_main_write_pending = Arc::clone(&main_write_pending);
        let commit_tracker = Arc::clone(&self.tracker);
        connection.commit_hook(Some(move || {
            if !commit_main_write_pending.swap(false, AtomicOrdering::AcqRel) {
                return false;
            }
            let changes = commit_pending.take();
            commit_tracker.record(changes);
            false
        }))?;

        let rollback_pending = pending;
        let rollback_main_write_pending = main_write_pending;
        connection.rollback_hook(Some(move || {
            rollback_main_write_pending.store(false, AtomicOrdering::Release);
            rollback_pending.reset();
        }))?;
        Ok(())
    }

    /// Executes the caller's search and snapshot-facts loader using one
    /// dedicated connection. `None` means that the caller must use the
    /// existing pooled SQLite path.
    pub(crate) fn try_with_snapshot<T>(
        self: &Arc<Self>,
        operation: impl FnOnce(
            &Transaction<'_>,
            &FilenameSearchIndex,
            &ChangeSet,
        ) -> Result<Option<T>, DbError>,
    ) -> Result<Option<T>, DbError> {
        #[cfg(test)]
        let manager_started = Instant::now();
        if !self.is_enabled() {
            return Ok(None);
        }

        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                self.counters
                    .fallback_busy
                    .fetch_add(1, AtomicOrdering::Relaxed);
                return Ok(None);
            }
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };

        if state.connection.is_none() {
            match open_reader(&self.path) {
                Ok(connection) => state.connection = Some(Arc::new(Mutex::new(connection))),
                Err(error) => {
                    self.record_error(format!("reader_open:{error}"));
                    self.counters
                        .fallback_not_ready
                        .fetch_add(1, AtomicOrdering::Relaxed);
                    drop(state);
                    self.request_rebuild();
                    return Ok(None);
                }
            }
        }

        if state.index.is_none() {
            self.counters
                .fallback_not_ready
                .fetch_add(1, AtomicOrdering::Relaxed);
            drop(state);
            self.request_rebuild();
            return Ok(None);
        }

        let index = Arc::clone(state.index.as_ref().expect("index checked above"));
        let shared_connection = Arc::clone(state.connection.as_ref().expect("reader opened above"));
        drop(state);

        let mut connection = match shared_connection.try_lock() {
            Ok(connection) => connection,
            Err(TryLockError::WouldBlock) => {
                self.counters
                    .fallback_busy
                    .fetch_add(1, AtomicOrdering::Relaxed);
                return Ok(None);
            }
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let transaction_result = connection.transaction();
        let transaction = match transaction_result {
            Ok(transaction) => transaction,
            Err(error) => {
                self.record_error(format!("reader_begin:{error}"));
                self.invalidate_if_current(&index);
                self.request_rebuild();
                return Ok(None);
            }
        };

        let version_before = match data_version(&transaction) {
            Ok(version) => version,
            Err(error) => {
                self.record_error(format!("reader_data_version:{error}"));
                drop(transaction);
                self.invalidate_if_current(&index);
                self.request_rebuild();
                return Ok(None);
            }
        };
        let generation_before = self.tracker.generation();
        if let Err(error) = pin_read_snapshot(&transaction) {
            self.record_error(format!("reader_snapshot_pin:{error}"));
            drop(transaction);
            self.invalidate_if_current(&index);
            self.request_rebuild();
            return Ok(None);
        }
        let version_after = match data_version(&transaction) {
            Ok(version) => version,
            Err(error) => {
                self.record_error(format!("reader_data_version:{error}"));
                drop(transaction);
                self.invalidate_if_current(&index);
                self.request_rebuild();
                return Ok(None);
            }
        };
        let generation_after = self.tracker.generation();
        if version_before != version_after || generation_before != generation_after {
            drop(transaction);
            self.invalidate_if_current(&index);
            self.counters
                .fallback_untracked_change
                .fetch_add(1, AtomicOrdering::Relaxed);
            self.request_rebuild();
            return Ok(None);
        }

        let changes = match self.tracker.changes_since(index.base_generation) {
            Ok(changes) => changes,
            Err(ChangeGap::OverlayLimit) => {
                drop(transaction);
                self.invalidate_if_current(&index);
                self.counters
                    .fallback_overlay_limit
                    .fetch_add(1, AtomicOrdering::Relaxed);
                self.request_rebuild();
                return Ok(None);
            }
            Err(ChangeGap::Incomplete | ChangeGap::Missing) => {
                drop(transaction);
                self.invalidate_if_current(&index);
                self.counters
                    .fallback_untracked_change
                    .fetch_add(1, AtomicOrdering::Relaxed);
                self.request_rebuild();
                return Ok(None);
            }
        };

        let local_commit_delta = changes.generation.saturating_sub(index.base_generation);
        let data_version_delta = version_after.checked_sub(index.base_data_version);
        if data_version_delta != i64::try_from(local_commit_delta).ok() {
            drop(transaction);
            self.invalidate_if_current(&index);
            self.counters
                .fallback_untracked_change
                .fetch_add(1, AtomicOrdering::Relaxed);
            self.request_rebuild();
            return Ok(None);
        }

        if changes.source_state_changed {
            #[cfg(test)]
            let source_check_started = Instant::now();
            match index.active_sources_match(&transaction) {
                Ok(true) => {
                    #[cfg(test)]
                    record_candidate_search_stage(
                        "source_state_revalidation",
                        source_check_started,
                    );
                }
                Ok(false) => {
                    #[cfg(test)]
                    record_candidate_search_stage(
                        "source_state_revalidation",
                        source_check_started,
                    );
                    drop(transaction);
                    self.invalidate_if_current(&index);
                    self.counters
                        .fallback_source_change
                        .fetch_add(1, AtomicOrdering::Relaxed);
                    self.request_rebuild();
                    return Ok(None);
                }
                Err(error) => {
                    #[cfg(test)]
                    record_candidate_search_stage(
                        "source_state_revalidation",
                        source_check_started,
                    );
                    self.record_error(format!("active_sources_check:{error}"));
                    drop(transaction);
                    self.invalidate_if_current(&index);
                    self.counters
                        .fallback_source_change
                        .fetch_add(1, AtomicOrdering::Relaxed);
                    self.request_rebuild();
                    return Ok(None);
                }
            }
        }

        #[cfg(test)]
        record_candidate_search_stage("manager_snapshot_validation", manager_started);
        match operation(&transaction, &index, &changes) {
            Ok(Some(value)) => {
                #[cfg(test)]
                let commit_started = Instant::now();
                if let Err(error) = transaction.commit() {
                    self.record_error(format!("reader_commit:{error}"));
                    self.invalidate_if_current(&index);
                    self.request_rebuild();
                    return Ok(None);
                }
                #[cfg(test)]
                record_candidate_search_stage("reader_transaction_commit", commit_started);
                self.counters
                    .accepted_queries
                    .fetch_add(1, AtomicOrdering::Relaxed);
                self.counters
                    .last_overlay_rows
                    .store(changes.changed_rowids.len() as u64, AtomicOrdering::Relaxed);
                Ok(Some(value))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                self.record_error(format!("candidate_query:{error}"));
                drop(transaction);
                self.invalidate_if_current(&index);
                self.request_rebuild();
                Ok(None)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn rebuild_now_for_test(&self) -> rusqlite::Result<()> {
        self.rebuild_once()
    }

    #[cfg(test)]
    pub(crate) fn enable_for_test(&self) {
        self.enabled.store(true, AtomicOrdering::Release);
    }

    #[cfg(test)]
    pub(crate) fn set_enabled_for_test(&self, enabled: bool) {
        self.enabled.store(enabled, AtomicOrdering::Release);
    }

    #[cfg(test)]
    pub(crate) fn diagnostics(&self) -> FilenameSearchDiagnostics {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = state.index.as_deref();
        FilenameSearchDiagnostics {
            enabled: self.is_enabled(),
            ready: index.is_some(),
            rebuild_in_progress: self.rebuild_in_progress.load(AtomicOrdering::Acquire),
            rebuilds: self.counters.rebuilds.load(AtomicOrdering::Relaxed),
            accepted_queries: self.counters.accepted_queries.load(AtomicOrdering::Relaxed),
            fallback_not_ready: self
                .counters
                .fallback_not_ready
                .load(AtomicOrdering::Relaxed),
            fallback_busy: self.counters.fallback_busy.load(AtomicOrdering::Relaxed),
            fallback_untracked_change: self
                .counters
                .fallback_untracked_change
                .load(AtomicOrdering::Relaxed),
            fallback_source_change: self
                .counters
                .fallback_source_change
                .load(AtomicOrdering::Relaxed),
            fallback_overlay_limit: self
                .counters
                .fallback_overlay_limit
                .load(AtomicOrdering::Relaxed),
            overlay_rows: self
                .counters
                .last_overlay_rows
                .load(AtomicOrdering::Relaxed) as usize,
            index_entries: index.map_or(0, |value| value.entry_count()),
            distinct_filename_count: index
                .map_or(0, |value| value.names.offsets.len().saturating_sub(1)),
            index_private_bytes: index.map_or(0, |value| value.estimated_bytes),
            index_build_peak_bytes: index.map_or(0, |value| value.peak_build_bytes),
            last_build_micros: self
                .counters
                .last_build_micros
                .load(AtomicOrdering::Relaxed),
            last_error: self
                .counters
                .last_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn build_memory_profile_for_test(&self) -> Option<FilenameIndexBuildMemoryProfile> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .index
            .as_deref()
            .map(|index| index.build_memory_profile.clone())
    }

    #[cfg(test)]
    pub(crate) fn with_reader_for_test<T>(
        &self,
        operation: impl FnOnce(&Connection) -> T,
    ) -> Option<T> {
        let shared_connection = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(state.connection.as_ref()?)
        };
        let connection = shared_connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(operation(&connection))
    }

    fn invalidate_if_current(&self, index: &Arc<FilenameSearchIndex>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .index
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, index))
        {
            state.index = None;
            self.tracker.disable_row_capture();
        }
    }

    fn request_rebuild(self: &Arc<Self>) {
        if !self.is_enabled() {
            return;
        }
        if self
            .rebuild_in_progress
            .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
            .is_err()
        {
            return;
        }

        let manager = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("global-search-filename-index".into())
            .spawn(move || {
                let result = manager.rebuild_once();
                if let Err(error) = result {
                    manager.record_error(format!("rebuild:{error}"));
                    let mut state = manager
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.retry_after = Some(Instant::now() + REBUILD_RETRY_DELAY);
                }
                manager
                    .rebuild_in_progress
                    .store(false, AtomicOrdering::Release);
            });
        if let Err(error) = spawned {
            self.rebuild_in_progress
                .store(false, AtomicOrdering::Release);
            self.record_error(format!("rebuild_spawn:{error}"));
        }
    }

    fn rebuild_once(&self) -> rusqlite::Result<()> {
        let started = Instant::now();
        let shared_connection = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state
                .retry_after
                .is_some_and(|retry_after| Instant::now() < retry_after)
            {
                return Ok(());
            }
            if state.connection.is_none() {
                state.connection = Some(Arc::new(Mutex::new(open_reader(&self.path)?)));
            }
            Arc::clone(state.connection.as_ref().expect("reader opened above"))
        };
        let mut connection = shared_connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction()?;

        let data_version_before = data_version(&transaction)?;
        let generation_before = self.tracker.generation();
        pin_read_snapshot(&transaction)?;
        let data_version_bound = data_version(&transaction)?;
        let generation_bound = self.tracker.generation();
        if data_version_before != data_version_bound || generation_before != generation_bound {
            drop(transaction);
            self.tracker.disable_row_capture();
            return Err(rusqlite::Error::InvalidQuery);
        }

        let index =
            match FilenameSearchIndex::build(&transaction, data_version_bound, generation_bound) {
                Ok(index) => Arc::new(index),
                Err(error) => {
                    drop(transaction);
                    self.tracker.disable_row_capture();
                    return Err(error);
                }
            };
        let data_version_after = data_version(&transaction)?;
        let generation_after = self.tracker.generation();
        if data_version_bound != data_version_after || generation_bound != generation_after {
            drop(transaction);
            self.tracker.disable_row_capture();
            return Err(rusqlite::Error::InvalidQuery);
        }
        transaction.commit()?;
        if !self.tracker.publish_baseline(generation_bound) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        // The build used the bounded page cache above. Search can now use
        // SQLite's mmap for selective FTS postings without faulting all the
        // scanned base-table pages into the process working set.
        connection.pragma_update(None, "mmap_size", 3_000_000_000_i64)?;

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry_count = index.rowids.len();
        let index_bytes = index.estimated_bytes;
        state.index = Some(index);
        state.retry_after = None;
        self.counters.rebuilds.fetch_add(1, AtomicOrdering::Relaxed);
        let build_micros = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        self.counters
            .last_build_micros
            .store(build_micros, AtomicOrdering::Relaxed);
        *self
            .counters
            .last_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        drop(state);
        log::info!(
            target: "global_index::filename_search",
            "compact filename candidate index is ready entries={} index_bytes={} build_micros={}",
            entry_count,
            index_bytes,
            build_micros
        );
        Ok(())
    }

    fn record_error(&self, error: String) {
        let category = error.split(':').next().unwrap_or("candidate_error");
        log::warn!(
            target: "global_index::filename_search",
            "filename candidate index failed; SQLite fallback remains active category={}",
            category
        );
        *self
            .counters
            .last_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
    }
}

fn candidate_enabled_from_environment() -> bool {
    #[cfg(feature = "global-search-filename-relevance-candidate")]
    {
        std::env::var("ZC_GLOBAL_SEARCH_FILENAME_RELEVANCE_CANDIDATE")
            .is_ok_and(|value| value == "1")
    }
    #[cfg(not(feature = "global-search-filename-relevance-candidate"))]
    {
        false
    }
}

fn open_reader(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "query_only", "ON")?;
    // This reader scans the filename projection once, so cap its page cache
    // at 4 MiB instead of retaining the larger default cache. The main
    // application pool retains its normal SQLite mmap configuration.
    connection.pragma_update(None, "mmap_size", 0_i64)?;
    connection.pragma_update(None, "cache_size", -FILENAME_READER_CACHE_LIMIT_KIB)?;
    Ok(connection)
}

fn data_version(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row("PRAGMA data_version", [], |row| row.get(0))
}

fn pin_read_snapshot(connection: &Connection) -> rusqlite::Result<()> {
    let _: Option<i64> = connection
        .query_row("SELECT rowid FROM global_entries LIMIT 1", [], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(())
}

struct PendingChanges {
    entries_changed: bool,
    entry_rowids_complete: bool,
    source_state_changed: bool,
    entry_rowids: Vec<i64>,
}

struct PendingHookState {
    entries_changed: AtomicBool,
    entry_rowids_complete: AtomicBool,
    source_state_changed: AtomicBool,
    rowid_count: AtomicUsize,
    rowid_slots: Box<[AtomicI64]>,
    entry_rowids: Mutex<Vec<i64>>,
}

impl Default for PendingHookState {
    fn default() -> Self {
        Self {
            entries_changed: AtomicBool::new(false),
            entry_rowids_complete: AtomicBool::new(true),
            source_state_changed: AtomicBool::new(false),
            rowid_count: AtomicUsize::new(0),
            rowid_slots: (0..MAX_ROWIDS_PER_TRANSACTION)
                .map(|_| AtomicI64::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            entry_rowids: Mutex::new(Vec::new()),
        }
    }
}

impl PendingHookState {
    fn take(&self) -> PendingChanges {
        let rowid_count = self.rowid_count.swap(0, AtomicOrdering::AcqRel);
        let mut rowids = self
            .entry_rowids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        rowids.clear();
        let entry_rowids_complete = self
            .entry_rowids_complete
            .swap(true, AtomicOrdering::AcqRel)
            && rowid_count <= MAX_ROWIDS_PER_TRANSACTION;
        if entry_rowids_complete {
            rowids.extend(
                self.rowid_slots[..rowid_count]
                    .iter()
                    .map(|slot| slot.load(AtomicOrdering::Acquire)),
            );
        }
        PendingChanges {
            entries_changed: self.entries_changed.swap(false, AtomicOrdering::AcqRel),
            entry_rowids_complete,
            source_state_changed: self
                .source_state_changed
                .swap(false, AtomicOrdering::AcqRel),
            entry_rowids: std::mem::take(&mut *rowids),
        }
    }

    fn reset(&self) {
        self.entries_changed.store(false, AtomicOrdering::Release);
        self.entry_rowids_complete
            .store(true, AtomicOrdering::Release);
        self.source_state_changed
            .store(false, AtomicOrdering::Release);
        self.rowid_count.store(0, AtomicOrdering::Release);
        self.entry_rowids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

impl Default for PendingChanges {
    fn default() -> Self {
        Self {
            entries_changed: false,
            entry_rowids_complete: true,
            source_state_changed: false,
            entry_rowids: Vec::new(),
        }
    }
}

struct CommitChange {
    generation: u64,
    entries_changed: bool,
    entry_rowids_complete: bool,
    source_state_changed: bool,
    entry_rowids: Vec<i64>,
}

#[derive(Default)]
struct MutationLog {
    generation: u64,
    floor_generation: u64,
    rowid_count: usize,
    events: VecDeque<CommitChange>,
}

#[derive(Default)]
pub(crate) struct MutationTracker {
    state: Mutex<MutationLog>,
    capture_rowids: AtomicBool,
}

#[derive(Debug)]
enum ChangeGap {
    Missing,
    Incomplete,
    OverlayLimit,
}

pub(crate) struct ChangeSet {
    pub(crate) generation: u64,
    pub(crate) changed_rowids: Vec<i64>,
    pub(crate) source_state_changed: bool,
}

impl MutationTracker {
    fn record(&self, changes: PendingChanges) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.generation = state.generation.saturating_add(1);
        let generation = state.generation;
        let entry_rowids_complete =
            changes.entry_rowids_complete && changes.entry_rowids.len() <= MAX_OVERLAY_ROWIDS;
        let row_count = changes.entry_rowids.len();

        if state.events.len() >= MAX_OVERLAY_BATCHES
            || state.rowid_count.saturating_add(row_count) > MAX_OVERLAY_ROWIDS
        {
            state.events.clear();
            state.rowid_count = 0;
            state.floor_generation = generation;
        }

        let entry_rowids = if entry_rowids_complete {
            changes.entry_rowids
        } else {
            Vec::new()
        };
        state.rowid_count = state.rowid_count.saturating_add(entry_rowids.len());
        let event_rows_complete = entry_rowids_complete
            && (state.floor_generation != generation || !changes.entries_changed);
        state.events.push_back(CommitChange {
            generation,
            entries_changed: changes.entries_changed,
            entry_rowids_complete: event_rows_complete,
            source_state_changed: changes.source_state_changed,
            entry_rowids,
        });
    }

    fn generation(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation
    }

    fn disable_row_capture(&self) {
        self.capture_rowids.store(false, AtomicOrdering::Release);
    }

    fn publish_baseline(&self, generation: u64) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.generation != generation {
            return false;
        }
        state.events.clear();
        state.rowid_count = 0;
        state.floor_generation = generation;
        self.capture_rowids.store(true, AtomicOrdering::Release);
        true
    }

    fn changes_since(&self, base_generation: u64) -> Result<ChangeSet, ChangeGap> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if base_generation < state.floor_generation || base_generation > state.generation {
            return Err(ChangeGap::Missing);
        }

        let mut next_generation = base_generation.saturating_add(1);
        let mut changed_rowids = Vec::new();
        let mut seen = HashSet::new();
        let mut source_state_changed = false;
        for event in state
            .events
            .iter()
            .filter(|event| event.generation > base_generation)
        {
            if event.generation != next_generation {
                return Err(ChangeGap::Missing);
            }
            if event.source_state_changed {
                source_state_changed = true;
            }
            if event.entries_changed && !event.entry_rowids_complete {
                return Err(ChangeGap::Incomplete);
            }
            for &rowid in &event.entry_rowids {
                if seen.insert(rowid) {
                    changed_rowids.push(rowid);
                    if changed_rowids.len() > MAX_OVERLAY_ROWIDS {
                        return Err(ChangeGap::OverlayLimit);
                    }
                }
            }
            next_generation = next_generation.saturating_add(1);
        }
        if next_generation != state.generation.saturating_add(1) {
            return Err(ChangeGap::Missing);
        }
        Ok(ChangeSet {
            generation: state.generation,
            changed_rowids,
            source_state_changed,
        })
    }
}

#[derive(Default)]
struct PackedStrings {
    bytes: Vec<u8>,
    offsets: Vec<u32>,
}

impl PackedStrings {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            offsets: vec![0],
        }
    }

    fn push(&mut self, value: &str) -> rusqlite::Result<u32> {
        let id =
            u32::try_from(self.offsets.len() - 1).map_err(|_| rusqlite::Error::InvalidQuery)?;
        self.bytes.extend_from_slice(value.as_bytes());
        let end = u32::try_from(self.bytes.len()).map_err(|_| rusqlite::Error::InvalidQuery)?;
        self.offsets.push(end);
        Ok(id)
    }

    fn get(&self, id: usize) -> &str {
        let start = self.offsets[id] as usize;
        let end = self.offsets[id + 1] as usize;
        // Every slice is copied from a rusqlite String during construction.
        std::str::from_utf8(&self.bytes[start..end]).expect("packed SQLite text is UTF-8")
    }

    fn allocated_bytes(&self) -> usize {
        self.bytes.capacity() + self.offsets.capacity() * std::mem::size_of::<u32>()
    }
}

pub(crate) struct FilenameSearchIndex {
    names: PackedStrings,
    extensions: PackedStrings,
    rowids: Vec<i64>,
    modified_at: Vec<i64>,
    modified_at_present: Vec<u8>,
    stable_ids: PackedStrings,
    name_ids: Vec<u32>,
    extension_ids: Vec<u32>,
    rowid_order: Vec<u32>,
    active_volume_ids: Vec<String>,
    base_data_version: i64,
    base_generation: u64,
    estimated_bytes: usize,
    peak_build_bytes: usize,
    #[cfg(test)]
    build_memory_profile: FilenameIndexBuildMemoryProfile,
}

impl FilenameSearchIndex {
    fn build(
        connection: &Connection,
        base_data_version: i64,
        base_generation: u64,
    ) -> rusqlite::Result<Self> {
        #[cfg(test)]
        let build_started = Instant::now();
        let active_volume_ids = {
            let mut statement = connection.prepare(
                "SELECT id FROM global_volumes
                 WHERE enabled=1 AND coverage_pending=0
                 ORDER BY id COLLATE BINARY",
            )?;
            let active_volume_ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            active_volume_ids
        };
        let mut index = Self {
            names: PackedStrings::new(),
            extensions: PackedStrings::new(),
            rowids: Vec::new(),
            modified_at: Vec::new(),
            modified_at_present: Vec::new(),
            stable_ids: PackedStrings::new(),
            name_ids: Vec::new(),
            extension_ids: Vec::new(),
            rowid_order: Vec::new(),
            active_volume_ids,
            base_data_version,
            base_generation,
            estimated_bytes: 0,
            peak_build_bytes: 0,
            #[cfg(test)]
            build_memory_profile: FilenameIndexBuildMemoryProfile::default(),
        };
        #[cfg(test)]
        let mut build_profile = FilenameIndexBuildMemoryProfile::default();
        let mut current_name: Option<String> = None;
        let mut current_name_id = 0_u32;
        // The dictionary is build-only. Its key allocations are included in
        // peak accounting and released before the immutable index is published.
        let mut extension_ids = HashMap::<String, u32>::new();
        let mut extension_key_capacity_bytes = 0_usize;
        #[cfg(test)]
        let row_scan_started = Instant::now();
        {
            let mut statement = connection.prepare(FILENAME_INDEX_BUILD_SQL)?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                if index.rowids.len() >= MAX_INDEX_ENTRIES {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let rowid = row.get::<_, i64>(0)?;
                let stable_id = row.get::<_, String>(1)?;
                let name = row.get::<_, String>(2)?;
                let extension = row.get::<_, String>(3)?;
                let modified_at = row.get::<_, Option<i64>>(4)?;

                #[cfg(test)]
                {
                    let row_text_capacity = stable_id
                        .capacity()
                        .saturating_add(name.capacity())
                        .saturating_add(extension.capacity());
                    build_profile.row_text_string_allocations =
                        build_profile.row_text_string_allocations.saturating_add(3);
                    build_profile.row_text_capacity_bytes = build_profile
                        .row_text_capacity_bytes
                        .saturating_add(row_text_capacity as u64);
                    build_profile.max_single_row_text_capacity_bytes = build_profile
                        .max_single_row_text_capacity_bytes
                        .max(row_text_capacity);
                }

                #[cfg(test)]
                let vector_capacities_before = index_vector_capacities(&index);

                if current_name.as_deref() != Some(name.as_str()) {
                    current_name_id = index.names.push(&name)?;
                    #[cfg(test)]
                    {
                        build_profile.distinct_name_string_capacity_bytes_total = build_profile
                            .distinct_name_string_capacity_bytes_total
                            .saturating_add(name.capacity() as u64);
                    }
                    current_name = Some(name);
                }
                let extension_id = if let Some(&extension_id) = extension_ids.get(&extension) {
                    extension_id
                } else {
                    #[cfg(test)]
                    let dictionary_table_bytes_before =
                        extension_dictionary_table_bytes(&extension_ids);
                    #[cfg(test)]
                    let dictionary_capacity_before = extension_ids.capacity();
                    let extension_id = index.extensions.push(&extension)?;
                    extension_key_capacity_bytes =
                        extension_key_capacity_bytes.saturating_add(extension.capacity());
                    #[cfg(test)]
                    {
                        build_profile.retained_extension_key_capacity_bytes = build_profile
                            .retained_extension_key_capacity_bytes
                            .saturating_add(extension.capacity() as u64);
                    }
                    extension_ids.insert(extension, extension_id);
                    #[cfg(test)]
                    {
                        let dictionary_table_bytes_after =
                            extension_dictionary_table_bytes(&extension_ids);
                        if extension_ids.capacity() != dictionary_capacity_before {
                            build_profile.extension_dictionary_growth_events = build_profile
                                .extension_dictionary_growth_events
                                .saturating_add(1);
                            build_profile.extension_dictionary_growth_bytes = build_profile
                                .extension_dictionary_growth_bytes
                                .saturating_add(
                                    dictionary_table_bytes_after
                                        .saturating_sub(dictionary_table_bytes_before)
                                        as u64,
                                );
                            build_profile
                                .extension_dictionary_reallocation_conservative_peak_bytes =
                                build_profile
                                    .extension_dictionary_reallocation_conservative_peak_bytes
                                    .max(
                                        dictionary_table_bytes_after
                                            .saturating_add(dictionary_table_bytes_before)
                                            .saturating_add(extension_key_capacity_bytes),
                                    );
                        }
                    }
                    extension_id
                };
                index.stable_ids.push(&stable_id)?;
                index.name_ids.push(current_name_id);
                index.extension_ids.push(extension_id);
                index.rowids.push(rowid);
                index.modified_at.push(modified_at.unwrap_or_default());
                index
                    .modified_at_present
                    .push(u8::from(modified_at.is_some()));

                #[cfg(test)]
                {
                    let vector_capacities_after = index_vector_capacities(&index);
                    let mut old_reallocated_bytes = 0_usize;
                    for (before, after) in vector_capacities_before
                        .into_iter()
                        .zip(vector_capacities_after)
                    {
                        if after > before {
                            build_profile.vector_capacity_growth_buffers = build_profile
                                .vector_capacity_growth_buffers
                                .saturating_add(1);
                            build_profile.vector_capacity_growth_bytes = build_profile
                                .vector_capacity_growth_bytes
                                .saturating_add(after.saturating_sub(before) as u64);
                            old_reallocated_bytes = old_reallocated_bytes.saturating_add(before);
                        }
                    }
                    if old_reallocated_bytes > 0 {
                        build_profile.vector_reallocation_conservative_peak_bytes = build_profile
                            .vector_reallocation_conservative_peak_bytes
                            .max(
                                index
                                    .allocated_bytes()
                                    .saturating_add(old_reallocated_bytes),
                            );
                    }
                }

                if index.rowids.len().is_multiple_of(16_384) {
                    let dictionary_bytes = extension_dictionary_build_bytes(
                        &extension_ids,
                        extension_key_capacity_bytes,
                    );
                    if dictionary_bytes > MAX_EXTENSION_DICTIONARY_BUILD_BYTES {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                    let estimated_peak = index.allocated_bytes().saturating_add(dictionary_bytes);
                    index.peak_build_bytes = index.peak_build_bytes.max(estimated_peak);
                    if estimated_peak > MAX_INDEX_PRIVATE_BYTES {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                }
            }
            drop(rows);
            #[cfg(test)]
            {
                build_profile.row_scan_micros =
                    row_scan_started.elapsed().as_micros().min(u64::MAX as u128) as u64;
                record_filename_index_build_phase(
                    &mut build_profile,
                    &index,
                    "row_scan_complete_extension_dictionary_live",
                    connection,
                    Some((&extension_ids, extension_key_capacity_bytes)),
                    Some(statement.get_status(StatementStatus::MemUsed).max(0) as usize),
                    build_started,
                );
            }
        }
        #[cfg(test)]
        record_filename_index_build_phase(
            &mut build_profile,
            &index,
            "sqlite_build_statement_dropped_extension_dictionary_live",
            connection,
            Some((&extension_ids, extension_key_capacity_bytes)),
            None,
            build_started,
        );
        let dictionary_bytes =
            extension_dictionary_build_bytes(&extension_ids, extension_key_capacity_bytes);
        if dictionary_bytes > MAX_EXTENSION_DICTIONARY_BUILD_BYTES {
            return Err(rusqlite::Error::InvalidQuery);
        }
        index.peak_build_bytes = index
            .peak_build_bytes
            .max(index.allocated_bytes().saturating_add(dictionary_bytes));
        drop(extension_ids);
        #[cfg(test)]
        {
            build_profile.current_name_live_capacity_bytes =
                current_name.as_ref().map_or(0, |name| name.capacity()) as u64;
            build_profile.temporary_row_text_capacity_bytes_total = build_profile
                .row_text_capacity_bytes
                .saturating_sub(build_profile.current_name_live_capacity_bytes)
                .saturating_sub(build_profile.retained_extension_key_capacity_bytes);
            record_filename_index_build_phase(
                &mut build_profile,
                &index,
                "extension_dictionary_dropped_before_rowid_order",
                connection,
                None,
                None,
                build_started,
            );
        }
        #[cfg(test)]
        let rowid_order_materialize_started = Instant::now();
        index.rowid_order = (0..index.rowids.len())
            .map(|entry_index| {
                u32::try_from(entry_index).map_err(|_| rusqlite::Error::InvalidQuery)
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        #[cfg(test)]
        {
            let rowid_order_capacity_bytes = index
                .rowid_order
                .capacity()
                .saturating_mul(std::mem::size_of::<u32>());
            if rowid_order_capacity_bytes > 0 {
                build_profile.vector_capacity_growth_buffers = build_profile
                    .vector_capacity_growth_buffers
                    .saturating_add(1);
                build_profile.vector_capacity_growth_bytes = build_profile
                    .vector_capacity_growth_bytes
                    .saturating_add(rowid_order_capacity_bytes as u64);
                build_profile.vector_reallocation_conservative_peak_bytes = build_profile
                    .vector_reallocation_conservative_peak_bytes
                    .max(index.allocated_bytes());
            }
            build_profile.rowid_order_materialize_micros = rowid_order_materialize_started
                .elapsed()
                .as_micros()
                .min(u64::MAX as u128)
                as u64;
            record_filename_index_build_phase(
                &mut build_profile,
                &index,
                "rowid_order_materialized_before_sort",
                connection,
                None,
                None,
                build_started,
            );
        }
        #[cfg(test)]
        let rowid_order_sort_started = Instant::now();
        index
            .rowid_order
            .sort_unstable_by_key(|entry_index| index.rowids[*entry_index as usize]);
        #[cfg(test)]
        {
            build_profile.rowid_order_sort_micros = rowid_order_sort_started
                .elapsed()
                .as_micros()
                .min(u64::MAX as u128) as u64;
            record_filename_index_build_phase(
                &mut build_profile,
                &index,
                "rowid_order_sorted",
                connection,
                None,
                None,
                build_started,
            );
        }
        index.estimated_bytes = index.allocated_bytes();
        let estimated_peak = index.allocated_bytes();
        index.peak_build_bytes = index.peak_build_bytes.max(estimated_peak);
        if estimated_peak > MAX_INDEX_PRIVATE_BYTES {
            return Err(rusqlite::Error::InvalidQuery);
        }
        #[cfg(test)]
        {
            build_profile.total_build_micros =
                build_started.elapsed().as_micros().min(u64::MAX as u128) as u64;
            index.build_memory_profile = build_profile;
        }
        Ok(index)
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.rowids.len()
    }

    fn active_sources_match(&self, connection: &Connection) -> rusqlite::Result<bool> {
        let mut statement = connection.prepare(
            "SELECT id FROM global_volumes
             WHERE enabled=1 AND coverage_pending=0
             ORDER BY id COLLATE BINARY",
        )?;
        let active_volume_ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(active_volume_ids == self.active_volume_ids)
    }

    pub(crate) fn overlay_rows(
        &self,
        connection: &Connection,
        changed_rowids: &[i64],
    ) -> rusqlite::Result<Vec<OverlayRow>> {
        if changed_rowids.is_empty() {
            return Ok(Vec::new());
        }
        let encoded =
            serde_json::to_string(changed_rowids).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut statement = connection.prepare(
            "SELECT ge.rowid, ge.id, ge.name_normalized, lower(ge.extension),
                    ge.modified_at_fs
             FROM json_each(?1) changed
             JOIN global_entries ge ON ge.rowid=CAST(changed.value AS INTEGER)
             JOIN global_volumes gv ON gv.id=ge.volume_id
             WHERE gv.enabled=1 AND gv.coverage_pending=0 AND ge.is_stale=0",
        )?;
        let rows = statement
            .query_map([encoded], |row| {
                Ok(OverlayRow {
                    rowid: row.get(0)?,
                    id: row.get(1)?,
                    name: row.get(2)?,
                    extension: row.get(3)?,
                    modified_at: row.get(4)?,
                })
            })?
            .collect();
        rows
    }

    pub(crate) fn rank_filename_candidates<'a>(
        &'a self,
        query: &str,
        trigram_candidates: &[FilenameTrigramCandidate],
        seen: &HashSet<String>,
        changed_rowids: &HashSet<i64>,
        overlay_rows: &'a [OverlayRow],
        capacity: usize,
    ) -> Vec<RankedFileCandidate<'a>> {
        if capacity == 0 {
            return Vec::new();
        }
        let mut heap = BinaryHeap::with_capacity(capacity.min(4096));
        // A replacement may leave the old rowid in the immutable base while
        // the current row is represented by the overlay. Prefer the current
        // SQLite-snapshot row even if the old rowid is not reusable.
        let overlay_ids = overlay_rows
            .iter()
            .map(|overlay| overlay.id.as_str())
            .collect::<HashSet<_>>();
        for trigram_candidate in trigram_candidates {
            let rowid = trigram_candidate.rowid;
            if changed_rowids.contains(&rowid) {
                continue;
            }
            let Ok(position) = self
                .rowid_order
                .binary_search_by_key(&rowid, |entry_index| self.rowids[*entry_index as usize])
            else {
                // A row inserted after the immutable base is evaluated from
                // the bounded overlay below.
                continue;
            };
            let entry_index = self.rowid_order[position] as usize;
            let id = self.stable_ids.get(entry_index).as_bytes();
            if std::str::from_utf8(id).is_ok_and(|id| seen.contains(id) || overlay_ids.contains(id))
            {
                continue;
            }
            let name = self.names.get(self.name_ids[entry_index] as usize);
            let name_class = filename_match_class(name, query);
            let extension_match = trigram_candidate.extension_match.unwrap_or_else(|| {
                self.extensions
                    .get(self.extension_ids[entry_index] as usize)
                    .contains(query)
            });
            let Some(class) = name_class.or_else(|| extension_match.then_some(SUBSTRING_CLASS))
            else {
                // FTS also indexes paths. Literal path hits are handled by the
                // path-only selector after filename candidates are ranked.
                continue;
            };
            push_ranked_candidate(
                &mut heap,
                capacity,
                RankedFileCandidate {
                    rowid,
                    class,
                    modified_at: (self.modified_at_present[entry_index] != 0)
                        .then_some(self.modified_at[entry_index]),
                    stable_id: Cow::Borrowed(id),
                },
            );
        }

        for overlay in overlay_rows {
            if seen.contains(&overlay.id) {
                continue;
            }
            let name_class = filename_match_class(&overlay.name, query);
            let extension_match = overlay.extension.contains(query);
            let Some(class) = name_class.or_else(|| extension_match.then_some(SUBSTRING_CLASS))
            else {
                continue;
            };
            push_ranked_candidate(
                &mut heap,
                capacity,
                RankedFileCandidate {
                    rowid: overlay.rowid,
                    class,
                    modified_at: overlay.modified_at,
                    stable_id: Cow::Borrowed(overlay.id.as_bytes()),
                },
            );
        }

        let mut ranked = heap.into_vec();
        ranked.sort_unstable();
        ranked
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_rank_stage_profile(
        &self,
        query: &str,
        trigram_candidates: &[FilenameTrigramCandidate],
        seen: &HashSet<String>,
        changed_rowids: &HashSet<i64>,
        overlay_rows: &[OverlayRow],
        capacity: usize,
    ) -> FilenameRankStageProfile {
        let expected = self.rank_filename_candidates(
            query,
            trigram_candidates,
            seen,
            changed_rowids,
            overlay_rows,
            capacity,
        );
        let identity_started = Instant::now();
        let overlay_ids = overlay_rows
            .iter()
            .map(|overlay| overlay.id.as_str())
            .collect::<HashSet<_>>();
        let identity_setup_ns = duration_ns(identity_started.elapsed());

        let lookup_started = Instant::now();
        let mut candidates = Vec::with_capacity(trigram_candidates.len() + overlay_rows.len());
        for trigram_candidate in trigram_candidates {
            let rowid = trigram_candidate.rowid;
            if changed_rowids.contains(&rowid) {
                continue;
            }
            let Ok(position) = self
                .rowid_order
                .binary_search_by_key(&rowid, |entry_index| self.rowids[*entry_index as usize])
            else {
                continue;
            };
            let entry_index = self.rowid_order[position] as usize;
            let id = self.stable_ids.get(entry_index).as_bytes();
            if std::str::from_utf8(id).is_ok_and(|id| seen.contains(id) || overlay_ids.contains(id))
            {
                continue;
            }
            let name = self.names.get(self.name_ids[entry_index] as usize);
            let name_class = filename_match_class(name, query);
            let extension_match = trigram_candidate.extension_match.unwrap_or_else(|| {
                self.extensions
                    .get(self.extension_ids[entry_index] as usize)
                    .contains(query)
            });
            let Some(class) = name_class.or_else(|| extension_match.then_some(SUBSTRING_CLASS))
            else {
                continue;
            };
            candidates.push(RankedFileCandidate {
                rowid,
                class,
                modified_at: (self.modified_at_present[entry_index] != 0)
                    .then_some(self.modified_at[entry_index]),
                stable_id: Cow::Borrowed(id),
            });
        }
        for overlay in overlay_rows {
            if seen.contains(&overlay.id) {
                continue;
            }
            let name_class = filename_match_class(&overlay.name, query);
            let extension_match = overlay.extension.contains(query);
            let Some(class) = name_class.or_else(|| extension_match.then_some(SUBSTRING_CLASS))
            else {
                continue;
            };
            candidates.push(RankedFileCandidate {
                rowid: overlay.rowid,
                class,
                modified_at: overlay.modified_at,
                stable_id: Cow::Borrowed(overlay.id.as_bytes()),
            });
        }
        let lookup_and_classification_ns = duration_ns(lookup_started.elapsed());
        let accepted_candidates = candidates.len();

        let top_k_started = Instant::now();
        let mut heap = BinaryHeap::with_capacity(capacity.min(4096));
        for candidate in candidates {
            push_ranked_candidate(&mut heap, capacity, candidate);
        }
        let top_k_heap_ns = duration_ns(top_k_started.elapsed());

        let sort_started = Instant::now();
        let mut ranked = heap.into_vec();
        ranked.sort_unstable();
        let heap_sort_ns = duration_ns(sort_started.elapsed());
        assert_eq!(
            expected.len(),
            ranked.len(),
            "diagnostic rank replay length must match the real interleaved rank path"
        );
        assert!(
            expected.iter().zip(&ranked).all(|(left, right)| {
                left.rowid == right.rowid
                    && left.class == right.class
                    && left.modified_at == right.modified_at
                    && left.stable_id == right.stable_id
            }),
            "diagnostic rank replay order must match the real interleaved rank path"
        );

        FilenameRankStageProfile {
            identity_setup_ns,
            lookup_and_classification_ns,
            top_k_heap_ns,
            heap_sort_ns,
            accepted_candidates,
            overlay_rows: overlay_rows.len(),
        }
    }

    fn allocated_bytes(&self) -> usize {
        self.names.allocated_bytes()
            + self.extensions.allocated_bytes()
            + self.rowids.capacity() * std::mem::size_of::<i64>()
            + self.modified_at.capacity() * std::mem::size_of::<i64>()
            + self.modified_at_present.capacity()
            + self.stable_ids.allocated_bytes()
            + self.name_ids.capacity() * std::mem::size_of::<u32>()
            + self.extension_ids.capacity() * std::mem::size_of::<u32>()
            + self.rowid_order.capacity() * std::mem::size_of::<u32>()
            + self
                .active_volume_ids
                .iter()
                .map(String::capacity)
                .sum::<usize>()
    }
}

#[cfg(test)]
fn index_vector_capacities(index: &FilenameSearchIndex) -> [usize; 12] {
    [
        index.names.bytes.capacity(),
        index
            .names
            .offsets
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        index.extensions.bytes.capacity(),
        index
            .extensions
            .offsets
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        index.stable_ids.bytes.capacity(),
        index
            .stable_ids
            .offsets
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        index
            .rowids
            .capacity()
            .saturating_mul(std::mem::size_of::<i64>()),
        index
            .modified_at
            .capacity()
            .saturating_mul(std::mem::size_of::<i64>()),
        index.modified_at_present.capacity(),
        index
            .name_ids
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        index
            .extension_ids
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        index
            .rowid_order
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
    ]
}

fn extension_dictionary_build_bytes(
    dictionary: &HashMap<String, u32>,
    key_capacity_bytes: usize,
) -> usize {
    extension_dictionary_table_bytes(dictionary).saturating_add(key_capacity_bytes)
}

fn extension_dictionary_table_bytes(dictionary: &HashMap<String, u32>) -> usize {
    dictionary
        .capacity()
        .saturating_mul(std::mem::size_of::<(String, u32)>() + 1)
}

#[cfg(test)]
fn sqlite_db_status_used_bytes(connection: &Connection, operation: i32) -> Option<usize> {
    let mut current = 0;
    let mut highwater = 0;
    let result = unsafe {
        rusqlite::ffi::sqlite3_db_status(
            connection.handle(),
            operation,
            &mut current,
            &mut highwater,
            0,
        )
    };
    (result == rusqlite::ffi::SQLITE_OK && current >= 0).then_some(current as usize)
}

#[cfg(test)]
fn record_filename_index_build_phase(
    profile: &mut FilenameIndexBuildMemoryProfile,
    index: &FilenameSearchIndex,
    stage: &'static str,
    connection: &Connection,
    extension_dictionary: Option<(&HashMap<String, u32>, usize)>,
    statement_memory_bytes: Option<usize>,
    build_started: Instant,
) {
    let dictionary = extension_dictionary.map(|(dictionary, key_capacity_bytes)| {
        (
            dictionary.len(),
            dictionary.capacity(),
            key_capacity_bytes,
            extension_dictionary_build_bytes(dictionary, key_capacity_bytes),
        )
    });
    profile.phases.push(FilenameIndexBuildMemoryPhase {
        stage,
        elapsed_micros: build_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        name_bytes_capacity: index.names.bytes.capacity(),
        name_offsets_capacity_bytes: index
            .names
            .offsets
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        extension_bytes_capacity: index.extensions.bytes.capacity(),
        extension_offsets_capacity_bytes: index
            .extensions
            .offsets
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        stable_id_bytes_capacity: index.stable_ids.bytes.capacity(),
        stable_id_offsets_capacity_bytes: index
            .stable_ids
            .offsets
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        rowid_capacity_bytes: index
            .rowids
            .capacity()
            .saturating_mul(std::mem::size_of::<i64>()),
        modified_at_capacity_bytes: index
            .modified_at
            .capacity()
            .saturating_mul(std::mem::size_of::<i64>()),
        modified_at_present_capacity_bytes: index.modified_at_present.capacity(),
        name_ids_capacity_bytes: index
            .name_ids
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        extension_ids_capacity_bytes: index
            .extension_ids
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        rowid_order_capacity_bytes: index
            .rowid_order
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>()),
        index_capacity_bytes: index.allocated_bytes(),
        extension_dictionary_entries: dictionary.map_or(0, |value| value.0),
        extension_dictionary_capacity: dictionary.map_or(0, |value| value.1),
        extension_dictionary_key_capacity_bytes: dictionary.map_or(0, |value| value.2),
        extension_dictionary_estimated_bytes: dictionary.map_or(0, |value| value.3),
        sqlite_cache_used_bytes: sqlite_db_status_used_bytes(
            connection,
            rusqlite::ffi::SQLITE_DBSTATUS_CACHE_USED,
        ),
        sqlite_schema_used_bytes: sqlite_db_status_used_bytes(
            connection,
            rusqlite::ffi::SQLITE_DBSTATUS_SCHEMA_USED,
        ),
        sqlite_statement_used_bytes: sqlite_db_status_used_bytes(
            connection,
            rusqlite::ffi::SQLITE_DBSTATUS_STMT_USED,
        ),
        sqlite_lookaside_used_bytes: sqlite_db_status_used_bytes(
            connection,
            rusqlite::ffi::SQLITE_DBSTATUS_LOOKASIDE_USED,
        ),
        sqlite_build_statement_memory_bytes: statement_memory_bytes,
    });
}

#[cfg(test)]
pub(crate) fn diagnostic_filename_index_build_sql() -> &'static str {
    FILENAME_INDEX_BUILD_SQL
}

pub(crate) const WHOLE_WORD_CLASS: u8 = 0;
pub(crate) const SUBSTRING_CLASS: u8 = 1;

#[derive(Clone)]
pub(crate) struct OverlayRow {
    pub rowid: i64,
    pub id: String,
    pub name: String,
    pub extension: String,
    pub modified_at: Option<i64>,
}

#[derive(Clone, Copy)]
pub(crate) struct FilenameTrigramCandidate {
    pub rowid: i64,
    /// `Some` preserves the frozen #516 SQL-join route for paired tests.
    /// Production rowid-only retrieval computes this from the compact index.
    pub extension_match: Option<bool>,
}

#[derive(Clone)]
pub(crate) struct RankedFileCandidate<'a> {
    pub rowid: i64,
    pub class: u8,
    pub modified_at: Option<i64>,
    pub stable_id: Cow<'a, [u8]>,
}

impl PartialEq for RankedFileCandidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        compare_candidates(self, other) == Ordering::Equal
    }
}
impl Eq for RankedFileCandidate<'_> {}
impl PartialOrd for RankedFileCandidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RankedFileCandidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_candidates(self, other)
    }
}

fn compare_candidates(left: &RankedFileCandidate<'_>, right: &RankedFileCandidate<'_>) -> Ordering {
    left.class
        .cmp(&right.class)
        .then_with(|| right.modified_at.cmp(&left.modified_at))
        .then_with(|| left.stable_id.cmp(&right.stable_id))
}

fn push_ranked_candidate<'a>(
    heap: &mut BinaryHeap<RankedFileCandidate<'a>>,
    capacity: usize,
    candidate: RankedFileCandidate<'a>,
) {
    if heap.len() < capacity {
        heap.push(candidate);
    } else if heap
        .peek()
        .is_some_and(|worst| compare_candidates(&candidate, worst) == Ordering::Less)
    {
        heap.pop();
        heap.push(candidate);
    }
}

fn filename_match_class(name: &str, query: &str) -> Option<u8> {
    if !name.contains(query) {
        return None;
    }
    Some(if has_ascii_bounded_occurrence(name, query) {
        WHOLE_WORD_CLASS
    } else {
        SUBSTRING_CLASS
    })
}

fn has_ascii_bounded_occurrence(name: &str, query: &str) -> bool {
    name.match_indices(query).any(|(start, matched)| {
        let before = name[..start].chars().next_back();
        let after = name[start + matched.len()..].chars().next();
        !before.is_some_and(is_ascii_word_character) && !after.is_some_and(is_ascii_word_character)
    })
}

fn is_ascii_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_occurrence_checks_later_occurrences_and_uses_ascii_word_contract() {
        assert!(has_ascii_bounded_occurrence("reportable report", "report"));
        assert!(has_ascii_bounded_occurrence(
            "x-reportable report",
            "report"
        ));
        assert!(!has_ascii_bounded_occurrence("x-reportable", "report"));
        assert!(has_ascii_bounded_occurrence("报告reporté", "report"));
        assert!(!has_ascii_bounded_occurrence("myreportable", "report"));
    }
}
