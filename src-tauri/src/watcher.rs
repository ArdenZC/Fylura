use crate::{
    db::{current_unix_seconds, Database, DbError},
    dedupe::DedupeJobManager,
    path_filter::is_ignored_dir_name,
    scanner::ScanJobManager,
    settings::{AppSettings, ScanRootSetting, SearchRootSetting},
};
use notify::{
    event::{ModifyKind, RenameMode},
    recommended_watcher, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use serde::Serialize;
#[cfg(feature = "native-qa")]
use std::sync::atomic::AtomicUsize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use thiserror::Error;

const FILE_EVENT_NAME: &str = "fs-event";
const WATCHER_READY_EVENT_NAME: &str = "fs-watcher-ready";
const WATCHER_ERROR_EVENT_NAME: &str = "fs-watcher-error";
pub const WATCHER_RECONCILIATION_STATUS_EVENT_NAME: &str = "watcher-reconciliation-status";
pub const WATCHER_BACKEND_ENV: &str = "ZEN_CANVAS_BACKEND_WATCHER_RECONCILIATION";
const WATCHER_CHANNEL_CAPACITY: usize = 2048;
const WATCHER_BATCH_LIMIT: usize = 500;
const WATCHER_MAX_ATTEMPTS: usize = 8;
const WATCHER_RULE_MAX_ATTEMPTS: usize = 3;
const WATCHER_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];
const WATCHER_RULE_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_millis(250), Duration::from_millis(500)];
const WATCHER_COALESCE_WINDOW: Duration = Duration::from_millis(150);
#[cfg(feature = "native-qa")]
const NATIVE_QA_WATCHER_TRACE_LIMIT: usize = 256;
#[cfg(feature = "native-qa")]
static NATIVE_QA_WATCHER_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

fn native_qa_watcher_trace(event: &'static str, details: impl FnOnce() -> String) {
    #[cfg(feature = "native-qa")]
    {
        if std::env::var("ZC_NATIVE_QA_WATCHER_TRACE").as_deref() != Ok("1") {
            return;
        }
        let sequence = NATIVE_QA_WATCHER_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
        if sequence >= NATIVE_QA_WATCHER_TRACE_LIMIT {
            return;
        }
        let details = details().chars().take(900).collect::<String>();
        eprintln!("native_qa watcher event={event} {details}");
    }
    #[cfg(not(feature = "native-qa"))]
    {
        let _ = (event, details);
    }
}

#[cfg(feature = "native-qa")]
fn native_qa_watcher_trace_relative_path(path: &Path) -> Option<String> {
    let configured_root = std::env::var_os("ZC_NATIVE_QA_WATCHER_TRACE_ROOT")?;
    let configured_root = PathBuf::from(configured_root);
    if !configured_root.is_absolute() {
        return None;
    }
    let root = configured_root.canonicalize().unwrap_or(configured_root);
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let normalize = |path: &Path| {
        let value = path.to_string_lossy().replace('\\', "/");
        value.strip_prefix("//?/").unwrap_or(&value).to_string()
    };
    let root = normalize(&root).trim_end_matches('/').to_string();
    let path = normalize(&path);
    if path.eq_ignore_ascii_case(&root) {
        return Some(".".to_string());
    }
    let prefix = format!("{root}/");
    if path
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&prefix))
    {
        return path
            .get(prefix.len()..)
            .map(|relative| format!("<trace-root>/{relative}"));
    }
    None
}

#[cfg(feature = "native-qa")]
fn native_qa_watcher_trace_path_prefix(path: &Path) -> &'static str {
    let raw = path.to_string_lossy().replace('\\', "/");
    if raw.starts_with("//?/UNC/") {
        "extended_unc"
    } else if raw.starts_with("//?/") {
        "extended_length"
    } else if raw.starts_with("//") {
        "unc"
    } else {
        "ordinary"
    }
}

#[cfg(not(feature = "native-qa"))]
fn native_qa_watcher_trace_relative_path(_path: &Path) -> Option<String> {
    None
}

fn trace_native_qa_notify(source: &'static str, event: &notify::Result<Event>) {
    #[cfg(feature = "native-qa")]
    match event {
        Ok(event) => {
            let paths = event
                .paths
                .iter()
                .filter_map(|path| native_qa_watcher_trace_relative_path(path))
                .take(8)
                .collect::<Vec<_>>();
            let prefixes = event
                .paths
                .iter()
                .filter(|path| native_qa_watcher_trace_relative_path(path).is_some())
                .map(|path| native_qa_watcher_trace_path_prefix(path))
                .take(8)
                .collect::<Vec<_>>();
            if !paths.is_empty() {
                native_qa_watcher_trace("notify_callback", || {
                    format!(
                        "source={source} kind={:?} raw_paths={} raw_path_prefixes={}",
                        event.kind,
                        paths.join("|"),
                        prefixes.join("|"),
                    )
                });
            }
        }
        Err(_) => native_qa_watcher_trace("notify_callback", || {
            format!("source={source} result=error")
        }),
    }
    #[cfg(not(feature = "native-qa"))]
    let _ = (source, event);
}

fn trace_native_qa_notify_queue(
    source: &'static str,
    result: &Result<(), TrySendError<WatcherInput>>,
) {
    #[cfg(feature = "native-qa")]
    {
        let result = match result {
            Ok(()) => "queued",
            Err(TrySendError::Full(_)) => "full",
            Err(TrySendError::Disconnected(_)) => "disconnected",
        };
        native_qa_watcher_trace("notify_queue", || {
            format!("source={source} result={result}")
        });
    }
    #[cfg(not(feature = "native-qa"))]
    let _ = (source, result);
}

fn trace_native_qa_active_roots(roots: &[PathBuf]) {
    #[cfg(feature = "native-qa")]
    native_qa_watcher_trace("active_session", || {
        let scoped = roots
            .iter()
            .filter_map(|root| native_qa_watcher_trace_relative_path(root))
            .take(8)
            .collect::<Vec<_>>();
        format!(
            "registered_root_count={} scoped_roots={}",
            roots.len(),
            scoped.join("|")
        )
    });
    #[cfg(not(feature = "native-qa"))]
    let _ = roots;
}

fn event_to_payload_with_native_qa_trace(
    source: &'static str,
    event: Event,
) -> Option<FileWatchEvent> {
    #[cfg(not(feature = "native-qa"))]
    let _ = source;

    #[cfg(feature = "native-qa")]
    let trace_input = {
        let paths = event
            .paths
            .iter()
            .filter_map(|path| native_qa_watcher_trace_relative_path(path))
            .take(8)
            .collect::<Vec<_>>();
        let prefixes = event
            .paths
            .iter()
            .filter(|path| native_qa_watcher_trace_relative_path(path).is_some())
            .map(|path| native_qa_watcher_trace_path_prefix(path))
            .take(8)
            .collect::<Vec<_>>();
        let ignored = event
            .paths
            .iter()
            .filter(|path| is_ignored_path(path))
            .filter_map(|path| native_qa_watcher_trace_relative_path(path))
            .take(8)
            .collect::<Vec<_>>();
        (!paths.is_empty()).then(|| (format!("{:?}", event.kind), paths, prefixes, ignored))
    };

    let payload = event_to_payload(event);

    #[cfg(feature = "native-qa")]
    if let Some((kind, paths, prefixes, ignored)) = trace_input {
        let normalized = payload
            .as_ref()
            .map(|payload| {
                payload
                    .paths
                    .iter()
                    .filter_map(|path| native_qa_watcher_trace_relative_path(Path::new(path)))
                    .take(8)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        native_qa_watcher_trace("event_to_payload", || {
            format!(
                "source={source} kind={kind} raw_paths={} raw_path_prefixes={} normalized={} ignored={} result={}",
                paths.join("|"),
                prefixes.join("|"),
                normalized.join("|"),
                ignored.join("|"),
                if payload.is_some() {
                    "payload"
                } else {
                    "filtered"
                },
            )
        });
    }

    payload
}

#[derive(Debug, Error)]
enum WatcherError {
    #[error("watch path does not exist: {0}")]
    MissingPath(String),
    #[error("watch path is not a directory: {0}")]
    NotDirectory(String),
    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tauri emit error: {0}")]
    Tauri(#[from] tauri::Error),
    #[error("failed to start watcher thread: {0}")]
    Thread(std::io::Error),
    #[error("watcher state lock poisoned")]
    StateLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsWatcherReloadFailure {
    RootSynchronization,
    RuntimeRestart,
    ReconciliationScheduling,
}

impl SettingsWatcherReloadFailure {
    pub(crate) const fn as_support_code(self) -> &'static str {
        match self {
            Self::RootSynchronization => "watcher_root_sync_failure",
            Self::RuntimeRestart => "watcher_runtime_failure",
            Self::ReconciliationScheduling => "watcher_reconciliation_schedule_failure",
        }
    }
}

impl std::fmt::Display for SettingsWatcherReloadFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_support_code())
    }
}

impl std::error::Error for SettingsWatcherReloadFailure {}

fn restart_and_schedule_for_settings(
    restart: impl FnOnce() -> Result<bool, SettingsWatcherReloadFailure>,
    schedule: impl FnOnce() -> Result<(), SettingsWatcherReloadFailure>,
) -> Result<bool, SettingsWatcherReloadFailure> {
    // Startup and lifecycle recovery must still admit durable reconciliation
    // work after a watcher restart failure. Preserve restart as the reported
    // error when both stages fail.
    let restart_result = restart();
    native_qa_watcher_trace("settings_reload_restart", || match &restart_result {
        Ok(changed) => format!("result=success changed={changed}"),
        Err(error) => format!("result=failure code={}", error.as_support_code()),
    });
    let schedule_result = schedule();
    native_qa_watcher_trace(
        "settings_reload_reconciliation_schedule",
        || match &schedule_result {
            Ok(()) => "result=success".to_string(),
            Err(error) => format!("result=failure code={}", error.as_support_code()),
        },
    );
    let changed = restart_result?;
    schedule_result?;
    Ok(changed)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileWatchEvent {
    pub event_type: String,
    pub paths: Vec<String>,
    pub stale_paths: Vec<String>,
    pub upsert_paths: Vec<String>,
    pub reconciliation_paths: Vec<String>,
    pub timestamp_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherReadyEvent {
    pub roots: Vec<String>,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherErrorEvent {
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherReconciliationStatusEvent {
    pub scan_root_id: String,
    pub path: String,
    pub root_revision: i64,
    pub watcher_revision: i64,
    pub watcher_applied_revision: i64,
    pub pending: bool,
    pub needs_reconciliation: bool,
    pub watcher_rule_recovery_required: bool,
    pub health_status: String,
    pub active_run_id: Option<String>,
    pub last_event_at: Option<i64>,
    pub last_applied_at: Option<i64>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub pending_batch: i64,
    pub timestamp: i64,
}

#[derive(Debug)]
enum WatcherInput {
    Notify(notify::Result<Event>),
    Stop,
}

type ReconciliationRetryMap = HashMap<String, (i64, mpsc::Sender<()>)>;
type SharedReconciliationRetries = Arc<Mutex<ReconciliationRetryMap>>;

pub struct FileWatcherManager {
    session: Mutex<Option<WatcherSession>>,
    reload_lock: Mutex<()>,
    reconciliation_retries: SharedReconciliationRetries,
}

impl Default for FileWatcherManager {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            reload_lock: Mutex::new(()),
            reconciliation_retries: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Drop for FileWatcherManager {
    fn drop(&mut self) {
        self.cancel_reconciliation_retries();
    }
}

struct WatcherSession {
    roots: Vec<PathBuf>,
    shutdown: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl WatcherSession {
    fn new(roots: Vec<PathBuf>, shutdown: impl FnOnce() + Send + 'static) -> Self {
        Self {
            roots,
            shutdown: Some(Box::new(shutdown)),
        }
    }

    fn detach(mut self) {
        self.shutdown.take();
    }
}

impl Drop for WatcherSession {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown();
        }
    }
}

impl FileWatcherManager {
    fn cancel_reconciliation_retries(&self) {
        let Ok(mut retries) = self.reconciliation_retries.lock() else {
            return;
        };
        for (_, (_, cancel)) in retries.drain() {
            let _ = cancel.send(());
        }
    }

    fn cancel_reconciliation_retry(&self, root_id: &str) {
        let Ok(mut retries) = self.reconciliation_retries.lock() else {
            return;
        };
        if let Some((_, cancel)) = retries.remove(root_id) {
            let _ = cancel.send(());
        }
    }

    fn schedule_reconciliation_retry<R: Runtime>(
        &self,
        app: AppHandle<R>,
        db: Database,
        jobs: ScanJobManager,
        dedupe_jobs: DedupeJobManager,
        root_id: String,
        retry_at: i64,
    ) {
        // The durable retry timestamp is authoritative; this is one cancellable wake, not a poll.
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let mut retries = match self.reconciliation_retries.lock() {
            Ok(retries) => retries,
            Err(_) => {
                emit_file_watcher_error(
                    &app,
                    "Unable to schedule watcher reconciliation retry.".to_string(),
                );
                return;
            }
        };
        if retries
            .get(&root_id)
            .is_some_and(|(existing_retry_at, _)| *existing_retry_at == retry_at)
        {
            return;
        }
        let previous = retries.insert(root_id.clone(), (retry_at, cancel_tx));
        drop(retries);
        if let Some((_, cancel)) = previous {
            let _ = cancel.send(());
        }

        let retries = Arc::clone(&self.reconciliation_retries);
        let timer_root_id = root_id.clone();
        let app_for_retry = app.clone();
        let spawn_result = thread::Builder::new()
            .name("zen-canvas-watcher-retry".to_string())
            .spawn(move || {
                let delay = Duration::from_secs(
                    retry_at.saturating_sub(current_unix_seconds()).max(0) as u64,
                );
                if !matches!(
                    cancel_rx.recv_timeout(delay),
                    Err(RecvTimeoutError::Timeout)
                ) {
                    return;
                }

                let is_current = match retries.lock() {
                    Ok(mut retries) => {
                        if retries
                            .get(&timer_root_id)
                            .is_some_and(|(current_retry_at, _)| *current_retry_at == retry_at)
                        {
                            retries.remove(&timer_root_id);
                            true
                        } else {
                            false
                        }
                    }
                    Err(_) => false,
                };
                if !is_current {
                    return;
                }

                if let Err(error) = crate::scanner::schedule_watcher_reconciliation_roots(
                    app_for_retry.clone(),
                    db,
                    jobs,
                    dedupe_jobs,
                    vec![timer_root_id],
                ) {
                    emit_file_watcher_error(&app_for_retry, error);
                }
            });
        if let Err(error) = spawn_result {
            if let Ok(mut retries) = self.reconciliation_retries.lock() {
                if retries
                    .get(&root_id)
                    .is_some_and(|(current_retry_at, _)| *current_retry_at == retry_at)
                {
                    retries.remove(&root_id);
                }
            }
            emit_file_watcher_error(
                &app,
                format!("Unable to start watcher reconciliation retry: {error}"),
            );
        }
    }

    fn restart<R: Runtime>(
        &self,
        app: AppHandle<R>,
        paths: Vec<PathBuf>,
    ) -> Result<bool, WatcherError> {
        let roots = normalize_watch_roots(paths)?;
        if roots.is_empty() {
            let changed = self.restart_with_roots(Vec::new(), |_| unreachable!(), |_, _| {})?;
            if changed {
                emit_watcher_ready(&app, Vec::new())?;
            }
            return Ok(changed);
        }

        self.restart_with_roots(
            roots,
            |roots| start_legacy_watcher_session(app, roots),
            |_, _| {},
        )
    }

    fn restart_backend<R: Runtime>(
        &self,
        app: AppHandle<R>,
        paths: Vec<PathBuf>,
        db: Database,
        jobs: ScanJobManager,
        dedupe_jobs: DedupeJobManager,
    ) -> Result<bool, WatcherError> {
        let roots = normalize_watch_roots(paths)?;
        if roots.is_empty() {
            let gap_app = app.clone();
            let gap_db = db.clone();
            return self.restart_with_roots(
                Vec::new(),
                |_| unreachable!(),
                move |old_roots, new_roots| {
                    mark_watcher_reload_gap(&gap_app, &gap_db, old_roots, new_roots)
                },
            );
        }
        let gap_app = app.clone();
        let gap_db = db.clone();
        self.restart_with_roots(
            roots,
            |roots| start_backend_watcher_session(app, roots, db, jobs, dedupe_jobs),
            move |old_roots, new_roots| {
                mark_watcher_reload_gap(&gap_app, &gap_db, old_roots, new_roots)
            },
        )
    }

    fn restart_with_roots(
        &self,
        roots: Vec<PathBuf>,
        start: impl FnOnce(Vec<PathBuf>) -> Result<WatcherSession, WatcherError>,
        on_handoff_gap: impl Fn(&[PathBuf], &[PathBuf]),
    ) -> Result<bool, WatcherError> {
        let _reload_guard = self
            .reload_lock
            .lock()
            .map_err(|_| WatcherError::StateLock)?;
        let mut session = self.session.lock().map_err(|_| WatcherError::StateLock)?;
        if session
            .as_ref()
            .is_some_and(|current| current.roots == roots)
        {
            trace_native_qa_active_roots(&roots);
            return Ok(false);
        }
        let previous = session.take();
        let previous_roots = previous
            .as_ref()
            .map(|current| current.roots.clone())
            .unwrap_or_default();
        drop(session);
        drop(previous);

        let had_previous = !previous_roots.is_empty();
        if had_previous {
            on_handoff_gap(&previous_roots, &roots);
        }
        if roots.is_empty() {
            return Ok(true);
        }

        let next = match start(roots.clone()) {
            Ok(next) => next,
            Err(error) => {
                native_qa_watcher_trace("session_install", || "result=failed".to_string());
                if !had_previous {
                    on_handoff_gap(&previous_roots, &roots);
                }
                return Err(error);
            }
        };
        let mut session = self.session.lock().map_err(|_| WatcherError::StateLock)?;
        *session = Some(next);
        trace_native_qa_active_roots(&roots);
        Ok(true)
    }

    pub fn active_roots(&self) -> Result<Vec<PathBuf>, String> {
        self.session
            .lock()
            .map(|session| {
                session
                    .as_ref()
                    .map(|session| session.roots.clone())
                    .unwrap_or_default()
            })
            .map_err(|_| WatcherError::StateLock.to_string())
    }
}

pub(crate) fn schedule_reconciliation_retry<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    jobs: ScanJobManager,
    dedupe_jobs: DedupeJobManager,
    root_id: String,
    retry_at: i64,
) {
    if let Some(manager) = app.try_state::<FileWatcherManager>() {
        manager.schedule_reconciliation_retry(
            app.clone(),
            db,
            jobs,
            dedupe_jobs,
            root_id,
            retry_at,
        );
    } else {
        eprintln!("Watcher reconciliation retry has no active watcher manager state.");
    }
}

pub(crate) fn cancel_reconciliation_retry<R: Runtime>(app: &AppHandle<R>, root_id: &str) {
    if let Some(manager) = app.try_state::<FileWatcherManager>() {
        manager.cancel_reconciliation_retry(root_id);
    }
}

pub fn setup_file_watcher<R: Runtime>(
    app: AppHandle<R>,
    paths: Vec<PathBuf>,
) -> Result<(), String> {
    setup_file_watcher_inner(app, paths).map_err(|error| error.to_string())
}

pub fn reload_file_watcher_for_settings<R: Runtime>(
    app: AppHandle<R>,
    manager: &FileWatcherManager,
    db: &Database,
    jobs: &ScanJobManager,
    dedupe_jobs: &DedupeJobManager,
    settings: &AppSettings,
) -> Result<bool, String> {
    reload_file_watcher_for_settings_with_stage(app, manager, db, jobs, dedupe_jobs, settings)
        .map_err(|failure| failure.as_support_code().to_string())
}

pub(crate) fn reload_file_watcher_for_settings_with_stage<R: Runtime>(
    app: AppHandle<R>,
    manager: &FileWatcherManager,
    db: &Database,
    jobs: &ScanJobManager,
    dedupe_jobs: &DedupeJobManager,
    settings: &AppSettings,
) -> Result<bool, SettingsWatcherReloadFailure> {
    db.sync_file_library_watcher_roots(&settings.default_scan_folders)
        .map_err(|_| SettingsWatcherReloadFailure::RootSynchronization)?;
    let backend_enabled = backend_watcher_reconciliation_enabled();
    native_qa_watcher_trace("watcher_mode", || {
        format!("backend_reconciliation_enabled={backend_enabled}")
    });
    if backend_enabled {
        manager.cancel_reconciliation_retries();
        let paths = existing_watch_paths_from_default_scan_folders(&settings.default_scan_folders);
        let root_labels = paths
            .iter()
            .map(|path| normalize_path(path))
            .collect::<Vec<_>>();
        let changed = restart_and_schedule_for_settings(
            || {
                manager
                    .restart_backend(
                        app.clone(),
                        paths,
                        db.clone(),
                        jobs.clone(),
                        dedupe_jobs.clone(),
                    )
                    .map_err(|_| SettingsWatcherReloadFailure::RuntimeRestart)
            },
            || {
                crate::scanner::schedule_watcher_reconciliations(
                    app.clone(),
                    db.clone(),
                    jobs.clone(),
                    dedupe_jobs.clone(),
                )
                .map(|_| ())
                .map_err(|_| SettingsWatcherReloadFailure::ReconciliationScheduling)
            },
        )?;
        if changed {
            emit_watcher_ready(&app, root_labels)
                .map_err(|_| SettingsWatcherReloadFailure::RuntimeRestart)?;
        }
        Ok(changed)
    } else {
        eprintln!(
            "{WATCHER_BACKEND_ENV}=false: using the legacy renderer watcher adapter; Rust will not mutate managed files or watcher revisions"
        );
        let paths = existing_legacy_watch_paths_from_settings(settings);
        manager
            .restart(app, paths)
            .map_err(|_| SettingsWatcherReloadFailure::RuntimeRestart)
    }
}

pub fn suspend_file_watcher_for_lifecycle<R: Runtime>(
    app: AppHandle<R>,
    manager: &FileWatcherManager,
    db: &Database,
    jobs: &ScanJobManager,
    dedupe_jobs: &DedupeJobManager,
) -> Result<bool, String> {
    manager.cancel_reconciliation_retries();
    if backend_watcher_reconciliation_enabled() {
        manager
            .restart_backend(
                app,
                Vec::new(),
                db.clone(),
                jobs.clone(),
                dedupe_jobs.clone(),
            )
            .map_err(|error| error.to_string())
    } else {
        manager
            .restart(app, Vec::new())
            .map_err(|error| error.to_string())
    }
}

pub fn backend_watcher_reconciliation_enabled() -> bool {
    backend_watcher_reconciliation_enabled_value(std::env::var(WATCHER_BACKEND_ENV).ok().as_deref())
}

fn backend_watcher_reconciliation_enabled_value(value: Option<&str>) -> bool {
    match value {
        Some(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no" | "disabled"
        ),
        None => true,
    }
}

pub fn reload_file_watcher<R: Runtime>(
    app: AppHandle<R>,
    manager: &FileWatcherManager,
    paths: Vec<PathBuf>,
) -> Result<bool, String> {
    manager
        .restart(app, paths)
        .map_err(|error| error.to_string())
}

pub fn watch_paths_from_default_scan_folders(folders: &[ScanRootSetting]) -> Vec<PathBuf> {
    folders
        .iter()
        .filter(|root| root.enabled)
        .map(|root| root.path.trim())
        .filter(|path| !path.is_empty() && looks_absolute_path(path))
        .map(PathBuf::from)
        .collect()
}

pub fn watch_paths_from_search_roots(roots: &[SearchRootSetting]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter(|root| root.enabled)
        .map(|root| root.path.trim())
        .filter(|path| !path.is_empty() && looks_absolute_path(path))
        .map(PathBuf::from)
        .collect()
}

pub fn watch_paths_from_settings(settings: &AppSettings) -> Vec<PathBuf> {
    let mut paths = watch_paths_from_default_scan_folders(&settings.default_scan_folders);
    paths.extend(watch_paths_from_search_roots(&settings.custom_search_roots));
    paths
}

pub fn existing_watch_paths_from_settings(settings: &AppSettings) -> Vec<PathBuf> {
    watch_paths_from_settings(settings)
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

fn existing_legacy_watch_paths_from_settings(settings: &AppSettings) -> Vec<PathBuf> {
    legacy_watch_paths_from_settings(settings)
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

fn legacy_watch_paths_from_settings(settings: &AppSettings) -> Vec<PathBuf> {
    watch_paths_from_default_scan_folders(&settings.default_scan_folders)
}

pub fn existing_watch_paths_from_default_scan_folders(folders: &[ScanRootSetting]) -> Vec<PathBuf> {
    watch_paths_from_default_scan_folders(folders)
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

pub fn emit_file_watcher_error<R: Runtime>(app: &AppHandle<R>, message: String) {
    let _ = app.emit(WATCHER_ERROR_EVENT_NAME, WatcherErrorEvent { message });
}

fn setup_file_watcher_inner<R: Runtime>(
    app: AppHandle<R>,
    paths: Vec<PathBuf>,
) -> Result<(), WatcherError> {
    let roots = normalize_watch_roots(paths)?;
    if roots.is_empty() {
        emit_watcher_ready(&app, Vec::new())?;
        return Ok(());
    }
    let session = start_legacy_watcher_session(app, roots)?;
    session.detach();
    Ok(())
}

fn start_legacy_watcher_session<R: Runtime>(
    app: AppHandle<R>,
    roots: Vec<PathBuf>,
) -> Result<WatcherSession, WatcherError> {
    let root_labels = roots
        .iter()
        .map(|path| normalize_path(path))
        .collect::<Vec<_>>();
    let (tx, rx) = mpsc::sync_channel::<WatcherInput>(WATCHER_CHANNEL_CAPACITY);
    let stop_tx = tx.clone();
    let stop_requested = Arc::new(AtomicBool::new(false));
    let overflow_reported = Arc::new(AtomicBool::new(false));
    let overflow_for_callback = Arc::clone(&overflow_reported);
    let overflow_app = app.clone();

    let mut watcher = recommended_watcher(move |event| {
        trace_native_qa_notify("legacy", &event);
        let queued = tx.try_send(WatcherInput::Notify(event));
        trace_native_qa_notify_queue("legacy", &queued);
        if let Err(TrySendError::Full(_)) = queued {
            if !overflow_for_callback.swap(true, Ordering::AcqRel) {
                emit_file_watcher_error(
                    &overflow_app,
                    "File watcher overflowed its bounded queue. A rescan is required to reconcile changes."
                        .to_string(),
                );
            }
        }
    })?;

    register_native_watcher_roots(&mut watcher, &roots)?;

    emit_watcher_ready(&app, root_labels)?;

    let handle = thread::Builder::new()
        .name("zen-canvas-file-watcher".to_string())
        .spawn({
            let stop_requested = Arc::clone(&stop_requested);
            move || run_legacy_watcher_loop(app, watcher, rx, stop_requested)
        })
        .map_err(WatcherError::Thread)?;

    Ok(WatcherSession::new(roots, move || {
        stop_watcher(stop_tx, stop_requested, handle)
    }))
}

fn start_backend_watcher_session<R: Runtime>(
    app: AppHandle<R>,
    roots: Vec<PathBuf>,
    db: Database,
    jobs: ScanJobManager,
    dedupe_jobs: DedupeJobManager,
) -> Result<WatcherSession, WatcherError> {
    let (tx, rx) = mpsc::sync_channel::<WatcherInput>(WATCHER_CHANNEL_CAPACITY);
    let stop_tx = tx.clone();
    let stop_requested = Arc::new(AtomicBool::new(false));
    let overflow_signal = Arc::new(AtomicBool::new(false));
    let overflow_burst_active = Arc::new(AtomicBool::new(false));
    let overflow_signal_for_callback = Arc::clone(&overflow_signal);
    let overflow_burst_for_callback = Arc::clone(&overflow_burst_active);

    let mut watcher = recommended_watcher(move |event| {
        trace_native_qa_notify("backend", &event);
        let queued = tx.try_send(WatcherInput::Notify(event));
        trace_native_qa_notify_queue("backend", &queued);
        if let Err(TrySendError::Full(_)) = queued {
            signal_overflow(&overflow_burst_for_callback, &overflow_signal_for_callback);
        }
    })?;

    register_native_watcher_roots(&mut watcher, &roots)?;

    let loop_stop_requested = Arc::clone(&stop_requested);
    let handle = thread::Builder::new()
        .name("zen-canvas-file-watcher-backend".to_string())
        .spawn(move || {
            run_backend_watcher_loop(
                app,
                watcher,
                rx,
                loop_stop_requested,
                db,
                jobs,
                dedupe_jobs,
                overflow_signal,
                overflow_burst_active,
            )
        })
        .map_err(WatcherError::Thread)?;

    Ok(WatcherSession::new(roots, move || {
        stop_watcher(stop_tx, stop_requested, handle)
    }))
}

fn register_native_watcher_roots(
    watcher: &mut RecommendedWatcher,
    roots: &[PathBuf],
) -> Result<(), WatcherError> {
    for root in roots {
        let scoped_path = native_qa_watcher_trace_relative_path(root);
        if let Some(path) = scoped_path.as_deref() {
            native_qa_watcher_trace("watch_registration_attempt", || {
                format!("path={path} recursive=true")
            });
        }
        match watcher.watch(root, RecursiveMode::Recursive) {
            Ok(()) => {
                if let Some(path) = scoped_path {
                    native_qa_watcher_trace("watch_registration_result", || {
                        format!("path={path} result=success")
                    });
                }
            }
            Err(error) => {
                if let Some(path) = scoped_path {
                    native_qa_watcher_trace("watch_registration_result", || {
                        format!("path={path} result=error")
                    });
                }
                return Err(error.into());
            }
        }
    }
    native_qa_watcher_trace("watch_registration_complete", || {
        format!("registered_root_count={}", roots.len())
    });
    Ok(())
}

fn stop_watcher(
    tx: SyncSender<WatcherInput>,
    stop_requested: Arc<AtomicBool>,
    handle: JoinHandle<()>,
) {
    stop_requested.store(true, Ordering::Release);
    let _ = tx.try_send(WatcherInput::Stop);
    let _ = handle.join();
}

fn emit_watcher_ready<R: Runtime>(
    app: &AppHandle<R>,
    roots: Vec<String>,
) -> Result<(), WatcherError> {
    app.emit(
        WATCHER_READY_EVENT_NAME,
        WatcherReadyEvent {
            roots,
            recursive: true,
        },
    )?;
    Ok(())
}

fn recv_watcher_input(
    rx: &Receiver<WatcherInput>,
    stop_requested: &AtomicBool,
) -> Option<WatcherInput> {
    if stop_requested.load(Ordering::Acquire) {
        return None;
    }
    match rx.recv() {
        Ok(WatcherInput::Stop) => None,
        Ok(input) if stop_requested.load(Ordering::Acquire) => {
            drop(input);
            None
        }
        Ok(input) => Some(input),
        Err(_) => None,
    }
}

fn run_legacy_watcher_loop(
    app: AppHandle<impl Runtime>,
    _watcher: RecommendedWatcher,
    rx: Receiver<WatcherInput>,
    stop_requested: Arc<AtomicBool>,
) {
    loop {
        if stop_requested.load(Ordering::Acquire) {
            break;
        }

        let Some(input) = recv_watcher_input(&rx, &stop_requested) else {
            break;
        };
        match input {
            WatcherInput::Stop => break,
            WatcherInput::Notify(event) => match event {
                Ok(event) => {
                    let mut payloads = event_to_payload_with_native_qa_trace("legacy", event)
                        .into_iter()
                        .collect::<Vec<_>>();
                    let deadline = Instant::now() + WATCHER_COALESCE_WINDOW;
                    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                        if stop_requested.load(Ordering::Acquire) {
                            return;
                        }
                        match rx.recv_timeout(remaining) {
                            Ok(WatcherInput::Notify(Ok(event))) => {
                                payloads
                                    .extend(event_to_payload_with_native_qa_trace("legacy", event));
                            }
                            Ok(WatcherInput::Notify(Err(error))) => {
                                emit_file_watcher_error(&app, error.to_string());
                            }
                            Ok(WatcherInput::Stop) => return,
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    if let Some(payload) = coalesce_payloads(payloads) {
                        let _ = app.emit(FILE_EVENT_NAME, payload);
                    }
                }
                Err(error) => emit_file_watcher_error(&app, error.to_string()),
            },
        }
    }
}

fn signal_overflow(burst_active: &AtomicBool, signal: &AtomicBool) {
    if !burst_active.swap(true, Ordering::AcqRel) {
        signal.store(true, Ordering::Release);
    }
}

pub(crate) fn bounded_retry<T, E, Operation, Delay>(
    max_attempts: usize,
    mut operation: Operation,
    mut delay: Delay,
) -> Result<T, E>
where
    Operation: FnMut() -> Result<T, E>,
    Delay: FnMut(usize),
{
    let attempts = max_attempts.max(1);
    let mut last_error = None;
    for attempt in 0..attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < attempts {
                    delay(attempt);
                }
            }
        }
    }
    Err(last_error.expect("bounded retry always records an error"))
}

#[allow(clippy::too_many_arguments)]
fn run_backend_watcher_loop<R: Runtime>(
    app: AppHandle<R>,
    _watcher: RecommendedWatcher,
    rx: Receiver<WatcherInput>,
    stop_requested: Arc<AtomicBool>,
    db: Database,
    jobs: ScanJobManager,
    dedupe_jobs: DedupeJobManager,
    overflow_signal: Arc<AtomicBool>,
    overflow_burst_active: Arc<AtomicBool>,
) {
    loop {
        if stop_requested.load(Ordering::Acquire) {
            break;
        }

        if overflow_signal.swap(false, Ordering::AcqRel) {
            mark_all_roots_for_reconciliation(
                &app,
                &db,
                "watcher_overflow",
                "The bounded watcher queue overflowed; a managed scan is required.",
            );
            emit_file_watcher_error(
                &app,
                "File watcher overflowed its bounded queue. Durable reconciliation was scheduled."
                    .to_string(),
            );
            schedule_reconciliation_for_dirty_roots(&app, &db, &jobs, &dedupe_jobs);
        }

        let Some(input) = recv_watcher_input(&rx, &stop_requested) else {
            break;
        };
        match input {
            WatcherInput::Stop => break,
            WatcherInput::Notify(Ok(event)) => {
                let mut payloads = event_to_payload_with_native_qa_trace("backend", event)
                    .into_iter()
                    .collect::<Vec<_>>();
                let deadline = Instant::now() + WATCHER_COALESCE_WINDOW;
                while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                    if stop_requested.load(Ordering::Acquire) {
                        return;
                    }
                    match rx.recv_timeout(remaining) {
                        Ok(WatcherInput::Notify(Ok(event))) => {
                            payloads
                                .extend(event_to_payload_with_native_qa_trace("backend", event));
                        }
                        Ok(WatcherInput::Notify(Err(error))) => {
                            mark_all_roots_for_reconciliation(
                                &app,
                                &db,
                                "watcher_notify_error",
                                &error.to_string(),
                            );
                            schedule_reconciliation_for_dirty_roots(&app, &db, &jobs, &dedupe_jobs);
                        }
                        Ok(WatcherInput::Stop) => return,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                overflow_burst_active.store(false, Ordering::Release);
                if let Some(payload) = coalesce_payloads(payloads) {
                    process_backend_payload(&app, &db, &jobs, &dedupe_jobs, payload);
                }
            }
            WatcherInput::Notify(Err(error)) => {
                mark_all_roots_for_reconciliation(
                    &app,
                    &db,
                    "watcher_notify_error",
                    &error.to_string(),
                );
                schedule_reconciliation_for_dirty_roots(&app, &db, &jobs, &dedupe_jobs);
            }
        }
    }
}

fn schedule_reconciliation_for_dirty_roots<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    jobs: &ScanJobManager,
    dedupe_jobs: &DedupeJobManager,
) {
    if let Err(error) = crate::scanner::schedule_watcher_reconciliations(
        app.clone(),
        db.clone(),
        jobs.clone(),
        dedupe_jobs.clone(),
    ) {
        emit_file_watcher_error(app, error);
    }
}

fn process_backend_payload<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    jobs: &ScanJobManager,
    dedupe_jobs: &DedupeJobManager,
    payload: FileWatchEvent,
) {
    native_qa_watcher_trace("coalesced_payload", || {
        let paths = payload
            .paths
            .iter()
            .filter_map(|path| native_qa_watcher_trace_relative_path(Path::new(path)))
            .take(8)
            .collect::<Vec<_>>();
        let extended_prefix_count = payload
            .paths
            .iter()
            .filter(|path| path.starts_with("//?/"))
            .count();
        format!(
            "path_count={} scoped_paths={} extended_prefix_count={}",
            payload.paths.len(),
            paths.join("|"),
            extended_prefix_count,
        )
    });
    let Ok(configs) = db.list_watcher_root_configs() else {
        emit_file_watcher_error(app, "Unable to load managed watcher roots.".to_string());
        return;
    };
    native_qa_watcher_trace("routing_configs", || {
        let scoped = configs
            .iter()
            .filter_map(|root| {
                native_qa_watcher_trace_relative_path(Path::new(&root.path))
                    .map(|path| format!("{}:{path}", root.id))
            })
            .take(16)
            .collect::<Vec<_>>();
        format!("config_count={} scoped={}", configs.len(), scoped.join("|"))
    });
    let directory_paths = payload
        .reconciliation_paths
        .iter()
        .map(|path| normalize_path(&PathBuf::from(path)))
        .collect::<HashSet<_>>();
    let paths = payload
        .paths
        .iter()
        .map(|path| normalize_path(&PathBuf::from(path)))
        .filter(|path| !is_ignored_path(Path::new(path)))
        .collect::<HashSet<_>>();
    let mut grouped = HashMap::<String, Vec<String>>::new();
    let mut ambiguous = HashMap::<String, Vec<String>>::new();
    let mut roots_requiring_reconciliation = HashSet::new();

    for path in paths {
        let matches = configs
            .iter()
            .filter(|root| path_within_root(&root.path, &path))
            .collect::<Vec<_>>();
        if let Some(relative) = native_qa_watcher_trace_relative_path(Path::new(&path)) {
            native_qa_watcher_trace("root_route", || {
                format!(
                    "path={relative} match_count={} root_ids={}",
                    matches.len(),
                    matches
                        .iter()
                        .map(|root| root.id.as_str())
                        .collect::<Vec<_>>()
                        .join("|"),
                )
            });
        }
        match matches.as_slice() {
            [root] => grouped.entry(root.id.clone()).or_default().push(path),
            [] => {}
            _ => {
                for root in matches {
                    ambiguous
                        .entry(root.id.clone())
                        .or_default()
                        .push(path.clone());
                }
            }
        }
    }

    for (root_id, paths) in ambiguous {
        let Some(batch) = begin_watcher_batch(app, db, &root_id) else {
            continue;
        };
        let message = format!(
            "Watcher path batch is ambiguous across managed roots: {}",
            paths.join(", ")
        );
        let _ = db.mark_watcher_reconciliation(&root_id, "ambiguous_root", &message);
        emit_root_status(app, db, &root_id, Some(batch.watcher_revision));
        roots_requiring_reconciliation.insert(root_id);
    }

    for (root_id, mut paths) in grouped {
        let oversized = paths.len() > WATCHER_BATCH_LIMIT;
        if oversized {
            paths.truncate(WATCHER_BATCH_LIMIT);
        }
        let Some(batch) = begin_watcher_batch(app, db, &root_id) else {
            continue;
        };
        trace_native_qa_durable_root_state(db, &root_id);
        let mut should_reconcile = false;
        native_qa_watcher_trace("exact_mutation_attempt", || {
            format!("root_id={root_id} path_count={}", paths.len())
        });
        let result =
            apply_watcher_exact_mutations_with_retry(db, &root_id, &paths, &directory_paths);
        match result {
            Ok(result) => {
                native_qa_watcher_trace("exact_mutation_result", || {
                    format!(
                        "root_id={root_id} result=success upserted_count={} reconciliation_required={}",
                        result.upserted_paths.len(),
                        result.reconciliation_required,
                    )
                });
                trace_native_qa_durable_root_state(db, &root_id);
                let mut reconciliation_required = result.reconciliation_required || oversized;
                let mut rule_warning = None;
                if let Some(warning) = result.warning.as_deref() {
                    let _ = db.record_watcher_warning(&root_id, "watcher_partial_update", warning);
                    emit_file_watcher_error(app, warning.to_string());
                }
                if !result.upserted_paths.is_empty() {
                    match execute_rules_for_paths_with_retry(db, &result.upserted_paths) {
                        Ok(_) => {}
                        Err(error) => {
                            let message = error.to_string();
                            rule_warning = Some(message.clone());
                            reconciliation_required = true;
                            emit_file_watcher_error(app, message);
                        }
                    }
                }
                if reconciliation_required {
                    let message = if oversized {
                        "Watcher event batch exceeded the bounded mutation batch; a full reconciliation is required."
                    } else if let Some(rule_warning) = rule_warning.as_deref() {
                        rule_warning
                    } else {
                        result.warning.as_deref().unwrap_or(
                            "Watcher directory or ambiguous event requires reconciliation.",
                        )
                    };
                    let _ = db.mark_watcher_reconciliation(
                        &root_id,
                        "watcher_reconciliation_required",
                        message,
                    );
                    should_reconcile = true;
                } else if !db
                    .complete_watcher_revision(&root_id, batch.watcher_revision)
                    .unwrap_or(false)
                {
                    let _ = db.mark_watcher_reconciliation(
                        &root_id,
                        "watcher_revision_cas_failed",
                        "Watcher applied revision CAS failed; a full reconciliation is required.",
                    );
                    should_reconcile = true;
                }
                if let Some(message) = rule_warning {
                    let _ = db.record_watcher_warning(&root_id, "watcher_rule_failure", &message);
                }
            }
            Err(error) => {
                native_qa_watcher_trace("exact_mutation_result", || {
                    format!("root_id={root_id} result=error")
                });
                let message = error.to_string();
                let _ =
                    db.mark_watcher_reconciliation(&root_id, "watcher_mutation_failed", &message);
                emit_file_watcher_error(app, message);
                should_reconcile = true;
            }
        }
        trace_native_qa_durable_root_state(db, &root_id);
        emit_root_status(app, db, &root_id, Some(batch.watcher_revision));
        if should_reconcile {
            roots_requiring_reconciliation.insert(root_id);
        }
    }

    if !roots_requiring_reconciliation.is_empty() {
        if let Err(error) = crate::scanner::schedule_watcher_reconciliation_roots(
            app.clone(),
            db.clone(),
            jobs.clone(),
            dedupe_jobs.clone(),
            roots_requiring_reconciliation.into_iter().collect(),
        ) {
            emit_file_watcher_error(app, error);
        }
    }
}

fn trace_native_qa_durable_root_state(db: &Database, root_id: &str) {
    #[cfg(feature = "native-qa")]
    {
        let Ok(conn) = db.conn() else {
            native_qa_watcher_trace("durable_root_state", || {
                format!("root_id={root_id} result=read_error")
            });
            return;
        };
        let state = conn.query_row(
            "SELECT watcher_revision, watcher_applied_revision, library_change_revision, watcher_last_event_at FROM scan_roots WHERE id=?1",
            [root_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        );
        native_qa_watcher_trace("durable_root_state", || {
            match state {
            Ok((watcher, applied, library, last_event)) => format!(
                "root_id={root_id} watcher_revision={watcher} watcher_applied_revision={applied} library_change_revision={library} watcher_last_event_at={last_event:?}"
            ),
            Err(_) => format!("root_id={root_id} result=query_error"),
        }
        });
    }
    #[cfg(not(feature = "native-qa"))]
    let _ = (db, root_id);
}

fn apply_watcher_exact_mutations_with_retry(
    db: &Database,
    root_id: &str,
    paths: &[String],
    directory_paths: &HashSet<String>,
) -> Result<crate::db::scan::WatcherMutationResult, DbError> {
    bounded_retry(
        WATCHER_MAX_ATTEMPTS,
        || db.apply_watcher_exact_mutations(root_id, paths, directory_paths),
        |attempt| {
            let delay = WATCHER_RETRY_DELAYS[attempt.min(WATCHER_RETRY_DELAYS.len() - 1)];
            thread::sleep(delay);
        },
    )
}

fn execute_rules_for_paths_with_retry(
    db: &Database,
    paths: &[String],
) -> Result<crate::db::RuleExecutionSummary, DbError> {
    bounded_retry(
        WATCHER_RULE_MAX_ATTEMPTS,
        || db.execute_authoritative_rules_for_paths(paths),
        |attempt| {
            let delay = WATCHER_RULE_RETRY_DELAYS[attempt.min(WATCHER_RULE_RETRY_DELAYS.len() - 1)];
            thread::sleep(delay);
        },
    )
}

fn begin_watcher_batch<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    root_id: &str,
) -> Option<crate::db::scan::WatcherRevisionStart> {
    match db.begin_watcher_revision(root_id) {
        Ok(Some(batch)) => {
            native_qa_watcher_trace("begin_watcher_revision", || {
                format!(
                    "root_id={root_id} result=some revision={}",
                    batch.watcher_revision
                )
            });
            Some(batch)
        }
        Ok(None) => {
            native_qa_watcher_trace("begin_watcher_revision", || {
                format!("root_id={root_id} result=none")
            });
            None
        }
        Err(error) => {
            native_qa_watcher_trace("begin_watcher_revision", || {
                format!("root_id={root_id} result=error")
            });
            emit_file_watcher_error(app, error.to_string());
            None
        }
    }
}

fn mark_all_roots_for_reconciliation<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    error_code: &str,
    message: &str,
) {
    let Ok(configs) = db.list_watcher_root_configs() else {
        emit_file_watcher_error(
            app,
            "Unable to load managed watcher roots for recovery.".to_string(),
        );
        return;
    };
    for root in configs {
        if let Some(batch) = begin_watcher_batch(app, db, &root.id) {
            let _ = db.mark_watcher_reconciliation(&root.id, error_code, message);
            emit_root_status(app, db, &root.id, Some(batch.watcher_revision));
        }
    }
}

fn mark_watcher_reload_gap<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    _old_roots: &[PathBuf],
    _new_roots: &[PathBuf],
) {
    // A notify watcher cannot prove that no filesystem event arrived between
    // stopping the old session and installing the new one. Mark every enabled
    // managed root before the new session starts so the next scheduler pass
    // performs a durable reconciliation instead of relying on the event stream.
    mark_all_roots_for_reconciliation(
        app,
        db,
        "watcher_reload_gap",
        "Watcher settings reload created a listening gap; a managed reconciliation is required.",
    );
}

pub(crate) fn emit_watcher_reconciliation_status<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    root_id: &str,
    _batch_revision: Option<i64>,
) {
    let Ok(root) = db.get_scan_root_health(Some(root_id), None) else {
        return;
    };
    let payload = WatcherReconciliationStatusEvent {
        scan_root_id: root.id,
        path: root.normalized_path,
        root_revision: root.revision,
        watcher_revision: root.watcher_revision,
        watcher_applied_revision: root.watcher_applied_revision,
        pending: root.watcher_revision > root.watcher_applied_revision
            || root.needs_reconciliation
            || root.watcher_rule_recovery_required,
        needs_reconciliation: root.needs_reconciliation || root.watcher_rule_recovery_required,
        watcher_rule_recovery_required: root.watcher_rule_recovery_required,
        health_status: if root.watcher_rule_recovery_required {
            "reconciliation_required".to_string()
        } else {
            root.health_status
        },
        active_run_id: root.active_run_id,
        last_event_at: root.watcher_last_event_at,
        last_applied_at: root.watcher_last_applied_at,
        last_error_code: root.watcher_last_error_code.or(root.last_error_code),
        last_error_message: root.watcher_last_error_message.or(root.last_error_message),
        pending_batch: (root.watcher_revision - root.watcher_applied_revision).max(0),
        timestamp: current_timestamp_ms() as i64,
    };
    if let Err(error) = app.emit(WATCHER_RECONCILIATION_STATUS_EVENT_NAME, payload) {
        eprintln!("Failed to emit watcher reconciliation status: {error}");
    }
}

fn emit_root_status<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    root_id: &str,
    batch_revision: Option<i64>,
) {
    emit_watcher_reconciliation_status(app, db, root_id, batch_revision);
}

fn path_within_root(root: &str, path: &str) -> bool {
    let root = normalize_path(Path::new(root))
        .trim_end_matches('/')
        .to_string();
    let path = normalize_path(Path::new(path));
    let (root, path) = if cfg!(windows) {
        (root.to_ascii_lowercase(), path.to_ascii_lowercase())
    } else {
        (root, path)
    };
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub fn recover_watcher_reconciliation_state<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
) -> Result<usize, String> {
    let roots = db.list_scan_roots().map_err(|error| error.to_string())?;
    let mut recovered = 0;
    for root in roots
        .into_iter()
        .filter(|root| root.enabled && root.source_kind == "file_library")
    {
        if root.watcher_revision > root.watcher_applied_revision {
            db.mark_watcher_reconciliation(
                &root.id,
                "startup_revision_gap",
                "A previous watcher batch was not durably applied before shutdown.",
            )
            .map_err(|error| error.to_string())?;
            emit_root_status(&app, &db, &root.id, None);
            recovered += 1;
        }
    }
    Ok(recovered)
}

fn coalesce_payloads(payloads: Vec<FileWatchEvent>) -> Option<FileWatchEvent> {
    if payloads.is_empty() {
        return None;
    }
    let mut paths = HashSet::new();
    let mut latest_route = HashMap::<String, bool>::new();
    let mut reconciliation_paths = HashSet::new();
    for payload in payloads {
        paths.extend(payload.paths);
        for path in payload.stale_paths {
            latest_route.insert(path, false);
        }
        for path in payload.upsert_paths {
            latest_route.insert(path, true);
        }
        reconciliation_paths.extend(payload.reconciliation_paths);
    }
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    let mut stale_paths = latest_route
        .iter()
        .filter_map(|(path, upsert)| (!upsert).then_some(path.clone()))
        .collect::<Vec<_>>();
    let mut upsert_paths = latest_route
        .into_iter()
        .filter_map(|(path, upsert)| upsert.then_some(path))
        .collect::<Vec<_>>();
    let mut reconciliation_paths = reconciliation_paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    stale_paths.sort();
    upsert_paths.sort();
    reconciliation_paths.sort();
    Some(FileWatchEvent {
        event_type: "batch".to_string(),
        paths,
        stale_paths,
        upsert_paths,
        reconciliation_paths,
        timestamp_ms: current_timestamp_ms(),
    })
}

fn event_to_payload(event: Event) -> Option<FileWatchEvent> {
    if matches!(event.kind, EventKind::Access(_)) {
        return None;
    }

    let paths = normalize_event_paths(&event.paths);

    if paths.is_empty() {
        return None;
    }

    let (stale_paths, upsert_paths) = route_event_paths(&event.kind, &event.paths);
    let reconciliation_paths = if is_directory_event(&event.kind) {
        normalize_event_paths(&event.paths)
    } else {
        Vec::new()
    };

    Some(FileWatchEvent {
        event_type: event_type(&event.kind).to_string(),
        paths,
        stale_paths,
        upsert_paths,
        reconciliation_paths,
        timestamp_ms: current_timestamp_ms(),
    })
}

fn is_directory_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(notify::event::CreateKind::Folder)
            | EventKind::Remove(notify::event::RemoveKind::Folder)
            // notify does not reliably carry directory metadata for both sides of a
            // rename. Treat every rename conservatively so old and new roots both
            // receive a full reconciliation instead of leaving directory descendants active.
            | EventKind::Modify(ModifyKind::Name(_))
    )
}

fn route_event_paths(kind: &EventKind, paths: &[PathBuf]) -> (Vec<String>, Vec<String>) {
    match kind {
        EventKind::Remove(_) => (normalize_event_paths(paths), Vec::new()),
        EventKind::Create(_) => (Vec::new(), normalize_event_paths(paths)),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            if paths.len() >= 2 {
                (
                    normalize_event_paths(&paths[0..1]),
                    normalize_event_paths(&paths[1..2]),
                )
            } else {
                (Vec::new(), normalize_event_paths(paths))
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            (normalize_event_paths(paths), Vec::new())
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            (Vec::new(), normalize_event_paths(paths))
        }
        EventKind::Modify(ModifyKind::Name(_)) | EventKind::Modify(_) | EventKind::Any => {
            (Vec::new(), normalize_event_paths(paths))
        }
        EventKind::Access(_) | EventKind::Other => (Vec::new(), Vec::new()),
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::settings::ScanRootSetting;
    use notify::event::{AccessKind, EventAttributes, RenameMode};
    use std::cell::Cell;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[cfg(feature = "performance-test-tauri")]
    use std::{fs, time::SystemTime};

    #[cfg(feature = "performance-test-tauri")]
    struct WatcherTestTree(PathBuf);

    #[cfg(feature = "performance-test-tauri")]
    impl WatcherTestTree {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(".tmp-tests")
                .join(format!("zb-02-{label}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("create watcher test fixture");
            Self(path)
        }
    }

    #[cfg(feature = "performance-test-tauri")]
    impl Drop for WatcherTestTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn watch_paths_follow_enabled_absolute_scan_root_settings() {
        let folders = vec![
            scan_root("downloads", "/Users/zen/Downloads", true),
            scan_root("projects", "/Volumes/Work/Projects", true),
            scan_root("archive", "/Volumes/Archive", false),
        ];

        let paths = watch_paths_from_default_scan_folders(&folders);

        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/zen/Downloads"),
                PathBuf::from("/Volumes/Work/Projects")
            ]
        );
    }

    #[test]
    fn settings_reload_still_schedules_pending_roots_after_runtime_restart_failure() {
        let schedule_attempted = Cell::new(false);
        let result = restart_and_schedule_for_settings(
            || Err(SettingsWatcherReloadFailure::RuntimeRestart),
            || {
                schedule_attempted.set(true);
                Ok(())
            },
        );

        assert!(schedule_attempted.get());
        assert_eq!(result, Err(SettingsWatcherReloadFailure::RuntimeRestart));
    }

    #[test]
    fn watch_paths_ignore_disabled_empty_and_relative_roots() {
        let folders = vec![
            scan_root("downloads", "/Users/zen/Downloads", false),
            scan_root("empty", "", true),
            scan_root("relative", "Downloads", true),
        ];

        let paths = watch_paths_from_default_scan_folders(&folders);

        assert!(paths.is_empty());
    }

    #[test]
    fn watch_paths_include_enabled_custom_search_roots() {
        let settings = AppSettings {
            default_scan_folders: vec![scan_root("downloads", "/Users/zen/Downloads", true)],
            custom_search_roots: vec![
                search_root("projects", "/Users/zen/Projects", true),
                search_root("disabled", "/Users/zen/Disabled", false),
            ],
            ..AppSettings::default()
        };

        let paths = watch_paths_from_settings(&settings);

        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/zen/Downloads"),
                PathBuf::from("/Users/zen/Projects")
            ]
        );
    }

    #[test]
    fn legacy_watcher_paths_exclude_custom_search_roots() {
        let settings = AppSettings {
            default_scan_folders: vec![scan_root("downloads", "/Users/zen/Downloads", true)],
            custom_search_roots: vec![search_root("projects", "/Users/zen/Projects", true)],
            ..AppSettings::default()
        };

        assert_eq!(
            legacy_watch_paths_from_settings(&settings),
            vec![PathBuf::from("/Users/zen/Downloads")]
        );
    }

    #[test]
    fn file_watcher_manager_restarts_when_roots_change() {
        let manager = FileWatcherManager::default();
        let starts = Arc::new(AtomicUsize::new(0));
        let shutdowns = Arc::new(AtomicUsize::new(0));

        restart_test_session(&manager, "/tmp/root-a", &starts, &shutdowns);
        manager
            .restart_with_roots(
                vec![PathBuf::from("/tmp/root-a")],
                |_| panic!("unchanged roots should not restart"),
                |_, _| panic!("unchanged roots should not report a handoff gap"),
            )
            .expect("same roots");
        restart_test_session(&manager, "/tmp/root-b", &starts, &shutdowns);

        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);

        drop(manager);
        assert_eq!(shutdowns.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn file_watcher_manager_stops_when_roots_become_empty() {
        let manager = FileWatcherManager::default();
        let starts = Arc::new(AtomicUsize::new(0));
        let shutdowns = Arc::new(AtomicUsize::new(0));

        restart_test_session(&manager, "/tmp/root-a", &starts, &shutdowns);
        manager
            .restart_with_roots(
                Vec::new(),
                |_| panic!("empty roots should not start"),
                |_, _| {},
            )
            .expect("empty roots");

        assert_eq!(
            manager.active_roots().expect("active roots"),
            Vec::<PathBuf>::new()
        );
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn watcher_reload_stops_old_owner_before_starting_new_owner() {
        let manager = FileWatcherManager::default();
        let active_owners = Arc::new(AtomicUsize::new(0));
        let handoff_gaps = Arc::new(AtomicUsize::new(0));
        let starts = Arc::new(AtomicUsize::new(0));

        let active_for_first = Arc::clone(&active_owners);
        manager
            .restart_with_roots(
                vec![PathBuf::from("/tmp/root-a")],
                move |roots| {
                    active_for_first.fetch_add(1, Ordering::SeqCst);
                    Ok(WatcherSession::new(roots, move || {
                        active_for_first.fetch_sub(1, Ordering::SeqCst);
                    }))
                },
                |_, _| {},
            )
            .expect("start first owner");

        let active_for_second = Arc::clone(&active_owners);
        let starts_for_second = Arc::clone(&starts);
        let gaps_for_second = Arc::clone(&handoff_gaps);
        manager
            .restart_with_roots(
                vec![PathBuf::from("/tmp/root-b")],
                move |roots| {
                    assert_eq!(active_for_second.load(Ordering::SeqCst), 0);
                    starts_for_second.fetch_add(1, Ordering::SeqCst);
                    active_for_second.fetch_add(1, Ordering::SeqCst);
                    Ok(WatcherSession::new(roots, move || {
                        active_for_second.fetch_sub(1, Ordering::SeqCst);
                    }))
                },
                move |old_roots, new_roots| {
                    assert_eq!(old_roots, &[PathBuf::from("/tmp/root-a")]);
                    assert_eq!(new_roots, &[PathBuf::from("/tmp/root-b")]);
                    gaps_for_second.fetch_add(1, Ordering::SeqCst);
                },
            )
            .expect("handoff to second owner");

        assert_eq!(active_owners.load(Ordering::SeqCst), 1);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(handoff_gaps.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn watcher_reload_start_failure_leaves_no_owner_and_reports_reconciliation_gap() {
        let manager = FileWatcherManager::default();
        let gaps = Arc::new(AtomicUsize::new(0));
        let gaps_for_callback = Arc::clone(&gaps);

        let result = manager.restart_with_roots(
            vec![PathBuf::from("/tmp/root-a")],
            |_| {
                Err(WatcherError::MissingPath(
                    "synthetic start failure".to_string(),
                ))
            },
            move |old_roots, new_roots| {
                assert!(old_roots.is_empty());
                assert_eq!(new_roots, &[PathBuf::from("/tmp/root-a")]);
                gaps_for_callback.fetch_add(1, Ordering::SeqCst);
            },
        );

        assert!(result.is_err());
        assert!(manager.active_roots().expect("active roots").is_empty());
        assert_eq!(gaps.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn valid_existing_root_can_fail_during_runtime_start_without_database_contention() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-issue-329-watcher-runtime-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create a valid existing watcher root");
        assert!(root.is_dir());

        let manager = FileWatcherManager::default();
        let result = manager.restart_with_roots(
            vec![root.clone()],
            |_| Err(WatcherError::StateLock),
            |_, _| {},
        );

        assert!(matches!(result, Err(WatcherError::StateLock)));
        assert!(manager
            .active_roots()
            .expect("read active watcher roots")
            .is_empty());
        std::fs::remove_dir_all(root).expect("remove valid watcher root fixture");
    }

    #[test]
    fn watcher_reload_start_failure_after_handoff_keeps_no_old_or_new_owner() {
        let manager = FileWatcherManager::default();
        let active_owners = Arc::new(AtomicUsize::new(0));
        let gaps = Arc::new(AtomicUsize::new(0));
        let active_for_first = Arc::clone(&active_owners);
        manager
            .restart_with_roots(
                vec![PathBuf::from("/tmp/root-a")],
                move |roots| {
                    active_for_first.fetch_add(1, Ordering::SeqCst);
                    Ok(WatcherSession::new(roots, move || {
                        active_for_first.fetch_sub(1, Ordering::SeqCst);
                    }))
                },
                |_, _| {},
            )
            .expect("start old owner");

        let active_for_failed_start = Arc::clone(&active_owners);
        let active_for_callback = Arc::clone(&active_owners);
        let gaps_for_callback = Arc::clone(&gaps);
        let result = manager.restart_with_roots(
            vec![PathBuf::from("/tmp/root-b")],
            move |_| {
                assert_eq!(active_for_failed_start.load(Ordering::SeqCst), 0);
                Err(WatcherError::MissingPath(
                    "synthetic reload failure".to_string(),
                ))
            },
            move |old_roots, new_roots| {
                assert_eq!(active_for_callback.load(Ordering::SeqCst), 0);
                assert_eq!(old_roots, &[PathBuf::from("/tmp/root-a")]);
                assert_eq!(new_roots, &[PathBuf::from("/tmp/root-b")]);
                gaps_for_callback.fetch_add(1, Ordering::SeqCst);
            },
        );

        assert!(result.is_err());
        assert!(manager.active_roots().expect("active roots").is_empty());
        assert_eq!(active_owners.load(Ordering::SeqCst), 0);
        assert_eq!(gaps.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn event_to_payload_ignores_access_events() {
        let event = Event {
            kind: EventKind::Access(AccessKind::Read),
            paths: vec![PathBuf::from("/Users/zen/Documents/report.pdf")],
            attrs: EventAttributes::new(),
        };

        assert!(event_to_payload(event).is_none());
    }

    #[test]
    fn event_to_payload_splits_rename_old_and_new_paths() {
        let event = Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            paths: vec![
                PathBuf::from("/Users/zen/Documents/old.pdf"),
                PathBuf::from("/Users/zen/Documents/new.pdf"),
            ],
            attrs: EventAttributes::new(),
        };

        let payload = event_to_payload(event).expect("rename payload");

        assert_eq!(payload.event_type, "renamed");
        assert_eq!(payload.stale_paths, vec!["/Users/zen/Documents/old.pdf"]);
        assert_eq!(payload.upsert_paths, vec!["/Users/zen/Documents/new.pdf"]);
        assert_eq!(
            payload.paths,
            vec![
                "/Users/zen/Documents/old.pdf".to_string(),
                "/Users/zen/Documents/new.pdf".to_string()
            ]
        );
        assert_eq!(
            payload.reconciliation_paths,
            vec![
                "/Users/zen/Documents/old.pdf".to_string(),
                "/Users/zen/Documents/new.pdf".to_string()
            ]
        );
    }

    #[test]
    fn event_to_payload_routes_delete_and_create_paths() {
        let deleted = event_to_payload(Event {
            kind: EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![PathBuf::from("/Users/zen/Documents/deleted.pdf")],
            attrs: EventAttributes::new(),
        })
        .expect("delete payload");
        let created = event_to_payload(Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/Users/zen/Documents/created.pdf")],
            attrs: EventAttributes::new(),
        })
        .expect("create payload");

        assert_eq!(
            deleted.stale_paths,
            vec!["/Users/zen/Documents/deleted.pdf"]
        );
        assert!(deleted.upsert_paths.is_empty());
        assert_eq!(
            created.upsert_paths,
            vec!["/Users/zen/Documents/created.pdf"]
        );
        assert!(created.stale_paths.is_empty());
    }

    #[test]
    fn directory_events_are_marked_for_full_reconciliation() {
        let payload = event_to_payload(Event {
            kind: EventKind::Create(notify::event::CreateKind::Folder),
            paths: vec![PathBuf::from("/Users/zen/Documents/new-folder")],
            attrs: EventAttributes::new(),
        })
        .expect("directory create payload");

        assert_eq!(
            payload.reconciliation_paths,
            vec!["/Users/zen/Documents/new-folder"]
        );
    }

    #[test]
    fn same_root_directory_rename_marks_old_and_new_paths_for_reconciliation() {
        let payload = event_to_payload(Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            paths: vec![
                PathBuf::from("/Users/zen/Documents/old-folder"),
                PathBuf::from("/Users/zen/Documents/new-folder"),
            ],
            attrs: EventAttributes::new(),
        })
        .expect("directory rename payload");

        assert_eq!(
            payload.reconciliation_paths,
            vec![
                "/Users/zen/Documents/old-folder".to_string(),
                "/Users/zen/Documents/new-folder".to_string()
            ]
        );
    }

    #[test]
    fn cross_root_directory_rename_marks_both_roots_for_reconciliation() {
        let payload = event_to_payload(Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            paths: vec![
                PathBuf::from("/Users/zen/Downloads/old-folder"),
                PathBuf::from("/Users/zen/Projects/new-folder"),
            ],
            attrs: EventAttributes::new(),
        })
        .expect("cross-root directory rename payload");

        assert_eq!(payload.stale_paths, vec!["/Users/zen/Downloads/old-folder"]);
        assert_eq!(payload.upsert_paths, vec!["/Users/zen/Projects/new-folder"]);
        assert_eq!(
            payload.reconciliation_paths,
            vec![
                "/Users/zen/Downloads/old-folder".to_string(),
                "/Users/zen/Projects/new-folder".to_string()
            ]
        );
    }

    #[test]
    fn watcher_root_matching_is_boundary_aware() {
        assert!(path_within_root(
            "/Users/zen/Library",
            "/Users/zen/Library/report.pdf"
        ));
        assert!(path_within_root("C:/Library", "C:/Library/report.pdf"));
        assert!(!path_within_root(
            "/Users/zen/Library",
            "/Users/zen/Library-old/report.pdf"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn persisted_watcher_scope_avoids_unwatched_nested_ambiguity_and_keeps_watched_overlap() {
        let temp =
            std::env::temp_dir().join(format!("pm02b-watcher-routing-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("create isolated watcher routing database root");
        let db = Database::open(temp.join("test.sqlite3")).expect("open watcher routing database");
        let conn = db.conn().expect("open fixture connection");
        conn.execute("INSERT INTO scan_roots(id,normalized_path,display_name,source_kind,enabled,health_status,created_at,updated_at) VALUES('parent','C:/Fixture/Root','Parent','file_library',1,'healthy',1,1)", []).expect("insert parent root");
        conn.execute("INSERT INTO scan_roots(id,normalized_path,display_name,source_kind,enabled,health_status,created_at,updated_at) VALUES('child','C:/Fixture/Root/child','Child','file_library',1,'healthy',1,1)", []).expect("insert nested ad-hoc root");
        drop(conn);

        let parent_setting = scan_root("parent-setting", "c:\\fixture\\root", true);
        let mut settings = AppSettings {
            default_scan_folders: vec![parent_setting.clone()],
            ..AppSettings::default()
        };
        crate::settings::save_app_settings(&db, &settings).expect("persist parent watcher scope");
        db.sync_file_library_watcher_roots(&settings.default_scan_folders)
            .expect("sync parent watcher scope");

        let path = "C:/Fixture/Root/child/file.txt";
        let parent_only = db
            .list_watcher_root_configs()
            .expect("parent watcher roots");
        let matches = parent_only
            .iter()
            .filter(|root| path_within_root(&root.path, path))
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "unwatched nested scan roots must not create route ambiguity"
        );
        assert_eq!(matches[0].id, "parent");

        settings.default_scan_folders.push(scan_root(
            "child-setting",
            "c:\\fixture\\root\\child",
            true,
        ));
        crate::settings::save_app_settings(&db, &settings)
            .expect("persist overlapping watcher scope");
        db.sync_file_library_watcher_roots(&settings.default_scan_folders)
            .expect("sync overlapping watcher scope");
        let both_watched = db
            .list_watcher_root_configs()
            .expect("overlapping watcher roots");
        let matches = both_watched
            .iter()
            .filter(|root| path_within_root(&root.path, path))
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            2,
            "genuine watched overlaps must remain ambiguous"
        );
        assert!(matches.iter().any(|root| root.id == "parent"));
        assert!(matches.iter().any(|root| root.id == "child"));

        drop(db);
        std::fs::remove_dir_all(temp).expect("remove isolated watcher routing database root");
    }

    #[cfg(all(windows, feature = "performance-test-tauri"))]
    #[test]
    fn windows_recommended_watcher_publishes_real_file_create_and_append() {
        struct Fixture {
            root: PathBuf,
            database_path: PathBuf,
            remove_parent: bool,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.root);
                for suffix in ["", "-wal", "-shm"] {
                    let _ =
                        std::fs::remove_file(format!("{}{}", self.database_path.display(), suffix));
                }
                if self.remove_parent {
                    if let Some(parent) = self.root.parent() {
                        let _ = std::fs::remove_dir(parent);
                    }
                }
            }
        }

        fn durable_state(db: &Database, root_id: &str) -> (i64, i64, i64, Option<i64>) {
            db.conn()
                .expect("open durable-state connection")
                .query_row(
                    "SELECT watcher_revision, watcher_applied_revision, library_change_revision, watcher_last_event_at FROM scan_roots WHERE id=?1",
                    [root_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .expect("read durable watcher state")
        }

        fn wait_for_publication(
            db: &Database,
            root_id: &str,
            previous: (i64, i64, i64, Option<i64>),
            file_path: &Path,
        ) -> (i64, i64, i64, Option<i64>) {
            let deadline = Instant::now() + Duration::from_secs(12);
            loop {
                let current = durable_state(db, root_id);
                if current.0 > previous.0
                    && current.1 >= current.0
                    && current.2 > previous.2
                    && current.3.is_some()
                {
                    let normalized_path =
                        crate::db::normalize_path_text(&file_path.to_string_lossy());
                    let persisted_size = db
                        .conn()
                        .expect("open file publication connection")
                        .query_row(
                            "SELECT size FROM files WHERE path=?1 AND is_stale=0",
                            [&normalized_path],
                            |row| row.get::<_, i64>(0),
                        )
                        .expect("read persisted file size");
                    assert_eq!(
                        persisted_size,
                        std::fs::metadata(file_path).unwrap().len() as i64
                    );
                    return current;
                }
                assert!(
                    Instant::now() < deadline,
                    "real RecommendedWatcher did not publish within the bounded deadline: current={current:?}, previous={previous:?}"
                );
                thread::sleep(Duration::from_millis(25));
            }
        }

        let supplied_root =
            std::env::var_os("ZC_NATIVE_QA_WATCHER_FIXTURE_ROOT").map(PathBuf::from);
        let remove_parent = supplied_root.is_none();
        let root = supplied_root.unwrap_or_else(|| {
            std::env::temp_dir()
                .join(format!("pm02b-windows-notify-{}", uuid::Uuid::new_v4()))
                .join("watch-root")
        });
        assert!(root.is_absolute(), "watch fixture root must be absolute");
        assert!(
            !root.exists(),
            "watch fixture root must be fresh and disposable: {}",
            root.display()
        );
        let parent = root.parent().expect("fixture root parent").to_path_buf();
        std::fs::create_dir_all(&parent).expect("create fixture parent");
        std::fs::create_dir_all(&root).expect("create disposable watched root");
        let database_path = parent.join(format!("state-{}.sqlite3", uuid::Uuid::new_v4()));
        let _fixture = Fixture {
            root: root.clone(),
            database_path: database_path.clone(),
            remove_parent,
        };

        let db = Database::open(&database_path).expect("open isolated watcher database");
        let setting = scan_root("native-event-root", &root.to_string_lossy(), true);
        let settings = AppSettings {
            default_scan_folders: vec![setting.clone()],
            ..AppSettings::default()
        };
        crate::settings::save_app_settings(&db, &settings).expect("persist enabled default root");
        db.sync_file_library_watcher_roots(&settings.default_scan_folders)
            .expect("enroll default scan folder as a watcher-owned root");
        let root_config = db
            .list_watcher_root_configs()
            .expect("load settings-owned watcher roots")
            .into_iter()
            .find(|root_config| {
                root_config.path == crate::db::normalize_path_text(&root.to_string_lossy())
            })
            .expect("persistent watcher-owned root from default_scan_folders");

        let backend_enabled = backend_watcher_reconciliation_enabled();
        native_qa_watcher_trace("windows_backend_test", || {
            format!("backend_reconciliation_enabled={backend_enabled}")
        });
        assert!(
            backend_enabled,
            "Windows filesystem publication test requires the backend watcher path"
        );

        let app = tauri::test::mock_app();
        let app_handle = app.handle().clone();
        let manager = FileWatcherManager::default();
        let jobs = ScanJobManager::default();
        let dedupe_jobs = DedupeJobManager::default();
        let watch_paths =
            existing_watch_paths_from_default_scan_folders(&settings.default_scan_folders);
        assert_eq!(watch_paths.len(), 1);
        assert!(manager
            .restart_backend(app_handle, watch_paths, db.clone(), jobs, dedupe_jobs,)
            .expect("start production backend watcher session"));
        assert_eq!(
            manager.active_roots().expect("active watcher roots"),
            vec![root.canonicalize().expect("canonical watched root")]
        );

        let before_create = durable_state(&db, &root_config.id);
        let file_path = root.join("native-event.txt");
        std::fs::write(&file_path, b"first durable event").expect("create file through std::fs");
        let after_create = wait_for_publication(&db, &root_config.id, before_create, &file_path);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&file_path)
            .expect("open file for a real append");
        use std::io::Write as _;
        file.write_all(b" plus append")
            .expect("append through std::fs");
        drop(file);
        let after_append = wait_for_publication(&db, &root_config.id, after_create, &file_path);

        assert!(after_create.0 > before_create.0);
        assert!(after_create.1 >= after_create.0);
        assert!(after_create.2 > before_create.2);
        assert!(after_create.3.is_some());
        assert!(after_append.0 > after_create.0);
        assert!(after_append.1 >= after_append.0);
        assert!(after_append.2 > after_create.2);
        assert!(after_append.3.is_some());
    }

    #[test]
    fn overflow_signal_is_once_per_burst() {
        let burst_active = AtomicBool::new(false);
        let signal = AtomicBool::new(false);

        signal_overflow(&burst_active, &signal);
        signal_overflow(&burst_active, &signal);
        assert!(signal.swap(false, Ordering::AcqRel));
        assert!(!signal.load(Ordering::Acquire));

        signal_overflow(&burst_active, &signal);
        assert!(!signal.load(Ordering::Acquire));
        burst_active.store(false, Ordering::Release);
        signal_overflow(&burst_active, &signal);
        assert!(signal.load(Ordering::Acquire));
    }

    #[test]
    fn idle_watcher_receiver_unblocks_on_explicit_stop() {
        let (tx, rx) = mpsc::sync_channel(1);
        let stop_requested = Arc::new(AtomicBool::new(false));
        let stop_for_waiter = Arc::clone(&stop_requested);
        let (waiting_tx, waiting_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            waiting_tx.send(()).expect("announce idle receiver");
            assert!(recv_watcher_input(&rx, &stop_for_waiter).is_none());
            finished_tx.send(()).expect("announce receiver exit");
        });

        waiting_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("receiver reached idle wait");
        assert!(finished_rx.recv_timeout(Duration::from_millis(25)).is_err());
        stop_requested.store(true, Ordering::Release);
        tx.try_send(WatcherInput::Stop)
            .expect("send explicit stop signal");
        finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("idle receiver exits promptly");
        waiter.join().expect("idle receiver exits");
    }

    #[test]
    fn idle_watcher_receiver_delivers_filesystem_events_without_a_timer_tick() {
        let (tx, rx) = mpsc::sync_channel(1);
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/Users/zen/Documents/created.pdf")],
            attrs: EventAttributes::new(),
        };
        tx.try_send(WatcherInput::Notify(Ok(event)))
            .expect("queue filesystem event");

        assert!(matches!(
            recv_watcher_input(&rx, &AtomicBool::new(false)),
            Some(WatcherInput::Notify(Ok(_)))
        ));
    }

    #[test]
    fn watcher_lifecycle_cancels_delayed_reconciliation_wakes() {
        let manager = FileWatcherManager::default();
        let (cancel_tx, cancel_rx) = mpsc::channel();
        manager
            .reconciliation_retries
            .lock()
            .expect("retry registry")
            .insert("root-a".to_string(), (i64::MAX, cancel_tx));

        manager.cancel_reconciliation_retries();

        assert!(cancel_rx.recv_timeout(Duration::from_secs(1)).is_ok());
        assert!(manager
            .reconciliation_retries
            .lock()
            .expect("retry registry")
            .is_empty());
    }

    #[cfg(feature = "performance-test-tauri")]
    #[test]
    fn filesystem_event_reconciles_only_the_affected_root_without_a_periodic_tick() {
        let fixture = WatcherTestTree::new("event-reconciliation");
        let first_root_path = fixture.0.join("library-a");
        let second_root_path = fixture.0.join("library-b");
        fs::create_dir_all(&first_root_path).expect("create first managed root");
        fs::create_dir_all(&second_root_path).expect("create second managed root");
        let db = Database::open(fixture.0.join("state.sqlite3")).expect("open watcher test db");
        let roots = [
            scan_root("library-a", &first_root_path.to_string_lossy(), true),
            scan_root("library-b", &second_root_path.to_string_lossy(), true),
        ];
        crate::settings::save_app_settings(
            &db,
            &AppSettings {
                default_scan_folders: roots.to_vec(),
                ..AppSettings::default()
            },
        )
        .expect("persist settings-owned roots");
        db.sync_file_library_watcher_roots(&roots)
            .expect("sync managed roots");
        let app = tauri::test::mock_app();
        app.manage(FileWatcherManager::default());
        let app_handle = app.handle().clone();
        let jobs = ScanJobManager::default();
        let dedupe_jobs = DedupeJobManager::default();
        let initial_roots = db.list_scan_roots().expect("list initial managed roots");
        assert_eq!(
            crate::scanner::schedule_watcher_reconciliations(
                app_handle.clone(),
                db.clone(),
                jobs.clone(),
                dedupe_jobs.clone(),
            )
            .expect("schedule initial root reconciliation"),
            2
        );
        for root in &initial_roots {
            wait_for_watcher_root_reconciliation(&db, &root.id, root.watcher_revision);
        }
        let first = db
            .get_scan_root_health(None, Some(&first_root_path.to_string_lossy()))
            .expect("read first root before event");
        let second = db
            .get_scan_root_health(None, Some(&second_root_path.to_string_lossy()))
            .expect("read second root before event");
        let new_directory = first_root_path.join("new-folder");
        fs::create_dir_all(&new_directory).expect("create directory from watcher event");
        let payload = event_to_payload(Event {
            kind: EventKind::Create(notify::event::CreateKind::Folder),
            paths: vec![new_directory],
            attrs: EventAttributes::new(),
        })
        .expect("directory event requests reconciliation");

        process_backend_payload(&app_handle, &db, &jobs, &dedupe_jobs, payload);

        let reconciled =
            wait_for_watcher_root_reconciliation(&db, &first.id, first.watcher_revision + 1);
        let untouched = db
            .get_scan_root_health(Some(&second.id), None)
            .expect("read unaffected root after event");
        assert_eq!(
            reconciled.watcher_revision,
            reconciled.watcher_applied_revision
        );
        assert!(!reconciled.needs_reconciliation);
        assert!(reconciled.last_successful_generation.is_some());
        assert_eq!(untouched.watcher_revision, second.watcher_revision);
        assert_eq!(untouched.current_generation, second.current_generation);
        assert!(untouched.active_run_id.is_none());
    }

    #[cfg(feature = "performance-test-tauri")]
    #[test]
    fn overflow_marks_managed_roots_for_durable_reconciliation() {
        let fixture = WatcherTestTree::new("overflow-state");
        let root_path = fixture.0.join("library");
        fs::create_dir_all(&root_path).expect("create managed root");
        let db = Database::open(fixture.0.join("state.sqlite3")).expect("open watcher test db");
        let root = scan_root("library", &root_path.to_string_lossy(), true);
        crate::settings::save_app_settings(
            &db,
            &AppSettings {
                default_scan_folders: vec![root.clone()],
                ..AppSettings::default()
            },
        )
        .expect("persist settings-owned root");
        db.sync_file_library_watcher_roots(&[root])
            .expect("sync managed root");
        let app = tauri::test::mock_app();
        let app_handle = app.handle().clone();
        let before = db
            .get_scan_root_health(None, Some(&root_path.to_string_lossy()))
            .expect("read root before overflow");

        mark_all_roots_for_reconciliation(
            &app_handle,
            &db,
            "watcher_overflow",
            "The bounded watcher queue overflowed; a managed scan is required.",
        );

        let after = db
            .get_scan_root_health(Some(&before.id), None)
            .expect("read root after overflow");
        assert!(after.needs_reconciliation);
        assert!(after.watcher_revision > before.watcher_revision);
        assert_eq!(
            after.watcher_last_error_code.as_deref(),
            Some("watcher_overflow")
        );
    }

    #[cfg(feature = "performance-test-tauri")]
    #[test]
    fn non_directory_managed_root_keeps_permission_required_health() {
        let fixture = WatcherTestTree::new("permission-state");
        let file_path = fixture.0.join("not-a-directory");
        fs::write(&file_path, b"fixture").expect("create non-directory root target");
        let db = Database::open(fixture.0.join("state.sqlite3")).expect("open watcher test db");
        db.sync_file_library_watcher_roots(&[scan_root(
            "library",
            &file_path.to_string_lossy(),
            true,
        )])
        .expect("sync managed root");
        let app = tauri::test::mock_app();
        app.manage(FileWatcherManager::default());
        let root = db
            .get_scan_root_health(None, Some(&file_path.to_string_lossy()))
            .expect("read root before permission check");

        let scheduled = crate::scanner::schedule_watcher_reconciliations(
            app.handle().clone(),
            db.clone(),
            ScanJobManager::default(),
            DedupeJobManager::default(),
        )
        .expect("check watcher roots");

        let after = db
            .get_scan_root_health(Some(&root.id), None)
            .expect("read root after permission check");
        assert_eq!(scheduled, 0);
        assert_eq!(after.health_status, "permission_required");
        assert_eq!(
            after.watcher_last_error_code.as_deref(),
            Some("permission_required")
        );
    }

    #[test]
    fn bounded_retry_recovers_after_a_transient_rule_failure() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let delays = Arc::new(AtomicUsize::new(0));
        let attempts_for_operation = Arc::clone(&attempts);
        let delays_for_callback = Arc::clone(&delays);

        let result = bounded_retry(
            3,
            move || {
                let attempt = attempts_for_operation.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err("temporary rule failure")
                } else {
                    Ok("recovered")
                }
            },
            move |_| {
                delays_for_callback.fetch_add(1, Ordering::SeqCst);
            },
        );

        assert_eq!(result.expect("bounded retry recovery"), "recovered");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert_eq!(delays.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn bounded_retry_returns_the_last_permanent_rule_failure() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_operation = Arc::clone(&attempts);

        let result = bounded_retry(
            3,
            move || {
                attempts_for_operation.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>("permanent rule failure")
            },
            |_| {},
        );

        assert_eq!(
            result.expect_err("permanent failure must remain visible"),
            "permanent rule failure"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn backend_owner_switch_defaults_on_and_accepts_explicit_legacy_values() {
        assert!(backend_watcher_reconciliation_enabled_value(None));
        assert!(backend_watcher_reconciliation_enabled_value(Some("true")));
        assert!(!backend_watcher_reconciliation_enabled_value(Some("false")));
        assert!(!backend_watcher_reconciliation_enabled_value(Some("0")));
        assert!(!backend_watcher_reconciliation_enabled_value(Some(
            "disabled"
        )));
    }

    fn scan_root(id: &str, path: &str, enabled: bool) -> ScanRootSetting {
        ScanRootSetting {
            id: id.to_string(),
            path: path.to_string(),
            label: id.to_string(),
            enabled,
            created_at: "2026-06-22T00:00:00.000Z".to_string(),
        }
    }

    #[cfg(feature = "performance-test-tauri")]
    fn wait_for_watcher_root_reconciliation(
        db: &Database,
        root_id: &str,
        minimum_revision: i64,
    ) -> crate::db::scan::ScanRootDto {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let root = db
                .get_scan_root_health(Some(root_id), None)
                .expect("read watcher root reconciliation state");
            if root.watcher_revision >= minimum_revision
                && root.watcher_revision == root.watcher_applied_revision
                && !root.needs_reconciliation
                && root.active_run_id.is_none()
            {
                return root;
            }
            assert!(
                Instant::now() < deadline,
                "watcher root reconciliation did not finish: {root:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn watcher_payloads_coalesce_and_keep_the_latest_route_for_each_path() {
        let path = "/Users/zen/Documents/report.pdf".to_string();
        let created = FileWatchEvent {
            event_type: "create".to_string(),
            paths: vec![path.clone()],
            stale_paths: Vec::new(),
            upsert_paths: vec![path.clone()],
            reconciliation_paths: Vec::new(),
            timestamp_ms: 1,
        };
        let removed = FileWatchEvent {
            event_type: "remove".to_string(),
            paths: vec![path.clone()],
            stale_paths: vec![path.clone()],
            upsert_paths: Vec::new(),
            reconciliation_paths: Vec::new(),
            timestamp_ms: 2,
        };

        let payload = coalesce_payloads(vec![created, removed]).expect("coalesced payload");

        assert_eq!(payload.event_type, "batch");
        assert_eq!(payload.paths, vec![path.clone()]);
        assert_eq!(payload.stale_paths, vec![path]);
        assert!(payload.upsert_paths.is_empty());
    }

    fn search_root(id: &str, path: &str, enabled: bool) -> SearchRootSetting {
        SearchRootSetting {
            id: id.to_string(),
            path: path.to_string(),
            label: id.to_string(),
            enabled,
            created_at: "2026-06-22T00:00:00.000Z".to_string(),
        }
    }

    fn restart_test_session(
        manager: &FileWatcherManager,
        root: &str,
        starts: &Arc<AtomicUsize>,
        shutdowns: &Arc<AtomicUsize>,
    ) {
        let starts = Arc::clone(starts);
        let shutdowns = Arc::clone(shutdowns);
        manager
            .restart_with_roots(
                vec![PathBuf::from(root)],
                move |roots| {
                    starts.fetch_add(1, Ordering::SeqCst);
                    Ok(WatcherSession::new(roots, move || {
                        shutdowns.fetch_add(1, Ordering::SeqCst);
                    }))
                },
                |_, _| {},
            )
            .expect("restart test session");
    }
}

fn event_type(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::Create(_) => "created",
        EventKind::Remove(_) => "deleted",
        EventKind::Modify(ModifyKind::Name(_)) => "renamed",
        EventKind::Modify(_) => "modified",
        EventKind::Access(_) => "accessed",
        EventKind::Any => "changed",
        EventKind::Other => "other",
    }
}

fn normalize_watch_roots(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, WatcherError> {
    let mut roots = Vec::new();

    for path in paths {
        if !path.exists() {
            return Err(WatcherError::MissingPath(normalize_path(&path)));
        }
        if !path.is_dir() {
            return Err(WatcherError::NotDirectory(normalize_path(&path)));
        }

        let canonical = path.canonicalize()?;
        if roots.iter().any(|root| root == &canonical) {
            continue;
        }
        roots.push(canonical);
    }

    Ok(roots)
}

fn is_ignored_path(path: &Path) -> bool {
    path.components()
        .any(|component| is_ignored_dir_name(component.as_os_str()))
}

fn normalize_event_paths(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| !is_ignored_path(path))
        .map(|path| normalize_path(path))
        .collect()
}

fn normalize_path(path: &Path) -> String {
    crate::db::normalize_path_text(&path.to_string_lossy())
}

fn looks_absolute_path(path: &str) -> bool {
    Path::new(path).is_absolute()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.as_bytes().get(0..3).is_some_and(|prefix| {
            prefix[0].is_ascii_alphabetic()
                && prefix[1] == b':'
                && (prefix[2] == b'/' || prefix[2] == b'\\')
        })
}

fn current_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
