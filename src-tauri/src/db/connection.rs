use super::schema::migrate;
use super::*;
use crate::global_index::filename_search::FilenameSearchManager;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, ErrorCode};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::SyncSender,
        Arc, Mutex, OnceLock, Weak,
    },
    thread,
    time::{Duration, Instant},
};

const LIBRARY_COUNT_CACHE_MAX_ENTRIES: usize = 32;
const GLOBAL_FTS_RECOVERY_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
const GLOBAL_FTS_RECOVERY_CANCEL_POLL: Duration = Duration::from_millis(50);
static ACTIVE_GLOBAL_FTS_RECOVERIES: OnceLock<Mutex<HashMap<PathBuf, Arc<GlobalFtsRecoverySlot>>>> =
    OnceLock::new();

#[derive(Default)]
struct InitialIndexWalCheckpointState {
    active_sessions: AtomicUsize,
    policy_lock: Mutex<()>,
}

pub(crate) struct InitialIndexWalCheckpointGuard {
    state: Arc<InitialIndexWalCheckpointState>,
}

impl InitialIndexWalCheckpointGuard {
    fn new(state: Arc<InitialIndexWalCheckpointState>) -> Self {
        let _policy = state
            .policy_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active_sessions.fetch_add(1, Ordering::AcqRel);
        Self { state }
    }
}

impl Drop for InitialIndexWalCheckpointGuard {
    fn drop(&mut self) {
        let _policy = self
            .state
            .policy_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.state.active_sessions.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Pooled lease which temporarily disables connection-local auto-checkpointing
/// during a large initial index. The original threshold is restored before the
/// connection returns to r2d2, so pooled handles cannot retain stale policy.
pub(crate) struct PooledSqliteConnection {
    connection: PooledConnection<SqliteConnectionManager>,
    restore_wal_autocheckpoint_pages: Option<i64>,
}

impl std::ops::Deref for PooledSqliteConnection {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

impl std::ops::DerefMut for PooledSqliteConnection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}

impl Drop for PooledSqliteConnection {
    fn drop(&mut self) {
        if let Some(previous_pages) = self.restore_wal_autocheckpoint_pages.take() {
            if let Err(error) =
                self.connection
                    .pragma_update(None, "wal_autocheckpoint", previous_pages)
            {
                log::warn!(
                    target: "db::connection",
                    "failed to restore pooled SQLite WAL auto-checkpoint policy: {error}"
                );
            }
        }
    }
}

#[cfg(feature = "native-qa")]
#[derive(Default)]
struct DiagnosticPoolSampler {
    stop: Arc<AtomicBool>,
}

#[cfg(feature = "native-qa")]
impl Drop for DiagnosticPoolSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[derive(Default)]
struct GlobalFtsRecoverySlot {
    active_session: Mutex<Weak<()>>,
    #[cfg(test)]
    attempts: AtomicUsize,
}

impl GlobalFtsRecoverySlot {
    fn bind_to_session(&self, session: &Arc<()>) {
        *self
            .active_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::downgrade(session);
    }

    fn session_is_active(&self) -> bool {
        self.active_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .strong_count()
            > 0
    }
}

enum GlobalFtsRecoveryOutcome {
    Completed { attempts: usize },
    Cancelled { attempts: usize },
    Failed { attempts: usize, error: DbError },
    RetryLimitReached { attempts: usize, error: DbError },
}

fn remove_active_global_fts_recovery(path: &Path, slot: &Arc<GlobalFtsRecoverySlot>) {
    let Some(active_recoveries) = ACTIVE_GLOBAL_FTS_RECOVERIES.get() else {
        return;
    };
    let mut active = active_recoveries
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if active
        .get(path)
        .is_some_and(|current| Arc::ptr_eq(current, slot))
    {
        active.remove(path);
    }
}

fn run_global_fts_recovery_with_retry(
    initial_delay: Duration,
    retry_delays: &[Duration],
    session_is_active: impl Fn() -> bool,
    recover: &mut impl FnMut() -> Result<(), DbError>,
) -> GlobalFtsRecoveryOutcome {
    if !wait_for_global_fts_recovery_session(initial_delay, &session_is_active) {
        return GlobalFtsRecoveryOutcome::Cancelled { attempts: 0 };
    }

    let max_attempts = retry_delays.len().saturating_add(1);
    for attempt in 1..=max_attempts {
        if !session_is_active() {
            return GlobalFtsRecoveryOutcome::Cancelled {
                attempts: attempt.saturating_sub(1),
            };
        }

        match recover() {
            Ok(()) if session_is_active() => {
                return GlobalFtsRecoveryOutcome::Completed { attempts: attempt };
            }
            Ok(()) => return GlobalFtsRecoveryOutcome::Cancelled { attempts: attempt },
            Err(_error) if !session_is_active() => {
                return GlobalFtsRecoveryOutcome::Cancelled { attempts: attempt };
            }
            Err(error) if !is_retryable_global_fts_recovery_error(&error) => {
                return GlobalFtsRecoveryOutcome::Failed {
                    attempts: attempt,
                    error,
                };
            }
            Err(error) => {
                crate::global_index::qa_trace::record_db_error("global_fts_recovery.retry", &error);
                let Some(delay) = retry_delays.get(attempt - 1).copied() else {
                    return GlobalFtsRecoveryOutcome::RetryLimitReached {
                        attempts: attempt,
                        error,
                    };
                };
                log::warn!(
                    target: "global_index::repository",
                    "temporary error during background global FTS recovery; retrying in {}ms attempt={attempt}/{max_attempts}: {error}",
                    delay.as_millis(),
                );
                if !wait_for_global_fts_recovery_session(delay, &session_is_active) {
                    return GlobalFtsRecoveryOutcome::Cancelled { attempts: attempt };
                }
            }
        }
    }

    unreachable!("the bounded FTS recovery loop returns on its last attempt")
}

fn wait_for_global_fts_recovery_session(
    delay: Duration,
    session_is_active: &impl Fn() -> bool,
) -> bool {
    let deadline = Instant::now() + delay;
    while session_is_active() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        thread::sleep(remaining.min(GLOBAL_FTS_RECOVERY_CANCEL_POLL));
    }
    false
}

fn is_retryable_global_fts_recovery_error(error: &DbError) -> bool {
    match error {
        DbError::Sqlite(rusqlite::Error::SqliteFailure(code, _)) => matches!(
            code.code,
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
        ),
        // r2d2's pool error represents a timed-out lease acquisition.
        DbError::Pool(_) => true,
        DbError::Io(error) => matches!(
            error.kind(),
            std::io::ErrorKind::Interrupted
                | std::io::ErrorKind::WouldBlock
                | std::io::ErrorKind::TimedOut
        ),
        _ => false,
    }
}

struct LibraryCountCacheEntry {
    revision: i64,
    membership_fingerprint: String,
    total_count: i64,
}

#[derive(Clone, Default)]
struct ManagedAiWakeSlot {
    sender: Arc<Mutex<Option<SyncSender<()>>>>,
    work_wakes_armed: Arc<AtomicBool>,
}

impl ManagedAiWakeSlot {
    fn set(&self, sender: SyncSender<()>) {
        self.set_work_wakes_armed(false);
        *self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sender);
    }

    fn set_work_wakes_armed(&self, armed: bool) {
        self.work_wakes_armed.store(armed, Ordering::Release);
    }

    fn notify_control(&self) {
        self.send_wake();
    }

    fn notify_work(&self) {
        if self.work_wakes_armed.load(Ordering::Acquire) {
            self.send_wake();
        }
    }

    fn send_wake(&self) {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(sender) = sender {
            // A full one-slot channel already contains a wake. The durable SQLite
            // queue remains the source of work; this signal only asks the worker
            // to inspect it.
            let _ = sender.try_send(());
        }
    }
}

#[derive(Clone)]
pub struct Database {
    path: PathBuf,
    pool: Pool<SqliteConnectionManager>,
    global_fts_recovery_session: Arc<()>,
    pub(crate) filename_search: Arc<FilenameSearchManager>,
    library_count_cache: Arc<Mutex<VecDeque<LibraryCountCacheEntry>>>,
    managed_ai_waker: ManagedAiWakeSlot,
    pub(crate) automation_wake: Arc<Mutex<Option<std::sync::Weak<AutomationWake>>>>,
    initial_index_wal_checkpoint_state: Arc<InitialIndexWalCheckpointState>,
    #[cfg(feature = "native-qa")]
    diagnostic_pool_sampler: Arc<DiagnosticPoolSampler>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        Self::open_with_upgrade_cancellation(path, Arc::new(AtomicBool::new(false)))
    }

    pub fn open_with_upgrade_cancellation(
        path: impl AsRef<Path>,
        cancel: super::upgrade::UpgradeCancellation,
    ) -> Result<Self, DbError> {
        Self::open_with_connection_hook_policy(path, cancel, None, Duration::ZERO)
    }

    #[cfg(test)]
    pub(crate) fn open_for_writer_qualification(
        path: impl AsRef<Path>,
        install_candidate_hooks: bool,
    ) -> Result<Self, DbError> {
        Self::open_with_connection_hook_policy(
            path,
            Arc::new(AtomicBool::new(false)),
            Some(install_candidate_hooks),
            Duration::ZERO,
        )
    }

    #[cfg(test)]
    pub(crate) fn open_with_delayed_fts_recovery_for_test(
        path: impl AsRef<Path>,
        recovery_delay: Duration,
    ) -> Result<Self, DbError> {
        Self::open_with_connection_hook_policy(
            path,
            Arc::new(AtomicBool::new(false)),
            None,
            recovery_delay,
        )
    }

    fn open_with_connection_hook_policy(
        path: impl AsRef<Path>,
        cancel: super::upgrade::UpgradeCancellation,
        _test_hook_policy: Option<bool>,
        recovery_delay: Duration,
    ) -> Result<Self, DbError> {
        super::upgrade::check_cancelled(&cancel)?;
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let filename_search = Arc::new(FilenameSearchManager::new(&path));
        #[cfg(test)]
        if let Some(enabled) = _test_hook_policy {
            filename_search.set_enabled_for_test(enabled);
        }
        let hook_manager = Arc::clone(&filename_search);
        let manager = SqliteConnectionManager::file(&path).with_init(move |connection| {
            configure_connection(connection)?;
            #[cfg(test)]
            let install_hooks =
                _test_hook_policy.unwrap_or_else(|| hook_manager.should_install_mutation_hooks());
            #[cfg(not(test))]
            let install_hooks = hook_manager.should_install_mutation_hooks();
            if install_hooks {
                hook_manager.install_mutation_hooks(connection)?;
            }
            Ok(())
        });
        let pool = Pool::builder()
            .max_size(8)
            .min_idle(Some(1))
            .build(manager)?;
        {
            let conn = pool.get()?;
            super::schema::migrate_with_cancellation(&conn, cancel)?;
        }

        let database = Self {
            path,
            pool,
            global_fts_recovery_session: Arc::new(()),
            filename_search,
            library_count_cache: Arc::new(Mutex::new(VecDeque::new())),
            managed_ai_waker: ManagedAiWakeSlot::default(),
            automation_wake: Arc::new(Mutex::new(None)),
            initial_index_wal_checkpoint_state: Arc::new(InitialIndexWalCheckpointState::default()),
            #[cfg(feature = "native-qa")]
            diagnostic_pool_sampler: Arc::new(DiagnosticPoolSampler::default()),
        };
        #[cfg(feature = "native-qa")]
        database.start_diagnostic_pool_sampler();
        database.schedule_pending_global_fts_recovery(recovery_delay);
        Ok(database)
    }

    #[cfg(feature = "native-qa")]
    fn start_diagnostic_pool_sampler(&self) {
        if !crate::global_index::qa_trace::is_enabled() {
            return;
        }
        static NEXT_POOL_ID: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(1);
        let pool_id = NEXT_POOL_ID.fetch_add(1, Ordering::Relaxed);
        let pool = self.pool.clone();
        let stop = Arc::clone(&self.diagnostic_pool_sampler.stop);
        let spawn_result = thread::Builder::new()
            .name("global-index-qa-db-pool-sampler".into())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let state = pool.state();
                    crate::global_index::qa_trace::record(&format!(
                        "sqlite_connection_sample pool_id={pool_id} connections={} idle_connections={} active_connections={} active_transactions={}",
                        state.connections,
                        state.idle_connections,
                        state.connections.saturating_sub(state.idle_connections),
                        crate::global_index::qa_trace::active_transaction_count(),
                    ));
                    thread::sleep(Duration::from_millis(100));
                }
            });
        if let Err(error) = spawn_result {
            crate::global_index::qa_trace::record(&format!(
                "sqlite_connection_sample pool_id={pool_id} spawn_error={error:?}"
            ));
        }
    }

    fn schedule_pending_global_fts_recovery(&self, recovery_delay: Duration) {
        let pending = self.conn().and_then(|connection| {
            connection
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM global_volumes
                         WHERE fts_bulk_build=1 AND enabled=1
                           AND (platform!='windows' OR scope_authorization_state='authorized')
                     )",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(DbError::from)
        });
        match pending {
            Ok(false) => return,
            Err(error) if is_retryable_global_fts_recovery_error(&error) => {
                crate::global_index::qa_trace::record_db_error(
                    "database_open.pending_fts_probe",
                    &error,
                );
                log::warn!(
                    target: "global_index::repository",
                    "temporary error checking for pending FTS recovery during database open; checking again in the background: {error}"
                );
            }
            Err(error) => {
                crate::global_index::qa_trace::record_db_error(
                    "database_open.pending_fts_probe",
                    &error,
                );
                log::warn!(
                    target: "global_index::repository",
                    "could not check for pending FTS recovery during database open; recovery will not be scheduled for this error: {error}"
                );
                return;
            }
            Ok(true) => {}
        }

        let active_recoveries = ACTIVE_GLOBAL_FTS_RECOVERIES.get_or_init(Default::default);
        let recovery_slot = {
            let mut active = active_recoveries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(slot) = active.get(&self.path) {
                // A second Database handle for the same file joins the existing
                // recovery session. This also lets a reopened app session take
                // over while an old bounded recovery attempt is winding down.
                slot.bind_to_session(&self.global_fts_recovery_session);
                return;
            }
            let slot = Arc::new(GlobalFtsRecoverySlot::default());
            slot.bind_to_session(&self.global_fts_recovery_session);
            active.insert(self.path.clone(), Arc::clone(&slot));
            slot
        };

        // Keep a pool handle for the bounded worker without keeping the app
        // session token alive. When the last app-owned Database handle drops,
        // the worker observes cancellation between FTS batches/retries.
        let mut database = self.clone();
        database.global_fts_recovery_session = Arc::new(());
        let recovery_path = self.path.clone();
        let worker_slot = Arc::clone(&recovery_slot);
        let spawn_result = thread::Builder::new()
            .name("global-index-fts-recovery".into())
            .spawn(move || {
                let session_is_active = || worker_slot.session_is_active();
                let mut recover = || {
                    #[cfg(test)]
                    worker_slot.attempts.fetch_add(1, Ordering::SeqCst);
                    let cancelled = || !session_is_active();
                    database.recover_pending_global_fts_bulk_builds_with_cancellation(cancelled)
                };
                let outcome = run_global_fts_recovery_with_retry(
                    recovery_delay,
                    &GLOBAL_FTS_RECOVERY_RETRY_DELAYS,
                    session_is_active,
                    &mut recover,
                );
                match outcome {
                    GlobalFtsRecoveryOutcome::Completed { attempts } => log::info!(
                        target: "global_index::repository",
                        "background pending global FTS recovery completed attempts={attempts}"
                    ),
                    GlobalFtsRecoveryOutcome::Cancelled { attempts } => log::debug!(
                        target: "global_index::repository",
                        "background pending global FTS recovery cancelled attempts={attempts}"
                    ),
                    GlobalFtsRecoveryOutcome::Failed { attempts, error } => log::warn!(
                        target: "global_index::repository",
                        "background pending global FTS recovery stopped after non-retryable error attempts={attempts}: {error}"
                    ),
                    GlobalFtsRecoveryOutcome::RetryLimitReached { attempts, error } => {
                        log::error!(
                            target: "global_index::repository",
                            "background pending global FTS recovery exhausted same-session retries attempts={attempts}; pending source remains hidden until a later recovery attempt: {error}"
                        )
                    }
                }
                remove_active_global_fts_recovery(&recovery_path, &worker_slot);
            });
        if let Err(error) = spawn_result {
            remove_active_global_fts_recovery(&self.path, &recovery_slot);
            log::warn!(
                target: "global_index::repository",
                "could not start background global FTS recovery; it will retry on next open: {error}"
            );
        }
    }

    pub(crate) fn set_managed_ai_waker(&self, sender: SyncSender<()>) {
        self.managed_ai_waker.set(sender);
    }

    /// Force the worker to re-read durable settings or policy state.
    pub(crate) fn notify_managed_ai_control_wake(&self) {
        self.managed_ai_waker.notify_control();
    }

    pub(crate) fn set_managed_ai_work_wakes_armed(&self, armed: bool) {
        self.managed_ai_waker.set_work_wakes_armed(armed);
    }

    /// Signal newly eligible durable work only while the worker is interested.
    pub(crate) fn notify_managed_ai_work(&self) {
        self.managed_ai_waker.notify_work();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn init(&self) -> Result<(), DbError> {
        let conn = self.conn()?;
        migrate(&conn)
    }

    pub(crate) fn conn(&self) -> Result<PooledSqliteConnection, DbError> {
        let connection = self.pool.get().map_err(DbError::from)?;
        let _policy = self
            .initial_index_wal_checkpoint_state
            .policy_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let restore_wal_autocheckpoint_pages = if self
            .initial_index_wal_checkpoint_state
            .active_sessions
            .load(Ordering::Acquire)
            > 0
        {
            let previous_pages = connection
                .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get::<_, i64>(0))?;
            if previous_pages == 0 {
                None
            } else {
                connection.pragma_update(None, "wal_autocheckpoint", 0)?;
                Some(previous_pages)
            }
        } else {
            None
        };
        drop(_policy);
        #[cfg(feature = "native-qa")]
        if crate::global_index::qa_trace::is_enabled() {
            let state = self.pool.state();
            let active = state.connections.saturating_sub(state.idle_connections);
            let wal_autocheckpoint_pages = connection
                .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get::<_, i64>(0))
                .unwrap_or(-1);
            crate::global_index::qa_trace::record(&format!(
                "sqlite_connections event=acquire connections={} idle_connections={} active_connections={active} wal_autocheckpoint_pages={wal_autocheckpoint_pages}",
                state.connections,
                state.idle_connections,
            ));
        }
        Ok(PooledSqliteConnection {
            connection,
            restore_wal_autocheckpoint_pages,
        })
    }

    pub(crate) fn begin_initial_index_wal_checkpoint_session(
        &self,
    ) -> InitialIndexWalCheckpointGuard {
        InitialIndexWalCheckpointGuard::new(Arc::clone(&self.initial_index_wal_checkpoint_state))
    }

    /// Pool state is sampled only by native qualification. `active` counts
    /// checked-out leases; neither this method nor the trace path executes SQL.
    #[cfg(all(target_os = "windows", feature = "native-qa"))]
    pub(crate) fn diagnostic_connection_counts(&self) -> (u32, u32, u32) {
        let state = self.pool.state();
        (
            state.connections,
            state.idle_connections,
            state.connections.saturating_sub(state.idle_connections),
        )
    }

    #[cfg(test)]
    pub(crate) fn fts_recovery_attempt_count_for_test(&self) -> usize {
        ACTIVE_GLOBAL_FTS_RECOVERIES
            .get()
            .and_then(|active| {
                active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(&self.path)
                    .map(|slot| slot.attempts.load(Ordering::SeqCst))
            })
            .unwrap_or_default()
    }

    /// Verify the normal timeout, then set the test fixture timeout on every pooled connection.
    #[cfg(test)]
    pub(crate) fn set_test_busy_timeout_for_all_connections(
        &self,
        expected_current_ms: i64,
        timeout: std::time::Duration,
    ) -> Result<(), DbError> {
        let pool_size = self.pool.max_size();
        let mut connections = Vec::with_capacity(pool_size as usize);
        for _ in 0..pool_size {
            connections.push(self.conn()?);
        }

        let timeout_ms = timeout.as_millis() as i64;
        for connection in &mut connections {
            let current_ms =
                connection.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))?;
            if current_ms != expected_current_ms {
                return Err(DbError::Validation(format!(
                    "test database busy timeout was {current_ms}ms, expected {expected_current_ms}ms before override"
                )));
            }

            connection.busy_timeout(timeout)?;
            let configured_ms =
                connection.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))?;
            if configured_ms != timeout_ms {
                return Err(DbError::Validation(format!(
                    "test database busy timeout override was {configured_ms}ms, expected {timeout_ms}ms"
                )));
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn for_each_test_connection(
        &self,
        mut action: impl FnMut(&Connection) -> rusqlite::Result<()>,
    ) -> Result<(), DbError> {
        let pool_size = self.pool.max_size();
        let mut connections = Vec::with_capacity(pool_size as usize);
        for _ in 0..pool_size {
            connections.push(self.conn()?);
        }
        for connection in &connections {
            action(connection)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_test_busy_handler_for_all_connections(
        &self,
        handler: fn(i32) -> bool,
    ) -> Result<(), DbError> {
        let pool_size = self.pool.max_size();
        let mut connections = Vec::with_capacity(pool_size as usize);
        for _ in 0..pool_size {
            connections.push(self.conn()?);
        }
        for connection in &mut connections {
            connection.busy_handler(Some(handler))?;
        }
        Ok(())
    }

    pub(crate) fn cached_library_count(
        &self,
        revision: i64,
        membership_fingerprint: &str,
    ) -> Option<i64> {
        let mut cache = self
            .library_count_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = cache.iter().position(|entry| {
            entry.revision == revision && entry.membership_fingerprint == membership_fingerprint
        })?;
        let entry = cache.remove(index)?;
        let total_count = entry.total_count;
        cache.push_back(entry);
        Some(total_count)
    }

    pub(crate) fn cache_library_count(
        &self,
        revision: i64,
        membership_fingerprint: String,
        total_count: i64,
    ) {
        let mut cache = self
            .library_count_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.retain(|entry| {
            entry.revision != revision || entry.membership_fingerprint != membership_fingerprint
        });
        cache.push_back(LibraryCountCacheEntry {
            revision,
            membership_fingerprint,
            total_count,
        });
        while cache.len() > LIBRARY_COUNT_CACHE_MAX_ENTRIES {
            cache.pop_front();
        }
    }
}

fn configure_connection(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 3_000_000_000_i64)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

#[cfg(test)]
mod pool_tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDatabaseDirectory(PathBuf);

    impl TestDatabaseDirectory {
        fn new() -> Self {
            let counter = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(".tmp-tests")
                .join("zb-01-pool-tests")
                .join(format!("{}-{nonce}-{counter}", std::process::id()));
            fs::create_dir_all(&path).expect("create test database directory");
            Self(path)
        }

        fn database_path(&self) -> PathBuf {
            self.0.join("database.sqlite3")
        }
    }

    impl Drop for TestDatabaseDirectory {
        fn drop(&mut self) {
            let mut last_error = None;
            for attempt in 0..20 {
                match fs::remove_dir_all(&self.0) {
                    Ok(()) => return,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                    Err(error) => last_error = Some(error),
                }
                if attempt < 19 {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            if let Some(error) = last_error {
                eprintln!(
                    "failed to remove test database directory {:?}: {error}",
                    self.0
                );
            }
        }
    }

    #[test]
    fn managed_ai_wake_slot_buffers_and_coalesces_wakes() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let wake_slot = ManagedAiWakeSlot::default();
        wake_slot.set(sender);

        // Disarmed ordinary work signals do not wake a worker that has read that
        // AI is disabled.
        wake_slot.notify_work();
        assert_eq!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );

        // A producer can signal after the worker's last queue check but before it
        // begins waiting; the buffered token makes that transition race-safe.
        wake_slot.set_work_wakes_armed(true);
        wake_slot.notify_work();
        wake_slot.notify_work();
        assert_eq!(receiver.try_recv(), Ok(()));
        assert_eq!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );

        // Separate clones share the same bounded, payload-free notification slot.
        let cloned_slot = wake_slot.clone();
        cloned_slot.set_work_wakes_armed(false);
        cloned_slot.notify_work();
        assert_eq!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );
        cloned_slot.notify_control();
        assert_eq!(receiver.try_recv(), Ok(()));
    }

    #[test]
    fn database_open_uses_minimum_idle_configuration_and_runs_migrations() {
        let test_dir = TestDatabaseDirectory::new();
        let db = Database::open(test_dir.database_path()).expect("open and migrate database");
        let state = db.pool.state();

        eprintln!(
            "pool state after Database::open: connections={}, idle_connections={}, max_size={}, min_idle={:?}",
            state.connections,
            state.idle_connections,
            db.pool.max_size(),
            db.pool.min_idle()
        );
        assert_eq!(db.pool.max_size(), 8);
        assert_eq!(db.pool.min_idle(), Some(1));

        assert!((1..=2).contains(&state.connections));
        assert_eq!(state.idle_connections, state.connections);
        assert!(state.connections < db.pool.max_size());

        let conn = db.conn().expect("get connection after migration");
        let files_table_count = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'files'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("query migrated schema");
        assert_eq!(files_table_count, 1);
        let query_result = conn
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .expect("run ordinary query");
        assert_eq!(query_result, 1);
    }

    #[test]
    fn database_pool_grows_for_multiple_leases_and_reuses_returned_connections() {
        let test_dir = TestDatabaseDirectory::new();
        let db = Database::open(test_dir.database_path()).expect("open and migrate database");

        let first = db.conn().expect("lease first connection");
        let second = db.conn().expect("lease second connection");
        let third = db.conn().expect("lease third connection");
        let held_state = db.pool.state();

        eprintln!(
            "pool state with three leases held: connections={}, idle_connections={}, max_size={}, min_idle={:?}",
            held_state.connections,
            held_state.idle_connections,
            db.pool.max_size(),
            db.pool.min_idle()
        );
        assert_eq!(db.pool.max_size(), 8);
        assert_eq!(db.pool.min_idle(), Some(1));
        assert!(held_state.connections >= 3);
        assert!(held_state.connections <= db.pool.max_size());
        assert!(held_state.idle_connections < held_state.connections);

        drop((first, second, third));
        let returned_state = db.pool.state();
        eprintln!(
            "pool state after returning three leases: connections={}, idle_connections={}",
            returned_state.connections, returned_state.idle_connections
        );
        assert!(returned_state.idle_connections >= 1);
        assert!(returned_state.connections <= db.pool.max_size());

        let conn = db.conn().expect("lease connection after returning leases");
        let query_result = conn
            .query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
            .expect("query using returned pool connection");
        assert_eq!(query_result, 42);
    }

    #[test]
    fn initial_index_autocheckpoint_suppression_is_scoped_and_restored_per_lease() {
        let test_dir = TestDatabaseDirectory::new();
        let db = Database::open(test_dir.database_path()).expect("open and migrate database");
        let wal_autocheckpoint = |connection: &Connection| {
            connection
                .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get::<_, i64>(0))
                .expect("read the connection-local autocheckpoint threshold")
        };

        {
            let _session = db.begin_initial_index_wal_checkpoint_session();
            let first = db.conn().expect("lease first suppressed connection");
            let second = db.conn().expect("lease second suppressed connection");
            assert_eq!(wal_autocheckpoint(&first), 0);
            assert_eq!(wal_autocheckpoint(&second), 0);
        }

        let first = db.conn().expect("lease first restored connection");
        let second = db.conn().expect("lease second restored connection");
        assert_eq!(wal_autocheckpoint(&first), 1_000);
        assert_eq!(wal_autocheckpoint(&second), 1_000);

        let session = db.begin_initial_index_wal_checkpoint_session();
        let held_across_session = db.conn().expect("lease during initial index");
        assert_eq!(wal_autocheckpoint(&held_across_session), 0);
        drop(session);
        assert_eq!(wal_autocheckpoint(&held_across_session), 0);
        drop(held_across_session);
        let restored_after_session = db.conn().expect("lease after active transaction ends");
        assert_eq!(wal_autocheckpoint(&restored_after_session), 1_000);
    }

    fn sqlite_busy_error() -> DbError {
        DbError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        ))
    }

    #[test]
    fn global_fts_recovery_retries_transient_busy_in_the_same_session() {
        let attempts = AtomicU64::new(0);
        let mut recover = || {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(sqlite_busy_error())
            } else {
                Ok(())
            }
        };
        let outcome = run_global_fts_recovery_with_retry(
            Duration::ZERO,
            &[Duration::ZERO],
            || true,
            &mut recover,
        );

        assert!(matches!(
            outcome,
            GlobalFtsRecoveryOutcome::Completed { attempts: 2 }
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn global_fts_recovery_stops_after_bounded_retries() {
        let attempts = AtomicU64::new(0);
        let mut recover = || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(sqlite_busy_error())
        };
        let outcome = run_global_fts_recovery_with_retry(
            Duration::ZERO,
            &[Duration::ZERO, Duration::ZERO],
            || true,
            &mut recover,
        );

        assert!(matches!(
            outcome,
            GlobalFtsRecoveryOutcome::RetryLimitReached { attempts: 3, .. }
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn global_fts_recovery_backoff_is_cancelled_when_session_ends() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_signal = Arc::clone(&cancelled);
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            cancel_signal.store(true, Ordering::Release);
        });
        let attempts = AtomicU64::new(0);
        let mut recover = || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(sqlite_busy_error())
        };
        let outcome = run_global_fts_recovery_with_retry(
            Duration::ZERO,
            &[Duration::from_secs(1)],
            || !cancelled.load(Ordering::Acquire),
            &mut recover,
        );
        cancel_thread.join().expect("cancel recovery test session");

        assert!(matches!(
            outcome,
            GlobalFtsRecoveryOutcome::Cancelled { attempts: 1 }
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
}
