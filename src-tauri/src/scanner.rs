use crate::db::scan::{
    ScanAdmissionOptions, ScanBatchInput, ScanErrorInput, ScanFinalization, ScanFinalizeInput,
    ScanRunRecord, WatcherReconciliationAdmission,
};
use crate::db::{
    current_unix_seconds, emit_search_index_optimized, run_search_index_optimize, Database,
    DbError, InsertFileRequest, LibraryScope, RuleExecutionMode,
};
use crate::dedupe::{spawn_duplicate_detection, DedupeJobManager};
use crate::file_workspace::WorkClass;
use crate::ids::new_job_id;
#[cfg(test)]
use crate::path_filter::is_ignored_dir_name;
use crate::path_filter::{metadata_is_link_or_reparse, ManagedPathFilter};
use crate::scheduler::{
    adapters::ManagedScanResourceLeaseAdapter, AcquireError, CancellationToken, ResourceLease,
};
use crate::window_auth::require_main_window;
use jwalk::{ClientState, DirEntry, Parallelism, WalkDir};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Runtime, State, WebviewWindow};
use thiserror::Error;

pub use crate::db::scan::{
    ManagedScanRequest, ManagedScanSnapshotDto, ManagedScanStartDto, ScanRootDto, ScanRunDto,
    ScanSessionDto, ScanSessionRootDto,
};

const SCAN_BATCH_SIZE: usize = 500;
const SCAN_EMIT_INTERVAL: Duration = Duration::from_millis(200);
const SCAN_STARTED_EVENT: &str = "scan-started";
const SCAN_BATCH_EVENT: &str = "scan-batch";
const SCAN_PROGRESS_EVENT: &str = "scan-progress";
const SCAN_COMPLETE_EVENT: &str = "scan-complete";
const SCAN_CANCELED_EVENT: &str = "scan-canceled";
const SCAN_ERROR_EVENT: &str = "scan-error";
pub const MANAGED_SCAN_EVENT: &str = "scan-run-updated";

fn should_prune_directory(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::package::is_package(path)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}
const RULE_RECOVERY_MAX_ATTEMPTS: usize = 3;
const RULE_RECOVERY_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_millis(250), Duration::from_millis(500)];

#[derive(Debug, Error)]
enum ScanError {
    #[error("scan root does not exist: {0}")]
    MissingRoot(String),
    #[error("scan root is not a readable file-system path: {0}")]
    InvalidRoot(String),
    #[error("metadata error at {path}: {source}")]
    Metadata {
        path: String,
        #[source]
        source: jwalk::Error,
    },
    #[error("database scan operation failed: {0}")]
    Database(#[from] DbError),
    #[error("scan task failed: {0}")]
    Join(String),
    #[error("scheduler resource acquisition failed: {0}")]
    Scheduler(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedScanEvent {
    pub event_id: String,
    pub run_id: String,
    pub scan_root_id: String,
    pub parent_session_id: Option<String>,
    pub generation: i64,
    pub run_revision: i64,
    pub session_revision: i64,
    pub status: String,
    pub run_phase: String,
    pub session_phase: String,
    pub scanned_files: i64,
    pub scanned_directories: i64,
    pub processed_bytes: i64,
    pub warnings_count: i64,
    pub errors_count: i64,
    pub current_path: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedEntry {
    pub path: String,
    pub name: String,
    pub extension: String,
    pub size: u64,
    pub mtime: i64,
    pub ctime: i64,
    pub is_dir: bool,
    pub state_code: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgressPayload {
    pub job_id: String,
    pub job_kind: String,
    pub root: String,
    pub scanned: u64,
    pub files: u64,
    pub directories: u64,
    pub skipped: u64,
    pub errors: u64,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStartedPayload {
    pub job_id: String,
    pub job_kind: String,
    pub root: String,
    pub batch_size: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanBatchPayload {
    pub job_id: String,
    pub job_kind: String,
    pub root: String,
    pub batch_index: u64,
    pub entries: Vec<ScannedEntry>,
    pub progress: ScanProgressPayload,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanErrorPayload {
    pub job_id: String,
    pub job_kind: String,
    pub root: String,
    pub path: String,
    pub message: String,
}

pub type ScanSummary = ScanProgressPayload;

struct ScanJobEntry {
    generation: u64,
    token: Arc<AtomicBool>,
}

struct ScanJobRegistry {
    jobs: HashMap<String, ScanJobEntry>,
    next_generation: u64,
}

impl Default for ScanJobRegistry {
    fn default() -> Self {
        Self {
            jobs: HashMap::new(),
            next_generation: 1,
        }
    }
}

#[derive(Clone, Default)]
pub struct ScanJobManager(Arc<Mutex<ScanJobRegistry>>);

struct ScanJobGuard {
    manager: ScanJobManager,
    job_id: String,
    generation: u64,
    token: Arc<AtomicBool>,
    released: bool,
}

impl ScanJobGuard {
    fn token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.token)
    }

    fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.manager
            .release_if_current(&self.job_id, self.generation, &self.token);
    }
}

impl Drop for ScanJobGuard {
    fn drop(&mut self) {
        self.release();
    }
}

impl ScanJobManager {
    fn register(&self, job_id: &str) -> Result<ScanJobGuard, String> {
        let job_id = job_id.trim();
        if job_id.is_empty() || job_id.len() > 128 {
            return Err("A valid scan job ID is required.".to_string());
        }
        let mut jobs = self
            .0
            .lock()
            .map_err(|_| "Scan job manager is unavailable.".to_string())?;
        if jobs.jobs.contains_key(job_id) {
            return Err(format!("Scan job already exists: {job_id}."));
        }
        let token = Arc::new(AtomicBool::new(false));
        let generation = jobs.next_generation;
        jobs.next_generation = jobs.next_generation.wrapping_add(1).max(1);
        jobs.jobs.insert(
            job_id.to_string(),
            ScanJobEntry {
                generation,
                token: Arc::clone(&token),
            },
        );
        Ok(ScanJobGuard {
            manager: self.clone(),
            job_id: job_id.to_string(),
            generation,
            token,
            released: false,
        })
    }

    fn token(&self, job_id: &str) -> Option<Arc<AtomicBool>> {
        self.0
            .lock()
            .ok()?
            .jobs
            .get(job_id.trim())
            .map(|entry| Arc::clone(&entry.token))
    }

    fn cancel(&self, job_id: &str) -> bool {
        let Ok(jobs) = self.0.lock() else {
            return false;
        };
        let Some(token) = jobs
            .jobs
            .get(job_id.trim())
            .map(|entry| Arc::clone(&entry.token))
        else {
            return false;
        };
        token.store(true, Ordering::Release);
        crate::scheduler::WorkScheduler::global().cancel_session(job_id.trim());
        true
    }

    pub fn cancel_all(&self) -> usize {
        let job_ids = {
            let Ok(jobs) = self.0.lock() else {
                return 0;
            };
            for entry in jobs.jobs.values() {
                entry.token.store(true, Ordering::Release);
            }
            jobs.jobs.keys().cloned().collect::<Vec<_>>()
        };
        let canceled = job_ids.len();
        for job_id in job_ids {
            crate::scheduler::WorkScheduler::global().cancel_session(&job_id);
        }
        canceled
    }

    fn release_if_current(&self, job_id: &str, generation: u64, token: &Arc<AtomicBool>) {
        if let Ok(mut jobs) = self.0.lock() {
            let is_current = jobs.jobs.get(job_id.trim()).is_some_and(|entry| {
                entry.generation == generation && Arc::ptr_eq(&entry.token, token)
            });
            if is_current {
                jobs.jobs.remove(job_id.trim());
            }
        }
    }
}

fn register_scan_guards(
    jobs: &ScanJobManager,
    runs: &[ScanRunDto],
) -> Result<HashMap<String, ScanJobGuard>, String> {
    let mut guards = HashMap::with_capacity(runs.len());
    for run in runs {
        guards.insert(run.id.clone(), jobs.register(&run.id)?);
    }
    Ok(guards)
}

#[tauri::command]
pub async fn start_managed_scan<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    jobs: State<'_, ScanJobManager>,
    dedupe_jobs: State<'_, DedupeJobManager>,
    request: ManagedScanRequest,
) -> Result<ManagedScanStartDto, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    let jobs = jobs.inner().clone();
    let dedupe_jobs = dedupe_jobs.inner().clone();
    let admission = db
        .admit_managed_scan(&ScanAdmissionOptions {
            request: request.clone(),
            run_id_override: None,
        })
        .map_err(|error| error.to_string())?;
    let start = ManagedScanStartDto {
        session: admission.session.clone(),
        runs: admission.runs.clone(),
    };

    for run in &admission.runs {
        if let Ok(record) = db.get_scan_run_record(&run.id) {
            emit_managed_event_best_effort(&app, &db, &record, None);
        }
    }

    if admission.created && !admission.runs.is_empty() {
        let guards = register_scan_guards(&jobs, &admission.runs)?;
        let session_id = admission.session.id.clone();
        let jobs_for_task = jobs.clone();
        let work = ManagedSessionWork {
            session_id,
            runs: admission.runs,
            guards,
            legacy: None,
        };
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = run_managed_session(app, db, jobs_for_task, dedupe_jobs, work) {
                eprintln!("Managed scan session failed: {error}");
            }
        });
    }

    Ok(start)
}

#[tauri::command]
pub fn get_scan_run(db: State<'_, Database>, run_id: String) -> Result<ScanRunDto, String> {
    db.inner()
        .get_scan_run_record(&run_id)
        .map(|record| record.dto)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_managed_scan_snapshot(
    db: State<'_, Database>,
    session_id: String,
) -> Result<ManagedScanSnapshotDto, String> {
    db.inner()
        .get_managed_scan_snapshot(&session_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_scan_runs(
    db: State<'_, Database>,
    session_id: Option<String>,
    root_id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ScanRunDto>, String> {
    db.inner()
        .list_scan_runs(
            session_id.as_deref(),
            root_id.as_deref(),
            limit.unwrap_or(100),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_scan_roots(db: State<'_, Database>) -> Result<Vec<ScanRootDto>, String> {
    db.inner()
        .list_scan_roots()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_scan_root_health(
    db: State<'_, Database>,
    root_id: Option<String>,
    path: Option<String>,
) -> Result<ScanRootDto, String> {
    db.inner()
        .get_scan_root_health(root_id.as_deref(), path.as_deref())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn cancel_scan_run<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    jobs: State<'_, ScanJobManager>,
    dedupe_jobs: State<'_, DedupeJobManager>,
    run_id: String,
) -> Result<ScanRunDto, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    let run = db
        .request_scan_cancellation(&run_id)
        .map_err(|error| error.to_string())?;
    if run.dto.cancel_requested || is_terminal_scan_status(&run.dto.status) {
        jobs.cancel(&run_id);
    }
    let dedupe_parent = run.dto.parent_session_id.as_deref().unwrap_or(&run_id);
    dedupe_jobs.cancel_for_scan(dedupe_parent);
    emit_managed_event_best_effort(&app, &db, &run, None);
    Ok(run.dto)
}

#[tauri::command]
pub async fn retry_interrupted_scan<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    jobs: State<'_, ScanJobManager>,
    dedupe_jobs: State<'_, DedupeJobManager>,
    run_id: String,
) -> Result<ManagedScanStartDto, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    let jobs = jobs.inner().clone();
    let dedupe_jobs = dedupe_jobs.inner().clone();
    let previous = db
        .get_scan_run_record(&run_id)
        .map_err(|error| error.to_string())?;
    if previous.dto.status != "interrupted" {
        return Err("Only an interrupted scan run can be retried.".to_string());
    }
    let admission = db
        .admit_managed_scan(&ScanAdmissionOptions {
            request: ManagedScanRequest {
                roots: vec![previous.dto.root_path.clone()],
                request_key: None,
                dedupe: false,
            },
            run_id_override: None,
        })
        .map_err(|error| error.to_string())?;
    let start = ManagedScanStartDto {
        session: admission.session.clone(),
        runs: admission.runs.clone(),
    };
    for run in &admission.runs {
        if let Ok(record) = db.get_scan_run_record(&run.id) {
            emit_managed_event_best_effort(&app, &db, &record, None);
        }
    }
    if admission.created && !admission.runs.is_empty() {
        let guards = register_scan_guards(&jobs, &admission.runs)?;
        let jobs_for_task = jobs.clone();
        let work = ManagedSessionWork {
            session_id: admission.session.id,
            runs: admission.runs,
            guards,
            legacy: None,
        };
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = run_managed_session(app, db, jobs_for_task, dedupe_jobs, work) {
                eprintln!("Retried scan session failed: {error}");
            }
        });
    }
    Ok(start)
}

#[tauri::command]
pub fn create_scan_job_id(job_kind: String) -> Result<String, String> {
    match job_kind.trim() {
        "foreground" => Ok(new_job_id("scan-foreground")),
        "background" => Ok(new_job_id("scan-background")),
        _ => Err("Scan job kind must be foreground or background.".to_string()),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn scan_directory<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    jobs: State<'_, ScanJobManager>,
    dedupe_jobs: State<'_, DedupeJobManager>,
    path: String,
    include_entries: bool,
    job_id: String,
    job_kind: String,
    run_dedupe: Option<bool>,
) -> Result<ScanSummary, String> {
    require_main_window(&window)?;
    if !matches!(job_kind.as_str(), "foreground" | "background") {
        return Err("Scan job kind must be foreground or background.".to_string());
    }
    let job_id = job_id.trim().to_string();
    if job_id.is_empty() || job_id.len() > 128 {
        return Err("A valid scan job ID is required.".to_string());
    }
    let db = db.inner().clone();
    let jobs = jobs.inner().clone();
    let dedupe_jobs = dedupe_jobs.inner().clone();
    let request = ManagedScanRequest {
        roots: vec![path],
        request_key: Some(job_id.clone()),
        dedupe: run_dedupe.unwrap_or(true),
    };
    let admission = db
        .admit_managed_scan(&ScanAdmissionOptions {
            request,
            run_id_override: Some(job_id.clone()),
        })
        .map_err(|error| error.to_string())?;
    if !admission.created {
        let run = admission
            .runs
            .first()
            .ok_or_else(|| "The legacy scan request has no effective run.".to_string())?;
        if is_terminal_scan_status(&run.status) {
            return legacy_summary_or_error(run, &job_kind, 0, Instant::now());
        }
        return Err(format!(
            "Scan request is already active: {}.",
            admission.session.id
        ));
    }
    let run = admission
        .runs
        .first()
        .ok_or_else(|| "The legacy scan request has no effective run.".to_string())?;
    let run_id = run.id.clone();
    let guard = jobs.register(&run_id)?;
    let legacy = LegacyScanContext {
        job_kind,
        include_entries,
    };
    let legacy_job_kind = legacy.job_kind.clone();
    let session_id = admission.session.id;
    let run_ids = admission.runs;
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<ScanSummary, String> {
        run_managed_session(
            app,
            db.clone(),
            jobs,
            dedupe_jobs,
            ManagedSessionWork {
                session_id,
                runs: run_ids,
                guards: HashMap::from([(run_id.clone(), guard)]),
                legacy: Some(legacy),
            },
        )
        .map_err(|error| error.to_string())?;
        let record = db
            .get_scan_run_record(&run_id)
            .map_err(|error| error.to_string())?;
        legacy_summary_or_error(&record.dto, &legacy_job_kind, 0, Instant::now())
    })
    .await
    .map_err(|error| ScanError::Join(error.to_string()).to_string())?;
    result
}

#[tauri::command]
pub fn cancel_scan<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    jobs: State<'_, ScanJobManager>,
    dedupe_jobs: State<'_, DedupeJobManager>,
    job_id: String,
) -> Result<(), String> {
    require_main_window(&window)?;
    let run = db
        .inner()
        .request_scan_cancellation(&job_id)
        .map_err(|error| error.to_string())?;
    if run.dto.cancel_requested || is_terminal_scan_status(&run.dto.status) {
        jobs.cancel(&job_id);
    }
    let dedupe_parent = run.dto.parent_session_id.as_deref().unwrap_or(&job_id);
    dedupe_jobs.cancel_for_scan(dedupe_parent);
    if is_terminal_scan_status(&run.dto.status) || jobs.token(&job_id).is_some() {
        Ok(())
    } else {
        Err(format!("Scan job not found: {job_id}."))
    }
}

pub fn recover_scan_state(db: &Database) -> Result<usize, DbError> {
    let recovered = db.recover_interrupted_scan_runs()?;
    db.prune_scan_observations()?;
    Ok(recovered)
}

pub(crate) fn schedule_watcher_reconciliations<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    jobs: ScanJobManager,
    dedupe_jobs: DedupeJobManager,
) -> Result<usize, String> {
    schedule_watcher_reconciliations_for_roots(app, db, jobs, dedupe_jobs, None)
}

pub(crate) fn schedule_watcher_reconciliation_roots<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    jobs: ScanJobManager,
    dedupe_jobs: DedupeJobManager,
    root_ids: Vec<String>,
) -> Result<usize, String> {
    schedule_watcher_reconciliations_for_roots(app, db, jobs, dedupe_jobs, Some(root_ids))
}

fn schedule_watcher_reconciliations_for_roots<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    jobs: ScanJobManager,
    dedupe_jobs: DedupeJobManager,
    root_ids: Option<Vec<String>>,
) -> Result<usize, String> {
    let roots = if let Some(root_ids) = root_ids {
        let mut seen = HashSet::new();
        root_ids
            .into_iter()
            .filter(|root_id| seen.insert(root_id.clone()))
            .map(|root_id| {
                db.get_scan_root_health(Some(&root_id), None)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        db.list_scan_roots().map_err(|error| error.to_string())?
    };
    let mut scheduled = 0;
    for root in roots
        .into_iter()
        .filter(|root| root.enabled && root.source_kind == "file_library")
    {
        let path = PathBuf::from(&root.normalized_path);
        if !path.exists() {
            crate::watcher::cancel_reconciliation_retry(&app, &root.id);
            db.mark_watcher_root_missing(
                &root.id,
                "missing",
                "Managed scan root is not available; automatic reconciliation is paused.",
            )
            .map_err(|error| error.to_string())?;
            continue;
        }
        if !path.is_dir() {
            crate::watcher::cancel_reconciliation_retry(&app, &root.id);
            db.mark_watcher_root_missing(
                &root.id,
                "permission_required",
                "Managed scan root is not a readable directory; automatic reconciliation is paused.",
            )
            .map_err(|error| error.to_string())?;
            continue;
        }
        if !root.needs_reconciliation
            && !root.watcher_rule_recovery_required
            && root.watcher_revision <= root.watcher_applied_revision
        {
            crate::watcher::cancel_reconciliation_retry(&app, &root.id);
            continue;
        }

        let request_key = match db
            .next_watcher_reconciliation_admission(
                &root.id,
                root.watcher_revision,
                current_unix_seconds(),
            )
            .map_err(|error| error.to_string())?
        {
            WatcherReconciliationAdmission::Start { request_key, .. } => {
                crate::watcher::cancel_reconciliation_retry(&app, &root.id);
                request_key
            }
            WatcherReconciliationAdmission::Active => {
                crate::watcher::cancel_reconciliation_retry(&app, &root.id);
                continue;
            }
            WatcherReconciliationAdmission::Backoff { retry_at } => {
                crate::watcher::schedule_reconciliation_retry(
                    app.clone(),
                    db.clone(),
                    jobs.clone(),
                    dedupe_jobs.clone(),
                    root.id.clone(),
                    retry_at,
                );
                continue;
            }
            WatcherReconciliationAdmission::Exhausted { attempts } => {
                crate::watcher::cancel_reconciliation_retry(&app, &root.id);
                let rule_recovery_exhausted = root.watcher_rule_recovery_required
                    || root.watcher_last_error_code.as_deref() == Some("watcher_rule_failure")
                    || root.watcher_last_error_code.as_deref()
                        == Some("watcher_rule_retry_exhausted");
                let exhausted_code = if rule_recovery_exhausted {
                    "watcher_rule_retry_exhausted"
                } else {
                    "watcher_reconciliation_retry_exhausted"
                };
                if root.watcher_last_error_code.as_deref() != Some(exhausted_code) {
                    let exhausted_message = if rule_recovery_exhausted {
                        format!(
                            "Automatic watcher rule recovery exhausted after {attempts} attempts; manual rule recovery is required."
                        )
                    } else {
                        format!(
                            "Automatic watcher reconciliation exhausted after {attempts} attempts; use manual retry."
                        )
                    };
                    db.mark_watcher_reconciliation(&root.id, exhausted_code, &exhausted_message)
                        .map_err(|error| error.to_string())?;
                }
                continue;
            }
        };
        let request = ManagedScanRequest {
            roots: vec![root.normalized_path.clone()],
            request_key: Some(request_key),
            dedupe: false,
        };
        let admission = match db.admit_managed_scan(&ScanAdmissionOptions {
            request,
            run_id_override: None,
        }) {
            Ok(admission) => admission,
            Err(error) if error.to_string().contains("active run") => continue,
            Err(error) if error.to_string().contains("active root lease") => continue,
            Err(error) => {
                db.mark_watcher_reconciliation(
                    &root.id,
                    "reconciliation_admission_failed",
                    &error.to_string(),
                )
                .map_err(|mark_error| mark_error.to_string())?;
                continue;
            }
        };
        if !admission.created || admission.runs.is_empty() {
            continue;
        }
        let guards = register_scan_guards(&jobs, &admission.runs)?;
        let session_id = admission.session.id.clone();
        let run_ids = admission.runs.clone();
        let app_for_task = app.clone();
        let db_for_task = db.clone();
        let jobs_for_task = jobs.clone();
        let dedupe_for_task = dedupe_jobs.clone();
        let work = ManagedSessionWork {
            session_id,
            runs: run_ids,
            guards,
            legacy: None,
        };
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = run_managed_session(
                app_for_task,
                db_for_task,
                jobs_for_task,
                dedupe_for_task,
                work,
            ) {
                eprintln!("Watcher reconciliation session failed: {error}");
            }
        });
        scheduled += 1;
    }
    Ok(scheduled)
}

pub fn resume_pending_dedupe_dispatches<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    dedupe_jobs: DedupeJobManager,
) -> Result<usize, DbError> {
    resume_dedupe_dispatches(&db, 1000, |session| {
        spawn_duplicate_detection(
            app.clone(),
            db.clone(),
            dedupe_jobs.clone(),
            Some(session.id.clone()),
        )
        .map_err(|error| error.to_string())
    })
}

fn resume_dedupe_dispatches<F>(
    db: &Database,
    limit: usize,
    mut dispatch: F,
) -> Result<usize, DbError>
where
    F: FnMut(&ScanSessionDto) -> Result<String, String>,
{
    let candidates = db.list_dedupe_dispatch_candidates(limit)?;
    let mut resumed = 0;
    for session in candidates {
        let Some(dispatching) = db.claim_dedupe_dispatch(&session.id)? else {
            continue;
        };
        match dispatch(&dispatching) {
            Ok(job_id) => {
                db.record_dedupe_dispatch(
                    &dispatching.id,
                    dispatching.revision,
                    Some(&job_id),
                    None,
                )?;
                resumed += 1;
            }
            Err(error) => {
                db.record_dedupe_dispatch(
                    &dispatching.id,
                    dispatching.revision,
                    None,
                    Some(&error),
                )?;
            }
        }
    }
    Ok(resumed)
}

#[derive(Debug, Clone)]
struct LegacyScanContext {
    job_kind: String,
    include_entries: bool,
}

struct ManagedSessionWork {
    session_id: String,
    runs: Vec<ScanRunDto>,
    guards: HashMap<String, ScanJobGuard>,
    legacy: Option<LegacyScanContext>,
}

#[derive(Debug, Clone, Copy)]
struct LedgerCursor {
    run_revision: i64,
    root_revision: i64,
    session_revision: i64,
}

impl LedgerCursor {
    fn from_record(record: &ScanRunRecord) -> Self {
        Self {
            run_revision: record.dto.revision,
            root_revision: record.root_revision,
            session_revision: record.session_revision,
        }
    }

    fn update(&mut self, record: &ScanRunRecord) {
        *self = Self::from_record(record);
    }
}

fn scan_work_class(legacy: Option<&LegacyScanContext>) -> WorkClass {
    match legacy.map(|legacy| legacy.job_kind.as_str()) {
        Some("foreground") => WorkClass::Foreground,
        Some("background") | None => WorkClass::Background,
        Some(_) => WorkClass::Background,
    }
}

fn acquire_scan_resource_lease(
    run_id: &str,
    cancel_flag: &Arc<AtomicBool>,
    legacy: Option<&LegacyScanContext>,
) -> Result<Option<ResourceLease>, ScanError> {
    let adapter = ManagedScanResourceLeaseAdapter::global();
    let cancellation = CancellationToken::from_flag(Arc::clone(cancel_flag));
    let class = scan_work_class(legacy);
    if is_scan_cancelled(cancel_flag) {
        return Ok(None);
    }
    match adapter.acquire(run_id, class, cancellation) {
        Ok(lease) => Ok(Some(lease)),
        Err(AcquireError::Cancelled) => Ok(None),
        Err(error) => Err(ScanError::Scheduler(error.to_string())),
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
pub(crate) struct PerformanceManagedScan {
    pub(crate) run_id: String,
    pub(crate) run_ids: Vec<String>,
    pub(crate) worker: std::thread::JoinHandle<()>,
}

/// Start the existing managed scan admission, durable run and worker path for
/// the File Workspace performance harness.  This is deliberately test-only:
/// the production command remains the sole IPC entry point, while this seam
/// lets the release performance binary exercise the real scanner authority
/// without manufacturing a second job system or a fake lease holder.
#[cfg(all(test, feature = "performance-test-tauri"))]
pub(crate) fn start_performance_managed_scan<R>(
    app: AppHandle<R>,
    db: Database,
    jobs: ScanJobManager,
    dedupe_jobs: DedupeJobManager,
    root: PathBuf,
    job_id: String,
    job_kind: &str,
) -> Result<PerformanceManagedScan, String>
where
    R: Runtime + 'static,
{
    start_performance_managed_scan_roots(app, db, jobs, dedupe_jobs, vec![root], job_id, job_kind)
}

/// Start one real managed-session worker over several fixture roots. The
/// causal matrix uses this to hold total workload constant while varying only
/// how many scanner workers contend for the existing scheduler leases.
#[cfg(all(test, feature = "performance-test-tauri"))]
pub(crate) fn start_performance_managed_scan_roots<R>(
    app: AppHandle<R>,
    db: Database,
    jobs: ScanJobManager,
    dedupe_jobs: DedupeJobManager,
    roots: Vec<PathBuf>,
    job_id: String,
    job_kind: &str,
) -> Result<PerformanceManagedScan, String>
where
    R: Runtime + 'static,
{
    if !matches!(job_kind, "foreground" | "background") {
        return Err("performance scan job kind must be foreground or background".to_string());
    }
    if roots.is_empty() {
        return Err("performance managed scan requires at least one root".to_string());
    }
    let request = ManagedScanRequest {
        roots: roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect(),
        request_key: Some(job_id.clone()),
        dedupe: false,
    };
    let admission = db
        .admit_managed_scan(&ScanAdmissionOptions {
            request,
            run_id_override: Some(job_id),
        })
        .map_err(|error| error.to_string())?;
    if !admission.created || admission.runs.is_empty() {
        return Err("performance managed scan admission did not create a run".to_string());
    }
    let run_ids = admission
        .runs
        .iter()
        .map(|run| run.id.clone())
        .collect::<Vec<_>>();
    let run_id = run_ids
        .first()
        .cloned()
        .ok_or_else(|| "performance managed scan admission returned no run".to_string())?;
    let guards = match register_scan_guards(&jobs, &admission.runs) {
        Ok(guards) => guards,
        Err(error) => {
            for run_id in &run_ids {
                let _ = db.request_scan_cancellation(run_id);
            }
            return Err(error);
        }
    };
    let session_id = admission.session.id.clone();
    let admitted_runs = admission.runs;
    let legacy = LegacyScanContext {
        job_kind: job_kind.to_string(),
        include_entries: true,
    };
    let worker_session_id = session_id.clone();
    let work = ManagedSessionWork {
        session_id: worker_session_id,
        runs: admitted_runs,
        guards,
        legacy: Some(legacy),
    };
    let worker = std::thread::spawn(move || {
        if let Err(error) = run_managed_session(app, db, jobs, dedupe_jobs, work) {
            eprintln!("Performance managed scan session failed: {error}");
        }
    });
    Ok(PerformanceManagedScan {
        run_id,
        run_ids,
        worker,
    })
}

/// Mirror the production cancellation boundary after the test-only worker has
/// been admitted.  The durable run is cancelled first; the process-local job
/// token then releases the scanner's scheduler lease and traversal promptly.
#[cfg(all(test, feature = "performance-test-tauri"))]
pub(crate) fn cancel_performance_managed_scan(
    db: &Database,
    jobs: &ScanJobManager,
    dedupe_jobs: &DedupeJobManager,
    run_id: &str,
) -> Result<(), String> {
    let run = db
        .request_scan_cancellation(run_id)
        .map_err(|error| error.to_string())?;
    if run.dto.cancel_requested || is_terminal_scan_status(&run.dto.status) {
        jobs.cancel(run_id);
    }
    let dedupe_parent = run.dto.parent_session_id.as_deref().unwrap_or(run_id);
    dedupe_jobs.cancel_for_scan(dedupe_parent);
    Ok(())
}

fn scan_walk_parallelism(resource_lease: &ResourceLease) -> Parallelism {
    let admitted_cpu_parallelism = resource_lease.resources().cpu.max(1) as usize;
    if admitted_cpu_parallelism == 1 {
        // Keep traversal on the scanner worker that already carries the
        // scheduler's background QoS. A one-thread Rayon pool would move the
        // actual filesystem work to a different, unscoped worker.
        Parallelism::Serial
    } else {
        Parallelism::RayonNewPool(admitted_cpu_parallelism)
    }
}

fn run_managed_session<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    jobs: ScanJobManager,
    dedupe_jobs: DedupeJobManager,
    work: ManagedSessionWork,
) -> Result<(), ScanError> {
    let ManagedSessionWork {
        session_id,
        runs,
        mut guards,
        legacy,
    } = work;
    let watcher_root_ids = if legacy.is_none() {
        runs.iter()
            .map(|run| run.scan_root_id.clone())
            .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    for run in runs {
        let Some(guard) = guards.remove(&run.id) else {
            continue;
        };
        let cancel_flag = guard.token();
        let result = run_scan_run(
            &app,
            &db,
            &dedupe_jobs,
            &session_id,
            &run.id,
            cancel_flag,
            legacy.as_ref(),
        );
        if let Err(error) = result {
            let record = db.get_scan_run_record(&run.id).ok();
            let still_active = record
                .as_ref()
                .is_some_and(|record| !is_terminal_scan_status(&record.dto.status));
            if still_active {
                if let Err(abort_error) = abort_scan_run(&app, &db, &run.id, error) {
                    eprintln!(
                        "Managed scan run {} failed to finalize: {abort_error}",
                        run.id
                    );
                }
            } else if record
                .as_ref()
                .is_none_or(|record| record.dto.status != "cancelled")
            {
                eprintln!("Managed scan run {} failed: {error}", run.id);
            }
        }
    }
    if !watcher_root_ids.is_empty() {
        if let Err(error) = schedule_watcher_reconciliation_roots(
            app.clone(),
            db.clone(),
            jobs.clone(),
            dedupe_jobs.clone(),
            watcher_root_ids.into_iter().collect(),
        ) {
            eprintln!("Unable to schedule watcher reconciliation after scan completion: {error}");
        }
    }
    Ok(())
}

fn run_scan_run<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    dedupe_jobs: &DedupeJobManager,
    _session_id: &str,
    run_id: &str,
    cancel_flag: Arc<AtomicBool>,
    legacy: Option<&LegacyScanContext>,
) -> Result<(), ScanError> {
    let claimed = match db.claim_queued_scan_run(run_id) {
        Ok(record) => record,
        Err(error) => {
            if db
                .get_scan_run_record(run_id)
                .ok()
                .is_some_and(|record| is_terminal_scan_status(&record.dto.status))
            {
                return Ok(());
            }
            return Err(error.into());
        }
    };
    if claimed.dto.status != "running" {
        return Ok(());
    }
    let started_at = Instant::now();
    let root_label = claimed.dto.root_path.clone();
    let mut cursor = LedgerCursor::from_record(&claimed);
    emit_managed_event_best_effort(app, db, &claimed, Some(root_label.clone()));
    if let Some(legacy) = legacy {
        emit_legacy_started(app, run_id, legacy, &root_label);
    }

    let skipped = Arc::new(AtomicU64::new(0));
    let resource_lease = match acquire_scan_resource_lease(run_id, &cancel_flag, legacy) {
        Ok(Some(lease)) => lease,
        Ok(None) => {
            let finalization = finish_scan_run(
                db,
                run_id,
                "cancelled",
                Some("cancelled"),
                Some("The scan was cancelled while waiting for scheduler capacity."),
                false,
                false,
            )?;
            emit_terminal_events(
                app,
                db,
                &finalization,
                skipped.load(Ordering::Acquire),
                started_at,
                legacy,
            );
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let _qos_scope = crate::resource_governor::scope_thread_qos(scan_work_class(legacy));
    let skipped_for_filter = Arc::clone(&skipped);
    let mut batch = ScanBatchBuffer::new(started_at);
    let root = PathBuf::from(&root_label);
    let rule_recovery_required = db
        .get_scan_root_health(Some(&claimed.dto.scan_root_id), None)?
        .watcher_rule_recovery_required;

    if let Err(error) = validate_root(&root) {
        // An unopenable root means the scan never ran, which is a failed job (axis 1) —
        // not "finished with incomplete coverage".  Carrying the specific error code
        // through also lets finalization resolve root health to `missing` or
        // `permission_required` instead of the generic reconciliation state.
        let root_error = scan_error_input_for_root(&error, &root_label);
        let root_error_code = root_error.error_code.clone();
        let root_error_message = root_error.error_message.clone();
        batch.push_error(root_error);
        if let Err(error) = flush_scan_batch(
            app,
            db,
            run_id,
            &mut cursor,
            &mut batch,
            &skipped,
            started_at,
            legacy,
        ) {
            return abort_scan_run(app, db, run_id, error);
        }
        let finalization = finish_scan_run(
            db,
            run_id,
            "failed",
            Some(&root_error_code),
            Some(&root_error_message),
            false,
            false,
        )?;
        emit_terminal_events(
            app,
            db,
            &finalization,
            skipped.load(Ordering::Acquire),
            started_at,
            legacy,
        );
        return Ok(());
    }

    let path_filter = ManagedPathFilter::new(&root);
    let walker = WalkDir::new(&root)
        .parallelism(scan_walk_parallelism(&resource_lease))
        .skip_hidden(true)
        .follow_links(false)
        .process_read_dir(move |_depth, path, _state, children| {
            if should_prune_directory(path) {
                skipped_for_filter.fetch_add(children.len() as u64, Ordering::Relaxed);
                children.clear();
                return;
            }
            children.retain(|entry_result| match entry_result {
                Ok(entry)
                    if path_filter.excludes(&entry.path(), entry.file_type().is_dir())
                        || entry.file_type().is_dir()
                            && entry
                                .metadata()
                                .is_ok_and(|metadata| metadata_is_link_or_reparse(&metadata)) =>
                {
                    skipped_for_filter.fetch_add(1, Ordering::Relaxed);
                    false
                }
                _ => true,
            });
        });

    for entry_result in walker {
        if is_scan_cancelled(&cancel_flag) {
            break;
        }

        match entry_result {
            Ok(entry) => match entry_to_payload(&entry) {
                Ok(Some(payload)) => {
                    batch.push_entry(payload);
                }
                Ok(None) => {
                    skipped.fetch_add(1, Ordering::Relaxed);
                }
                Err(error) => {
                    batch.push_error(scan_error_input_for_error(&error));
                }
            },
            Err(error) => {
                batch.push_error(ScanErrorInput {
                    path: Some(root_label.clone()),
                    error_code: "traversal_error".to_string(),
                    error_message: error.to_string(),
                    affects_coverage: true,
                    metadata_error: false,
                });
            }
        }

        if batch.should_flush(Instant::now()) {
            if let Err(error) = flush_scan_batch(
                app,
                db,
                run_id,
                &mut cursor,
                &mut batch,
                &skipped,
                started_at,
                legacy,
            ) {
                return abort_scan_run(app, db, run_id, error);
            }
        }
    }

    let durable_before_flush = db.get_scan_run_record(run_id)?;
    let cancelled = is_scan_cancelled(&cancel_flag)
        || durable_before_flush.dto.cancel_requested
        || durable_before_flush.dto.status == "cancelling";
    if !cancelled && !batch.is_empty() {
        if let Err(error) = flush_scan_batch(
            app,
            db,
            run_id,
            &mut cursor,
            &mut batch,
            &skipped,
            started_at,
            legacy,
        ) {
            return abort_scan_run(app, db, run_id, error);
        }
    } else if cancelled {
        batch.clear();
    }

    let latest = db.get_scan_run_record(run_id)?;
    if cancelled || latest.dto.cancel_requested || latest.dto.status == "cancelling" {
        let finalization = finish_scan_run(
            db,
            run_id,
            "cancelled",
            Some("cancelled"),
            Some("The scan was cancelled before finalization."),
            false,
            false,
        )?;
        emit_terminal_events(
            app,
            db,
            &finalization,
            skipped.load(Ordering::Acquire),
            started_at,
            legacy,
        );
        return Ok(());
    }
    cursor.update(&latest);

    if latest.dto.coverage_error_count > 0 {
        let finalization = finish_scan_run(
            db,
            run_id,
            "requires_reconciliation",
            Some("coverage_incomplete"),
            Some("The scan encountered a metadata or traversal error; stale reconciliation was skipped."),
            false,
            false,
        )?;
        emit_terminal_events(
            app,
            db,
            &finalization,
            skipped.load(Ordering::Acquire),
            started_at,
            legacy,
        );
        return Ok(());
    }

    let stale_gate_enabled = should_run_stale_cleanup(is_scan_cancelled(&cancel_flag));
    if stale_gate_enabled {
        let reconciled = db.reconcile_missing(
            run_id,
            cursor.run_revision,
            cursor.root_revision,
            cursor.session_revision,
        )?;
        cursor.update(&reconciled);
    } else {
        // The kill switch is engaged.  That is an operator decision, not evidence that
        // the index drifted, so the run stays on the normal success path: the terminal
        // status, root health and generation pointer are all unaffected.  We only make
        // the skipped check visible.  `finish_scan_run` records the same fact durably
        // via `result_json.staleReconciliation = false`.
        eprintln!(
            "scan run {run_id}: stale reconciliation was skipped because the \
             ZEN_CANVAS_SCAN_STALE_RECONCILIATION kill switch is disabled; deleted \
             files stay indexed until it is re-enabled"
        );
    }
    let optimizing = db.transition_scan_run_phase(
        run_id,
        cursor.run_revision,
        cursor.root_revision,
        "optimizing_search",
    )?;
    cursor.update(&optimizing);
    emit_managed_event_best_effort(app, db, &optimizing, None);

    let mut rule_recovery_succeeded = false;
    let mut rule_recovery_error = None;
    if rule_recovery_required {
        match execute_rules_for_root_with_retry(db, &root_label) {
            Ok(()) => rule_recovery_succeeded = true,
            Err(error) => rule_recovery_error = Some(error.to_string()),
        }
    }

    let report = run_search_index_optimize("scan_complete", db);
    emit_search_index_optimized(app, &report);
    let mut completed_with_warnings = false;
    if !report.success {
        completed_with_warnings = true;
        let warned = db.record_scan_warning(
            run_id,
            cursor.run_revision,
            cursor.root_revision,
            cursor.session_revision,
            "search_index_optimize_failed",
            report
                .error
                .as_deref()
                .unwrap_or("Search index optimize failed."),
        )?;
        cursor.update(&warned);
        emit_managed_event_best_effort(app, db, &warned, None);
    }
    let finalizing = db.transition_scan_run_phase(
        run_id,
        cursor.run_revision,
        cursor.root_revision,
        "finalizing",
    )?;
    cursor.update(&finalizing);
    let finalization = finish_scan_run(
        db,
        run_id,
        if completed_with_warnings || rule_recovery_error.is_some() {
            "completed_with_warnings"
        } else {
            "completed"
        },
        rule_recovery_error
            .as_deref()
            .map(|_| "watcher_rule_failure"),
        rule_recovery_error.as_deref(),
        stale_gate_enabled,
        rule_recovery_succeeded,
    )?;
    emit_terminal_events(
        app,
        db,
        &finalization,
        skipped.load(Ordering::Acquire),
        started_at,
        legacy,
    );
    if finalization.dedupe_pending {
        dispatch_dedupe_if_pending(app, db, dedupe_jobs.clone(), &finalization, legacy);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn flush_scan_batch<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    run_id: &str,
    cursor: &mut LedgerCursor,
    batch: &mut ScanBatchBuffer,
    skipped: &AtomicU64,
    started_at: Instant,
    legacy: Option<&LegacyScanContext>,
) -> Result<ScanRunRecord, ScanError> {
    if batch.is_empty() {
        return db.get_scan_run_record(run_id).map_err(ScanError::from);
    }
    let requests = batch
        .entries
        .iter()
        .map(scanned_entry_to_insert_request)
        .collect::<Vec<_>>();
    let current_path = batch.current_path();
    let input = ScanBatchInput {
        entries: &requests,
        errors: &batch.errors,
        scanned_files: batch.scanned_files,
        scanned_directories: batch.scanned_directories,
        processed_bytes: batch.processed_bytes,
        warnings: batch.warnings,
    };
    let updated = db.persist_scan_batch(
        run_id,
        cursor.run_revision,
        cursor.root_revision,
        cursor.session_revision,
        &input,
    )?;
    cursor.update(&updated);
    let entries = std::mem::take(&mut batch.entries);
    let errors = std::mem::take(&mut batch.errors);
    let progress = progress_payload_from_record(
        &updated,
        legacy
            .map(|value| value.job_kind.as_str())
            .unwrap_or("managed"),
        skipped.load(Ordering::Acquire),
        started_at,
    );
    emit_managed_event_best_effort(app, db, &updated, current_path);
    if let Some(legacy) = legacy {
        app.emit(
            SCAN_BATCH_EVENT,
            scan_batch_payload(
                &updated.dto.id,
                &legacy.job_kind,
                &updated.dto.root_path,
                batch.batch_index,
                entries.clone(),
                progress.clone(),
                legacy.include_entries,
            ),
        )
        .ok();
        app.emit(SCAN_PROGRESS_EVENT, progress).ok();
        for error in errors {
            app.emit(
                SCAN_ERROR_EVENT,
                ScanErrorPayload {
                    job_id: updated.dto.id.clone(),
                    job_kind: legacy.job_kind.clone(),
                    root: updated.dto.root_path.clone(),
                    path: error.path.unwrap_or_else(|| updated.dto.root_path.clone()),
                    message: error.error_message,
                },
            )
            .ok();
        }
    }
    batch.scanned_files = 0;
    batch.scanned_directories = 0;
    batch.processed_bytes = 0;
    batch.warnings = 0;
    batch.batch_index += 1;
    batch.last_emit_at = Instant::now();
    batch.entries.reserve(SCAN_BATCH_SIZE);
    Ok(updated)
}

fn execute_rules_for_root_with_retry(db: &Database, root_path: &str) -> Result<(), DbError> {
    let scope = LibraryScope::Roots {
        roots: vec![root_path.to_string()],
    };
    crate::watcher::bounded_retry(
        RULE_RECOVERY_MAX_ATTEMPTS,
        || {
            db.execute_authoritative_rules_for_scope(
                &scope,
                RuleExecutionMode::AllChangedOrRuleChanged,
            )
            .map(|_| ())
        },
        |attempt| {
            let delay =
                RULE_RECOVERY_RETRY_DELAYS[attempt.min(RULE_RECOVERY_RETRY_DELAYS.len() - 1)];
            std::thread::sleep(delay);
        },
    )
}

fn finish_scan_run(
    db: &Database,
    run_id: &str,
    desired_status: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
    allow_stale_reconciliation: bool,
    rule_recovery_succeeded: bool,
) -> Result<ScanFinalization, ScanError> {
    let current = db.get_scan_run_record(run_id)?;
    if is_terminal_scan_status(&current.dto.status) {
        return terminal_finalization(db, current);
    }
    let terminal_status = if current.dto.cancel_requested || current.dto.status == "cancelling" {
        "cancelled"
    } else {
        desired_status
    };
    let input = ScanFinalizeInput {
        terminal_status: terminal_status.to_string(),
        error_code: error_code.map(str::to_string),
        error_message: error_message.map(str::to_string),
        allow_stale_reconciliation,
        rule_recovery_succeeded,
    };
    db.finalize_scan_run(
        run_id,
        current.dto.revision,
        current.root_revision,
        current.session_revision,
        &input,
    )
    .map_err(ScanError::from)
}

fn terminal_finalization(db: &Database, run: ScanRunRecord) -> Result<ScanFinalization, ScanError> {
    let session_id = run
        .dto
        .parent_session_id
        .as_deref()
        .ok_or_else(|| DbError::Validation("Scan run has no parent session.".to_string()))?;
    let session = db.get_scan_session(session_id)?;
    Ok(ScanFinalization {
        dedupe_pending: session.dedupe_dispatch_state == "pending",
        run,
        session,
    })
}

fn abort_scan_run<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    run_id: &str,
    error: ScanError,
) -> Result<(), ScanError> {
    let finalization = finish_scan_run(
        db,
        run_id,
        "failed",
        Some("worker_failure"),
        Some(&error.to_string()),
        false,
        false,
    );
    if let Ok(finalization) = finalization {
        emit_managed_event_best_effort(app, db, &finalization.run, None);
    }
    Err(error)
}

fn dispatch_dedupe_if_pending<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    dedupe_jobs: DedupeJobManager,
    finalization: &ScanFinalization,
    _legacy: Option<&LegacyScanContext>,
) {
    let _ = dispatch_dedupe_session(
        app,
        db,
        dedupe_jobs,
        &finalization.session.id,
        Some(&finalization.run),
    );
}

fn dispatch_dedupe_session<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    dedupe_jobs: DedupeJobManager,
    session_id: &str,
    event_run: Option<&ScanRunRecord>,
) -> Result<bool, DbError> {
    let Some(dispatching) = db.claim_dedupe_dispatch(session_id)? else {
        return Ok(false);
    };
    if let Some(run) = event_run {
        emit_managed_event_with_session_best_effort(app, run, &dispatching);
    }
    let dispatch_result = spawn_duplicate_detection(
        app.clone(),
        db.clone(),
        dedupe_jobs,
        Some(session_id.to_string()),
    );
    let session = match dispatch_result {
        Ok(job_id) => {
            db.record_dedupe_dispatch(session_id, dispatching.revision, Some(&job_id), None)?
        }
        Err(error) => {
            db.record_dedupe_dispatch(session_id, dispatching.revision, None, Some(&error))?
        }
    };
    if let Some(run) = event_run {
        emit_managed_event_with_session_best_effort(app, run, &session);
    }
    Ok(true)
}

fn emit_terminal_events<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    finalization: &ScanFinalization,
    skipped: u64,
    started_at: Instant,
    legacy: Option<&LegacyScanContext>,
) {
    emit_managed_event_with_session_best_effort(app, &finalization.run, &finalization.session);
    crate::watcher::emit_watcher_reconciliation_status(
        app,
        db,
        &finalization.run.dto.scan_root_id,
        None,
    );
    let Some(legacy) = legacy else {
        return;
    };
    let summary =
        progress_payload_from_record(&finalization.run, &legacy.job_kind, skipped, started_at);
    match finalization.run.dto.status.as_str() {
        "cancelled" => {
            app.emit(SCAN_CANCELED_EVENT, summary).ok();
        }
        // `requires_reconciliation` means the scan ran to the end and persisted what it
        // observed; the index may be incomplete, which is a health signal rather than a
        // scan failure.  The legacy protocol has no health channel, so it reports
        // completion and surfaces the shortfall through the summary's error counters.
        "completed" | "completed_with_warnings" | "requires_reconciliation" => {
            app.emit(SCAN_COMPLETE_EVENT, summary).ok();
        }
        _ => {
            if let Some(message) = finalization.run.dto.error_message.as_deref() {
                app.emit(
                    SCAN_ERROR_EVENT,
                    ScanErrorPayload {
                        job_id: finalization.run.dto.id.clone(),
                        job_kind: legacy.job_kind.clone(),
                        root: finalization.run.dto.root_path.clone(),
                        path: finalization.run.dto.root_path.clone(),
                        message: message.to_string(),
                    },
                )
                .ok();
            }
        }
    }
    let _ = db;
}

fn emit_legacy_started<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    legacy: &LegacyScanContext,
    root: &str,
) {
    app.emit(
        SCAN_STARTED_EVENT,
        ScanStartedPayload {
            job_id: run_id.to_string(),
            job_kind: legacy.job_kind.clone(),
            root: root.to_string(),
            batch_size: SCAN_BATCH_SIZE,
        },
    )
    .ok();
}

fn emit_managed_event_best_effort<R: Runtime>(
    app: &AppHandle<R>,
    db: &Database,
    record: &ScanRunRecord,
    current_path: Option<String>,
) {
    let Ok(session_id) = record
        .dto
        .parent_session_id
        .clone()
        .ok_or_else(|| "scan run has no parent session".to_string())
    else {
        return;
    };
    let Ok(session) = db.get_scan_session(&session_id) else {
        return;
    };
    emit_managed_event_with_session(app, record, &session, current_path);
}

fn emit_managed_event_with_session_best_effort<R: Runtime>(
    app: &AppHandle<R>,
    record: &ScanRunRecord,
    session: &ScanSessionDto,
) {
    emit_managed_event_with_session(app, record, session, None);
}

fn emit_managed_event_with_session<R: Runtime>(
    app: &AppHandle<R>,
    record: &ScanRunRecord,
    session: &ScanSessionDto,
    current_path: Option<String>,
) {
    let event = ManagedScanEvent {
        event_id: new_job_id("scan-event"),
        run_id: record.dto.id.clone(),
        scan_root_id: record.dto.scan_root_id.clone(),
        parent_session_id: record.dto.parent_session_id.clone(),
        generation: record.dto.generation,
        run_revision: record.dto.revision,
        session_revision: session.revision,
        status: record.dto.status.clone(),
        run_phase: record.dto.phase.clone(),
        session_phase: session.phase.clone(),
        scanned_files: record.dto.scanned_files,
        scanned_directories: record.dto.scanned_directories,
        processed_bytes: record.dto.processed_bytes,
        warnings_count: record.dto.warnings_count,
        errors_count: record.dto.errors_count,
        current_path,
        error_code: record.dto.error_code.clone(),
        error_message: record.dto.error_message.clone(),
        timestamp: current_unix_seconds(),
    };
    if let Err(error) = app.emit(MANAGED_SCAN_EVENT, event) {
        eprintln!("Managed scan event emit failed: {error}");
    }
}

struct ScanBatchBuffer {
    entries: Vec<ScannedEntry>,
    errors: Vec<ScanErrorInput>,
    scanned_files: i64,
    scanned_directories: i64,
    processed_bytes: i64,
    warnings: i64,
    batch_index: u64,
    last_emit_at: Instant,
}

impl ScanBatchBuffer {
    fn new(started_at: Instant) -> Self {
        Self {
            entries: Vec::with_capacity(SCAN_BATCH_SIZE),
            errors: Vec::new(),
            scanned_files: 0,
            scanned_directories: 0,
            processed_bytes: 0,
            warnings: 0,
            batch_index: 0,
            last_emit_at: started_at,
        }
    }

    fn push_entry(&mut self, entry: ScannedEntry) {
        if entry.is_dir {
            self.scanned_directories += 1;
        } else {
            self.scanned_files += 1;
            self.processed_bytes = self
                .processed_bytes
                .saturating_add(i64::try_from(entry.size).unwrap_or(i64::MAX));
        }
        self.entries.push(entry);
    }

    fn push_error(&mut self, error: ScanErrorInput) {
        self.errors.push(error);
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.errors.is_empty()
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.errors.clear();
        self.scanned_files = 0;
        self.scanned_directories = 0;
        self.processed_bytes = 0;
        self.warnings = 0;
    }

    fn current_path(&self) -> Option<String> {
        self.errors
            .last()
            .and_then(|error| error.path.clone())
            .or_else(|| self.entries.last().map(|entry| entry.path.clone()))
    }

    fn should_flush(&self, now: Instant) -> bool {
        !self.is_empty()
            && (self.entries.len() >= SCAN_BATCH_SIZE
                || now.duration_since(self.last_emit_at) >= SCAN_EMIT_INTERVAL)
    }
}

fn progress_payload_from_record(
    record: &ScanRunRecord,
    job_kind: &str,
    skipped: u64,
    started_at: Instant,
) -> ScanProgressPayload {
    ScanProgressPayload {
        job_id: record.dto.id.clone(),
        job_kind: job_kind.to_string(),
        root: record.dto.root_path.clone(),
        scanned: (record.dto.scanned_files + record.dto.scanned_directories).max(0) as u64,
        files: record.dto.scanned_files.max(0) as u64,
        directories: record.dto.scanned_directories.max(0) as u64,
        skipped,
        errors: record.dto.errors_count.max(0) as u64,
        elapsed_ms: started_at.elapsed().as_millis(),
    }
}

fn legacy_summary_or_error(
    run: &ScanRunDto,
    job_kind: &str,
    skipped: u64,
    started_at: Instant,
) -> Result<ScanSummary, String> {
    let summary = ScanProgressPayload {
        job_id: run.id.clone(),
        job_kind: job_kind.to_string(),
        root: run.root_path.clone(),
        scanned: (run.scanned_files + run.scanned_directories).max(0) as u64,
        files: run.scanned_files.max(0) as u64,
        directories: run.scanned_directories.max(0) as u64,
        skipped,
        errors: run.errors_count.max(0) as u64,
        elapsed_ms: started_at.elapsed().as_millis(),
    };
    if matches!(
        run.status.as_str(),
        "completed" | "completed_with_warnings" | "cancelled" | "requires_reconciliation"
    ) {
        Ok(summary)
    } else {
        Err(run
            .error_message
            .clone()
            .unwrap_or_else(|| format!("Scan ended in {}.", run.status)))
    }
}

fn scanned_entry_to_insert_request(entry: &ScannedEntry) -> InsertFileRequest {
    InsertFileRequest {
        id: entry.path.clone(),
        path: entry.path.clone(),
        name: entry.name.clone(),
        extension: entry.extension.clone(),
        size: i64::try_from(entry.size).unwrap_or(i64::MAX),
        mtime: entry.mtime,
        ctime: entry.ctime,
        is_dir: entry.is_dir,
        state_code: i64::from(entry.state_code),
    }
}

fn entry_to_payload<C: ClientState>(
    entry: &DirEntry<C>,
) -> Result<Option<ScannedEntry>, ScanError> {
    let path = entry.path();
    if entry.file_type().is_symlink() {
        return Ok(None);
    }

    let is_dir = entry.file_type().is_dir();
    let metadata = entry.metadata().map_err(|source| ScanError::Metadata {
        path: normalize_path(&path),
        source,
    })?;
    if metadata_is_link_or_reparse(&metadata) {
        return Ok(None);
    }
    let mtime = modified_unix_seconds(&metadata);

    Ok(Some(ScannedEntry {
        path: normalize_path(&path),
        name: entry.file_name().to_string_lossy().into_owned(),
        extension: path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase(),
        size: if is_dir { 0 } else { metadata.len() },
        mtime,
        ctime: metadata
            .created()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(mtime),
        is_dir,
        state_code: 0,
    }))
}

fn scan_error_input_for_root(error: &ScanError, root: &str) -> ScanErrorInput {
    ScanErrorInput {
        path: Some(root.to_string()),
        error_code: match error {
            ScanError::MissingRoot(_) => "root_missing",
            ScanError::InvalidRoot(_) => "root_invalid",
            _ => "root_unavailable",
        }
        .to_string(),
        error_message: error.to_string(),
        affects_coverage: true,
        metadata_error: false,
    }
}

fn scan_error_input_for_error(error: &ScanError) -> ScanErrorInput {
    match error {
        ScanError::Metadata { path, source } => ScanErrorInput {
            path: Some(path.clone()),
            error_code: "metadata_error".to_string(),
            error_message: source.to_string(),
            affects_coverage: true,
            metadata_error: true,
        },
        other => ScanErrorInput {
            path: None,
            error_code: "scan_error".to_string(),
            error_message: other.to_string(),
            affects_coverage: true,
            metadata_error: false,
        },
    }
}

fn validate_root(root: &Path) -> Result<(), ScanError> {
    if ManagedPathFilter::new(root).excludes(root, root.is_dir()) {
        return Err(ScanError::InvalidRoot(format!(
            "{} (excluded managed path)",
            normalize_path(root)
        )));
    }
    if !root.exists() {
        return Err(ScanError::MissingRoot(normalize_path(root)));
    }
    if crate::fs_safety::identity::ensure_supported_entry(root).is_err()
        || (!root.is_dir() && !root.is_file())
    {
        return Err(ScanError::InvalidRoot(normalize_path(root)));
    }
    Ok(())
}

fn modified_unix_seconds(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_scan_cancelled(cancel_flag: &AtomicBool) -> bool {
    cancel_flag.load(Ordering::Acquire)
}

/// Stale reconciliation is the default behaviour: master executed it unconditionally
/// after every non-cancelled scan, and the managed implementation is strictly safer
/// (coverage gate, per-run `scan_seen` observations, ignored-subtree contract, CAS).
///
/// The environment variable is an emergency kill switch, not a rollout flag: it only
/// exists so the new implementation can be switched off in production without shipping
/// a build. Disabling it does NOT mean the index is known to be stale, so it must not
/// fabricate a reconciliation signal — it only stops us from looking.
fn stale_reconciliation_enabled() -> bool {
    std::env::var("ZEN_CANVAS_SCAN_STALE_RECONCILIATION")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off"
            )
        })
        .unwrap_or(true)
}

fn is_terminal_scan_status(status: &str) -> bool {
    matches!(
        status,
        "cancelled"
            | "completed"
            | "completed_with_warnings"
            | "failed"
            | "interrupted"
            | "requires_reconciliation"
    )
}

fn should_run_stale_cleanup(cancelled: bool) -> bool {
    !cancelled && stale_reconciliation_enabled()
}

fn scan_batch_payload(
    job_id: &str,
    job_kind: &str,
    root: &str,
    batch_index: u64,
    entries: Vec<ScannedEntry>,
    progress: ScanProgressPayload,
    include_entries: bool,
) -> ScanBatchPayload {
    ScanBatchPayload {
        job_id: job_id.to_string(),
        job_kind: job_kind.to_string(),
        root: root.to_string(),
        batch_index,
        entries: if include_entries { entries } else { Vec::new() },
        progress,
    }
}

#[cfg(test)]
fn emit_scan_complete_then_schedule_dedupe(
    emit_complete: impl FnOnce() -> Result<(), ScanError>,
    schedule_dedupe: impl FnOnce(),
) -> Result<(), ScanError> {
    emit_complete()?;
    schedule_dedupe();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "performance-test-tauri")]
    use crate::db::{FileLibraryScopeV2, FileQueryFiltersV2, FileQueryRequestV2, FileQuerySpecV2};
    #[cfg(feature = "performance-test-tauri")]
    use crate::dedupe::DedupeJobManager;
    use std::{
        cell::RefCell,
        ffi::OsStr,
        fs,
        sync::{
            atomic::{AtomicBool, AtomicU64, Ordering},
            Arc,
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_db(label: &str) -> Database {
        let sequence = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        Database::open(std::env::temp_dir().join(format!(
            "zen-canvas-scanner-{label}-{}-{timestamp}-{sequence}.sqlite3",
            std::process::id()
        )))
        .expect("open scanner test database")
    }

    #[cfg(feature = "performance-test-tauri")]
    #[test]
    #[ignore = "Issue #345 deterministic real managed-scan writer contention evidence"]
    fn issue345_concurrent_real_managed_scan_writers_hold_and_release_authority() {
        use crate::{
            db::scan::{set_managed_scan_write_test_hook, ManagedScanWriteTestPoint},
            scheduler::{ResourceHints, WorkRequest, WorkScheduler},
        };

        let scheduler = WorkScheduler::global();
        let initial_snapshot = scheduler.snapshot();
        let background_policy = scheduler.policy_decision(WorkClass::Background);
        let concurrent_scan_capacity = background_policy
            .effective_capacity
            .cpu
            .min(background_policy.effective_capacity.io)
            .max(1) as usize;
        let available_scan_capacity =
            concurrent_scan_capacity.saturating_sub(initial_snapshot.running_background);
        assert!(background_policy.allow_background);
        assert!(
            available_scan_capacity >= 2,
            "D4 requires two available real background leases; effective capacity was {concurrent_scan_capacity}, initially running {}",
            initial_snapshot.running_background
        );
        let filler_count = available_scan_capacity.saturating_sub(2);

        let db = test_db("issue345-d4-concurrent-writers");
        let fixture_root =
            std::env::temp_dir().join(format!("zen-canvas-issue345-d4-{}", new_job_id("fixture")));
        let owner_root = fixture_root.join("owner");
        let contender_root = fixture_root.join("contender");
        let replacement_root = fixture_root.join("replacement");
        let filler_roots = (0..filler_count)
            .map(|index| fixture_root.join(format!("capacity-filler-{index}")))
            .collect::<Vec<_>>();
        for root in [&owner_root, &contender_root, &replacement_root]
            .into_iter()
            .chain(filler_roots.iter())
        {
            fs::create_dir_all(root).expect("create isolated D4 managed scan root");
            fs::write(root.join("one.txt"), b"issue345 fixture")
                .expect("create D4 managed scan file");
        }

        let owner_run_id = format!("issue345-d4-owner-{}", new_job_id("run"));
        let contender_run_id = format!("issue345-d4-contender-{}", new_job_id("run"));
        let replacement_run_id = format!("issue345-d4-replacement-{}", new_job_id("run"));
        let filler_run_ids = (0..filler_count)
            .map(|index| format!("issue345-d4-filler-{index}-{}", new_job_id("run")))
            .collect::<Vec<_>>();
        let mut filler_release_senders = Vec::with_capacity(filler_count);
        let mut filler_release_receivers = std::collections::HashMap::with_capacity(filler_count);
        for run_id in &filler_run_ids {
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            filler_release_senders.push(release_tx);
            filler_release_receivers.insert(run_id.clone(), release_rx);
        }
        let filler_release_receivers = Arc::new(std::sync::Mutex::new(filler_release_receivers));
        let (events_tx, events_rx) = std::sync::mpsc::channel::<(&'static str, String)>();
        let (owner_begin_tx, owner_begin_rx) = std::sync::mpsc::channel();
        let (contender_begin_tx, contender_begin_rx) = std::sync::mpsc::channel();
        let (owner_transaction_release_tx, owner_transaction_release_rx) =
            std::sync::mpsc::channel();
        let (owner_after_commit_release_tx, owner_after_commit_release_rx) =
            std::sync::mpsc::channel();
        let (contender_after_commit_release_tx, contender_after_commit_release_rx) =
            std::sync::mpsc::channel();
        let owner_begin_rx = Arc::new(std::sync::Mutex::new(owner_begin_rx));
        let contender_begin_rx = Arc::new(std::sync::Mutex::new(contender_begin_rx));
        let owner_transaction_release_rx =
            Arc::new(std::sync::Mutex::new(owner_transaction_release_rx));
        let owner_after_commit_release_rx =
            Arc::new(std::sync::Mutex::new(owner_after_commit_release_rx));
        let contender_after_commit_release_rx =
            Arc::new(std::sync::Mutex::new(contender_after_commit_release_rx));

        let owner_id_for_hook = owner_run_id.clone();
        let contender_id_for_hook = contender_run_id.clone();
        let contender_after_commit_rx_for_hook = Arc::clone(&contender_after_commit_release_rx);
        let filler_ids_for_hook = filler_run_ids.clone();
        let filler_receivers_for_hook = Arc::clone(&filler_release_receivers);
        let hook_events = events_tx.clone();
        set_managed_scan_write_test_hook(Some(Arc::new(move |run_id, point| {
            if run_id == owner_id_for_hook && point == ManagedScanWriteTestPoint::BeforeTransaction
            {
                let _ = hook_events.send(("owner_before_begin", run_id.to_string()));
                let _ = owner_begin_rx
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .recv_timeout(Duration::from_secs(20));
            } else if run_id == contender_id_for_hook
                && point == ManagedScanWriteTestPoint::BeforeTransaction
            {
                let _ = hook_events.send(("contender_before_begin", run_id.to_string()));
                let _ = contender_begin_rx
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .recv_timeout(Duration::from_secs(20));
            } else if run_id == owner_id_for_hook
                && point == ManagedScanWriteTestPoint::TransactionAcquired
            {
                let _ = hook_events.send(("owner_transaction_acquired", run_id.to_string()));
                let _ = owner_transaction_release_rx
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .recv_timeout(Duration::from_secs(20));
            } else if run_id == contender_id_for_hook
                && point == ManagedScanWriteTestPoint::TransactionAcquired
            {
                let _ = hook_events.send(("contender_transaction_acquired", run_id.to_string()));
            } else if point == ManagedScanWriteTestPoint::BeforeTransaction
                && filler_ids_for_hook
                    .iter()
                    .any(|filler_run_id| filler_run_id.as_str() == run_id)
            {
                let _ = hook_events.send(("filler_before_begin", run_id.to_string()));
                let release_rx = filler_receivers_for_hook
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(run_id);
                if let Some(release_rx) = release_rx {
                    let _ = release_rx.recv_timeout(Duration::from_secs(20));
                }
            } else if run_id == owner_id_for_hook && point == ManagedScanWriteTestPoint::AfterCommit
            {
                let _ = hook_events.send(("owner_batch_committed", run_id.to_string()));
                let _ = owner_after_commit_release_rx
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .recv_timeout(Duration::from_secs(20));
            } else if run_id == contender_id_for_hook
                && point == ManagedScanWriteTestPoint::AfterCommit
            {
                let _ = hook_events.send(("contender_batch_committed", run_id.to_string()));
                let _ = contender_after_commit_rx_for_hook
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .recv_timeout(Duration::from_secs(20));
            }
        })));
        struct ClearManagedScanHook;
        impl Drop for ClearManagedScanHook {
            fn drop(&mut self) {
                crate::db::scan::set_managed_scan_write_test_hook(None);
            }
        }
        let _clear_hook = ClearManagedScanHook;

        let jobs = ScanJobManager::default();
        let dedupe_jobs = DedupeJobManager::default();
        let app = tauri::test::mock_app();
        let app_handle = app.handle().clone();
        let owner = start_performance_managed_scan(
            app_handle.clone(),
            db.clone(),
            jobs.clone(),
            dedupe_jobs.clone(),
            owner_root,
            owner_run_id.clone(),
            "background",
        )
        .expect("start first real managed scan");
        let owner_before = events_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("owner reaches its deterministic pre-transaction barrier");
        assert_eq!(owner_before, ("owner_before_begin", owner_run_id.clone()));

        let contender = start_performance_managed_scan(
            app_handle.clone(),
            db.clone(),
            jobs.clone(),
            dedupe_jobs.clone(),
            contender_root,
            contender_run_id.clone(),
            "background",
        )
        .expect("start second real managed scan");
        let contender_before = events_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("contender reaches its deterministic pre-transaction barrier");
        assert_eq!(
            contender_before,
            ("contender_before_begin", contender_run_id.clone())
        );
        assert_eq!(
            db.get_scan_run_record(&owner.run_id)
                .expect("owner durable run")
                .dto
                .status,
            "running"
        );
        assert_eq!(
            db.get_scan_run_record(&contender.run_id)
                .expect("contender durable run")
                .dto
                .status,
            "running"
        );

        let mut fillers = Vec::with_capacity(filler_count);
        for (index, run_id) in filler_run_ids.iter().enumerate() {
            let filler = start_performance_managed_scan(
                app_handle.clone(),
                db.clone(),
                jobs.clone(),
                dedupe_jobs.clone(),
                filler_roots[index].clone(),
                run_id.clone(),
                "background",
            )
            .expect("start real scan to fill the remaining background capacity");
            let filler_before = events_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("capacity-filling scan reaches its deterministic pre-transaction barrier");
            assert_eq!(filler_before, ("filler_before_begin", run_id.clone()));
            fillers.push(filler);
        }

        let replacement = start_performance_managed_scan(
            app_handle,
            db.clone(),
            jobs.clone(),
            dedupe_jobs.clone(),
            replacement_root,
            replacement_run_id.clone(),
            "background",
        )
        .expect("admit replacement scan before holding SQLite writer");
        let queue_deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < queue_deadline
            && scheduler.snapshot().queued <= initial_snapshot.queued
        {
            std::thread::yield_now();
        }
        let active_snapshot = scheduler.snapshot();
        assert!(
            active_snapshot.running_background
                >= initial_snapshot.running_background + available_scan_capacity,
            "D4 holds all available background lease capacity before replacement admission"
        );
        assert!(
            active_snapshot.queued > initial_snapshot.queued,
            "replacement scan should wait behind real managed scans filling background capacity"
        );

        owner_begin_tx
            .send(())
            .expect("release owner to acquire SQLite writer");
        let owner_acquired = events_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("owner holds the SQLite writer inside persist_scan_batch");
        assert_eq!(
            owner_acquired,
            ("owner_transaction_acquired", owner_run_id.clone())
        );
        let contender_begin_at = Instant::now();
        contender_begin_tx
            .send(())
            .expect("release contender to request the same SQLite writer");
        match events_rx.recv_timeout(Duration::from_millis(250)) {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Ok(("contender_transaction_acquired", _)) => {
                panic!("contender acquired the writer while owner transaction remained open")
            }
            Ok(event) => panic!("unexpected D4 hook event while owner held writer: {event:?}"),
            Err(error) => panic!("D4 event channel closed while writer was held: {error}"),
        }

        let foreground_scheduler = Arc::clone(&scheduler);
        let (foreground_tx, foreground_rx) = std::sync::mpsc::sync_channel(1);
        let foreground_started = Instant::now();
        let foreground = std::thread::spawn(move || {
            let result = foreground_scheduler.acquire(
                WorkRequest::new(
                    "issue345-d4-foreground-probe",
                    WorkClass::Foreground,
                    ResourceHints {
                        cpu: 1,
                        io: 1,
                        open_handles: 1,
                        ..ResourceHints::empty()
                    },
                )
                .with_session_id("issue345-d4-foreground-session"),
            );
            let admitted = result.is_ok();
            drop(result);
            let _ = foreground_tx.send((admitted, foreground_started.elapsed().as_millis()));
        });
        let foreground_deadline = Instant::now() + Duration::from_secs(30);
        let early_foreground_result = foreground_rx.recv_timeout(Duration::from_millis(100));

        owner_transaction_release_tx
            .send(())
            .expect("release writer transaction after proving contender overlap");
        let owner_committed = events_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("D4 writer events arrive after releasing owner transaction");
        let mut owner_commit_seen = false;
        let mut contender_acquire_seen = false;
        let mut contender_commit_seen = false;
        let mut contender_writer_wait_ms = None;
        let mut writer_events = vec![owner_committed];
        while !(owner_commit_seen && contender_acquire_seen && contender_commit_seen) {
            if writer_events.is_empty() {
                writer_events.push(
                    events_rx
                        .recv_timeout(Duration::from_secs(10))
                        .expect("owner and contender reach both SQLite write barriers"),
                );
            }
            for event in writer_events.drain(..) {
                match event {
                    ("owner_batch_committed", run_id) if run_id == owner_run_id => {
                        owner_commit_seen = true;
                    }
                    ("contender_transaction_acquired", run_id) if run_id == contender_run_id => {
                        contender_acquire_seen = true;
                        contender_writer_wait_ms = Some(contender_begin_at.elapsed().as_millis());
                    }
                    ("contender_batch_committed", run_id) if run_id == contender_run_id => {
                        contender_commit_seen = true;
                    }
                    other => panic!("unexpected D4 writer event: {other:?}"),
                }
            }
        }
        assert!(
            owner_commit_seen,
            "owner must commit its batch before cancel"
        );
        assert!(
            contender_acquire_seen,
            "contender must acquire the writer after the owner commits"
        );
        assert!(
            contender_commit_seen,
            "contender must commit while its real scheduler lease remains held"
        );
        let contender_writer_wait_ms = contender_writer_wait_ms
            .expect("record contender wait until its SQLite writer transaction is acquired");

        cancel_performance_managed_scan(&db, &jobs, &dedupe_jobs, &owner.run_id)
            .expect("request cancellation through the production-managed scan boundary");
        owner_after_commit_release_tx
            .send(())
            .expect("let cancelled owner leave persist_scan_batch");
        let foreground_result = match early_foreground_result {
            Ok(result) => Ok(result),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => foreground_rx
                .recv_timeout(foreground_deadline.saturating_duration_since(Instant::now())),
            Err(error) => Err(error),
        };

        contender_after_commit_release_tx
            .send(())
            .expect("release contender after foreground admission has used the cancelled lease");

        for release in &filler_release_senders {
            release
                .send(())
                .expect("release a capacity-filling real scan");
        }

        owner
            .worker
            .join()
            .expect("owner managed-scan worker joins");
        contender
            .worker
            .join()
            .expect("contender managed-scan worker joins");
        for filler in fillers {
            filler
                .worker
                .join()
                .expect("capacity-filling managed-scan worker joins");
        }
        replacement
            .worker
            .join()
            .expect("replacement managed-scan worker joins after lease release");
        foreground.join().expect("foreground scheduler probe joins");

        let owner_status = db
            .get_scan_run_record(&owner.run_id)
            .expect("read cancelled owner result")
            .dto
            .status;
        let contender_status = db
            .get_scan_run_record(&contender.run_id)
            .expect("read contender result")
            .dto
            .status;
        let replacement_status = db
            .get_scan_run_record(&replacement.run_id)
            .expect("read replacement result")
            .dto
            .status;
        let filler_statuses = filler_run_ids
            .iter()
            .map(|run_id| {
                db.get_scan_run_record(run_id)
                    .expect("read capacity-filling managed-scan result")
                    .dto
                    .status
            })
            .collect::<Vec<_>>();
        let settled_snapshot = scheduler.snapshot();
        let scheduler_settled = settled_snapshot.running == initial_snapshot.running
            && settled_snapshot.queued == initial_snapshot.queued;
        let foreground_admitted = foreground_result
            .as_ref()
            .is_ok_and(|(admitted, _)| *admitted);
        let foreground_wait_ms = foreground_result
            .as_ref()
            .ok()
            .map(|(_, wait_ms)| *wait_ms)
            .unwrap_or(30_000);
        let run_statuses_settled = owner_status == "cancelled"
            && contender_status == "completed"
            && replacement_status == "completed"
            && filler_statuses.iter().all(|status| status == "completed");

        println!(
            "[issue345-d4] {}",
            serde_json::json!({
                "scan_writer_overlap_proved": true,
                "owner_transaction_acquired_before_contender": true,
                "contender_writer_wait_ms": contender_writer_wait_ms,
                "foreground_wait_ms": foreground_wait_ms,
                "foreground_admitted": foreground_admitted,
                "pressure_slots": active_snapshot.running_background.saturating_sub(initial_snapshot.running_background),
                "effective_background_capacity": concurrent_scan_capacity,
                "capacity_filler_count": filler_count,
                "replacement_queued_before_release": true,
                "background_progress_after_cancel": replacement_status == "completed",
                "run_statuses": {
                    "owner_cancelled": owner_status,
                    "contender": contender_status,
                    "replacement": replacement_status,
                    "capacity_fillers": filler_statuses,
                },
                "scan_runs_settled_without_failure": run_statuses_settled,
                "scheduler_running_before": initial_snapshot.running,
                "scheduler_queued_before": initial_snapshot.queued,
                "scheduler_running_after": settled_snapshot.running,
                "scheduler_queued_after": settled_snapshot.queued,
                "scheduler_settled": scheduler_settled,
                "foreground_thread_joined": true,
            })
        );
        assert!(
            foreground_result.is_ok(),
            "foreground admission respects the 30-second boundary"
        );
        assert!(foreground_admitted);
        assert!(
            run_statuses_settled,
            "D4 runs must settle without a failed run"
        );
        assert!(
            scheduler_settled,
            "D4 scheduler grants and queue must settle"
        );
        set_managed_scan_write_test_hook(None);
        fs::remove_dir_all(fixture_root).expect("remove isolated D4 scan fixture");
    }

    #[test]
    fn backend_issues_authoritative_uuid_scan_job_ids() {
        let foreground = create_scan_job_id("foreground".to_string()).expect("foreground id");
        let background = create_scan_job_id("background".to_string()).expect("background id");

        assert!(foreground.starts_with("scan-foreground-"));
        assert!(background.starts_with("scan-background-"));
        assert_ne!(foreground, background);
        assert!(create_scan_job_id("forged".to_string()).is_err());
    }

    #[test]
    fn should_skip_dir_matches_case_insensitive_generated_variants() {
        assert!(is_ignored_dir_name(OsStr::new("Node_Modules")));
        assert!(is_ignored_dir_name(OsStr::new("node_modules.cache")));
        assert!(is_ignored_dir_name(OsStr::new(".git-worktree")));
        assert!(is_ignored_dir_name(OsStr::new(".zen-canvas-trash")));
        assert!(is_ignored_dir_name(OsStr::new("System Volume Information")));
        assert!(!is_ignored_dir_name(OsStr::new("Library")));
        assert!(!is_ignored_dir_name(OsStr::new("client-documents")));
    }

    /// Issue #375's historical characterization is retained in PR #378.
    /// #381 intentionally changes false-exclusion assertions to desired coverage
    /// through the real worker, while preserving ignored historical rows.
    #[cfg(feature = "performance-test-tauri")]
    #[test]
    fn issue375_managed_scan_covers_user_names_and_preserves_ignored_history() {
        let fixture = Issue375Fixture::new();
        let main_root = fixture.0.join("library");
        let selected_build_root = fixture.0.join("chosen").join("build");
        fs::create_dir_all(&main_root).expect("create managed root");

        let excluded_paths = [
            "Projects/Windows/README.md",
            "Design/build/logo.ai",
            "Configs/AppData/important.json",
            "Project/.vscode/settings.json",
            "Project/node_modules/pkg/index.js",
            "Project/.venv/pyvenv.cfg",
            "Project/build/generated.js",
            "Projects/wInDoWs/设计.md",
        ];
        for relative in excluded_paths {
            fixture.write(&main_root, relative, b"previously indexed fixture");
        }
        let newly_created_build_file = fixture.write(
            &main_root,
            "Design/build/new-logo.ai",
            b"not previously indexed",
        );
        fixture.write(&main_root, "Project/package.json", b"{}");
        fixture.write(&fixture.0, "chosen/package.json", b"{}");
        let current_file = fixture.write(&main_root, "Project/src/current.rs", b"current");
        let selected_root_file = fixture.write(&selected_build_root, "README.md", b"selected root");

        let db = Database::open(fixture.0.join("managed-scan.sqlite3"))
            .expect("open isolated managed-scan database");
        let deleted_ignored = main_root.join("Project/node_modules/removed/index.js");
        let mut seeded_paths = excluded_paths
            .iter()
            .map(|relative| main_root.join(relative))
            .collect::<Vec<_>>();
        seeded_paths.push(deleted_ignored.clone());
        for path in &seeded_paths {
            seed_issue375_file_row(&db, path);
        }

        {
            let conn = db.conn().expect("seed retained user facts");
            conn.execute_batch("INSERT INTO user_tags (id, display_name, normalized_name, created_at, updated_at) VALUES ('retained-tag','Keep','keep',1,1)").unwrap();
            conn.execute("INSERT INTO file_user_tags (file_id, tag_id, created_at) VALUES (?1,'retained-tag',1)", [&normalize_path(&deleted_ignored)]).unwrap();
            conn.execute(
                "UPDATE files SET suggested_name = 'user correction' WHERE id = ?1",
                [&normalize_path(&deleted_ignored)],
            )
            .unwrap();
            conn.execute("INSERT INTO content_artifacts (id,file_id,source_size,source_mtime,source_is_dir,extractor_family,extractor_version,policy_revision,content_fingerprint,status,summary,raw_text,created_at,updated_at) VALUES ('retained-content',?1,1,1,0,'text','1',1,'retained','current','user note','retained content fact',1,1)", [&normalize_path(&deleted_ignored)]).unwrap();
        }
        let jobs = ScanJobManager::default();
        let dedupe_jobs = DedupeJobManager::default();
        let app = tauri::test::mock_app();
        let first = start_performance_managed_scan_roots(
            app.handle().clone(),
            db.clone(),
            jobs.clone(),
            dedupe_jobs.clone(),
            vec![main_root.clone(), selected_build_root.clone()],
            new_job_id("issue375-first"),
            "background",
        )
        .expect("start real managed scan over both selected roots");
        first
            .worker
            .join()
            .expect("first managed-scan worker joins");

        let first_records = first
            .run_ids
            .iter()
            .map(|run_id| db.get_scan_run_record(run_id).expect("read first scan run"))
            .collect::<Vec<_>>();
        for record in &first_records {
            assert_eq!(record.dto.status, "completed", "run {}", record.dto.id);
            assert_eq!(record.dto.errors_count, 0, "run {}", record.dto.id);
            assert_eq!(record.dto.coverage_error_count, 0, "run {}", record.dto.id);
        }

        let main_run = first_records
            .iter()
            .find(|record| record.dto.root_path == normalize_path(&main_root))
            .expect("main fixture root run");
        let selected_build_run = first_records
            .iter()
            .find(|record| record.dto.root_path == normalize_path(&selected_build_root))
            .expect("explicit build root run");
        assert!(
            scan_seen_path(&db, &main_run.dto.id, &current_file),
            "ordinary files are discovered by the managed scan"
        );
        assert!(
            scan_seen_path(&db, &selected_build_run.dto.id, &selected_root_file),
            "a user-selected build root is scanned"
        );
        for relative in excluded_paths {
            let path = main_root.join(relative);
            assert!(
                scan_seen_path(&db, &main_run.dto.id, &path) != relative.starts_with("Project/"),
                "only dependency/IDE subtrees remain unseen: {relative}"
            );
        }
        assert!(
            scan_seen_path(&db, &main_run.dto.id, &newly_created_build_file),
            "a new user build descendant is discovered"
        );

        let main_health = db
            .get_scan_root_health(Some(&main_run.dto.scan_root_id), None)
            .expect("read main scan-root health");
        let selected_build_health = db
            .get_scan_root_health(Some(&selected_build_run.dto.scan_root_id), None)
            .expect("read selected build-root health");
        assert_eq!(main_health.health_status, "healthy");
        assert!(!main_health.needs_reconciliation);
        assert_eq!(selected_build_health.health_status, "healthy");

        let first_query = db
            .query_file_library_v2(issue375_library_query("issue375-first-query"))
            .expect("query File Library after first scan");
        let first_visible = first_query
            .files
            .iter()
            .map(|file| {
                db.resolve_file_library_path(&file.id)
                    .expect("resolve visible file path")
            })
            .collect::<HashSet<_>>();
        // Legacy rows without SourceUID proof may be parked by the existing
        // identity owner; assert visible paths, not transfer of historical IDs.
        for relative in excluded_paths {
            let path = main_root.join(relative);
            let id = normalize_path(&path);
            assert!(
                first_visible.contains(id.as_str()),
                "previously indexed ignored row remains visible in File Library: {relative}"
            );
        }
        assert!(
            first_visible.contains(normalize_path(&newly_created_build_file).as_str()),
            "File Library returns newly scanned user build data"
        );
        assert!(
            first_visible.contains(normalize_path(&selected_root_file).as_str()),
            "File Library returns the selected build-root file"
        );

        let deleted_ignored_id = normalize_path(&deleted_ignored);
        let conn = db.conn().expect("open first reconciliation connection");
        let ignored_stale: i64 = conn
            .query_row(
                "SELECT is_stale FROM files WHERE id = ?1",
                [&deleted_ignored_id],
                |row| row.get(0),
            )
            .expect("read ignored file stale state");
        assert_eq!(
            ignored_stale, 0,
            "ignored-subtree stale protection keeps the pre-existing row non-stale"
        );
        drop(conn);

        // `last_seen_at` is stored in seconds. Make the ordinary row older than
        // the next scan's cutoff so this assertion does not depend on whether
        // both scans happen to start during the same second.
        let conn = db.conn().expect("open timestamp setup connection");
        let aged_rows = conn
            .execute(
                "UPDATE files SET last_seen_at = 1 WHERE id = ?1",
                [&normalize_path(&current_file)],
            )
            .expect("age the observed ordinary file before the next scan");
        assert_eq!(aged_rows, 1, "first scan inserted the ordinary file row");
        drop(conn);

        fs::remove_file(&current_file).expect("remove ordinary file before incremental scan");
        let new_appdata_file = fixture.write(
            &main_root,
            "Configs/AppData/new-important.json",
            b"created between scans",
        );
        let second = start_performance_managed_scan(
            app.handle().clone(),
            db.clone(),
            jobs,
            dedupe_jobs,
            main_root.clone(),
            new_job_id("issue375-second"),
            "background",
        )
        .expect("start incremental managed scan");
        second.worker.join().expect("incremental scan worker joins");
        let second_run = db
            .get_scan_run_record(&second.run_id)
            .expect("read incremental scan run");
        assert_eq!(second_run.dto.status, "completed");
        assert!(
            second_run.dto.generation > main_run.dto.generation,
            "the next scan advances the generation despite the filtered subtree"
        );
        assert!(!scan_seen_path(&db, &second.run_id, &current_file));
        assert!(
            scan_seen_path(&db, &second.run_id, &new_appdata_file),
            "a new user AppData descendant is discovered during the next scan"
        );
        assert_eq!(
            db.get_scan_root_health(Some(&second_run.dto.scan_root_id), None)
                .expect("read incremental root health")
                .health_status,
            "healthy"
        );

        let conn = db
            .conn()
            .expect("open incremental reconciliation connection");
        let current_stale: i64 = conn
            .query_row(
                "SELECT is_stale FROM files WHERE id = ?1",
                [&normalize_path(&current_file)],
                |row| row.get(0),
            )
            .expect("read deleted ordinary file stale state");
        let ignored_stale: i64 = conn
            .query_row(
                "SELECT is_stale FROM files WHERE id = ?1",
                [&deleted_ignored_id],
                |row| row.get(0),
            )
            .expect("read deleted ignored file stale state");
        let retained: (String, String, String, i64) = conn.query_row(
            "SELECT f.suggested_name, a.summary, a.raw_text, (SELECT COUNT(*) FROM file_user_tags WHERE file_id=f.id) FROM files f JOIN content_artifacts a ON a.file_id=f.id WHERE f.id=?1",
            [&deleted_ignored_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(
            retained,
            (
                "user correction".into(),
                "user note".into(),
                "retained content fact".into(),
                1
            )
        );
        assert_eq!(current_stale, 1, "deleted ordinary file becomes stale");
        assert_eq!(
            ignored_stale, 0,
            "deleted ignored file remains non-stale by policy"
        );
        drop(conn);

        let second_query = db
            .query_file_library_v2(issue375_library_query("issue375-second-query"))
            .expect("query File Library after incremental scan");
        let second_visible = second_query
            .files
            .iter()
            .map(|file| {
                db.resolve_file_library_path(&file.id)
                    .expect("resolve visible file path")
            })
            .collect::<HashSet<_>>();
        assert!(
            !second_visible.contains(normalize_path(&current_file).as_str()),
            "the incrementally deleted file is no longer visible in File Library"
        );
        assert!(second_visible.contains(deleted_ignored_id.as_str()));
        assert!(second_visible.contains(normalize_path(&new_appdata_file).as_str()));
    }

    #[cfg(feature = "performance-test-tauri")]
    struct Issue375Fixture(PathBuf);

    #[cfg(feature = "performance-test-tauri")]
    impl Issue375Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("zen-canvas-issue375-{}", new_job_id("fixture")));
            fs::create_dir_all(&root).expect("create isolated Issue #375 fixture");
            Self(root)
        }

        fn write(&self, root: &Path, relative: &str, contents: &[u8]) -> PathBuf {
            assert!(
                root.starts_with(&self.0),
                "Issue #375 fixture writes must remain inside the disposable fixture root"
            );
            let path = root.join(relative);
            fs::create_dir_all(path.parent().expect("fixture parent"))
                .expect("create fixture directory");
            fs::write(&path, contents).expect("write fixture file");
            path
        }
    }

    #[cfg(feature = "performance-test-tauri")]
    impl Drop for Issue375Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(feature = "performance-test-tauri")]
    fn seed_issue375_file_row(db: &Database, path: &Path) {
        let path_text = normalize_path(path);
        let name = path
            .file_name()
            .expect("seed file name")
            .to_string_lossy()
            .into_owned();
        db.insert_file(InsertFileRequest {
            id: path_text.clone(),
            path: path_text.clone(),
            name: name.clone(),
            extension: path
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_ascii_lowercase(),
            size: 1,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("seed previously indexed File Library row");
        db.conn()
            .expect("open seed timestamp connection")
            .execute(
                "UPDATE files SET last_seen_at = 1 WHERE id = ?1",
                [&path_text],
            )
            .expect("age seeded File Library row before scan");
    }

    #[cfg(feature = "performance-test-tauri")]
    fn scan_seen_path(db: &Database, run_id: &str, path: &Path) -> bool {
        let path = normalize_path(path);
        db.conn()
            .expect("open scan_seen connection")
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM scan_seen WHERE run_id = ?1 AND (observed_path = ?2 OR file_id = ?2))",
                rusqlite::params![run_id, path],
                |row| row.get(0),
            )
            .expect("query scan_seen for path")
    }

    #[cfg(feature = "performance-test-tauri")]
    fn issue375_library_query(request_id: &str) -> FileQueryRequestV2 {
        FileQueryRequestV2 {
            version: 2,
            request_id: request_id.to_string(),
            query: FileQuerySpecV2 {
                scope: FileLibraryScopeV2::AllEnabledRoots,
                text: None,
                filters: FileQueryFiltersV2::default(),
                sort: Default::default(),
            },
            page_size: 200,
            cursor: None,
        }
    }

    #[test]
    fn scan_cancellation_flag_reports_requested_cancel() {
        let cancel_flag = AtomicBool::new(true);

        assert!(is_scan_cancelled(&cancel_flag));
    }

    #[test]
    fn scan_walk_uses_serial_traversal_for_one_cpu_lease() {
        use crate::scheduler::{
            adapters::ManagedScanResourceLeaseAdapter, PermissiveResourcePolicy,
            ResourceCapacities, SchedulerConfig, WorkScheduler,
        };

        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(4, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        let adapter = ManagedScanResourceLeaseAdapter::new(Arc::clone(&scheduler));
        let lease = adapter
            .try_acquire(
                "scan-fixture",
                WorkClass::Background,
                CancellationToken::new(),
            )
            .expect("scan resource lease");

        assert_eq!(lease.resources().cpu, 1);
        assert!(matches!(scan_walk_parallelism(&lease), Parallelism::Serial));
        assert_eq!(scheduler.snapshot().granted.cpu, 1);
        drop(lease);
    }

    #[test]
    fn scan_walk_uses_bounded_rayon_pool_for_multi_cpu_lease() {
        use crate::file_workspace::WorkClass;
        use crate::scheduler::{
            CancellationToken, PermissiveResourcePolicy, ResourceCapacities, ResourceHints,
            SchedulerConfig, WorkRequest, WorkScheduler,
        };

        let scheduler = WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(4, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        );
        let lease = scheduler
            .try_acquire(
                WorkRequest::new(
                    "scan-fixture-multi-cpu",
                    WorkClass::Background,
                    ResourceHints {
                        cpu: 3,
                        io: 1,
                        open_handles: 1,
                        ..ResourceHints::empty()
                    },
                )
                .with_cancellation(CancellationToken::new()),
            )
            .expect("multi-cpu scheduler lease");

        assert_eq!(lease.resources().cpu, 3);
        assert_eq!(scheduler.snapshot().granted.cpu, lease.resources().cpu);
        assert!(matches!(
            scan_walk_parallelism(&lease),
            Parallelism::RayonNewPool(3)
        ));
        drop(lease);
    }

    /// master executed stale cleanup unconditionally after every non-cancelled scan.
    /// The managed implementation must keep that default, otherwise shipping builds
    /// silently stop retiring deleted files from the index.
    #[test]
    fn stale_cleanup_runs_by_default_and_never_after_a_cancelled_scan() {
        assert!(stale_reconciliation_enabled());
        assert!(should_run_stale_cleanup(false));
        assert!(!should_run_stale_cleanup(true));
    }

    /// Acceptance test for the kill-switch path (BRIEF 裁决 1 + 裁决 3 修正).
    ///
    /// Disabling the switch means "we did not look", not "we found drift".  It must
    /// therefore behave exactly like a normal successful scan on every axis the rest of
    /// the system reads — terminal status, root health, and the generation pointer —
    /// and must not fabricate a reconciliation signal.  The only observable difference
    /// is that unseen rows keep their previous stale flag, because no check ran.
    ///
    /// This test pins behaviour, not status names: if a future change reintroduces a
    /// degraded health value or a stalled generation pointer here, it fails.
    #[test]
    fn kill_switch_path_completes_normally_without_fabricating_a_reconciliation_signal() {
        let db = test_db("default-stale-gate");
        let root_path = std::env::temp_dir().join(format!(
            "zen-canvas-default-stale-gate-{}",
            new_job_id("root")
        ));
        fs::create_dir_all(&root_path).expect("create scan root");
        let root = root_path.to_string_lossy().replace('\\', "/");
        let old_path = root_path.join("old.txt");
        fs::write(&old_path, "old").expect("create file before scan");
        let old_path_text = old_path.to_string_lossy().replace('\\', "/");
        let first_admission = db
            .admit_managed_scan(&ScanAdmissionOptions {
                request: ManagedScanRequest {
                    roots: vec![root.clone()],
                    request_key: Some("default-stale-gate-first".to_string()),
                    dedupe: false,
                },
                run_id_override: None,
            })
            .expect("admit first scan");
        let first = db
            .claim_queued_scan_run(&first_admission.runs[0].id)
            .expect("claim first scan");
        let first_batch = db
            .persist_scan_batch(
                &first.dto.id,
                first.dto.revision,
                first.root_revision,
                first.session_revision,
                &ScanBatchInput {
                    entries: &[InsertFileRequest {
                        id: old_path_text.clone(),
                        path: old_path_text.clone(),
                        name: "old.txt".to_string(),
                        extension: "txt".to_string(),
                        size: 3,
                        mtime: 1,
                        ctime: 1,
                        is_dir: false,
                        state_code: 0,
                    }],
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: 3,
                    warnings: 0,
                },
            )
            .expect("persist first scan");
        let first_finalizing = db
            .transition_scan_run_phase(
                &first_batch.dto.id,
                first_batch.dto.revision,
                first_batch.root_revision,
                "finalizing",
            )
            .expect("enter first finalization");
        // Mirrors what the scanner does when the kill switch is engaged: skip
        // `reconcile_missing`, then finalize on the normal success path.
        let first_finalization = finish_scan_run(
            &db,
            &first_finalizing.dto.id,
            "completed",
            None,
            None,
            false,
            false,
        )
        .expect("finalize first scan");
        assert_eq!(first_finalization.run.dto.status, "completed");

        fs::remove_file(&old_path).expect("delete file between scans");
        let second_admission = db
            .admit_managed_scan(&ScanAdmissionOptions {
                request: ManagedScanRequest {
                    roots: vec![root],
                    request_key: Some("default-stale-gate-second".to_string()),
                    dedupe: false,
                },
                run_id_override: None,
            })
            .expect("admit rescan");
        let second = db
            .claim_queued_scan_run(&second_admission.runs[0].id)
            .expect("claim rescan");
        let second_finalizing = db
            .transition_scan_run_phase(
                &second.dto.id,
                second.dto.revision,
                second.root_revision,
                "finalizing",
            )
            .expect("enter rescan finalization");
        let second_finalization = finish_scan_run(
            &db,
            &second_finalizing.dto.id,
            "completed",
            None,
            None,
            false,
            false,
        )
        .expect("finalize rescan");
        let health = db
            .get_scan_root_health(Some(&second_finalization.run.dto.scan_root_id), None)
            .expect("root health after rescan");
        let stale: i64 = db
            .conn()
            .expect("db connection")
            .query_row(
                "SELECT is_stale FROM files WHERE id = ?1",
                rusqlite::params![old_path_text],
                |row| row.get(0),
            )
            .expect("stale state after rescan");

        assert_eq!(second_finalization.run.dto.generation, 2);
        // Axis 1 — job outcome: the scan ran to completion.
        assert_eq!(second_finalization.run.dto.status, "completed");
        // Axis 2 — index health: not degraded, because nothing was found to be wrong.
        assert!(!health.needs_reconciliation);
        assert_ne!(health.health_status, "reconciliation_required");
        // Axis 3 — generation pointer: advances normally.
        assert_eq!(health.last_successful_generation, Some(2));
        // The only consequence of not looking: the deleted file keeps its prior flag.
        assert_eq!(stale, 0);
        fs::remove_dir_all(root_path).expect("remove scan root fixture");
    }

    #[test]
    fn startup_replays_pending_unknown_and_failed_dedupe_dispatches_after_restart() {
        let db = test_db("dedupe-restart");
        let states = ["pending", "unknown", "failed"];
        let mut session_ids = Vec::new();
        for (index, state) in states.iter().enumerate() {
            let root = format!(
                "/tmp/zen-canvas-dedupe-restart-{}-{}",
                index,
                new_job_id("root")
            );
            let admission = db
                .admit_managed_scan(&ScanAdmissionOptions {
                    request: ManagedScanRequest {
                        roots: vec![root],
                        request_key: Some(format!("dedupe-restart-{index}")),
                        dedupe: true,
                    },
                    run_id_override: None,
                })
                .expect("admit dedupe session");
            let run = db
                .claim_queued_scan_run(&admission.runs[0].id)
                .expect("claim dedupe session");
            let finalization = db
                .finalize_scan_run(
                    &run.dto.id,
                    run.dto.revision,
                    run.root_revision,
                    run.session_revision,
                    &ScanFinalizeInput {
                        terminal_status: "completed".to_string(),
                        error_code: None,
                        error_message: None,
                        allow_stale_reconciliation: false,
                        rule_recovery_succeeded: false,
                    },
                )
                .expect("complete dedupe session");
            assert_eq!(finalization.session.dedupe_dispatch_state, "pending");
            if *state != "pending" {
                db.conn()
                    .expect("db connection")
                    .execute(
                        "UPDATE scan_sessions SET dedupe_dispatch_state = ?1, dedupe_last_error = ?2 WHERE id = ?3",
                        rusqlite::params![state, "restart fixture", admission.session.id],
                    )
                    .expect("seed restart dispatch state");
            }
            session_ids.push(admission.session.id);
        }

        let attempts = RefCell::new(Vec::<String>::new());
        let first_pass = resume_dedupe_dispatches(&db, 1000, |session| {
            let attempt = attempts.borrow().len();
            attempts.borrow_mut().push(session.id.clone());
            if attempt == 0 {
                Err("simulated manager restart gap".to_string())
            } else {
                Ok(format!("dedupe-job-{}", session.id))
            }
        })
        .expect("replay durable dispatch candidates");
        assert_eq!(first_pass, 2);
        assert_eq!(attempts.borrow().len(), 3);

        let second_pass = resume_dedupe_dispatches(&db, 1000, |session| {
            Ok(format!("dedupe-retry-{}", session.id))
        })
        .expect("replay failed dispatch after retry");
        assert_eq!(second_pass, 1);
        for session_id in session_ids {
            assert_eq!(
                db.get_scan_session(&session_id)
                    .expect("replayed session")
                    .dedupe_dispatch_state,
                "dispatched"
            );
        }
    }

    #[test]
    fn scan_batch_buffer_flushes_after_emit_interval() {
        let started_at = Instant::now();
        let mut buffer = ScanBatchBuffer::new(started_at);

        assert!(!buffer.should_flush(started_at + Duration::from_millis(250)));

        buffer.push_entry(test_scanned_entry(1));

        assert!(!buffer.should_flush(started_at + Duration::from_millis(199)));
        assert!(buffer.should_flush(started_at + SCAN_EMIT_INTERVAL));
    }

    #[test]
    fn scan_batch_buffer_flushes_when_batch_is_full() {
        let started_at = Instant::now();
        let mut buffer = ScanBatchBuffer::new(started_at);

        for index in 0..SCAN_BATCH_SIZE {
            buffer.push_entry(test_scanned_entry(index));
        }

        assert!(buffer.should_flush(started_at + Duration::from_millis(1)));
    }

    #[test]
    fn scan_batch_payload_omits_entries_when_not_requested() {
        let payload = scan_batch_payload(
            "job-1",
            "foreground",
            "/tmp/root",
            2,
            vec![test_scanned_entry(1), test_scanned_entry(2)],
            test_scan_progress(),
            false,
        );

        assert_eq!(payload.root, "/tmp/root");
        assert_eq!(payload.batch_index, 2);
        assert_eq!(payload.progress.scanned, 2);
        assert!(payload.entries.is_empty());
    }

    #[test]
    fn scan_batch_payload_includes_entries_when_requested() {
        let payload = scan_batch_payload(
            "job-1",
            "foreground",
            "/tmp/root",
            3,
            vec![test_scanned_entry(1), test_scanned_entry(2)],
            test_scan_progress(),
            true,
        );

        assert_eq!(payload.root, "/tmp/root");
        assert_eq!(payload.batch_index, 3);
        assert_eq!(payload.entries.len(), 2);
    }

    #[test]
    fn scan_completion_emits_complete_before_scheduling_dedupe() {
        let events = std::cell::RefCell::new(Vec::new());

        emit_scan_complete_then_schedule_dedupe(
            || {
                events.borrow_mut().push("scan-complete");
                Ok(())
            },
            || events.borrow_mut().push("dedupe-started"),
        )
        .expect("finish scan");

        assert_eq!(*events.borrow(), vec!["scan-complete", "dedupe-started"]);
    }

    #[test]
    fn scan_jobs_have_independent_cancellation_tokens() {
        let jobs = ScanJobManager::default();
        let foreground = jobs.register("foreground-1").expect("foreground job");
        let background = jobs.register("background-1").expect("background job");
        let foreground_token = foreground.token();
        let background_token = background.token();

        assert!(jobs.cancel("background-1"));

        assert!(!is_scan_cancelled(&foreground_token));
        assert!(is_scan_cancelled(&background_token));
    }

    #[test]
    fn scan_job_guard_drop_releases_after_panic_and_is_idempotent() {
        let jobs = ScanJobManager::default();
        let panic_jobs = jobs.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = panic_jobs.register("panic-scan").expect("panic scan owner");
            panic!("simulated scan worker panic");
        }));
        assert!(result.is_err());
        assert!(jobs.token("panic-scan").is_none());

        let mut guard = jobs
            .register("idempotent-scan")
            .expect("idempotent scan owner");
        guard.release();
        guard.release();
        assert!(jobs.token("idempotent-scan").is_none());
    }

    #[test]
    fn stale_scan_job_guard_cannot_release_a_new_generation() {
        let jobs = ScanJobManager::default();
        let old = jobs.register("reused-scan").expect("old scan owner");
        jobs.release_if_current("reused-scan", old.generation, &old.token);
        let current = jobs.register("reused-scan").expect("current scan owner");
        drop(old);
        assert!(jobs.token("reused-scan").is_some());
        drop(current);
        assert!(jobs.token("reused-scan").is_none());
    }

    fn test_scanned_entry(index: usize) -> ScannedEntry {
        ScannedEntry {
            path: format!("/tmp/file-{index}.txt"),
            name: format!("file-{index}.txt"),
            extension: "txt".to_string(),
            size: 1,
            mtime: 0,
            ctime: 0,
            is_dir: false,
            state_code: 0,
        }
    }

    fn test_scan_progress() -> ScanProgressPayload {
        ScanProgressPayload {
            job_id: "job-1".to_string(),
            job_kind: "foreground".to_string(),
            root: "/tmp/root".to_string(),
            scanned: 2,
            files: 2,
            directories: 0,
            skipped: 0,
            errors: 0,
            elapsed_ms: 100,
        }
    }
}
