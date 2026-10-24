use super::models::*;
use super::repository::SourceOperationOwner;
pub use super::repository::{SourceOperationKind, SourceOperationRoute};
use super::wake::{GlobalIndexWaitResult, GlobalIndexWakeReason, GlobalIndexWakeSlot};
use crate::db::{Database, DbError};
use crate::file_workspace::WorkClass;
use crate::resource_governor::scope_thread_qos;
use crate::scheduler::{CancellationToken, ResourceHints, WorkRequest, WorkScheduler};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GlobalIndexError {
    #[error("database error: {0}")]
    Database(#[from] DbError),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("service request busy: {0}")]
    ServiceBusy(String),
    #[error("source operation is already owned: {0}")]
    SourceBusy(String),
    #[error("permission required: {0}")]
    PermissionRequired(String),
    #[error("invalid service protocol: {0}")]
    ServiceProtocol(String),
    #[error("service request outcome unknown: {0}")]
    ServiceOutcomeUnknown(String),
    #[error("rebuild required: {0}")]
    RebuildRequired(String),
    #[error("Windows I/O failed (Win32 error {0})")]
    WindowsIo(u32),
    #[error("indexing paused")]
    Paused,
}

pub trait GlobalIndexSink: Send {
    fn admit_source_operation(
        &mut self,
        _source: &GlobalVolume,
        _operation_id: &str,
        _kind: SourceOperationKind,
        _route: SourceOperationRoute,
    ) -> Result<(), GlobalIndexError> {
        Ok(())
    }
    fn source_operation_admitted(&self, _volume_id: &str) -> bool {
        false
    }
    fn mark_source_operation_unknown(&mut self, _volume_id: &str) -> Result<(), GlobalIndexError> {
        Ok(())
    }
    fn finish_source_operation(
        &mut self,
        volume_id: &str,
        status: &str,
        error: Option<&str>,
        _full_index_at: Option<i64>,
        _incremental_sync_at: Option<i64>,
        _complete_rebuild_intent: bool,
    ) -> Result<(), GlobalIndexError> {
        self.set_source_state(volume_id, status, error)
    }
    fn write_batch(&mut self, entries: &[GlobalEntryInput]) -> Result<usize, GlobalIndexError>;
    fn mark_entry_stale(&mut self, entry_id: &str) -> Result<(), GlobalIndexError>;
    fn checkpoint(
        &mut self,
        volume_id: &str,
        journal_id: Option<&str>,
        journal_cursor: Option<&str>,
    ) -> Result<(), GlobalIndexError>;
    fn set_source_state(
        &mut self,
        volume_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), GlobalIndexError>;
    fn set_source_provider(
        &mut self,
        volume_id: &str,
        provider: &str,
    ) -> Result<(), GlobalIndexError>;
    fn resolve_parent_path(
        &mut self,
        volume_id: &str,
        parent_platform_file_id: &str,
    ) -> Result<Option<String>, GlobalIndexError>;
    fn find_entry_by_identity(
        &mut self,
        volume_id: &str,
        platform_file_id: &str,
        parent_platform_file_id: &str,
        name: &str,
    ) -> Result<Option<GlobalEntry>, GlobalIndexError>;
    fn mark_volume_entries_stale(&mut self, volume_id: &str) -> Result<(), GlobalIndexError>;
}

pub trait GlobalIndexProvider: Send + Sync {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError>;
    fn start_initial_index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError>;
    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError>;
    fn pause(&self) -> Result<(), GlobalIndexError>;
    fn rebuild(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.start_initial_index(source, sink, cancel)
    }
    fn status(&self) -> Result<String, GlobalIndexError>;
    fn shutdown(&self) -> Result<(), GlobalIndexError>;

    /// Cheap topology-only discovery used by Windows' low-frequency source
    /// availability safety audit. It must not start indexing work.
    fn audit_source_topology(
        &self,
    ) -> Result<Option<Vec<GlobalSourceDescriptor>>, GlobalIndexError> {
        Ok(None)
    }

    fn topology_audit_interval(&self) -> Option<Duration> {
        None
    }

    /// Providers can mark pending reconciliation as heavy before it enters
    /// the shared resource admission path.
    fn requires_background_admission(&self, _source: &GlobalSourceDescriptor) -> bool {
        false
    }
    /// Windows owns admission in its IPC/direct route. Other providers are
    /// admitted by the Coordinator after its existing scheduler lease.
    fn has_external_source_operation_admission(&self) -> bool {
        false
    }

    fn source_enabled_changed(&self, _source_id: &str, _enabled: bool) {}
}

#[derive(Debug, Default)]
struct CoordinatorState {
    running: bool,
    paused: bool,
    last_error: Option<String>,
}

#[derive(Clone)]
pub struct GlobalIndexCoordinator {
    db: Database,
    provider: Arc<dyn GlobalIndexProvider>,
    state: Arc<Mutex<CoordinatorState>>,
    cancel: Arc<AtomicBool>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
    wake: Arc<GlobalIndexWakeSlot>,
}

impl GlobalIndexCoordinator {
    pub fn new(db: Database) -> Self {
        let wake = Arc::new(GlobalIndexWakeSlot::default());
        let provider = platform_provider(wake.clone(), db.path().to_path_buf());
        Self::with_provider(db, Arc::from(provider), wake)
    }

    fn with_provider(
        db: Database,
        provider: Arc<dyn GlobalIndexProvider>,
        wake: Arc<GlobalIndexWakeSlot>,
    ) -> Self {
        Self {
            db,
            provider,
            state: Arc::new(Mutex::new(CoordinatorState::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            worker: Arc::new(Mutex::new(None)),
            wake,
        }
    }

    pub fn start(&self) -> Result<(), GlobalIndexError> {
        {
            let mut state = self.state.lock().map_err(|_| {
                GlobalIndexError::Provider("coordinator state lock poisoned".to_string())
            })?;
            if state.running {
                state.paused = false;
                self.wake.notify(GlobalIndexWakeReason::ExplicitCommand);
                return Ok(());
            }
            state.running = true;
            state.paused = false;
            state.last_error = None;
        }
        self.cancel.store(false, Ordering::Release);
        self.wake.notify(GlobalIndexWakeReason::Startup);
        let db = self.db.clone();
        let provider = self.provider.clone();
        let state = self.state.clone();
        let cancel = self.cancel.clone();
        let wake = self.wake.clone();
        let worker = thread::Builder::new()
            .name("zen-canvas-global-index".to_string())
            .spawn(move || {
                let result = run_index(provider, db, cancel.clone(), wake);
                #[cfg(feature = "native-qa")]
                if let Err(error) = &result {
                    super::qa_trace::record_error("coordinator_worker", &error.to_string());
                }
                if let Ok(mut state) = state.lock() {
                    state.running = false;
                    state.paused = cancel.load(Ordering::Acquire);
                    state.last_error = result.err().map(|error| error.to_string());
                }
            })
            .map_err(|error| {
                GlobalIndexError::Provider(format!("failed to start index worker: {error}"))
            })?;
        if let Ok(mut slot) = self.worker.lock() {
            if let Some(previous) = slot.take() {
                let _ = previous.join();
            }
            *slot = Some(worker);
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<(), GlobalIndexError> {
        self.cancel.store(true, Ordering::Release);
        self.wake.notify(GlobalIndexWakeReason::Shutdown);
        self.provider.pause()?;
        if let Ok(mut slot) = self.worker.lock() {
            if let Some(worker) = slot.take() {
                let _ = worker.join();
            }
        }
        let mut state = self.state.lock().map_err(|_| {
            GlobalIndexError::Provider("coordinator state lock poisoned".to_string())
        })?;
        state.paused = true;
        Ok(())
    }

    pub fn resume(&self) -> Result<(), GlobalIndexError> {
        self.cancel.store(false, Ordering::Release);
        self.wake.notify(GlobalIndexWakeReason::LifecycleResume);
        self.start()
    }

    pub fn rebuild(&self, source_id: Option<String>) -> Result<(), GlobalIndexError> {
        if let Some(id) = source_id {
            if !self.db.request_global_index_rebuild(&id)? {
                return Err(GlobalIndexError::Provider(
                    "rebuild source is missing or disabled".into(),
                ));
            }
        } else {
            for volume in self.db.list_global_volumes()? {
                if volume.enabled {
                    self.db.request_global_index_rebuild(&volume.id)?;
                }
            }
        }
        self.start()
    }

    pub fn set_source_enabled(
        &self,
        source_id: &str,
        enabled: bool,
    ) -> Result<(), GlobalIndexError> {
        self.db.set_global_volume_enabled(source_id, enabled)?;
        self.provider.source_enabled_changed(source_id, enabled);
        self.wake.notify(GlobalIndexWakeReason::ExplicitCommand);
        Ok(())
    }

    pub fn status(&self) -> Result<GlobalIndexStatus, GlobalIndexError> {
        Ok(self.db.global_index_status()?)
    }

    pub fn shutdown(&self) -> Result<(), GlobalIndexError> {
        self.cancel.store(true, Ordering::Release);
        self.wake.shutdown();
        self.provider.shutdown()?;
        if let Ok(mut slot) = self.worker.lock() {
            if let Some(worker) = slot.take() {
                let _ = worker.join();
            }
        }
        if let Ok(mut state) = self.state.lock() {
            state.running = false;
            state.paused = true;
        }
        Ok(())
    }

    pub fn provider_status(&self) -> Result<String, GlobalIndexError> {
        self.provider.status()
    }

    pub fn notify_runtime_resume(&self) {
        self.wake.notify(GlobalIndexWakeReason::LifecycleResume);
    }
}

struct DatabaseIndexSink {
    db: Database,
    owners: HashMap<String, SourceOperationOwner>,
}

impl DatabaseIndexSink {
    fn owner(&self, volume_id: &str) -> Result<&SourceOperationOwner, GlobalIndexError> {
        self.owners.get(volume_id).ok_or_else(|| {
            GlobalIndexError::ServiceProtocol(format!(
                "source write before operation admission: {volume_id}"
            ))
        })
    }

    fn map_owned_write_error(error: DbError) -> GlobalIndexError {
        match error {
            DbError::Validation(message)
                if message == "global_index_operation_owner_lost"
                    || message == "global_index_operation_foreign_volume_write" =>
            {
                GlobalIndexError::ServiceOutcomeUnknown(message)
            }
            other => GlobalIndexError::Database(other),
        }
    }

    fn only_owner(&self) -> Result<&SourceOperationOwner, GlobalIndexError> {
        if self.owners.len() != 1 {
            return Err(GlobalIndexError::ServiceProtocol(
                "source write does not have exactly one admitted owner".into(),
            ));
        }
        self.owners.values().next().ok_or_else(|| {
            GlobalIndexError::ServiceProtocol("source write before operation admission".into())
        })
    }
}

impl Drop for DatabaseIndexSink {
    fn drop(&mut self) {
        for owner in self.owners.values() {
            self.db.forget_local_global_index_operation(owner);
        }
    }
}

impl GlobalIndexSink for DatabaseIndexSink {
    fn admit_source_operation(
        &mut self,
        source: &GlobalVolume,
        operation_id: &str,
        kind: SourceOperationKind,
        route: SourceOperationRoute,
    ) -> Result<(), GlobalIndexError> {
        let Some(owner) =
            self.db
                .claim_global_index_operation(source, operation_id, kind, route)?
        else {
            return Err(GlobalIndexError::SourceBusy(format!(
                "source {} changed, is disabled, or has another active owner",
                source.id
            )));
        };
        self.owners.insert(source.id.clone(), owner);
        Ok(())
    }

    fn source_operation_admitted(&self, volume_id: &str) -> bool {
        self.owners.contains_key(volume_id)
    }

    fn mark_source_operation_unknown(&mut self, volume_id: &str) -> Result<(), GlobalIndexError> {
        if let Some(owner) = self.owners.get(volume_id).cloned() {
            let _ = self.db.mark_global_index_operation_unknown(&owner)?;
            self.owners.remove(volume_id);
            self.db.forget_local_global_index_operation(&owner);
        }
        Ok(())
    }

    fn finish_source_operation(
        &mut self,
        volume_id: &str,
        status: &str,
        error: Option<&str>,
        full_index_at: Option<i64>,
        incremental_sync_at: Option<i64>,
        complete_rebuild_intent: bool,
    ) -> Result<(), GlobalIndexError> {
        let Some(owner) = self.owners.get(volume_id).cloned() else {
            return Err(GlobalIndexError::ServiceProtocol(format!(
                "source completion without operation admission: {volume_id}"
            )));
        };
        let completed = self.db.finish_global_index_operation(
            &owner,
            status,
            error,
            full_index_at,
            incremental_sync_at,
            complete_rebuild_intent,
        )?;
        self.owners.remove(volume_id);
        self.db.forget_local_global_index_operation(&owner);
        if completed {
            Ok(())
        } else {
            Err(GlobalIndexError::ServiceOutcomeUnknown(
                "source operation ownership changed before completion".into(),
            ))
        }
    }

    fn write_batch(&mut self, entries: &[GlobalEntryInput]) -> Result<usize, GlobalIndexError> {
        if entries.is_empty() {
            return Ok(0);
        }
        let owner = self.owner(&entries[0].volume_id)?;
        self.db
            .upsert_global_entries_batch_owned(entries, owner)
            .map_err(Self::map_owned_write_error)
    }

    fn mark_entry_stale(&mut self, entry_id: &str) -> Result<(), GlobalIndexError> {
        let owner = self.only_owner()?;
        self.db
            .mark_global_entry_stale_owned(entry_id, owner)
            .map_err(Self::map_owned_write_error)?;
        Ok(())
    }

    fn checkpoint(
        &mut self,
        volume_id: &str,
        journal_id: Option<&str>,
        journal_cursor: Option<&str>,
    ) -> Result<(), GlobalIndexError> {
        let owner = self.owner(volume_id)?;
        if !self
            .db
            .update_global_volume_checkpoint_owned(owner, journal_id, journal_cursor)?
        {
            return Err(GlobalIndexError::ServiceOutcomeUnknown(
                "source checkpoint rejected by owner revision".into(),
            ));
        }
        Ok(())
    }

    fn set_source_state(
        &mut self,
        volume_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), GlobalIndexError> {
        let owner = self.owner(volume_id)?;
        if !self
            .db
            .update_global_volume_state_owned(owner, status, error)?
        {
            return Err(GlobalIndexError::ServiceOutcomeUnknown(
                "source state rejected by owner revision".into(),
            ));
        }
        Ok(())
    }

    fn set_source_provider(
        &mut self,
        volume_id: &str,
        provider: &str,
    ) -> Result<(), GlobalIndexError> {
        let owner = self.owner(volume_id)?;
        if !self
            .db
            .update_global_volume_provider_owned(owner, provider)?
        {
            return Err(GlobalIndexError::ServiceOutcomeUnknown(
                "source provider rejected by owner revision".into(),
            ));
        }
        Ok(())
    }

    fn resolve_parent_path(
        &mut self,
        volume_id: &str,
        parent_platform_file_id: &str,
    ) -> Result<Option<String>, GlobalIndexError> {
        Ok(self
            .db
            .global_path_by_platform_identity(volume_id, parent_platform_file_id)?)
    }

    fn find_entry_by_identity(
        &mut self,
        volume_id: &str,
        platform_file_id: &str,
        parent_platform_file_id: &str,
        name: &str,
    ) -> Result<Option<GlobalEntry>, GlobalIndexError> {
        Ok(self.db.global_entry_by_identity(
            volume_id,
            platform_file_id,
            parent_platform_file_id,
            name,
        )?)
    }

    fn mark_volume_entries_stale(&mut self, volume_id: &str) -> Result<(), GlobalIndexError> {
        let owner = self.owner(volume_id)?;
        self.db
            .mark_global_entries_stale_for_volume_owned(owner)
            .map_err(Self::map_owned_write_error)?;
        Ok(())
    }
}

fn run_index(
    provider: Arc<dyn GlobalIndexProvider>,
    db: Database,
    cancel: Arc<AtomicBool>,
    wake: Arc<GlobalIndexWakeSlot>,
) -> Result<(), GlobalIndexError> {
    let mut previous_topology = None;
    let mut run_cycle = true;
    while !cancel.load(Ordering::Acquire) {
        if run_cycle {
            // Consume a command/start signal before beginning its immediate
            // catch-up cycle. Any signal arriving after this point remains
            // pending and is delivered by the next wait, including the
            // catch-up-to-idle boundary.
            let _ = wake.wait(&cancel, Some(Duration::ZERO));
            super::qa_trace::record("coordinator_cycle");
            let mut cycle_topology = previous_topology.clone();
            match run_index_cycle_with_topology(
                provider.as_ref(),
                &db,
                &cancel,
                &wake,
                &mut cycle_topology,
            ) {
                Ok(topology) => {
                    previous_topology = Some(topology);
                }
                Err(error) if sqlite_contention_kind(&error).is_some() => {
                    let kind = sqlite_contention_kind(&error).expect("matched SQLite contention");
                    eprintln!(
                        "Global Index cycle deferred after SQLite {kind:?}; no immediate retry scheduled: {error}"
                    );
                    // Keep the topology snapshot collected before the failed
                    // database write. It prevents an unchanged topology audit
                    // from replaying this cycle, while still allowing a later
                    // topology change to wake one owner-fenced cycle.
                    previous_topology = cycle_topology;
                }
                Err(error) => return Err(error),
            }
            if cancel.load(Ordering::Acquire) {
                break;
            }
        }

        super::qa_trace::record("coordinator_wait");
        match wake.wait(&cancel, provider.topology_audit_interval()) {
            GlobalIndexWaitResult::Notified(_reason) => {
                #[cfg(feature = "native-qa")]
                super::qa_trace::record(&format!("coordinator_wake_{_reason:?}"));
                run_cycle = true;
            }
            GlobalIndexWaitResult::Cancelled | GlobalIndexWaitResult::Shutdown => break,
            GlobalIndexWaitResult::TimedOut => match provider.audit_source_topology() {
                Ok(Some(discovered)) => {
                    let current_topology = topology_signature(&discovered);
                    run_cycle = previous_topology.as_ref() != Some(&current_topology);
                    if run_cycle {
                        wake.notify(GlobalIndexWakeReason::SourceTopology);
                        previous_topology = Some(current_topology);
                    }
                }
                Ok(None) => run_cycle = false,
                Err(error) => {
                    eprintln!("Global Index topology audit failed: {error}");
                    run_cycle = false;
                }
            },
        }
    }
    Ok(())
}

#[cfg(all(windows, feature = "native-qa"))]
pub(crate) fn run_service_recovery_qa_cycle(
    provider: &dyn GlobalIndexProvider,
    db: &Database,
) -> Result<(), GlobalIndexError> {
    run_index_cycle(
        provider,
        db,
        &Arc::new(AtomicBool::new(false)),
        &GlobalIndexWakeSlot::default(),
    )
    .map(|_| ())
}

type SourceTopologySignature = Vec<(String, String, String, String, String, String, bool)>;

#[cfg(any(test, all(windows, feature = "native-qa")))]
fn run_index_cycle(
    provider: &dyn GlobalIndexProvider,
    db: &Database,
    cancel: &Arc<AtomicBool>,
    wake: &GlobalIndexWakeSlot,
) -> Result<SourceTopologySignature, GlobalIndexError> {
    let mut discovered_topology = None;
    run_index_cycle_with_topology(provider, db, cancel, wake, &mut discovered_topology)
}

fn run_index_cycle_with_topology(
    provider: &dyn GlobalIndexProvider,
    db: &Database,
    cancel: &Arc<AtomicBool>,
    wake: &GlobalIndexWakeSlot,
    discovered_topology: &mut Option<SourceTopologySignature>,
) -> Result<SourceTopologySignature, GlobalIndexError> {
    let discovered = provider.discover_sources()?;
    let topology = topology_signature(&discovered);
    *discovered_topology = Some(topology.clone());
    for source in &discovered {
        let existing = db.get_global_volume(&source.volume.id)?;
        let legacy_recursive_fallback_needs_rebuild = existing.as_ref().is_some_and(|current| {
            current.provider == PROVIDER_WINDOWS_RECURSIVE_FALLBACK
                && source.volume.provider == PROVIDER_WINDOWS_MFT_USN
        });
        db.upsert_global_volume(&source.volume)?;
        if legacy_recursive_fallback_needs_rebuild {
            let error = existing
                .as_ref()
                .and_then(|current| current.last_error.as_deref())
                .unwrap_or("legacy recursive fallback requires native MFT rebuild");
            db.update_global_volume_state_if_unowned(
                &source.volume.id,
                INDEX_STATUS_REBUILD_REQUIRED,
                Some(error),
            )?;
        }
        if source.volume.provider == PROVIDER_WINDOWS_UNSUPPORTED {
            let message = source
                .volume
                .last_error
                .as_deref()
                .unwrap_or("windows_global_index_native_provider_unsupported");
            let current = db.get_global_volume(&source.volume.id)?;
            if current.as_ref().is_none_or(|current| {
                current.index_status != INDEX_STATUS_UNAVAILABLE
                    || current.last_error.as_deref() != Some(message)
            }) {
                db.update_global_volume_state_if_unowned(
                    &source.volume.id,
                    INDEX_STATUS_UNAVAILABLE,
                    Some(message),
                )?;
            }
        }
    }
    let discovered_by_id = discovered
        .into_iter()
        .map(|source| (source.volume.id.clone(), source))
        .collect::<HashMap<_, _>>();
    let volumes = db.list_global_volumes()?;
    let mut sink = DatabaseIndexSink {
        db: db.clone(),
        owners: HashMap::new(),
    };
    for volume in volumes {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        if !volume.enabled {
            continue;
        }
        let Some(discovered_source) = discovered_by_id.get(&volume.id) else {
            if volume.index_status != INDEX_STATUS_UNAVAILABLE
                || volume.last_error.as_deref() != Some("global_index_source_unavailable")
            {
                db.update_global_volume_state_if_unowned(
                    &volume.id,
                    INDEX_STATUS_UNAVAILABLE,
                    Some("global_index_source_unavailable"),
                )?;
            }
            continue;
        };
        if discovered_source.volume.provider == PROVIDER_WINDOWS_UNSUPPORTED {
            continue;
        }
        let source_reappeared = volume.index_status == INDEX_STATUS_UNAVAILABLE
            && volume.last_error.as_deref() == Some("global_index_source_unavailable");
        let mut source = discovered_source.clone();
        source.volume.enabled = volume.enabled;
        source.volume.index_status = volume.index_status.clone();
        source.volume.last_error = volume.last_error.clone();
        source.volume.journal_id = volume.journal_id.clone();
        source.volume.journal_cursor = volume.journal_cursor.clone();
        source.volume.last_full_index_at = volume.last_full_index_at;
        source.volume.last_incremental_sync_at = volume.last_incremental_sync_at;
        source.volume.entry_count = volume.entry_count;
        source.volume.created_at = volume.created_at;
        let operation_state = db
            .global_index_operation_state(&volume.id)?
            .ok_or_else(|| GlobalIndexError::Provider("source operation state missing".into()))?;
        let is_rebuild = operation_state.rebuild_pending()
            || volume.index_status == INDEX_STATUS_REBUILD_REQUIRED;
        let is_initial = volume.last_full_index_at.is_none() || source_reappeared;
        let operation_kind = if is_rebuild {
            SourceOperationKind::Rebuild
        } else if is_initial {
            SourceOperationKind::Initial
        } else {
            SourceOperationKind::Incremental
        };
        let use_background_admission =
            is_rebuild || is_initial || provider.requires_background_admission(&source);
        let result = if use_background_admission {
            run_with_background_admission(
                provider,
                &source,
                &mut sink,
                cancel,
                is_initial,
                is_rebuild,
                operation_kind,
            )
        } else {
            match admit_coordinator_operation(provider, &source, &mut sink, operation_kind) {
                Ok(()) => run_provider_operation(
                    provider, &source, &mut sink, cancel, is_initial, is_rebuild,
                ),
                Err(error) => Err(error),
            }
        };
        if !sink.source_operation_admitted(&volume.id) {
            match result {
                Err(GlobalIndexError::Paused) if cancel.load(Ordering::Acquire) => break,
                // A busy/rejected/unknown request has no source owner. Its
                // durable health, checkpoint and timestamps remain untouched.
                _ => continue,
            }
        }
        match result {
            Ok(()) => {
                let now = unix_now();
                let current = db.get_global_volume(&volume.id)?;
                let current_status = current.as_ref().map(|source| source.index_status.as_str());
                let operation_reported_degraded = current_status
                    .filter(|status| is_degraded_index_status(status))
                    .filter(|status| Some(*status) != Some(volume.index_status.as_str()));
                let prior_degraded = (!source_reappeared
                    && !(is_rebuild && volume.index_status == INDEX_STATUS_REBUILD_REQUIRED)
                    && is_persistent_degraded_index_status(&volume.index_status))
                .then_some(volume.index_status.as_str());
                let degraded_status = operation_reported_degraded.or(prior_degraded);
                let preserved_error = degraded_status.and_then(|_| {
                    current
                        .as_ref()
                        .and_then(|current| current.last_error.as_deref())
                        .or(volume.last_error.as_deref())
                });
                sink.finish_source_operation(
                    &volume.id,
                    if cancel.load(Ordering::Acquire) {
                        INDEX_STATUS_PAUSED
                    } else if let Some(status) = degraded_status {
                        status
                    } else {
                        INDEX_STATUS_READY
                    },
                    preserved_error,
                    if volume.last_full_index_at.is_some() && !source_reappeared && !is_rebuild {
                        None
                    } else {
                        Some(now)
                    },
                    Some(now),
                    is_rebuild && !cancel.load(Ordering::Acquire),
                )?;
            }
            Err(GlobalIndexError::Paused) => {
                sink.finish_source_operation(
                    &volume.id,
                    INDEX_STATUS_PAUSED,
                    None,
                    None,
                    None,
                    false,
                )?;
                break;
            }
            Err(GlobalIndexError::RebuildRequired(message)) => {
                #[cfg(feature = "native-qa")]
                super::qa_trace::record_error("global_index_source", &message);
                sink.finish_source_operation(
                    &volume.id,
                    INDEX_STATUS_REBUILD_REQUIRED,
                    Some(&message),
                    None,
                    None,
                    false,
                )?;
                if !is_rebuild {
                    wake.notify(GlobalIndexWakeReason::RecoveryRequired);
                }
            }
            Err(GlobalIndexError::ServiceBusy(_) | GlobalIndexError::SourceBusy(_)) => {
                sink.mark_source_operation_unknown(&volume.id)?;
            }
            Err(
                GlobalIndexError::ServiceProtocol(_) | GlobalIndexError::ServiceOutcomeUnknown(_),
            ) => {
                sink.mark_source_operation_unknown(&volume.id)?;
            }
            Err(error) => {
                #[cfg(feature = "native-qa")]
                super::qa_trace::record_error("global_index_source", &error.to_string());
                let current = db.get_global_volume(&volume.id)?;
                let current_status = current
                    .as_ref()
                    .map(|current| current.index_status.as_str());
                let was_rebuild_required = volume.index_status == INDEX_STATUS_REBUILD_REQUIRED;
                let preserved_status =
                    current_status.filter(|status| is_degraded_index_status(status));
                let status = preserved_status
                    .map(str::to_string)
                    .unwrap_or_else(|| source_error_status(&error).to_string());
                sink.finish_source_operation(
                    &volume.id,
                    &status,
                    Some(&error.to_string()),
                    None,
                    None,
                    false,
                )?;
                if !was_rebuild_required && current_status == Some(INDEX_STATUS_REBUILD_REQUIRED) {
                    wake.notify(GlobalIndexWakeReason::RecoveryRequired);
                }
            }
        }
    }
    Ok(topology)
}

#[cfg(all(test, target_os = "windows"))]
pub(crate) fn run_index_cycle_for_test(
    provider: &dyn GlobalIndexProvider,
    db: &Database,
    cancel: &Arc<AtomicBool>,
    wake: &GlobalIndexWakeSlot,
) -> Result<(), GlobalIndexError> {
    run_index_cycle(provider, db, cancel, wake).map(|_| ())
}

fn run_provider_operation(
    provider: &dyn GlobalIndexProvider,
    source: &GlobalSourceDescriptor,
    sink: &mut dyn GlobalIndexSink,
    cancel: &AtomicBool,
    initial: bool,
    rebuild: bool,
) -> Result<(), GlobalIndexError> {
    if rebuild {
        provider.rebuild(source, sink, cancel)
    } else if initial {
        provider.start_initial_index(source, sink, cancel)
    } else {
        provider.resume_incremental_sync(source, sink, cancel)
    }
}

fn admit_coordinator_operation(
    provider: &dyn GlobalIndexProvider,
    source: &GlobalSourceDescriptor,
    sink: &mut dyn GlobalIndexSink,
    kind: SourceOperationKind,
) -> Result<(), GlobalIndexError> {
    if provider.has_external_source_operation_admission() {
        return Ok(());
    }
    sink.admit_source_operation(
        &source.volume,
        &uuid::Uuid::new_v4().to_string(),
        kind,
        SourceOperationRoute::Coordinator,
    )
}

fn source_error_status(error: &GlobalIndexError) -> &'static str {
    match error {
        GlobalIndexError::Paused => INDEX_STATUS_PAUSED,
        GlobalIndexError::RebuildRequired(_) => INDEX_STATUS_REBUILD_REQUIRED,
        GlobalIndexError::PermissionRequired(_) => INDEX_STATUS_PERMISSION_REQUIRED,
        GlobalIndexError::WindowsIo(5 | 1314) => INDEX_STATUS_PERMISSION_REQUIRED,
        _ => INDEX_STATUS_ERROR,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqliteContentionKind {
    Busy,
    BusySnapshot,
    Locked,
}

/// Classify only real SQLite result codes. Provider diagnostics, Service
/// request contention, and transport/protocol errors remain separate classes.
pub(crate) fn sqlite_contention_kind(error: &GlobalIndexError) -> Option<SqliteContentionKind> {
    let GlobalIndexError::Database(DbError::Sqlite(rusqlite::Error::SqliteFailure(code, _))) =
        error
    else {
        return None;
    };

    if code.extended_code == rusqlite::ffi::SQLITE_BUSY_SNAPSHOT {
        Some(SqliteContentionKind::BusySnapshot)
    } else if code.code == rusqlite::ErrorCode::DatabaseBusy {
        Some(SqliteContentionKind::Busy)
    } else if code.code == rusqlite::ErrorCode::DatabaseLocked {
        Some(SqliteContentionKind::Locked)
    } else {
        None
    }
}

fn run_with_background_admission(
    provider: &dyn GlobalIndexProvider,
    source: &GlobalSourceDescriptor,
    sink: &mut dyn GlobalIndexSink,
    cancel: &Arc<AtomicBool>,
    initial: bool,
    rebuild: bool,
    operation_kind: SourceOperationKind,
) -> Result<(), GlobalIndexError> {
    let request = WorkRequest::new(
        format!("global-index:{}", source.volume.id),
        WorkClass::Background,
        ResourceHints::cpu_io(1, 1),
    )
    .with_coalesce_key(format!("global-index:{}", source.volume.id))
    .with_cancellation(CancellationToken::from_flag(cancel.clone()));
    let _lease = WorkScheduler::global()
        .acquire_with_backpressure(request)
        .map_err(|error| {
            if matches!(error, crate::scheduler::AcquireError::Cancelled)
                && cancel.load(Ordering::Acquire)
            {
                GlobalIndexError::Paused
            } else {
                GlobalIndexError::Provider(format!("background admission failed: {error}"))
            }
        })?;
    if rebuild {
        super::qa_trace::record("windows_recovery_rebuild_admitted");
    }
    admit_coordinator_operation(provider, source, sink, operation_kind)?;
    let _qos = scope_thread_qos(WorkClass::Background);
    run_provider_operation(provider, source, sink, cancel, initial, rebuild)
}

fn topology_signature(sources: &[GlobalSourceDescriptor]) -> SourceTopologySignature {
    let mut signature = sources
        .iter()
        .map(|source| {
            (
                source.volume.id.clone(),
                source.volume.stable_volume_id.clone(),
                source.volume.mount_path.clone(),
                source.volume.filesystem_type.to_ascii_lowercase(),
                source.volume.drive_kind.to_ascii_lowercase(),
                source.volume.provider.clone(),
                source.volume.enabled,
            )
        })
        .collect::<Vec<_>>();
    signature.sort();
    signature
}

fn is_degraded_index_status(status: &str) -> bool {
    matches!(
        status,
        INDEX_STATUS_PERMISSION_REQUIRED
            | INDEX_STATUS_REBUILD_REQUIRED
            | INDEX_STATUS_SPOTLIGHT_UNAVAILABLE
            | INDEX_STATUS_SPOTLIGHT_NOT_INDEXED
            | INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED
            | INDEX_STATUS_FSEVENTS_UNAVAILABLE
            | INDEX_STATUS_UNAVAILABLE
    )
}

fn is_persistent_degraded_index_status(status: &str) -> bool {
    matches!(
        status,
        INDEX_STATUS_PERMISSION_REQUIRED
            | INDEX_STATUS_SPOTLIGHT_UNAVAILABLE
            | INDEX_STATUS_SPOTLIGHT_NOT_INDEXED
            | INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED
            | INDEX_STATUS_FSEVENTS_UNAVAILABLE
    )
}

fn platform_provider(
    wake: Arc<GlobalIndexWakeSlot>,
    database_path: PathBuf,
) -> Box<dyn GlobalIndexProvider> {
    #[cfg(target_os = "windows")]
    {
        Box::new(
            crate::global_index::windows::WindowsGlobalIndexProvider::with_wake(
                wake,
                database_path,
            ),
        )
    }
    #[cfg(all(not(target_os = "windows"), target_os = "macos"))]
    {
        let _ = database_path;
        Box::new(crate::global_index::macos::MacosSpotlightProvider::with_wake(wake))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (wake, database_path);
        Box::new(RecursiveFallbackProvider::new())
    }
}

pub struct RecursiveFallbackProvider {
    stopped: AtomicBool,
}

impl RecursiveFallbackProvider {
    pub fn new() -> Self {
        Self {
            stopped: AtomicBool::new(false),
        }
    }
}

impl Default for RecursiveFallbackProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalIndexProvider for RecursiveFallbackProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        let root = if cfg!(windows) {
            PathBuf::from("C:\\")
        } else {
            PathBuf::from("/")
        };
        let mount_path = root.to_string_lossy().into_owned();
        let stable_volume_id = format!("{}-root", std::env::consts::OS);
        let now = unix_now();
        Ok(vec![GlobalSourceDescriptor {
            volume: GlobalVolume {
                id: format!("gv_{}", blake3::hash(stable_volume_id.as_bytes()).to_hex()),
                platform: std::env::consts::OS.to_string(),
                stable_volume_id,
                display_name: mount_path.clone(),
                mount_path,
                filesystem_type: "unknown".to_string(),
                drive_kind: "fixed".to_string(),
                enabled: true,
                provider: PROVIDER_RECURSIVE_FALLBACK.to_string(),
                index_status: INDEX_STATUS_DISCOVERED.to_string(),
                last_error: None,
                journal_id: None,
                journal_cursor: None,
                last_full_index_at: None,
                last_incremental_sync_at: None,
                entry_count: 0,
                created_at: now,
                updated_at: now,
            },
        }])
    }

    fn start_initial_index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.stopped.store(false, Ordering::Release);
        let mut batch = Vec::with_capacity(512);
        walk_directory(
            Path::new(&source.volume.mount_path),
            &mut batch,
            sink,
            cancel,
            &self.stopped,
        )?;
        if !batch.is_empty() {
            sink.write_batch(&batch)?;
        }
        Ok(())
    }

    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        // Recursive fallback has no durable journal.  A reconciliation scan is
        // explicit in the provider status and is kept separate from MFT/USN.
        self.start_initial_index(source, sink, cancel)
    }

    fn pause(&self) -> Result<(), GlobalIndexError> {
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }

    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok(PROVIDER_RECURSIVE_FALLBACK.to_string())
    }

    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
}

fn walk_directory(
    root: &Path,
    batch: &mut Vec<GlobalEntryInput>,
    sink: &mut dyn GlobalIndexSink,
    cancel: &AtomicBool,
    stopped: &AtomicBool,
) -> Result<(), GlobalIndexError> {
    if cancel.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
        return Err(GlobalIndexError::Paused);
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    let volume_id = format!(
        "gv_{}",
        blake3::hash(format!("{}-root", std::env::consts::OS).as_bytes()).to_hex()
    );
    for item in entries {
        if cancel.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
            return Err(GlobalIndexError::Paused);
        }
        let item = match item {
            Ok(item) => item,
            Err(_) => continue,
        };
        let path = item.path();
        let input = GlobalEntryInput::from_path(&volume_id, &path, PROVIDER_RECURSIVE_FALLBACK);
        let is_directory = item.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        batch.push(input);
        if batch.len() >= 512 {
            sink.write_batch(batch)?;
            batch.clear();
        }
        if is_directory {
            walk_directory(&path, batch, sink, cancel, stopped)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::{sync_channel, SyncSender};
    use std::time::Instant;

    static TEST_DATABASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TestDatabasePath(PathBuf);

    impl TestDatabasePath {
        fn new() -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(".tmp-tests")
                .join("zb05-global-index-coordinator");
            fs::create_dir_all(&root).expect("create task-local Global Index test directory");
            let id = TEST_DATABASE_COUNTER.fetch_add(1, Ordering::Relaxed);
            Self(root.join(format!("idle-start-{}-{id}.sqlite3", std::process::id())))
        }
    }

    impl Drop for TestDatabasePath {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let mut path = self.0.as_os_str().to_os_string();
                path.push(suffix);
                let _ = fs::remove_file(PathBuf::from(path));
            }
        }
    }

    #[cfg(target_os = "windows")]
    struct WindowsNativeSmokeRun {
        root: PathBuf,
        database: Option<Database>,
        coordinator: Option<GlobalIndexCoordinator>,
        temporary_environment: Option<WindowsNativeSmokeTempEnvironment>,
        owned_files: Vec<PathBuf>,
    }

    #[cfg(target_os = "windows")]
    struct WindowsNativeSmokeTempEnvironment {
        previous_temp: Option<std::ffi::OsString>,
        previous_tmp: Option<std::ffi::OsString>,
    }

    #[cfg(target_os = "windows")]
    impl WindowsNativeSmokeTempEnvironment {
        fn new(path: &Path) -> Self {
            fs::create_dir_all(path).expect("create worktree-local native-smoke temp directory");
            let previous_temp = std::env::var_os("TEMP");
            let previous_tmp = std::env::var_os("TMP");
            std::env::set_var("TEMP", path);
            std::env::set_var("TMP", path);
            let environment = Self {
                previous_temp,
                previous_tmp,
            };
            assert!(
                std::env::temp_dir().starts_with(path),
                "Windows temp directory must remain inside the task-owned worktree root"
            );
            environment
        }
    }

    #[cfg(target_os = "windows")]
    impl Drop for WindowsNativeSmokeTempEnvironment {
        fn drop(&mut self) {
            restore_environment_variable("TEMP", self.previous_temp.take());
            restore_environment_variable("TMP", self.previous_tmp.take());
        }
    }

    #[cfg(target_os = "windows")]
    fn restore_environment_variable(name: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }

    #[cfg(target_os = "windows")]
    impl WindowsNativeSmokeRun {
        fn new() -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(".tmp-tests")
                .join("zb05-windows-native-smoke")
                .join(format!("run-{}-{}", std::process::id(), unix_now()));
            assert!(
                !root.exists(),
                "native smoke run root already exists: {}",
                root.display()
            );
            fs::create_dir_all(root.join("fixture"))
                .expect("create isolated native-smoke fixture root");
            let mut run = Self {
                root,
                database: None,
                coordinator: None,
                temporary_environment: None,
                owned_files: Vec::new(),
            };
            run.temporary_environment = Some(WindowsNativeSmokeTempEnvironment::new(
                &run.root.join("temp"),
            ));
            let database = Database::open(run.root.join("candidate.sqlite3"))
                .expect("open isolated native-smoke candidate database");
            let wake = Arc::new(GlobalIndexWakeSlot::default());
            let provider: Arc<dyn GlobalIndexProvider> = Arc::new(
                crate::global_index::windows::WindowsGlobalIndexProvider::with_direct_native_smoke(
                    wake.clone(),
                    database.path().to_path_buf(),
                    run.root.join("temp"),
                ),
            );
            run.coordinator = Some(GlobalIndexCoordinator::with_provider(
                database.clone(),
                provider,
                wake,
            ));
            run.database = Some(database);
            run
        }

        fn database(&self) -> &Database {
            self.database.as_ref().expect("candidate database")
        }

        fn coordinator(&self) -> &GlobalIndexCoordinator {
            self.coordinator.as_ref().expect("candidate coordinator")
        }

        fn own_file(&mut self, path: PathBuf) {
            self.owned_files.push(path);
        }
    }

    #[cfg(target_os = "windows")]
    impl Drop for WindowsNativeSmokeRun {
        fn drop(&mut self) {
            if let Some(coordinator) = self.coordinator.take() {
                // Stop only this candidate coordinator. The installed service
                // is deliberately left alone; the smoke waits until its
                // metadata request has completed before reaching this guard.
                coordinator.cancel.store(true, Ordering::Release);
                coordinator.wake.notify(GlobalIndexWakeReason::Shutdown);
                if let Ok(mut slot) = coordinator.worker.lock() {
                    if let Some(worker) = slot.take() {
                        let _ = worker.join();
                    }
                }
                drop(coordinator);
            }
            self.database.take();
            for path in self.owned_files.drain(..) {
                let _ = fs::remove_file(path);
            }
            self.temporary_environment.take();
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct DiscoveryCounterProvider {
        discoveries: Arc<AtomicUsize>,
        discovered: SyncSender<()>,
    }

    impl GlobalIndexProvider for DiscoveryCounterProvider {
        fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
            self.discoveries.fetch_add(1, Ordering::AcqRel);
            let _ = self.discovered.try_send(());
            Ok(Vec::new())
        }

        fn start_initial_index(
            &self,
            _source: &GlobalSourceDescriptor,
            _sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            Ok(())
        }

        fn resume_incremental_sync(
            &self,
            _source: &GlobalSourceDescriptor,
            _sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            Ok(())
        }

        fn pause(&self) -> Result<(), GlobalIndexError> {
            Ok(())
        }

        fn status(&self) -> Result<String, GlobalIndexError> {
            Ok("test".to_string())
        }

        fn shutdown(&self) -> Result<(), GlobalIndexError> {
            Ok(())
        }
    }

    struct CommandWakeProvider {
        source: GlobalSourceDescriptor,
        discoveries: AtomicUsize,
        initial_runs: AtomicUsize,
        incremental_runs: AtomicUsize,
        rebuild_runs: AtomicUsize,
        enabled_changes: AtomicUsize,
        last_enabled: AtomicBool,
        pauses: AtomicUsize,
        shutdowns: AtomicUsize,
    }

    impl GlobalIndexProvider for CommandWakeProvider {
        fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
            self.discoveries.fetch_add(1, Ordering::AcqRel);
            Ok(vec![self.source.clone()])
        }

        fn start_initial_index(
            &self,
            _source: &GlobalSourceDescriptor,
            _sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            self.initial_runs.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        fn resume_incremental_sync(
            &self,
            _source: &GlobalSourceDescriptor,
            _sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            self.incremental_runs.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        fn rebuild(
            &self,
            _source: &GlobalSourceDescriptor,
            _sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            self.rebuild_runs.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        fn pause(&self) -> Result<(), GlobalIndexError> {
            self.pauses.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        fn status(&self) -> Result<String, GlobalIndexError> {
            Ok("test".to_string())
        }

        fn shutdown(&self) -> Result<(), GlobalIndexError> {
            self.shutdowns.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        fn source_enabled_changed(&self, _source_id: &str, enabled: bool) {
            self.last_enabled.store(enabled, Ordering::Release);
            self.enabled_changes.fetch_add(1, Ordering::AcqRel);
        }
    }

    struct CoordinatorShutdownGuard {
        coordinator: GlobalIndexCoordinator,
        active: bool,
    }

    impl Drop for CoordinatorShutdownGuard {
        fn drop(&mut self) {
            if self.active {
                let _ = self.coordinator.shutdown();
            }
        }
    }

    fn wait_for_discoveries(discoveries: &AtomicUsize, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while discoveries.load(Ordering::Acquire) < expected && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            discoveries.load(Ordering::Acquire) >= expected,
            "expected at least {expected} source discoveries, observed {}",
            discoveries.load(Ordering::Acquire)
        );
    }

    fn wait_for_idle_wait(wake: &GlobalIndexWakeSlot, previous_blocked_waits: u64) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while wake.snapshot().blocked_waits <= previous_blocked_waits && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            wake.snapshot().blocked_waits > previous_blocked_waits,
            "coordinator did not return to its blocking idle wait"
        );
    }

    #[test]
    fn startup_runs_one_immediate_cycle_then_blocks_until_a_signal() {
        let path = TestDatabasePath::new();
        let db = Database::open(&path.0).expect("open test database");
        let discoveries = Arc::new(AtomicUsize::new(0));
        let (discovered_tx, discovered_rx) = sync_channel(2);
        let provider = Arc::new(DiscoveryCounterProvider {
            discoveries: discoveries.clone(),
            discovered: discovered_tx,
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(GlobalIndexWakeSlot::default());
        let worker = {
            let db = db.clone();
            let cancel = cancel.clone();
            let wake = wake.clone();
            thread::spawn(move || run_index(provider, db, cancel, wake))
        };

        discovered_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("startup discovery");
        let deadline = Instant::now() + Duration::from_secs(3);
        while wake.snapshot().blocked_waits == 0 && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            wake.snapshot().blocked_waits > 0,
            "coordinator entered idle wait"
        );
        assert_eq!(discoveries.load(Ordering::Acquire), 1);

        cancel.store(true, Ordering::Release);
        wake.notify(GlobalIndexWakeReason::Shutdown);
        worker
            .join()
            .expect("join coordinator")
            .expect("coordinator exits cleanly");
        assert_eq!(discoveries.load(Ordering::Acquire), 1);

        drop(db);
        drop(path);
    }

    #[test]
    fn explicit_commands_wake_and_join_the_blocked_coordinator() {
        let path = TestDatabasePath::new();
        let db = Database::open(&path.0).expect("open test database");
        let source = GlobalSourceDescriptor {
            volume: GlobalVolume {
                id: "gv_control_test".to_string(),
                platform: "test".to_string(),
                stable_volume_id: "control-test-volume".to_string(),
                display_name: "Control test volume".to_string(),
                mount_path: "test://control-volume".to_string(),
                filesystem_type: "test".to_string(),
                drive_kind: "fixed".to_string(),
                enabled: true,
                provider: "control_test".to_string(),
                index_status: INDEX_STATUS_READY.to_string(),
                last_error: None,
                journal_id: None,
                journal_cursor: None,
                last_full_index_at: Some(1),
                last_incremental_sync_at: Some(1),
                entry_count: 0,
                created_at: 1,
                updated_at: 1,
            },
        };
        db.upsert_global_volume(&source.volume)
            .expect("seed durable test source");
        let provider = Arc::new(CommandWakeProvider {
            source,
            discoveries: AtomicUsize::new(0),
            initial_runs: AtomicUsize::new(0),
            incremental_runs: AtomicUsize::new(0),
            rebuild_runs: AtomicUsize::new(0),
            enabled_changes: AtomicUsize::new(0),
            last_enabled: AtomicBool::new(false),
            pauses: AtomicUsize::new(0),
            shutdowns: AtomicUsize::new(0),
        });
        let wake = Arc::new(GlobalIndexWakeSlot::default());
        let coordinator =
            GlobalIndexCoordinator::with_provider(db.clone(), provider.clone(), wake.clone());
        let mut shutdown_guard = CoordinatorShutdownGuard {
            coordinator: coordinator.clone(),
            active: true,
        };

        coordinator.start().expect("start coordinator");
        wait_for_discoveries(&provider.discoveries, 1);
        wait_for_idle_wait(&wake, 0);
        assert_eq!(provider.incremental_runs.load(Ordering::Acquire), 1);

        let prior_discoveries = provider.discoveries.load(Ordering::Acquire);
        let prior_idle_waits = wake.snapshot().blocked_waits;
        coordinator
            .rebuild(Some("gv_control_test".to_string()))
            .expect("request immediate rebuild");
        // Rebuild persists intent, then the explicit wake causes one worker cycle.
        wait_for_discoveries(&provider.discoveries, prior_discoveries + 1);
        wait_for_idle_wait(&wake, prior_idle_waits);
        assert_eq!(provider.rebuild_runs.load(Ordering::Acquire), 1);
        assert_eq!(
            db.get_global_volume("gv_control_test")
                .expect("read rebuilt test source")
                .expect("test source remains present")
                .index_status,
            INDEX_STATUS_READY
        );

        let prior_discoveries = provider.discoveries.load(Ordering::Acquire);
        let prior_idle_waits = wake.snapshot().blocked_waits;
        coordinator
            .set_source_enabled("gv_control_test", false)
            .expect("disable source");
        assert_eq!(provider.enabled_changes.load(Ordering::Acquire), 1);
        assert!(!provider.last_enabled.load(Ordering::Acquire));
        assert!(
            !db.get_global_volume("gv_control_test")
                .expect("read disabled test source")
                .expect("test source remains present")
                .enabled
        );
        wait_for_discoveries(&provider.discoveries, prior_discoveries + 1);
        wait_for_idle_wait(&wake, prior_idle_waits);
        assert_eq!(provider.incremental_runs.load(Ordering::Acquire), 1);

        let prior_discoveries = provider.discoveries.load(Ordering::Acquire);
        let prior_idle_waits = wake.snapshot().blocked_waits;
        coordinator
            .set_source_enabled("gv_control_test", true)
            .expect("enable source");
        assert_eq!(provider.enabled_changes.load(Ordering::Acquire), 2);
        assert!(provider.last_enabled.load(Ordering::Acquire));
        assert!(
            db.get_global_volume("gv_control_test")
                .expect("read enabled test source")
                .expect("test source remains present")
                .enabled
        );
        wait_for_discoveries(&provider.discoveries, prior_discoveries + 1);
        wait_for_idle_wait(&wake, prior_idle_waits);
        assert_eq!(provider.incremental_runs.load(Ordering::Acquire), 2);

        let pause_started = Instant::now();
        coordinator.pause().expect("pause coordinator");
        assert!(
            pause_started.elapsed() < Duration::from_secs(1),
            "pause must release and join the blocked worker promptly"
        );
        assert_eq!(provider.pauses.load(Ordering::Acquire), 1);
        assert!(coordinator
            .worker
            .lock()
            .expect("coordinator worker lock")
            .is_none());

        let prior_discoveries = provider.discoveries.load(Ordering::Acquire);
        let prior_idle_waits = wake.snapshot().blocked_waits;
        coordinator.resume().expect("resume coordinator");
        wait_for_discoveries(&provider.discoveries, prior_discoveries + 1);
        wait_for_idle_wait(&wake, prior_idle_waits);
        assert_eq!(provider.incremental_runs.load(Ordering::Acquire), 3);

        let shutdown_started = Instant::now();
        coordinator.shutdown().expect("shutdown coordinator");
        assert!(
            shutdown_started.elapsed() < Duration::from_secs(1),
            "shutdown must release and join the blocked worker promptly"
        );
        assert_eq!(provider.shutdowns.load(Ordering::Acquire), 1);
        assert!(coordinator
            .worker
            .lock()
            .expect("coordinator worker lock")
            .is_none());
        shutdown_guard.active = false;

        drop(coordinator);
        drop(db);
        drop(path);
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "bounded native smoke; isolated database, wake-only watcher, and direct MFT/USN fallback for test-binary identity"]
    fn windows_native_change_signal_search_smoke() {
        use crate::global_index::models::{
            INDEX_STATUS_READY, INDEX_STATUS_REBUILD_REQUIRED, PROVIDER_WINDOWS_MFT_USN,
        };

        const BASELINE_TIMEOUT: Duration = Duration::from_secs(8 * 60);
        const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

        let mut run = WindowsNativeSmokeRun::new();
        // This test binary is not the installed product image, so the service
        // rejects it before command handling. Use the existing direct MFT/USN
        // provider explicitly while retaining the same wake-only watcher.
        let discovery_provider =
            crate::global_index::windows::DirectWindowsGlobalIndexProvider::new();
        let discovered = discovery_provider
            .discover_sources()
            .expect("discover sources through native Windows discovery");
        let smoke_root = run.root.join("fixture");
        let source = discovered
            .iter()
            .filter(|source| {
                source.volume.provider == PROVIDER_WINDOWS_MFT_USN
                    && source.volume.enabled
                    && Path::new(&smoke_root).starts_with(&source.volume.mount_path)
            })
            .max_by_key(|source| source.volume.mount_path.len())
            .expect("candidate fixture must be on an enabled fixed NTFS source");
        let source_id = source.volume.id.clone();
        let source_mount = source.volume.mount_path.clone();
        let run_id = format!("{}-{}", std::process::id(), unix_now());
        let baseline_name = format!("zb05-native-baseline-{run_id}.txt");
        let baseline_path = smoke_root.join(&baseline_name);
        run.own_file(baseline_path.clone());
        fs::write(
            &baseline_path,
            b"pre-existing task-owned baseline fixture\n",
        )
        .expect("create task-owned fixture before native baseline");

        // Restrict this isolated candidate profile to the one fixture volume.
        // The real/default per-volume policy is untouched in the installed
        // product database.
        for discovered_source in &discovered {
            run.database()
                .upsert_global_volume(&discovered_source.volume)
                .expect("seed candidate source descriptors");
            if discovered_source.volume.id != source_id {
                run.database()
                    .set_global_volume_enabled(&discovered_source.volume.id, false)
                    .expect("disable non-fixture candidate sources");
            }
        }

        run.coordinator()
            .start()
            .expect("start isolated native-smoke coordinator");
        let baseline_started = Instant::now();
        let baseline_deadline = baseline_started + BASELINE_TIMEOUT;
        let ready_source = loop {
            let current = run
                .database()
                .get_global_volume(&source_id)
                .expect("read candidate source state")
                .expect("candidate source remains discovered");
            if current.index_status == INDEX_STATUS_READY && current.last_full_index_at.is_some() {
                break current;
            }
            assert!(
                Instant::now() < baseline_deadline,
                "candidate baseline did not reach ready within the bounded smoke window: status={}, error={:?}",
                current.index_status,
                current.last_error
            );
            assert_ne!(
                current.index_status, INDEX_STATUS_PERMISSION_REQUIRED,
                "native service reported a truthful permission limitation"
            );
            assert_ne!(
                current.index_status, INDEX_STATUS_ERROR,
                "native service reported a source error"
            );
            assert_ne!(
                current.index_status, INDEX_STATUS_REBUILD_REQUIRED,
                "native MFT baseline is incomplete and requires rebuild"
            );
            thread::sleep(Duration::from_millis(100));
        };
        assert!(
            ready_source.entry_count > 0,
            "ready fixed NTFS baseline must emit searchable entries"
        );
        assert!(
            run.database()
                .search_global_entries(&baseline_name, 20, 0)
                .expect("search pre-existing baseline fixture")
                .iter()
                .any(|entry| Path::new(&entry.path) == baseline_path),
            "pre-existing fixture must be searchable from the initial MFT baseline"
        );
        let candidate_status = run
            .database()
            .global_index_status()
            .expect("read isolated candidate index status");
        assert!(candidate_status.enabled);
        assert_eq!(candidate_status.ready_volumes, 1);
        assert!(matches!(
            candidate_status.status.as_str(),
            "ready" | "partial"
        ));

        let original_name = format!("zb05-native-create-{run_id}.txt");
        let renamed_name = format!("zb05-native-rename-{run_id}.txt");
        // The direct-provider smoke uses its pre-existing fixture above to
        // require a complete initial baseline before testing native wakes.
        let original_path = Path::new(&source_mount).join(&original_name);
        let renamed_path = Path::new(&source_mount).join(&renamed_name);
        let baseline_ready_ms = baseline_started.elapsed().as_millis();

        wait_for_wake_idle(&run.coordinator().wake, Duration::from_secs(15));
        let before_create_wake = run.coordinator().wake.snapshot();
        let create_started = Instant::now();
        assert!(!original_path.exists(), "unique smoke path must be unused");
        run.own_file(original_path.clone());
        fs::write(&original_path, b"ZB-05 task-owned metadata smoke fixture\n")
            .expect("create task-owned native smoke file");
        wait_for_provider_signal(
            &run.coordinator().wake,
            before_create_wake.delivered,
            EVENT_TIMEOUT,
        );
        wait_for_search_path(
            run.database(),
            &source_id,
            &original_name,
            &original_path,
            EVENT_TIMEOUT,
        );
        let create_to_search = create_started.elapsed();

        let before_rename_wake = run.coordinator().wake.snapshot();
        let rename_started = Instant::now();
        assert!(!renamed_path.exists(), "unique rename path must be unused");
        run.own_file(renamed_path.clone());
        fs::rename(&original_path, &renamed_path).expect("rename task-owned native smoke file");
        wait_for_provider_signal(
            &run.coordinator().wake,
            before_rename_wake.delivered,
            EVENT_TIMEOUT,
        );
        wait_for_search_path(
            run.database(),
            &source_id,
            &renamed_name,
            &renamed_path,
            EVENT_TIMEOUT,
        );
        assert!(
            run.database()
                .search_global_entries(&original_name, 20, 0)
                .expect("search old native path")
                .is_empty(),
            "renamed metadata must no longer be returned under its old name"
        );
        let rename_to_search = rename_started.elapsed();

        let before_delete_wake = run.coordinator().wake.snapshot();
        let delete_started = Instant::now();
        fs::remove_file(&renamed_path).expect("delete task-owned native smoke file");
        wait_for_provider_signal(
            &run.coordinator().wake,
            before_delete_wake.delivered,
            EVENT_TIMEOUT,
        );
        wait_for_search_absent(run.database(), &renamed_name, EVENT_TIMEOUT);
        assert!(
            run.database()
                .global_entry_for_path(&renamed_path)
                .expect("read deleted native path")
                .is_none(),
            "deleted metadata must not remain active at its old path"
        );
        let delete_to_search = delete_started.elapsed();

        wait_for_wake_idle(&run.coordinator().wake, Duration::from_secs(15));
        let idle_source = source_idle_signature(run.database(), &source_id);
        let idle_wake_waits = run.coordinator().wake.snapshot().waits;
        let idle_started = Instant::now();
        thread::sleep(Duration::from_secs(5));
        assert_eq!(
            source_idle_signature(run.database(), &source_id),
            idle_source,
            "settled idle must not rewrite source status/checkpoint"
        );
        assert_eq!(
            run.coordinator().wake.snapshot().waits,
            idle_wake_waits,
            "settled idle must not run another coordinator cycle"
        );

        eprintln!(
            "ZB05_NATIVE_SMOKE source={} baseline_ready_ms={} indexed_entries={} create_to_search_ms={} rename_to_search_ms={} delete_to_search_ms={} idle_window_ms={} idle_wait_delta=0 provider_route=direct_mft_usn_fallback",
            source_mount,
            baseline_ready_ms,
            ready_source.entry_count,
            create_to_search.as_millis(),
            rename_to_search.as_millis(),
            delete_to_search.as_millis(),
            idle_started.elapsed().as_millis(),
        );
    }

    #[cfg(target_os = "windows")]
    fn wait_for_provider_signal(
        wake: &GlobalIndexWakeSlot,
        delivered_before: u64,
        timeout: Duration,
    ) {
        let deadline = Instant::now() + timeout;
        while wake.snapshot().delivered <= delivered_before && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            wake.snapshot().delivered > delivered_before,
            "native provider change did not wake the coordinator"
        );
    }

    #[cfg(target_os = "windows")]
    fn wait_for_search_path(
        db: &Database,
        source_id: &str,
        query: &str,
        path: &Path,
        timeout: Duration,
    ) {
        let deadline = Instant::now() + timeout;
        loop {
            let results = db
                .search_global_entries(query, 20, 0)
                .expect("query candidate Global Search index");
            if results.iter().any(|entry| {
                entry.name
                    == Path::new(path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                    && Path::new(&entry.path) == path
            }) {
                return;
            }
            if Instant::now() >= deadline {
                let source = db.get_global_volume(source_id).ok().flatten();
                let source_details = source.map_or_else(
                    || "source missing".to_string(),
                    |source| {
                        format!(
                            "status={} error={:?} journal_id={:?} cursor={:?} entries={} last_incremental_sync_at={:?}",
                            source.index_status,
                            source.last_error,
                            source.journal_id,
                            source.journal_cursor,
                            source.entry_count,
                            source.last_incremental_sync_at
                        )
                    },
                );
                let result_details = results
                    .iter()
                    .take(5)
                    .map(|entry| format!("{} @ {}", entry.name, entry.path))
                    .collect::<Vec<_>>();
                panic!(
                    "Global Search did not find task-owned native path: {}; source={}; query_results={result_details:?}",
                    path.display(),
                    source_details
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[cfg(target_os = "windows")]
    fn wait_for_search_absent(db: &Database, query: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let results = db
                .search_global_entries(query, 20, 0)
                .expect("query candidate Global Search index");
            if results.is_empty() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "deleted task-owned file is still returned by Global Search"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[cfg(target_os = "windows")]
    fn wait_for_wake_idle(wake: &GlobalIndexWakeSlot, stable_for: Duration) {
        let deadline = Instant::now() + stable_for + Duration::from_secs(15);
        let mut last_waits = wake.snapshot().waits;
        let mut stable_since = Instant::now();
        while Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
            let waits = wake.snapshot().waits;
            if waits != last_waits {
                last_waits = waits;
                stable_since = Instant::now();
            } else if Instant::now().duration_since(stable_since) >= stable_for {
                return;
            }
        }
        panic!(
            "Global Index coordinator did not settle into its blocking wait: {:?}",
            wake.snapshot()
        );
    }

    #[cfg(target_os = "windows")]
    fn source_idle_signature(db: &Database, source_id: &str) -> (String, Option<i64>, i64) {
        let source = db
            .get_global_volume(source_id)
            .expect("read idle source")
            .expect("idle source remains available");
        (
            source.index_status,
            source.last_incremental_sync_at,
            source.updated_at,
        )
    }

    struct EmptyMftBaselineProvider {
        source: GlobalSourceDescriptor,
    }

    impl GlobalIndexProvider for EmptyMftBaselineProvider {
        fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
            Ok(vec![self.source.clone()])
        }

        fn start_initial_index(
            &self,
            source: &GlobalSourceDescriptor,
            sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            const ERROR: &str = "windows_mft_baseline_empty: staged_records=0 resolved_entries=0 emitted_entries=0 checkpoint_usn=123";
            sink.set_source_state(
                &source.volume.id,
                INDEX_STATUS_REBUILD_REQUIRED,
                Some(ERROR),
            )?;
            Err(GlobalIndexError::Provider(ERROR.to_string()))
        }

        fn resume_incremental_sync(
            &self,
            _source: &GlobalSourceDescriptor,
            _sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            unreachable!("initial source has no completed baseline")
        }

        fn pause(&self) -> Result<(), GlobalIndexError> {
            Ok(())
        }

        fn status(&self) -> Result<String, GlobalIndexError> {
            Ok("test".to_string())
        }

        fn shutdown(&self) -> Result<(), GlobalIndexError> {
            Ok(())
        }
    }

    #[test]
    fn coordinator_preserves_rebuild_required_after_empty_mft_baseline() {
        let path = TestDatabasePath::new();
        let db = Database::open(&path.0).expect("open test database");
        let source = GlobalSourceDescriptor {
            volume: GlobalVolume {
                id: "gv_empty_mft_baseline".to_string(),
                platform: "windows".to_string(),
                stable_volume_id: "empty-mft-baseline".to_string(),
                display_name: "C".to_string(),
                mount_path: "C:\\".to_string(),
                filesystem_type: "ntfs".to_string(),
                drive_kind: "fixed".to_string(),
                enabled: true,
                provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
                index_status: INDEX_STATUS_DISCOVERED.to_string(),
                last_error: None,
                journal_id: None,
                journal_cursor: None,
                last_full_index_at: None,
                last_incremental_sync_at: None,
                entry_count: 0,
                created_at: 1,
                updated_at: 1,
            },
        };
        db.upsert_global_volume(&source.volume)
            .expect("seed fixed NTFS source");
        let provider = EmptyMftBaselineProvider { source };
        let cancel = Arc::new(AtomicBool::new(false));
        let wake = GlobalIndexWakeSlot::default();

        run_index_cycle(&provider, &db, &cancel, &wake)
            .expect("provider error is recorded on the durable source");

        let stored = db
            .get_global_volume("gv_empty_mft_baseline")
            .expect("read durable source")
            .expect("source remains present");
        assert_eq!(stored.index_status, INDEX_STATUS_REBUILD_REQUIRED);
        assert!(stored
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("windows_mft_baseline_empty")));
        assert_ne!(stored.index_status, INDEX_STATUS_READY);
        assert!(stored.last_full_index_at.is_none());

        drop(db);
        drop(path);
    }
    include!("coordinator_recovery_tests.rs");
}
