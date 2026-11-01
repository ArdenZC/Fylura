use crate::{
    analysis::AnalysisRunManager,
    db::{
        AnalysisFindingDto, AnalysisFindingFilter, AnalysisRunDto, Database, DbError,
        OperationPreviewDto, OperationPreviewScopeResult,
    },
    ids::new_job_id,
    path_identity::{
        current_platform, normalize_for_compare, normalize_path, normalize_text_for_compare,
        validate_windows_path_text, PathPlatform, WindowsPathNamespace,
    },
    window_auth::{is_main_window_label, require_main_window},
};
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};

const LARGE_FILE_THRESHOLD: u64 = 100 * 1024 * 1024;
const LARGE_DIR_THRESHOLD: u64 = 500 * 1024 * 1024;
const MAX_RETAINED_CLEANUP_JOBS: usize = 8;
const STORAGE_CLEANUP_PAGE_SIZE: usize = 200;
#[allow(dead_code)]
const STORAGE_CLEANUP_PROGRESS_EVENT: &str = "storage-cleanup-progress";
#[allow(dead_code)]
const STORAGE_CLEANUP_COMPLETED_EVENT: &str = "storage-cleanup-completed";
#[allow(dead_code)]
const STORAGE_CLEANUP_FAILED_EVENT: &str = "storage-cleanup-failed";
#[allow(dead_code)]
const STORAGE_CLEANUP_CANCELLED_EVENT: &str = "storage-cleanup-cancelled";
pub const CLEANUP_RESTORE_PROGRESS_EVENT: &str = "cleanup-restore-progress";
const STORAGE_CLEANUP_EMIT_INTERVAL: Duration = Duration::from_millis(300);
const STORAGE_CLEANUP_EMIT_ENTRY_INTERVAL: u64 = 100;

#[derive(Debug)]
enum SafeTrashMutationError {
    Validation(String),
    Atomic(crate::fs_safety::AtomicMoveError),
}

impl From<String> for SafeTrashMutationError {
    fn from(error: String) -> Self {
        Self::Validation(error)
    }
}

impl From<crate::fs_safety::AtomicMoveError> for SafeTrashMutationError {
    fn from(error: crate::fs_safety::AtomicMoveError) -> Self {
        Self::Atomic(error)
    }
}

impl std::fmt::Display for SafeTrashMutationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(error) => formatter.write_str(error),
            Self::Atomic(error) => error.fmt(formatter),
        }
    }
}

impl SafeTrashMutationError {
    fn journal_phase(&self) -> &'static str {
        match self {
            Self::Validation(_) => "rolled_back",
            Self::Atomic(error) => match error {
                crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown
                | crate::fs_safety::AtomicMoveError::TargetCommittedIdentityMismatch => {
                    "target_committed"
                }
                crate::fs_safety::AtomicMoveError::TargetCommittedSourceCleanupPending
                | crate::fs_safety::AtomicMoveError::TargetCommittedSourceDeleteFailed(_) => {
                    "source_cleanup_pending"
                }
                _ => match error.commit_state() {
                    crate::fs_safety::AtomicMoveCommitState::RolledBack => "rolled_back",
                    crate::fs_safety::AtomicMoveCommitState::SourceClaimed => "source_claimed",
                    crate::fs_safety::AtomicMoveCommitState::TargetCommitted => "target_committed",
                    crate::fs_safety::AtomicMoveCommitState::SourceCleanupPending => {
                        "source_cleanup_pending"
                    }
                    crate::fs_safety::AtomicMoveCommitState::Completed => "completed",
                    crate::fs_safety::AtomicMoveCommitState::ManualReview => "manual_review",
                },
            },
        }
    }

    fn requires_recovery(&self) -> bool {
        match self {
            Self::Validation(_) => false,
            Self::Atomic(error) => !matches!(
                error.commit_state(),
                crate::fs_safety::AtomicMoveCommitState::RolledBack
            ),
        }
    }

    fn is_cancelled(&self) -> bool {
        matches!(
            self,
            Self::Atomic(crate::fs_safety::AtomicMoveError::Cancelled)
        )
    }
}

fn safe_trash_error_message(error: &SafeTrashMutationError, restore: bool) -> String {
    let detail = error.to_string();
    match error {
        SafeTrashMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown,
        ) => crate::recovery::format_recovery_message(
            crate::recovery::RecoveryErrorCode::TargetCommittedDurabilityUnknown,
            &format!(
                "target may have committed, but durability is unknown. Check the target before retrying; do not retry automatically. ({detail})"
            ),
        ),
        SafeTrashMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedIdentityMismatch,
        ) => crate::recovery::format_recovery_message(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch,
            &format!(
                "target may have committed, but post-commit identity verification failed. Check the target before retrying; do not retry automatically. ({detail})"
            ),
        ),
        SafeTrashMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedSourceCleanupPending,
        ) => crate::recovery::format_recovery_message(
            crate::recovery::RecoveryErrorCode::TargetCommittedSourceCleanupPending,
            &format!(
                "target is committed and source cleanup is still pending. Check both paths before retrying; do not retry automatically. ({detail})"
            ),
        ),
        SafeTrashMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::TargetCommittedSourceDeleteFailed(_),
        ) => crate::recovery::format_recovery_message(
            crate::recovery::RecoveryErrorCode::TargetCommittedSourceDeleteFailed,
            &format!(
                "target is committed but source deletion failed. Check both paths before retrying; do not retry automatically. ({detail})"
            ),
        ),
        SafeTrashMutationError::Atomic(crate::fs_safety::AtomicMoveError::SourceClaimMismatch) => {
            crate::recovery::format_recovery_message(
                crate::recovery::RecoveryErrorCode::ClaimIdentityMismatch,
                &format!(
                    "source claim identity did not match; manual review is required. ({detail})"
                ),
            )
        }
        SafeTrashMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::MacClaimNamespaceRebound,
        ) => crate::recovery::format_recovery_message(
            crate::recovery::RecoveryErrorCode::ClaimNamespaceRebound,
            "the claim pathname now points to a different object; manual review is required",
        ),
        SafeTrashMutationError::Atomic(crate::fs_safety::AtomicMoveError::MacClaimIdentityMismatch) => {
            crate::recovery::format_recovery_message(
                crate::recovery::RecoveryErrorCode::ClaimIdentityMismatch,
                "the claimed object identity no longer matches; manual review is required",
            )
        }
        SafeTrashMutationError::Atomic(crate::fs_safety::AtomicMoveError::MacClaimPathMissing) => {
            crate::recovery::format_recovery_message(
                crate::recovery::RecoveryErrorCode::ClaimPathMissing,
                "the claim pathname disappeared; manual review is required",
            )
        }
        SafeTrashMutationError::Atomic(crate::fs_safety::AtomicMoveError::MacClaimPathUnreadable) => {
            crate::recovery::format_recovery_message(
                crate::recovery::RecoveryErrorCode::ClaimPathUnreadable,
                "the claim pathname could not be read; manual review is required",
            )
        }
        SafeTrashMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::SourceClaimRecoveryRequired(_)
            | crate::fs_safety::AtomicMoveError::SourceClaimRollbackFailed(_),
        ) if restore => crate::recovery::format_recovery_message(
            crate::recovery::RecoveryErrorCode::RestorePendingReconciliation,
            &format!(
                "restore state could not be finalized; startup reconciliation is required. Do not retry automatically. ({detail})"
            ),
        ),
        SafeTrashMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::SourceClaimRecoveryRequired(_)
            | crate::fs_safety::AtomicMoveError::SourceClaimRollbackFailed(_),
        ) => crate::recovery::format_recovery_message(
            crate::recovery::RecoveryErrorCode::ManualReviewRequired,
            &format!(
                "source claim state could not be finalized; startup reconciliation is required. Do not retry automatically. ({detail})"
            ),
        ),
        _ => detail,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageAnalysis {
    pub total_size: u64,
    pub reclaimable_estimate: u64,
    pub review_estimate: u64,
    pub candidates: Vec<StorageCandidate>,
    pub denied_paths: Vec<String>,
    pub warnings: Vec<String>,
    pub candidate_total: usize,
    pub candidate_offset: usize,
    pub candidate_limit: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageCandidate {
    pub id: String,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub tier: CleanupTier,
    pub category: String,
    pub reason: String,
    pub suggested_action: CleanupActionKind,
    pub risk_note: Option<String>,
    pub trash_allowed: bool,
    pub selected_by_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupFindingSelection {
    pub finding_id: String,
    pub expected_revision: i64,
    pub review_confirmation: Option<ReviewFindingConfirmation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFindingConfirmation {
    pub decision_revision: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum CleanupTier {
    Safe,
    Review,
    Caution,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum CleanupActionKind {
    MoveToTrash,
    Reveal,
    UninstallAdvice,
    AppInternalCleanup,
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupPreviewItem {
    pub id: String,
    pub candidate_id: String,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub tier: CleanupTier,
    pub category: String,
    pub reason: String,
    pub operation_type: String,
    pub target_path: String,
    pub status: String,
    pub requires_confirmation: bool,
    pub is_executable: bool,
    pub blocking_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupExecutionResult {
    pub moved: usize,
    pub skipped: usize,
    pub failed: usize,
    pub logs: Vec<CleanupExecutionLog>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupExecutionLog {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub status: String,
    pub message: String,
    pub item_id: Option<String>,
    pub trash_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleanupProgress {
    pub job_id: String,
    pub scanned_entries: u64,
    pub current_path: Option<String>,
    pub total_size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleanupCompleted {
    pub job_id: String,
    pub analysis: StorageAnalysis,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleanupJobMessage {
    pub job_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleanupScanStatus {
    pub job_id: String,
    pub status: String,
    pub progress: StorageCleanupProgress,
    pub analysis: Option<StorageAnalysis>,
    pub error: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupTrashItem {
    pub id: String,
    pub batch_id: String,
    pub original_path: String,
    pub trash_path: String,
    pub name: String,
    pub size: u64,
    pub moved_at: String,
    pub restored_at: Option<String>,
    pub status: String,
    pub message: Option<String>,
    #[serde(skip_serializing)]
    pub source_modified_ns: Option<String>,
    #[serde(skip_serializing)]
    pub source_platform_volume_id: Option<String>,
    #[serde(skip_serializing)]
    pub source_platform_file_id: Option<String>,
    #[serde(skip_serializing)]
    pub source_quick_hash: Option<String>,
    #[serde(skip_serializing)]
    pub source_full_hash: Option<String>,
    #[serde(skip_serializing)]
    pub trash_modified_ns: Option<String>,
    #[serde(skip_serializing)]
    pub trash_platform_volume_id: Option<String>,
    #[serde(skip_serializing)]
    pub trash_platform_file_id: Option<String>,
    #[serde(skip_serializing)]
    pub trash_quick_hash: Option<String>,
    #[serde(skip_serializing)]
    pub trash_full_hash: Option<String>,
    #[serde(skip_serializing)]
    pub identity_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_claim_path: Option<String>,
    #[serde(skip_serializing)]
    pub operation_phase: String,
    #[serde(skip_serializing)]
    pub claim_created_at: Option<String>,
    #[serde(skip_serializing)]
    pub claim_platform_file_id: Option<String>,
    #[serde(skip_serializing)]
    pub claim_full_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupTrashBatch {
    pub id: String,
    pub created_at: String,
    pub root: Option<String>,
    pub total_items: usize,
    pub total_size: u64,
    pub status: String,
    pub items: Vec<CleanupTrashItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRestorePreview {
    pub batch_id: String,
    pub items: Vec<CleanupRestorePreviewItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRestorePreviewItem {
    #[serde(flatten)]
    pub item: CleanupTrashItem,
    pub can_restore: bool,
    pub blocking_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRestoreLog {
    pub item_id: String,
    pub original_path: String,
    pub trash_path: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRestoreResult {
    pub restored: usize,
    pub conflicts: usize,
    pub missing: usize,
    pub failed: usize,
    pub canceled: usize,
    pub logs: Vec<CleanupRestoreLog>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRestoreProgressPayload {
    pub job_id: String,
    pub processed: usize,
    pub total: usize,
    pub current_item_id: Option<String>,
    pub current_path: Option<String>,
    pub restored: usize,
    pub conflicts: usize,
    pub missing: usize,
    pub failed: usize,
    pub canceled: usize,
    pub cancel_requested: bool,
}

#[derive(Clone, Default)]
pub struct StorageCleanupState {
    inner: Arc<StorageCleanupStateInner>,
}

#[derive(Clone, Default)]
pub struct CleanupRestoreState {
    inner: Arc<CleanupRestoreStateInner>,
}

#[derive(Default)]
struct CleanupRestoreStateInner {
    jobs: Mutex<HashMap<String, CleanupRestoreJob>>,
    next_sequence: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupRestoreJobStatus {
    Running,
    Completed,
    Canceled,
    Failed,
}

impl CleanupRestoreJobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone)]
struct CleanupRestoreJob {
    cancel_flag: Arc<AtomicBool>,
    status: CleanupRestoreJobStatus,
    sequence: u64,
}

impl CleanupRestoreState {
    fn start_job(&self, job_id: String) -> Result<Arc<AtomicBool>, String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Cleanup restore job state is unavailable.".to_string())?;
        if jobs
            .values()
            .any(|job| job.status == CleanupRestoreJobStatus::Running)
        {
            return Err("Another cleanup restore is already running.".to_string());
        }
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let sequence = self.inner.next_sequence.fetch_add(1, Ordering::Relaxed);
        jobs.insert(
            job_id,
            CleanupRestoreJob {
                cancel_flag: Arc::clone(&cancel_flag),
                status: CleanupRestoreJobStatus::Running,
                sequence,
            },
        );
        while jobs.len() > MAX_RETAINED_CLEANUP_JOBS {
            let oldest = jobs
                .iter()
                .filter(|(_, job)| job.status != CleanupRestoreJobStatus::Running)
                .min_by_key(|(_, job)| job.sequence)
                .map(|(id, _)| id.clone());
            let Some(oldest) = oldest else { break };
            jobs.remove(&oldest);
        }
        Ok(cancel_flag)
    }

    fn cancel_job(&self, job_id: &str) -> Result<(), String> {
        let jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Cleanup restore job state is unavailable.".to_string())?;
        let job = jobs
            .get(job_id)
            .ok_or_else(|| format!("Cleanup restore job not found: {job_id}"))?;
        if job.status == CleanupRestoreJobStatus::Running {
            job.cancel_flag.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn cancel_for_lifecycle(&self) -> Result<usize, String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Cleanup restore job state is unavailable.".to_string())?;
        let mut canceled = 0;
        for job in jobs.values_mut() {
            if job.status == CleanupRestoreJobStatus::Running {
                job.cancel_flag.store(true, Ordering::Relaxed);
                job.status = CleanupRestoreJobStatus::Canceled;
                canceled += 1;
            }
        }
        Ok(canceled)
    }

    fn finish_job(&self, job_id: &str, status: CleanupRestoreJobStatus) -> Result<(), String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Cleanup restore job state is unavailable.".to_string())?;
        if let Some(job) = jobs.get_mut(job_id) {
            if job.status == CleanupRestoreJobStatus::Running {
                job.status = status;
            }
        }
        Ok(())
    }

    pub fn status_for_test(&self, job_id: &str) -> Option<CleanupRestoreJobStatus> {
        self.inner
            .jobs
            .lock()
            .ok()?
            .get(job_id)
            .map(|job| job.status)
    }

    pub fn start_job_for_test(&self, job_id: impl Into<String>) -> Result<Arc<AtomicBool>, String> {
        self.start_job(job_id.into())
    }

    pub fn cancel_job_for_test(&self, job_id: &str) -> Result<(), String> {
        self.cancel_job(job_id)
    }

    pub fn finish_job_for_test(
        &self,
        job_id: &str,
        status: CleanupRestoreJobStatus,
    ) -> Result<(), String> {
        self.finish_job(job_id, status)
    }
}

struct CleanupRestoreJobGuard {
    state: CleanupRestoreState,
    job_id: String,
    finished: bool,
}

impl CleanupRestoreJobGuard {
    fn new(state: CleanupRestoreState, job_id: String) -> Self {
        Self {
            state,
            job_id,
            finished: false,
        }
    }

    fn finish(&mut self, status: CleanupRestoreJobStatus) {
        let _ = self.state.finish_job(&self.job_id, status);
        self.finished = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupRestoreTestOutcome {
    Completed,
    Canceled,
    Failed,
    Panic,
}

pub fn run_cleanup_restore_job_for_test(
    state: &CleanupRestoreState,
    job_id: &str,
    outcome: CleanupRestoreTestOutcome,
) -> Result<(), String> {
    state.start_job_for_test(job_id.to_string())?;
    let state = state.clone();
    let job_id = job_id.to_string();
    let result = catch_unwind(AssertUnwindSafe(move || {
        let mut guard = CleanupRestoreJobGuard::new(state, job_id);
        match outcome {
            CleanupRestoreTestOutcome::Completed => {
                guard.finish(CleanupRestoreJobStatus::Completed)
            }
            CleanupRestoreTestOutcome::Canceled => guard.finish(CleanupRestoreJobStatus::Canceled),
            CleanupRestoreTestOutcome::Failed => guard.finish(CleanupRestoreJobStatus::Failed),
            CleanupRestoreTestOutcome::Panic => panic!("simulated cleanup restore worker panic"),
        }
    }));
    result.map_err(|_| "simulated cleanup restore worker panic".to_string())
}

impl Drop for CleanupRestoreJobGuard {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self
                .state
                .finish_job(&self.job_id, CleanupRestoreJobStatus::Failed);
        }
    }
}

trait CleanupRestoreProgressEmitter {
    fn emit_progress(&self, payload: CleanupRestoreProgressPayload);
}

struct NoopCleanupRestoreProgressEmitter;

impl CleanupRestoreProgressEmitter for NoopCleanupRestoreProgressEmitter {
    fn emit_progress(&self, _payload: CleanupRestoreProgressPayload) {}
}

struct TauriCleanupRestoreProgressEmitter<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriCleanupRestoreProgressEmitter<R> {
    fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> CleanupRestoreProgressEmitter for TauriCleanupRestoreProgressEmitter<R> {
    fn emit_progress(&self, payload: CleanupRestoreProgressPayload) {
        if let Err(error) = self.app.emit(CLEANUP_RESTORE_PROGRESS_EVENT, payload) {
            eprintln!("Cleanup restore progress event failed: {error}");
        }
    }
}

#[derive(Default)]
struct StorageCleanupStateInner {
    jobs: Mutex<HashMap<String, StorageCleanupJob>>,
    active_job_id: Mutex<Option<String>>,
}

#[derive(Clone)]
struct StorageCleanupJob {
    status: StorageCleanupScanStatus,
    cancel_flag: Arc<AtomicBool>,
    candidates_by_id: HashMap<String, StorageCandidate>,
    consumed_ids: HashSet<String>,
}

impl StorageCleanupState {
    pub(crate) fn candidates_by_job_and_ids(
        &self,
        job_id: &str,
        ids: &[String],
    ) -> Result<Vec<StorageCandidate>, String> {
        let jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        let job = jobs
            .get(job_id)
            .ok_or_else(|| format!("Storage cleanup scan job not found: {job_id}"))?;
        if job.status.status != "completed" {
            return Err(format!(
                "Storage cleanup scan job is not completed: {job_id}"
            ));
        }
        let mut seen = HashSet::with_capacity(ids.len());
        ids.iter()
            .map(|id| {
                if !seen.insert(id.as_str()) {
                    return Err(format!(
                        "Storage cleanup candidate request contains duplicate ID: {id}"
                    ));
                }
                if job.consumed_ids.contains(id) {
                    return Err(format!(
                        "Storage cleanup candidate was already consumed for job {job_id}: {id}"
                    ));
                }
                job.candidates_by_id.get(id).cloned().ok_or_else(|| {
                    format!("Storage cleanup candidate does not belong to job {job_id}: {id}")
                })
            })
            .collect()
    }

    fn mark_candidates_consumed(&self, job_id: &str, ids: &[String]) -> Result<(), String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| format!("Storage cleanup scan job not found: {job_id}"))?;
        for id in ids {
            if !job.candidates_by_id.contains_key(id) {
                return Err(format!(
                    "Storage cleanup candidate does not belong to job {job_id}: {id}"
                ));
            }
        }
        job.consumed_ids.extend(ids.iter().cloned());
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn update_candidates_for_job(
        &self,
        job_id: &str,
        candidates: &[StorageCandidate],
    ) -> Result<(), String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| format!("Storage cleanup scan job not found: {job_id}"))?;
        if job.status.status != "completed" {
            return Err(format!(
                "Storage cleanup scan job is not completed: {job_id}"
            ));
        }
        for candidate in candidates {
            if !job.candidates_by_id.contains_key(&candidate.id) {
                return Err(format!(
                    "Storage cleanup candidate does not belong to job {job_id}: {}",
                    candidate.id
                ));
            }
        }
        let analysis = job
            .status
            .analysis
            .as_mut()
            .ok_or_else(|| format!("Completed storage cleanup job not found: {job_id}"))?;
        for candidate in candidates {
            let stored = analysis
                .candidates
                .iter_mut()
                .find(|stored| stored.id == candidate.id)
                .ok_or_else(|| {
                    format!(
                        "Storage cleanup candidate does not belong to job {job_id}: {}",
                        candidate.id
                    )
                })?;
            *stored = candidate.clone();
            job.candidates_by_id
                .insert(candidate.id.clone(), candidate.clone());
        }
        Ok(())
    }

    fn start_job(&self, job_id: String, roots: &[PathBuf]) -> Result<Arc<AtomicBool>, String> {
        self.cancel_active_job()?;
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let progress = StorageCleanupProgress {
            job_id: job_id.clone(),
            scanned_entries: 0,
            current_path: roots.first().map(|path| normalize_path(path)),
            total_size: 0,
        };
        let status = StorageCleanupScanStatus {
            job_id: job_id.clone(),
            status: "running".to_string(),
            progress,
            analysis: None,
            error: None,
            started_at: current_timestamp_ms().to_string(),
            completed_at: None,
        };
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        jobs.insert(
            job_id.clone(),
            StorageCleanupJob {
                status,
                cancel_flag: Arc::clone(&cancel_flag),
                candidates_by_id: HashMap::new(),
                consumed_ids: HashSet::new(),
            },
        );
        if jobs.len() > MAX_RETAINED_CLEANUP_JOBS {
            let mut completed = jobs
                .iter()
                .filter(|(id, job)| *id != &job_id && job.status.status != "running")
                .map(|(id, job)| (id.clone(), job.status.started_at.clone()))
                .collect::<Vec<_>>();
            completed.sort_by(|left, right| left.1.cmp(&right.1));
            let remove_count = jobs.len().saturating_sub(MAX_RETAINED_CLEANUP_JOBS);
            for (id, _) in completed.into_iter().take(remove_count) {
                jobs.remove(&id);
            }
        }
        *self
            .inner
            .active_job_id
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())? = Some(job_id);
        Ok(cancel_flag)
    }

    fn cancel_active_job(&self) -> Result<(), String> {
        let active = self
            .inner
            .active_job_id
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?
            .clone();
        if let Some(job_id) = active {
            let mut jobs = self
                .inner
                .jobs
                .lock()
                .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
            if let Some(job) = jobs.get_mut(&job_id) {
                if job.status.status == "running" {
                    job.cancel_flag.store(true, Ordering::Relaxed);
                    job.status.status = "cancelled".to_string();
                    job.status.completed_at = Some(current_timestamp_ms().to_string());
                }
            }
        }
        Ok(())
    }

    fn cancel_job(&self, job_id: &str) -> Result<(), String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| format!("Storage cleanup scan job not found: {job_id}"))?;
        job.cancel_flag.store(true, Ordering::Relaxed);
        if job.status.status == "running" {
            job.status.status = "cancelled".to_string();
            job.status.completed_at = Some(current_timestamp_ms().to_string());
        }
        Ok(())
    }

    fn update_job_progress(&self, progress: StorageCleanupProgress) -> Result<(), String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        if let Some(job) = jobs.get_mut(&progress.job_id) {
            job.status.progress = progress;
        }
        Ok(())
    }

    fn complete_job(
        &self,
        job_id: &str,
        mut analysis: StorageAnalysis,
    ) -> Result<StorageAnalysis, String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| format!("Storage cleanup scan job not found: {job_id}"))?;
        for candidate in &mut analysis.candidates {
            candidate.id = candidate_id_for_job(job_id, &candidate.path);
        }
        job.candidates_by_id = analysis
            .candidates
            .iter()
            .cloned()
            .map(|candidate| (candidate.id.clone(), candidate))
            .collect();
        job.status.status = "completed".to_string();
        job.status.progress.total_size = analysis.total_size;
        job.status.analysis = Some(analysis);
        job.status.completed_at = Some(current_timestamp_ms().to_string());
        let completed_page = storage_analysis_page(
            job.status.analysis.as_ref().expect("completed analysis"),
            0,
            STORAGE_CLEANUP_PAGE_SIZE,
        );
        drop(jobs);
        self.clear_active_job(job_id)?;
        Ok(completed_page)
    }

    fn fail_job(&self, job_id: &str, message: String) -> Result<(), String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        if let Some(job) = jobs.get_mut(job_id) {
            job.status.status = "failed".to_string();
            job.status.error = Some(message);
            job.status.completed_at = Some(current_timestamp_ms().to_string());
        }
        self.clear_active_job(job_id)?;
        Ok(())
    }

    fn mark_job_cancelled(&self, job_id: &str) -> Result<(), String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        if let Some(job) = jobs.get_mut(job_id) {
            job.status.status = "cancelled".to_string();
            job.status.completed_at = Some(current_timestamp_ms().to_string());
        }
        self.clear_active_job(job_id)?;
        Ok(())
    }

    fn clear_active_job(&self, job_id: &str) -> Result<(), String> {
        let mut active = self
            .inner
            .active_job_id
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        if active.as_deref() == Some(job_id) {
            *active = None;
        }
        Ok(())
    }

    fn job_status(&self, job_id: &str) -> Result<StorageCleanupScanStatus, String> {
        let jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        jobs.get(job_id)
            .map(|job| {
                let mut status = job.status.clone();
                status.analysis = status
                    .analysis
                    .as_ref()
                    .map(|analysis| storage_analysis_page(analysis, 0, STORAGE_CLEANUP_PAGE_SIZE));
                status
            })
            .ok_or_else(|| format!("Storage cleanup scan job not found: {job_id}"))
    }

    fn job_analysis_page(
        &self,
        job_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<StorageAnalysis, String> {
        let jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| "Storage cleanup job state is unavailable.".to_string())?;
        let analysis = jobs
            .get(job_id)
            .and_then(|job| job.status.analysis.as_ref())
            .ok_or_else(|| format!("Completed storage cleanup job not found: {job_id}"))?;
        Ok(storage_analysis_page(analysis, offset, limit.clamp(1, 500)))
    }
}

#[tauri::command]
pub fn start_storage_cleanup_scan<R: Runtime>(
    window: WebviewWindow<R>,
    roots: Vec<String>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    manager: State<'_, AnalysisRunManager>,
) -> Result<String, String> {
    require_main_window(&window)?;
    let roots = validate_cleanup_roots(roots)?;
    let run = crate::analysis::start_cleanup_analysis_run(
        app,
        db.inner(),
        manager.inner(),
        roots
            .into_iter()
            .map(|path| normalize_path(&path))
            .collect(),
    )?;
    Ok(run.id)
}

#[tauri::command]
pub fn cancel_storage_cleanup_scan<R: Runtime>(
    window: WebviewWindow<R>,
    job_id: String,
    app: AppHandle<R>,
    db: State<'_, Database>,
    manager: State<'_, AnalysisRunManager>,
) -> Result<(), String> {
    require_main_window(&window)?;
    manager.cancel(job_id.trim());
    let run = db
        .request_analysis_cancellation(job_id.trim())
        .map_err(|error| error.to_string())?;
    let _ = app.emit(crate::analysis::ANALYSIS_RUN_UPDATED_EVENT, run);
    Ok(())
}

#[tauri::command]
pub fn get_storage_cleanup_scan_status(
    job_id: String,
    db: State<'_, Database>,
) -> Result<StorageCleanupScanStatus, String> {
    let run = db
        .get_analysis_run(job_id.trim())
        .map_err(|error| error.to_string())?;
    durable_cleanup_status(db.inner(), &run)
}

#[tauri::command]
pub fn get_storage_cleanup_candidate_page(
    job_id: String,
    offset: usize,
    limit: Option<usize>,
    db: State<'_, Database>,
) -> Result<StorageAnalysis, String> {
    let run = db
        .get_analysis_run(job_id.trim())
        .map_err(|error| error.to_string())?;
    durable_cleanup_analysis(
        db.inner(),
        &run,
        offset,
        limit.unwrap_or(STORAGE_CLEANUP_PAGE_SIZE),
    )
}

#[tauri::command]
pub fn reveal_storage_candidate(path: String) -> Result<(), String> {
    crate::file_ops::reveal_in_folder(path)
}

#[tauri::command]
pub fn preview_cleanup_candidates(
    job_id: String,
    selections: Vec<CleanupFindingSelection>,
    db: State<'_, Database>,
) -> Result<Vec<CleanupPreviewItem>, String> {
    let ids = selections
        .iter()
        .map(|selection| selection.finding_id.clone())
        .collect::<Vec<_>>();
    let candidates =
        resolve_analysis_candidates_for_cleanup(db.inner(), &job_id, &selections, true)?;
    cleanup_preview_items_for_candidates(ids, &candidates)
}

#[tauri::command]
pub fn preview_cleanup_operations<R: Runtime>(
    job_id: String,
    selections: Vec<CleanupFindingSelection>,
    app: AppHandle<R>,
    db: State<'_, Database>,
) -> Result<OperationPreviewScopeResult, String> {
    let app_data_dir = app.path().app_data_dir().ok();
    let ids = selections
        .iter()
        .map(|selection| selection.finding_id.clone())
        .collect::<Vec<_>>();
    let candidates =
        resolve_analysis_candidates_for_cleanup(db.inner(), &job_id, &selections, true)?;
    preview_cleanup_operations_for_candidates(ids, &candidates, app_data_dir.as_deref())
}

#[tauri::command]
pub fn move_cleanup_candidates_to_safe_trash<R: Runtime>(
    window: WebviewWindow<R>,
    job_id: String,
    selections: Vec<CleanupFindingSelection>,
    app: AppHandle<R>,
    db: State<'_, Database>,
) -> Result<CleanupExecutionResult, String> {
    require_main_window(&window)?;
    let app_data_dir = app.path().app_data_dir().ok();
    let ids = selections
        .iter()
        .map(|selection| selection.finding_id.clone())
        .collect::<Vec<_>>();
    let candidates =
        resolve_analysis_candidates_for_cleanup(db.inner(), &job_id, &selections, true)?;
    let result = move_cleanup_candidates_to_safe_trash_for_candidates(
        ids.clone(),
        &candidates,
        db.inner(),
        app_data_dir.as_deref(),
    )?;
    let consumed = successful_cleanup_ids(&ids, &result);
    mark_analysis_candidates_consumed(db.inner(), &consumed);
    Ok(result)
}

fn successful_cleanup_ids(ids: &[String], result: &CleanupExecutionResult) -> Vec<String> {
    ids.iter()
        .zip(&result.logs)
        .filter(|(_, log)| log.status == "success")
        .map(|(id, _)| id.clone())
        .collect()
}

/// Resolve the legacy cleanup adapter's durable selections from the analysis
/// finding ledger. `StorageCleanupState` remains available only for old Rust
/// unit-test helpers; production commands never use it as an authority after
/// the Task 03 migration.
pub(crate) fn resolve_analysis_candidates_for_cleanup(
    db: &Database,
    job_id: &str,
    selections: &[CleanupFindingSelection],
    require_executable: bool,
) -> Result<Vec<StorageCandidate>, String> {
    let run = db
        .get_analysis_run(job_id.trim())
        .map_err(|error| error.to_string())?;
    if run.scope.get("kind").and_then(serde_json::Value::as_str) != Some("approved_cleanup_paths") {
        return Err("The requested analysis run is not an approved cleanup scope.".to_string());
    }
    if !matches!(run.status.as_str(), "completed" | "completed_with_warnings") {
        return Err(format!("Analysis run is not executable: {}", run.status));
    }
    let mut seen = HashSet::with_capacity(selections.len());
    let mut candidates = Vec::with_capacity(selections.len());
    for selection in selections {
        let id = selection.finding_id.trim();
        if id.is_empty() || !seen.insert(id.to_string()) {
            return Err(format!(
                "Storage cleanup candidate request contains duplicate ID: {id}"
            ));
        }
        let finding = db
            .get_analysis_finding(id.trim())
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Storage cleanup finding was not found: {id}"))?;
        if finding.run_id != run.id {
            return Err(format!(
                "Storage cleanup finding does not belong to run: {id}"
            ));
        }
        if finding.status != "active" {
            return Err(format!("Storage cleanup finding is no longer active: {id}"));
        }
        if finding.revision != selection.expected_revision {
            return Err(format!(
                "Storage cleanup finding revision is stale: {id} (expected {}, current {})",
                selection.expected_revision, finding.revision
            ));
        }
        let Some(path) = finding.path_snapshot.as_deref() else {
            return Err(format!(
                "Storage cleanup finding has no path snapshot: {id}"
            ));
        };
        if !Path::new(path).exists() {
            let _ = db.mark_analysis_finding_stale(&finding.id);
            return Err(format!(
                "Storage cleanup finding path is no longer present: {id}"
            ));
        }
        if !crate::analysis::finding_identity_matches(db, &finding) {
            let _ = db.mark_analysis_finding_stale(&finding.id);
            return Err(format!("Storage cleanup finding identity changed: {id}"));
        }
        if require_executable && !crate::ai::cleanup::has_current_ai_assessment(db, &finding.id) {
            return Err(format!(
                "Storage cleanup finding requires a current AI assessment: {id}"
            ));
        }
        let mut candidate = storage_candidate_from_analysis_finding(&finding)?;
        if require_executable {
            authorize_cleanup_candidate(
                &finding,
                &mut candidate,
                selection.review_confirmation.as_ref(),
            )?;
        } else if selection.review_confirmation.is_some() {
            return Err(format!(
                "Review confirmation is only valid for cleanup execution: {id}"
            ));
        }
        candidates.push(candidate);
    }
    Ok(candidates)
}

pub(crate) fn authorize_cleanup_candidate(
    finding: &AnalysisFindingDto,
    candidate: &mut StorageCandidate,
    review_confirmation: Option<&ReviewFindingConfirmation>,
) -> Result<(), String> {
    // Saved Safe flags and acknowledged Review decisions are not evidence that
    // an ambiguously named directory contains regenerable output.
    if generated_output_requires_review(Path::new(&candidate.path)) {
        return Err(format!(
            "Generated output name has no independent cleanup safety evidence: {}",
            finding.id
        ));
    }
    match finding.tier.as_str() {
        "safe" => {
            if finding.executable
                && finding.action_kind == "safe_trash_candidate"
                && candidate.trash_allowed
            {
                Ok(())
            } else {
                Err(format!(
                    "Storage cleanup finding is not Safe Trash executable: {}",
                    finding.id
                ))
            }
        }
        "review" => {
            if finding.primary_subject_kind == "duplicate_group"
                || finding.action_kind != "safe_trash_candidate"
            {
                return Err(format!(
                    "Review finding is reveal-only or duplicate-blocked: {}",
                    finding.id
                ));
            }
            let Some(confirmation) = review_confirmation else {
                return Err(format!(
                    "Review finding requires explicit acknowledged confirmation: {}",
                    finding.id
                ));
            };
            if finding.decision.as_deref() != Some("acknowledged")
                || finding.decision_revision != Some(confirmation.decision_revision)
            {
                return Err(format!(
                    "Review finding confirmation is stale or not acknowledged: {}",
                    finding.id
                ));
            }
            // The durable finding remains Review; this internal projection is
            // the server-authorized Safe Trash capability for this exact CAS
            // selection and cannot be reused without resolving it again.
            candidate.tier = CleanupTier::Safe;
            candidate.suggested_action = CleanupActionKind::MoveToTrash;
            candidate.trash_allowed = true;
            candidate.selected_by_default = false;
            Ok(())
        }
        "caution" => Err(format!(
            "Caution findings are never executable: {}",
            finding.id
        )),
        other => Err(format!("Invalid analysis finding tier: {other}")),
    }
}

pub(crate) fn storage_candidate_from_analysis_finding(
    finding: &AnalysisFindingDto,
) -> Result<StorageCandidate, String> {
    let path = finding
        .path_snapshot
        .clone()
        .ok_or_else(|| "Analysis finding has no path snapshot.".to_string())?;
    let path_ref = Path::new(&path);
    let name = path_ref
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(&finding.title)
        .to_string();
    let tier = match finding.tier.as_str() {
        "safe" => CleanupTier::Safe,
        "review" => CleanupTier::Review,
        "caution" => CleanupTier::Caution,
        other => return Err(format!("Analysis finding has invalid tier: {other}")),
    };
    let suggested_action = match finding.action_kind.as_str() {
        "safe_trash_candidate" => CleanupActionKind::MoveToTrash,
        "reveal" => CleanupActionKind::Reveal,
        "uninstall_advice" => CleanupActionKind::UninstallAdvice,
        "app_internal_cleanup" => CleanupActionKind::AppInternalCleanup,
        "none" | "review_duplicate_group" => CleanupActionKind::None,
        other => return Err(format!("Analysis finding has invalid action: {other}")),
    };
    let trash_allowed = finding
        .evidence_summary
        .get("trashAllowed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(finding.executable);
    let needs_review = generated_output_requires_review(path_ref);
    let mut candidate = StorageCandidate {
        id: finding.id.clone(),
        path,
        name,
        size: finding.size_bytes.max(0) as u64,
        tier,
        category: finding.category.clone(),
        reason: finding.reason.clone(),
        suggested_action,
        risk_note: finding.risk_note.clone(),
        trash_allowed,
        selected_by_default: finding.executable && finding.tier == "safe",
    };
    if needs_review {
        if candidate.tier != CleanupTier::Caution {
            let review =
                classify_candidate_without_temp_text(Path::new(&candidate.path), candidate.size);
            candidate.tier = review.tier;
            candidate.category = review.category;
            candidate.reason = review.reason;
            candidate.suggested_action = review.suggested_action;
            candidate.risk_note = review.risk_note;
        }
        candidate.trash_allowed = false;
        candidate.selected_by_default = false;
    }
    Ok(candidate)
}

fn mark_analysis_candidates_consumed(db: &Database, ids: &[String]) {
    for id in ids {
        if let Err(error) = db.mark_analysis_finding_stale(id) {
            eprintln!("Analysis finding could not be invalidated after cleanup {id}: {error}");
        }
    }
}

fn durable_cleanup_status(
    db: &Database,
    run: &AnalysisRunDto,
) -> Result<StorageCleanupScanStatus, String> {
    let analysis = if matches!(run.status.as_str(), "completed" | "completed_with_warnings") {
        Some(durable_cleanup_analysis(
            db,
            run,
            0,
            STORAGE_CLEANUP_PAGE_SIZE,
        )?)
    } else {
        None
    };
    let detectors = db
        .list_analysis_run_detectors(&run.id)
        .map_err(|error| error.to_string())?;
    let scanned_entries = detectors
        .iter()
        .map(|detector| detector.scanned_subjects.max(0) as u64)
        .sum();
    let current_path = run
        .source_snapshot
        .get("paths")
        .and_then(serde_json::Value::as_array)
        .and_then(|paths| paths.first())
        .and_then(|path| path.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Ok(StorageCleanupScanStatus {
        job_id: run.id.clone(),
        status: run.status.clone(),
        progress: StorageCleanupProgress {
            job_id: run.id.clone(),
            scanned_entries,
            current_path,
            total_size: run.potential_reclaimable_bytes.max(0) as u64,
        },
        analysis,
        error: run.error_message.clone(),
        started_at: run.started_at.unwrap_or(run.created_at).to_string(),
        completed_at: run.finished_at.map(|value| value.to_string()),
    })
}

fn durable_cleanup_analysis(
    db: &Database,
    run: &AnalysisRunDto,
    offset: usize,
    limit: usize,
) -> Result<StorageAnalysis, String> {
    let filter = AnalysisFindingFilter {
        run_id: Some(run.id.clone()),
        status: Some("active".to_string()),
        ..AnalysisFindingFilter::default()
    };
    let page_limit = limit.clamp(1, STORAGE_CLEANUP_PAGE_SIZE).saturating_add(1);
    let findings = db
        .list_analysis_findings_offset(&filter, offset, page_limit)
        .map_err(|error| error.to_string())?;
    let has_more = findings.len() > limit.clamp(1, STORAGE_CLEANUP_PAGE_SIZE);
    let candidates = findings
        .into_iter()
        .take(limit.clamp(1, STORAGE_CLEANUP_PAGE_SIZE))
        .filter_map(|finding| storage_candidate_from_analysis_finding(&finding).ok())
        .collect::<Vec<_>>();
    let candidate_total = db
        .count_analysis_findings_for_run(&run.id, "active")
        .map_err(|error| error.to_string())?
        .max(0) as usize;
    let review_estimate = db
        .sum_analysis_finding_size_for_run(&run.id, "review")
        .map_err(|error| error.to_string())?
        .max(0) as u64;
    Ok(StorageAnalysis {
        total_size: run.potential_reclaimable_bytes.max(0) as u64,
        reclaimable_estimate: run.exact_reclaimable_bytes.max(0) as u64,
        review_estimate,
        candidates,
        denied_paths: Vec::new(),
        warnings: run
            .error_message
            .clone()
            .into_iter()
            .chain((run.warning_count > 0).then(|| "Analysis completed with warnings.".to_string()))
            .collect(),
        candidate_total,
        candidate_offset: offset,
        candidate_limit: limit.clamp(1, STORAGE_CLEANUP_PAGE_SIZE),
        has_more,
    })
}

#[tauri::command]
pub fn list_cleanup_trash_batches(
    db: State<'_, Database>,
) -> Result<Vec<CleanupTrashBatch>, String> {
    db.list_cleanup_trash_batches()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_restore_cleanup_trash(
    batch_id: String,
    db: State<'_, Database>,
) -> Result<CleanupRestorePreview, String> {
    let items = db
        .cleanup_trash_items_for_batch(&batch_id)
        .map_err(|error| error.to_string())?;
    Ok(CleanupRestorePreview {
        batch_id,
        items: items
            .into_iter()
            .map(cleanup_restore_preview_item)
            .collect(),
    })
}

#[tauri::command]
pub async fn restore_cleanup_trash_items<R: Runtime>(
    window: WebviewWindow<R>,
    item_ids: Vec<String>,
    job_id: Option<String>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    state: State<'_, CleanupRestoreState>,
) -> Result<CleanupRestoreResult, String> {
    require_main_window(&window)?;
    let job_id = job_id.unwrap_or_else(|| new_job_id("cleanup-restore"));
    let cancel_flag = state.start_job(job_id.clone())?;
    let state = state.inner().clone();
    let state_for_join_error = state.clone();
    let db = db.inner().clone();
    let job_id_for_join = job_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let emitter = TauriCleanupRestoreProgressEmitter::new(app);
        let mut guard = CleanupRestoreJobGuard::new(state, job_id.clone());
        let result = restore_cleanup_trash_items_for_db_with_progress(
            item_ids,
            &db,
            cancel_flag,
            job_id.clone(),
            &emitter,
        );
        let status = match result.as_ref() {
            Ok(value) if value.canceled > 0 => CleanupRestoreJobStatus::Canceled,
            Ok(_) => CleanupRestoreJobStatus::Completed,
            Err(_) => CleanupRestoreJobStatus::Failed,
        };
        guard.finish(status);
        result
    })
    .await
    .map_err(|error| {
        let _ = state_for_join_error.finish_job(&job_id_for_join, CleanupRestoreJobStatus::Failed);
        format!("Cleanup restore worker failed: {error}")
    })?;
    result
}

#[tauri::command]
pub fn cancel_cleanup_restore<R: Runtime>(
    window: WebviewWindow<R>,
    job_id: String,
    state: State<'_, CleanupRestoreState>,
) -> Result<(), String> {
    require_main_window(&window)?;
    state.cancel_job(&job_id)
}

fn cleanup_restore_preview_item(item: CleanupTrashItem) -> CleanupRestorePreviewItem {
    let status = item.status.to_ascii_lowercase();
    let active_recovery_journal = status != "restored"
        && (matches!(
            item.operation_phase.as_str(),
            "source_claimed" | "target_committed" | "source_cleanup_pending" | "manual_review"
        ) || matches!(
            item.identity_status.as_str(),
            "pending_recovery" | "restore_pending" | "restore_pending_recovery"
        ));
    let blocking_reason = if status == "restored" {
        Some("already restored".to_string())
    } else if active_recovery_journal {
        Some("manual_review".to_string())
    } else {
        match status.as_str() {
            "pending" => {
                if matches!(
                    item.operation_phase.as_str(),
                    "source_claimed"
                        | "target_committed"
                        | "source_cleanup_pending"
                        | "manual_review"
                ) || item.identity_status == "pending_recovery"
                {
                    Some("manual_review".to_string())
                } else {
                    Some("pending".to_string())
                }
            }
            "missing" => Some("missing".to_string()),
            "failed" => Some("failed".to_string()),
            "manual_review" => Some("manual_review".to_string()),
            "moved" => {
                let original_exists = Path::new(&item.original_path).exists();
                let trash_exists = Path::new(&item.trash_path).exists();
                if original_exists {
                    Some("conflict".to_string())
                } else if !trash_exists {
                    Some("missing".to_string())
                } else if item.identity_status != "verified"
                    || !cleanup_has_explicit_macos_identity_provenance(&item)
                {
                    Some("manual_review".to_string())
                } else if !safe_trash_identity_matches(&item, Path::new(&item.trash_path)) {
                    Some("replacement_detected".to_string())
                } else {
                    None
                }
            }
            _ => Some("unavailable".to_string()),
        }
    };
    CleanupRestorePreviewItem {
        can_restore: blocking_reason.is_none(),
        blocking_reason,
        item,
    }
}

pub fn preview_cleanup_restore_item_for_test(item: CleanupTrashItem) -> CleanupRestorePreviewItem {
    cleanup_restore_preview_item(item)
}

pub fn is_main_window_label_for_test(label: &str) -> bool {
    is_main_window_label(label)
}

pub fn analyze_storage_roots_for_test(
    roots: Vec<PathBuf>,
    excluded_paths: Vec<PathBuf>,
) -> Result<StorageAnalysis, String> {
    analyze_storage_roots(roots, excluded_paths)
}

pub fn validate_cleanup_roots_for_test(roots: Vec<String>) -> Result<Vec<PathBuf>, String> {
    validate_cleanup_roots(roots)
}

pub fn start_storage_cleanup_scan_for_test(
    roots: Vec<PathBuf>,
    state: &StorageCleanupState,
) -> Result<String, String> {
    let job_id = new_job_id("storage-cleanup-scan-test");
    let cancel_flag = state.start_job(job_id.clone(), &roots)?;
    let state = state.clone();
    let job_id_for_task = job_id.clone();
    std::thread::spawn(move || {
        run_storage_cleanup_scan_job_without_events(
            roots,
            Vec::new(),
            state,
            job_id_for_task,
            cancel_flag,
        );
    });
    Ok(job_id)
}

pub fn start_storage_cleanup_job_state_for_test(
    state: &StorageCleanupState,
    job_id: &str,
) -> Result<(), String> {
    state.start_job(job_id.to_string(), &[])?;
    Ok(())
}

pub fn cancel_storage_cleanup_scan_for_test(
    job_id: &str,
    state: &StorageCleanupState,
) -> Result<(), String> {
    state.cancel_job(job_id)
}

pub fn get_storage_cleanup_scan_status_for_test(
    job_id: &str,
    state: &StorageCleanupState,
) -> Result<StorageCleanupScanStatus, String> {
    state.job_status(job_id)
}

pub fn get_storage_cleanup_candidate_page_for_test(
    state: &StorageCleanupState,
    job_id: &str,
    offset: usize,
    limit: usize,
) -> Result<StorageAnalysis, String> {
    state.job_analysis_page(job_id, offset, limit)
}

pub fn candidates_by_job_and_ids_for_test(
    state: &StorageCleanupState,
    job_id: &str,
    ids: &[String],
) -> Result<Vec<StorageCandidate>, String> {
    state.candidates_by_job_and_ids(job_id, ids)
}

pub fn mark_cleanup_candidates_consumed_for_test(
    state: &StorageCleanupState,
    job_id: &str,
    ids: &[String],
) -> Result<(), String> {
    state.mark_candidates_consumed(job_id, ids)
}

pub fn store_completed_cleanup_analysis_for_test(
    state: &StorageCleanupState,
    job_id: &str,
    analysis: StorageAnalysis,
) -> Result<StorageAnalysis, String> {
    state.start_job(job_id.to_string(), &[])?;
    state.complete_job(job_id, analysis)
}

pub fn classify_candidate_for_test(path: &Path, size: u64) -> StorageCandidate {
    classify_candidate(path, size)
}

pub fn default_scan_roots_for_test() -> Vec<PathBuf> {
    default_scan_roots()
}

pub fn move_cleanup_candidates_to_safe_trash_for_candidates(
    ids: Vec<String>,
    candidates: &[StorageCandidate],
    db: &Database,
    app_data_dir: Option<&Path>,
) -> Result<CleanupExecutionResult, String> {
    crate::fs_safety::platform_support::ensure_supported_cleanup_mutation()
        .map_err(|error| error.to_string())?;
    let by_id = candidates
        .iter()
        .map(|candidate| (candidate.id.as_str(), candidate))
        .collect::<HashMap<_, _>>();
    let batch_id = new_job_id("cleanup-safe-trash");
    let moved_at = current_timestamp_ms().to_string();
    let mut result = CleanupExecutionResult {
        moved: 0,
        skipped: 0,
        failed: 0,
        logs: Vec::new(),
    };
    let mut items = Vec::new();

    for (index, id) in ids.into_iter().enumerate() {
        let Some(candidate) = by_id.get(id.as_str()) else {
            result.skipped += 1;
            result.logs.push(CleanupExecutionLog {
                path: String::new(),
                name: id,
                size: 0,
                status: "skipped".to_string(),
                message: "Candidate id was not found in the latest storage cleanup scan."
                    .to_string(),
                item_id: None,
                trash_path: None,
            });
            continue;
        };

        let source = Path::new(&candidate.path);
        if !cleanup_candidate_can_enter_operation_preview(candidate, app_data_dir) {
            result.skipped += 1;
            result.logs.push(CleanupExecutionLog {
                path: candidate.path.clone(),
                name: candidate.name.clone(),
                size: candidate.size,
                status: "skipped".to_string(),
                message:
                    "Only safe candidates from the latest scan can be moved to Fylura Safe Trash."
                        .to_string(),
                item_id: None,
                trash_path: None,
            });
            continue;
        }

        let item_id = candidate_item_id(candidate, index);
        let trash_path =
            safe_trash_item_path(source, &batch_id, &item_id, &candidate.name, app_data_dir);
        let trash_path_text = normalize_path(&trash_path);
        if trash_path.exists() {
            result.failed += 1;
            result.logs.push(CleanupExecutionLog {
                path: candidate.path.clone(),
                name: candidate.name.clone(),
                size: candidate.size,
                status: "failed".to_string(),
                message: "Safe trash destination already exists.".to_string(),
                item_id: Some(item_id),
                trash_path: Some(trash_path_text),
            });
            continue;
        }

        #[cfg(target_os = "macos")]
        if let Some(source_parent) = source.parent() {
            if let Err(code) =
                crate::platform::macos::mutation::ensure_path_eligible(source, source_parent)
            {
                result.failed += 1;
                result.logs.push(CleanupExecutionLog {
                    path: candidate.path.clone(),
                    name: candidate.name.clone(),
                    size: candidate.size,
                    status: "failed".to_string(),
                    message: format!("Safe Trash source is not eligible: {code}"),
                    item_id: Some(item_id),
                    trash_path: Some(trash_path_text),
                });
                continue;
            }
        }
        let fingerprint = match safe_trash_operation_fingerprint(source, None, None) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                result.failed += 1;
                result.logs.push(CleanupExecutionLog {
                    path: candidate.path.clone(),
                    name: candidate.name.clone(),
                    size: candidate.size,
                    status: "failed".to_string(),
                    message: format!("Cannot journal Safe Trash source identity: {error}"),
                    item_id: Some(item_id),
                    trash_path: Some(trash_path_text),
                });
                continue;
            }
        };
        let item = CleanupTrashItem {
            id: item_id.clone(),
            batch_id: batch_id.clone(),
            original_path: candidate.path.clone(),
            trash_path: trash_path_text.clone(),
            name: candidate.name.clone(),
            size: fingerprint.size,
            moved_at: moved_at.clone(),
            restored_at: None,
            status: "pending".to_string(),
            message: Some("Pending move to Fylura Safe Trash.".to_string()),
            source_modified_ns: fingerprint.modified_ns.map(|value| value.to_string()),
            source_platform_volume_id: if cfg!(target_os = "macos") {
                fingerprint.platform_volume_id.clone()
            } else {
                None
            },
            source_platform_file_id: fingerprint.platform_file_id.clone(),
            source_quick_hash: fingerprint.quick_hash,
            source_full_hash: fingerprint.full_hash.clone(),
            trash_modified_ns: None,
            trash_platform_volume_id: None,
            trash_platform_file_id: None,
            trash_quick_hash: None,
            trash_full_hash: None,
            identity_status: "pending".to_string(),
            source_claim_path: Some(
                crate::fs_safety::source_claim::planned_claim_path(source, &item_id)
                    .map_err(|error| error.to_string())?
                    .to_string_lossy()
                    .into_owned(),
            ),
            operation_phase: "prepared".to_string(),
            claim_created_at: Some(moved_at.clone()),
            claim_platform_file_id: fingerprint.platform_file_id.clone(),
            claim_full_hash: fingerprint.full_hash.clone(),
        };
        items.push(item);
    }

    if !items.is_empty() {
        let root = candidates
            .first()
            .and_then(|candidate| Path::new(&candidate.path).parent().map(normalize_path));
        db.save_cleanup_trash_batch(&CleanupTrashBatch {
            id: batch_id.clone(),
            created_at: moved_at.clone(),
            root: root.clone(),
            total_items: items.len(),
            total_size: items.iter().map(|item| item.size).sum(),
            status: "pending".to_string(),
            items: items.clone(),
        })
        .map_err(|error| {
            format!("failed to persist safe-trash journal before moving files: {error}")
        })?;

        for item in &mut items {
            let move_result = {
                let original_path = item.original_path.clone();
                let trash_path = item.trash_path.clone();
                let expected_identity = SafeTrashExpectedIdentity {
                    size: item.size,
                    modified_ns: item.source_modified_ns.clone(),
                    quick_hash: item.source_quick_hash.clone(),
                    full_hash: item.source_full_hash.clone(),
                    platform_volume_id: item.source_platform_volume_id.clone(),
                    platform_file_id: item.source_platform_file_id.clone(),
                };
                let claim_path = item.source_claim_path.clone();
                let actual_paths = std::cell::RefCell::new(None::<(PathBuf, PathBuf)>);
                let mut phase_observer = |phase: &str| {
                    if let Some((actual_source, actual_target)) = actual_paths.borrow().clone() {
                        item.original_path = normalize_path(&actual_source);
                        item.trash_path = normalize_path(&actual_target);
                    }
                    item.operation_phase = phase.to_string();
                    item.status = "pending".to_string();
                    item.message = Some(format!("Safe Trash operation phase: {phase}."));
                    db.update_cleanup_trash_item_status(item).map_err(|error| {
                        if matches!(
                            phase,
                            "target_committed" | "source_cleanup_pending" | "completed"
                        ) {
                            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown
                        } else {
                            crate::fs_safety::AtomicMoveError::SourceClaimRecoveryRequired(format!(
                                "safe-trash journal phase persistence failed: {error}"
                            ))
                        }
                    })?;
                    Ok(())
                };
                let mut actual_path_observer =
                    |actual_source: &Path, actual_target: &Path, _actual_claim: Option<&Path>| {
                        actual_paths.replace(Some((
                            actual_source.to_path_buf(),
                            actual_target.to_path_buf(),
                        )));
                        Ok(())
                    };
                let result = move_path_to_safe_trash(
                    Path::new(&original_path),
                    Path::new(&trash_path),
                    expected_identity,
                    claim_path.as_deref().map(Path::new),
                    Some(&mut phase_observer),
                    Some(&mut actual_path_observer),
                );
                if let Some((actual_source, actual_target)) = actual_paths.into_inner() {
                    item.original_path = normalize_path(&actual_source);
                    item.trash_path = normalize_path(&actual_target);
                }
                result
            };
            let (status, log_status, message) = match move_result {
                Ok(()) => {
                    match safe_trash_operation_fingerprint(
                        Path::new(&item.trash_path),
                        item.source_full_hash.as_deref(),
                        item.source_quick_hash.as_deref(),
                    ) {
                        Ok(trash_fingerprint)
                            if trash_fingerprint.size == item.size
                                && trash_fingerprint.quick_hash == item.source_quick_hash
                                && trash_fingerprint.full_hash == item.source_full_hash =>
                        {
                            item.trash_modified_ns =
                                trash_fingerprint.modified_ns.map(|value| value.to_string());
                            item.trash_platform_volume_id = trash_fingerprint.platform_volume_id;
                            item.trash_platform_file_id = trash_fingerprint.platform_file_id;
                            item.trash_quick_hash = trash_fingerprint.quick_hash;
                            item.trash_full_hash = trash_fingerprint.full_hash;
                            item.identity_status = "verified".to_string();
                            result.moved += 1;
                            (
                                "moved",
                                "success",
                                "Moved to Fylura Safe Trash and verified file identity."
                                    .to_string(),
                            )
                        }
                        Ok(trash_fingerprint) => {
                            item.trash_modified_ns =
                                trash_fingerprint.modified_ns.map(|value| value.to_string());
                            item.trash_platform_volume_id = trash_fingerprint.platform_volume_id;
                            item.trash_platform_file_id = trash_fingerprint.platform_file_id;
                            item.trash_quick_hash = trash_fingerprint.quick_hash;
                            item.trash_full_hash = trash_fingerprint.full_hash;
                            item.identity_status = "mismatch".to_string();
                            result.failed += 1;
                            item.operation_phase = "target_committed".to_string();
                            (
                            "manual_review",
                            "manual_review",
                            "target_committed_identity_mismatch: Safe Trash destination identity did not match the journal; the target may have committed and manual review is required."
                                .to_string(),
                        )
                        }
                        Err(error) => {
                            item.identity_status = "unverifiable".to_string();
                            result.failed += 1;
                            item.operation_phase = "target_committed".to_string();
                            (
                            "manual_review",
                            "manual_review",
                            format!(
                                "target_committed_identity_mismatch: Safe Trash destination identity could not be verified; the target may have committed and manual review is required: {error}"
                            ),
                        )
                        }
                    }
                }
                Err(error) => {
                    let phase = error.journal_phase();
                    let requires_recovery = error.requires_recovery();
                    item.operation_phase = phase.to_string();
                    item.identity_status = if requires_recovery {
                        "pending_recovery".to_string()
                    } else {
                        "move_failed".to_string()
                    };
                    result.failed += 1;
                    (
                        if requires_recovery {
                            "manual_review"
                        } else {
                            "failed"
                        },
                        if requires_recovery {
                            "manual_review"
                        } else {
                            "failed"
                        },
                        safe_trash_error_message(&error, false),
                    )
                }
            };
            if status == "moved" {
                item.operation_phase = "completed".to_string();
            }
            item.status = status.to_string();
            item.message = Some(message.clone());
            db.update_cleanup_trash_item_status(item).map_err(|error| {
                format!("safe-trash item moved but journal update failed: {error}")
            })?;
            result.logs.push(CleanupExecutionLog {
                path: item.original_path.clone(),
                name: item.name.clone(),
                size: item.size,
                status: log_status.to_string(),
                message,
                item_id: Some(item.id.clone()),
                trash_path: Some(item.trash_path.clone()),
            });
        }

        let status = if result.failed > 0 {
            "partial_failed"
        } else {
            "success"
        };
        db.save_cleanup_trash_batch(&CleanupTrashBatch {
            id: batch_id,
            created_at: moved_at,
            root,
            total_items: items.len(),
            total_size: items.iter().map(|item| item.size).sum(),
            status: status.to_string(),
            items,
        })
        .map_err(|error| error.to_string())?;
    }

    Ok(result)
}

pub fn restore_cleanup_trash_items_for_db(
    item_ids: Vec<String>,
    db: &Database,
) -> Result<CleanupRestoreResult, String> {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let emitter = NoopCleanupRestoreProgressEmitter;
    restore_cleanup_trash_items_for_db_with_progress(
        item_ids,
        db,
        cancel_flag,
        new_job_id("cleanup-restore-test"),
        &emitter,
    )
}

pub fn restore_cleanup_trash_items_for_db_with_cancel_for_test(
    item_ids: Vec<String>,
    db: &Database,
    cancel_flag: Arc<AtomicBool>,
) -> Result<CleanupRestoreResult, String> {
    let emitter = NoopCleanupRestoreProgressEmitter;
    restore_cleanup_trash_items_for_db_with_progress(
        item_ids,
        db,
        cancel_flag,
        new_job_id("cleanup-restore-test"),
        &emitter,
    )
}

fn restore_cleanup_trash_items_for_db_with_progress(
    item_ids: Vec<String>,
    db: &Database,
    cancel_flag: Arc<AtomicBool>,
    job_id: String,
    emitter: &impl CleanupRestoreProgressEmitter,
) -> Result<CleanupRestoreResult, String> {
    crate::fs_safety::platform_support::ensure_supported_cleanup_mutation()
        .map_err(|error| error.to_string())?;
    let total = item_ids.len();
    let mut result = CleanupRestoreResult {
        restored: 0,
        conflicts: 0,
        missing: 0,
        failed: 0,
        canceled: 0,
        logs: Vec::new(),
    };
    let progress_context = CleanupRestoreProgressContext {
        job_id: &job_id,
        total,
        cancel_flag: &cancel_flag,
    };

    for item_id in item_ids {
        let mut current_path = None;
        if cancel_flag.load(Ordering::Relaxed) {
            result.canceled += 1;
            result.logs.push(CleanupRestoreLog {
                item_id: item_id.clone(),
                original_path: String::new(),
                trash_path: String::new(),
                status: "canceled".to_string(),
                message: "Cleanup restore canceled.".to_string(),
            });
            emit_cleanup_restore_progress(
                emitter,
                &progress_context,
                result.logs.len(),
                Some(item_id),
                current_path,
                &result,
            );
            continue;
        }

        let Some(mut item) = db
            .cleanup_trash_item(&item_id)
            .map_err(|error| error.to_string())?
        else {
            result.failed += 1;
            result.logs.push(CleanupRestoreLog {
                item_id: item_id.clone(),
                original_path: String::new(),
                trash_path: String::new(),
                status: "failed".to_string(),
                message: "Cleanup trash item was not found.".to_string(),
            });
            emit_cleanup_restore_progress(
                emitter,
                &progress_context,
                result.logs.len(),
                Some(item_id),
                current_path,
                &result,
            );
            continue;
        };

        let original = PathBuf::from(&item.original_path);
        let trash_path = PathBuf::from(&item.trash_path);
        current_path = Some(normalize_path(&trash_path));
        if cancel_flag.load(Ordering::Relaxed) {
            item.status = "canceled".to_string();
            item.operation_phase = "rolled_back".to_string();
            item.identity_status = "canceled".to_string();
            item.message = Some("Cleanup restore canceled.".to_string());
            item.source_claim_path = None;
            item.claim_created_at = None;
            item.claim_platform_file_id = None;
            item.claim_full_hash = None;
            db.finalize_cleanup_restore_outcome(&item)
                .map_err(|error| error.to_string())?;
            result.canceled += 1;
            result.logs.push(CleanupRestoreLog {
                item_id: item.id.clone(),
                original_path: normalize_path(&original),
                trash_path: normalize_path(&trash_path),
                status: "canceled".to_string(),
                message: "Cleanup restore canceled.".to_string(),
            });
            emit_cleanup_restore_progress(
                emitter,
                &progress_context,
                result.logs.len(),
                Some(item_id),
                current_path,
                &result,
            );
            continue;
        }
        let restore_recovery_pending = matches!(
            item.identity_status.as_str(),
            "restore_pending" | "restore_pending_recovery"
        );
        let active_restore_phase = matches!(
            item.operation_phase.as_str(),
            "source_claimed"
                | "copying"
                | "target_committed"
                | "source_cleanup_pending"
                | "manual_review"
        );
        if restore_recovery_pending || active_restore_phase {
            item.status = "manual_review".to_string();
            if item.operation_phase == "completed" {
                item.operation_phase = "manual_review".to_string();
            }
            if item.operation_phase != "manual_review"
                && !matches!(
                    item.identity_status.as_str(),
                    "restore_pending" | "restore_pending_recovery"
                )
            {
                item.identity_status = "restore_pending_recovery".to_string();
            }
            item.message = Some(
                "restore_pending_reconciliation: Safe Trash restore has an active journal phase or source claim; reconcile it before retrying and do not auto retry."
                    .to_string(),
            );
            db.finalize_cleanup_restore_outcome(&item)
                .map_err(|error| error.to_string())?;
            result.failed += 1;
            result.logs.push(CleanupRestoreLog {
                item_id: item.id.clone(),
                original_path: normalize_path(&original),
                trash_path: normalize_path(&trash_path),
                status: "manual_review".to_string(),
                message: item.message.clone().unwrap_or_default(),
            });
            emit_cleanup_restore_progress(
                emitter,
                &progress_context,
                result.logs.len(),
                Some(item_id),
                current_path,
                &result,
            );
            continue;
        }
        if item.status != "moved" && !restore_recovery_pending {
            item.status = "failed".to_string();
            item.operation_phase = "rolled_back".to_string();
            item.identity_status = "restore_failed".to_string();
            item.message = Some("Cleanup trash item is no longer restorable.".to_string());
            db.finalize_cleanup_restore_outcome(&item)
                .map_err(|error| error.to_string())?;
            result.failed += 1;
            result.logs.push(CleanupRestoreLog {
                item_id: item.id.clone(),
                original_path: normalize_path(&original),
                trash_path: normalize_path(&trash_path),
                status: "failed".to_string(),
                message: "Cleanup trash item is no longer restorable.".to_string(),
            });
            emit_cleanup_restore_progress(
                emitter,
                &progress_context,
                result.logs.len(),
                Some(item_id),
                current_path,
                &result,
            );
            continue;
        }
        if !trash_path.exists() {
            item.status = "missing".to_string();
            item.operation_phase = "rolled_back".to_string();
            item.identity_status = "missing".to_string();
            item.message = Some("Safe trash path is missing.".to_string());
            db.finalize_cleanup_restore_outcome(&item)
                .map_err(|error| error.to_string())?;
            result.missing += 1;
            result.logs.push(CleanupRestoreLog {
                item_id: item.id.clone(),
                original_path: normalize_path(&original),
                trash_path: normalize_path(&trash_path),
                status: "missing".to_string(),
                message: "Safe trash path is missing.".to_string(),
            });
            emit_cleanup_restore_progress(
                emitter,
                &progress_context,
                result.logs.len(),
                Some(item_id),
                current_path,
                &result,
            );
            continue;
        }
        let trash_is_dir = match safe_trash_restore_source_identity_check(&item, &trash_path) {
            Ok(is_dir) => is_dir,
            Err(identity_error) => {
                let identity_was_verified = item.identity_status == "verified";
                item.status = "manual_review".to_string();
                item.operation_phase = "manual_review".to_string();
                item.identity_status = if matches!(
                    identity_error.code,
                    crate::recovery::RecoveryErrorCode::ClaimIdentityUnreadable
                        | crate::recovery::RecoveryErrorCode::RestoreSourceIdentityUnreadable
                ) {
                    "unverifiable"
                } else {
                    "mismatch"
                }
                .to_string();
                item.message = Some(if identity_was_verified {
                    identity_error.message()
                } else {
                    "manual_review_required: Legacy Safe Trash item has no identity fingerprint; automatic restore is blocked for manual review."
                    .to_string()
                });
                db.finalize_cleanup_restore_outcome(&item)
                    .map_err(|error| error.to_string())?;
                result.failed += 1;
                result.logs.push(CleanupRestoreLog {
                    item_id: item.id.clone(),
                    original_path: normalize_path(&original),
                    trash_path: normalize_path(&trash_path),
                    status: "manual_review".to_string(),
                    message: item.message.unwrap_or_default(),
                });
                continue;
            }
        };
        if original.exists() {
            item.status = "manual_review".to_string();
            item.operation_phase = "manual_review".to_string();
            item.identity_status = "restore_pending_recovery".to_string();
            item.message = Some(crate::recovery::format_recovery_message(
                crate::recovery::RecoveryErrorCode::ManualReviewRequired,
                "Restore is blocked because the original path already exists",
            ));
            db.finalize_cleanup_restore_outcome(&item)
                .map_err(|error| error.to_string())?;
            result.conflicts += 1;
            result.logs.push(CleanupRestoreLog {
                item_id: item.id.clone(),
                original_path: normalize_path(&original),
                trash_path: normalize_path(&trash_path),
                status: "conflict".to_string(),
                message: "Restore is blocked because the original path already exists.".to_string(),
            });
            emit_cleanup_restore_progress(
                emitter,
                &progress_context,
                result.logs.len(),
                Some(item_id),
                current_path,
                &result,
            );
            continue;
        }

        let restore_claim_path =
            match crate::fs_safety::source_claim::planned_claim_path(&trash_path, &item.id) {
                Ok(path) => path,
                Err(error) => {
                    item.status = "failed".to_string();
                    item.operation_phase = "rolled_back".to_string();
                    item.identity_status = "restore_failed".to_string();
                    item.message = Some(error.to_string());
                    db.finalize_cleanup_restore_outcome(&item)
                        .map_err(|error| error.to_string())?;
                    result.failed += 1;
                    result.logs.push(CleanupRestoreLog {
                        item_id: item.id.clone(),
                        original_path: normalize_path(&original),
                        trash_path: normalize_path(&trash_path),
                        status: "failed".to_string(),
                        message: item.message.unwrap_or_default(),
                    });
                    continue;
                }
            };
        item.source_claim_path = Some(normalize_path(&restore_claim_path));
        item.claim_created_at = Some(current_timestamp_ms().to_string());
        item.claim_platform_file_id = item.trash_platform_file_id.clone();
        item.claim_full_hash = item.trash_full_hash.clone();
        item.status = "pending".to_string();
        item.identity_status = "restore_pending".to_string();
        item.operation_phase = "prepared".to_string();
        item.message =
            Some("Safe Trash restore is journaled before filesystem mutation.".to_string());
        db.finalize_cleanup_restore_outcome(&item)
            .map_err(|error| error.to_string())?;

        let restore_result = {
            let expected_identity = SafeTrashExpectedIdentity {
                size: item.size,
                modified_ns: item.trash_modified_ns.clone(),
                platform_volume_id: item.trash_platform_volume_id.clone(),
                platform_file_id: item.trash_platform_file_id.clone(),
                quick_hash: item.trash_quick_hash.clone(),
                full_hash: item.trash_full_hash.clone(),
            };
            let mut phase_observer = |phase: &str| {
                item.operation_phase = phase.to_string();
                item.status = "pending".to_string();
                item.identity_status = "restore_pending".to_string();
                item.message = Some(format!("Safe Trash restore phase: {phase}."));
                db.finalize_cleanup_restore_outcome(&item)
                    .map_err(|error| {
                        if matches!(
                            phase,
                            "target_committed" | "source_cleanup_pending" | "completed"
                        ) {
                            crate::fs_safety::AtomicMoveError::TargetCommittedDurabilityUnknown
                        } else {
                            crate::fs_safety::AtomicMoveError::SourceClaimRecoveryRequired(format!(
                                "safe-trash restore phase persistence failed: {error}"
                            ))
                        }
                    })?;
                Ok(())
            };
            move_path_to_restore_location(
                &trash_path,
                &original,
                expected_identity,
                Some(&restore_claim_path),
                Some(&mut phase_observer),
            )
        };
        match restore_result {
            Ok(()) => {
                let restored_identity =
                    safe_trash_restore_identity_check(&item, &original, trash_is_dir);
                if restored_identity.is_ok() {
                    let recovery_phase = if item.operation_phase == "source_cleanup_pending" {
                        "source_cleanup_pending"
                    } else {
                        "target_committed"
                    };
                    item.status = "restored".to_string();
                    item.restored_at = Some(current_timestamp_ms().to_string());
                    item.operation_phase = "completed".to_string();
                    item.identity_status = "verified".to_string();
                    item.message = Some("Restored from Fylura Safe Trash.".to_string());
                    match db.finalize_successful_cleanup_restore(&item) {
                        Ok(()) => {
                            item.source_claim_path = None;
                            item.claim_created_at = None;
                            item.claim_platform_file_id = None;
                            item.claim_full_hash = None;
                            result.restored += 1;
                            result.logs.push(CleanupRestoreLog {
                                item_id: item.id.clone(),
                                original_path: normalize_path(&original),
                                trash_path: normalize_path(&trash_path),
                                status: "restored".to_string(),
                                message: "Restored from Fylura Safe Trash.".to_string(),
                            });
                        }
                        Err(error) => {
                            item.status = "manual_review".to_string();
                            item.restored_at = None;
                            item.operation_phase = recovery_phase.to_string();
                            item.identity_status = "restore_pending_recovery".to_string();
                            item.message = Some(format!(
                                "target_committed_durability_unknown: Safe Trash restore filesystem commit succeeded but final transaction failed: {error}; do not auto retry."
                            ));
                            db.finalize_cleanup_restore_outcome(&item)
                                .map_err(|persist_error| persist_error.to_string())?;
                            result.failed += 1;
                            result.logs.push(CleanupRestoreLog {
                                item_id: item.id.clone(),
                                original_path: normalize_path(&original),
                                trash_path: normalize_path(&trash_path),
                                status: "manual_review".to_string(),
                                message: item.message.clone().unwrap_or_default(),
                            });
                        }
                    }
                } else {
                    let identity_error = restored_identity
                        .err()
                        .map(|error| error.message())
                        .unwrap_or_else(|| {
                            crate::recovery::RecoveryFailure::new(
                                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                                "restored target identity could not be verified",
                            )
                            .message()
                        });
                    item.status = "manual_review".to_string();
                    item.operation_phase = "target_committed".to_string();
                    item.identity_status = "restore_pending_recovery".to_string();
                    item.message = Some(identity_error);
                    db.finalize_cleanup_restore_outcome(&item)
                        .map_err(|error| error.to_string())?;
                    result.failed += 1;
                    result.logs.push(CleanupRestoreLog {
                        item_id: item.id.clone(),
                        original_path: normalize_path(&original),
                        trash_path: normalize_path(&trash_path),
                        status: "manual_review".to_string(),
                        message: item.message.clone().unwrap_or_default(),
                    });
                }
            }
            Err(error) => {
                let recovery_required = safe_trash_error_message(&error, true);
                let canceled = error.is_cancelled();
                let phase = if canceled {
                    "rolled_back"
                } else if error.requires_recovery() {
                    error.journal_phase()
                } else {
                    "rolled_back"
                };
                let is_recovery = error.requires_recovery();
                item.status = if canceled {
                    "canceled".to_string()
                } else if is_recovery {
                    "manual_review".to_string()
                } else {
                    "failed".to_string()
                };
                item.operation_phase = phase.to_string();
                item.identity_status = if canceled {
                    "canceled".to_string()
                } else if is_recovery {
                    "restore_pending_recovery".to_string()
                } else {
                    "restore_failed".to_string()
                };
                item.message = Some(recovery_required.clone());
                db.finalize_cleanup_restore_outcome(&item)
                    .map_err(|error| error.to_string())?;
                if canceled {
                    result.canceled += 1;
                } else {
                    result.failed += 1;
                }
                result.logs.push(CleanupRestoreLog {
                    item_id: item.id.clone(),
                    original_path: normalize_path(&original),
                    trash_path: normalize_path(&trash_path),
                    status: if canceled {
                        "canceled".to_string()
                    } else if is_recovery {
                        "manual_review".to_string()
                    } else {
                        "failed".to_string()
                    },
                    message: recovery_required,
                });
            }
        }
        emit_cleanup_restore_progress(
            emitter,
            &progress_context,
            result.logs.len(),
            Some(item_id),
            current_path,
            &result,
        );
    }

    Ok(result)
}

struct CleanupRestoreProgressContext<'a> {
    job_id: &'a str,
    total: usize,
    cancel_flag: &'a AtomicBool,
}

fn emit_cleanup_restore_progress(
    emitter: &impl CleanupRestoreProgressEmitter,
    context: &CleanupRestoreProgressContext<'_>,
    processed: usize,
    current_item_id: Option<String>,
    current_path: Option<String>,
    result: &CleanupRestoreResult,
) {
    emitter.emit_progress(CleanupRestoreProgressPayload {
        job_id: context.job_id.to_string(),
        processed,
        total: context.total,
        current_item_id,
        current_path,
        restored: result.restored,
        conflicts: result.conflicts,
        missing: result.missing,
        failed: result.failed,
        canceled: result.canceled,
        cancel_requested: context.cancel_flag.load(Ordering::Relaxed),
    });
}

pub fn reconcile_pending_cleanup_journal(db: &Database) -> Result<usize, String> {
    let mut items = db
        .pending_cleanup_trash_items()
        .map_err(|error| error.to_string())?;
    for item in &mut items {
        let original = Path::new(&item.original_path);
        let trash = Path::new(&item.trash_path);
        let claim = item.source_claim_path.as_deref().map(Path::new);
        let original_state = cleanup_journal_path_state(original, |path| {
            pending_safe_trash_source_identity_matches(item, path)
        });
        let trash_state = cleanup_journal_path_state(trash, |path| {
            pending_safe_trash_target_identity_matches(item, path)
        });
        let claim_state = claim.map_or(JournalPathState::Missing, |path| {
            cleanup_journal_path_state(path, |candidate| {
                pending_safe_trash_claim_identity_matches(item, candidate)
            })
        });

        if matches!(
            item.identity_status.as_str(),
            "restore_pending" | "restore_pending_recovery"
        ) {
            let previous_phase = item.operation_phase.clone();
            let decision = classify_cleanup_restore_reconciliation(
                original_state,
                trash_state,
                claim_state,
                &previous_phase,
                &item.status,
                &item.identity_status,
            );
            item.status = decision.status.to_string();
            item.operation_phase = decision.operation_phase.to_string();
            item.identity_status = decision.identity_status.to_string();
            item.restored_at = decision
                .restored
                .then(|| current_timestamp_ms().to_string());
            item.message = Some(decision.message.to_string());

            if decision.restored {
                match db.finalize_successful_cleanup_restore(item) {
                    Ok(()) => {
                        item.source_claim_path = None;
                        item.claim_created_at = None;
                        item.claim_platform_file_id = None;
                        item.claim_full_hash = None;
                    }
                    Err(error) => {
                        item.status = "manual_review".to_string();
                        item.restored_at = None;
                        item.operation_phase = if previous_phase == "source_cleanup_pending" {
                            "source_cleanup_pending"
                        } else {
                            "target_committed"
                        }
                        .to_string();
                        item.identity_status = "restore_pending_recovery".to_string();
                        item.message = Some(format!(
                            "target_committed_durability_unknown: Safe Trash restore final transaction failed: {error}; do not auto retry."
                        ));
                        db.finalize_cleanup_restore_outcome(item)
                            .map_err(|persist_error| persist_error.to_string())?;
                    }
                }
            } else {
                if decision.clear_claim {
                    item.source_claim_path = None;
                    item.claim_created_at = None;
                    item.claim_platform_file_id = None;
                    item.claim_full_hash = None;
                }
                db.finalize_cleanup_restore_outcome(item)
                    .map_err(|error| error.to_string())?;
            }
        } else {
            match (original_state, trash_state, claim_state) {
                (
                    JournalPathState::Missing,
                    JournalPathState::Matches,
                    JournalPathState::Missing,
                ) if matches!(
                    item.operation_phase.as_str(),
                    "target_committed" | "source_cleanup_pending"
                ) || (item.operation_phase == "manual_review"
                    && item.identity_status == "pending_recovery") =>
                {
                    item.status = "manual_review".to_string();
                    item.operation_phase = match item.operation_phase.as_str() {
                        "source_cleanup_pending" => "source_cleanup_pending",
                        "target_committed" => "target_committed",
                        _ => "manual_review",
                    }
                    .to_string();
                    item.identity_status = "pending_recovery".to_string();
                    item.message = Some(
                        "target_committed_durability_unknown: Safe Trash target may have committed; verify the target before retrying."
                            .to_string(),
                    );
                }
                (
                    JournalPathState::Missing,
                    JournalPathState::Matches,
                    JournalPathState::Missing,
                ) if item.status != "manual_review"
                    && item.operation_phase != "manual_review"
                    && item.identity_status != "mismatch" =>
                {
                    if let Ok(trash_fingerprint) = crate::file_ops::file_identity_fingerprint(trash)
                    {
                        item.trash_modified_ns =
                            trash_fingerprint.modified_ns.map(|value| value.to_string());
                        item.trash_platform_volume_id = trash_fingerprint.platform_volume_id;
                        item.trash_platform_file_id = trash_fingerprint.platform_file_id;
                        item.trash_quick_hash = trash_fingerprint.quick_hash;
                        item.trash_full_hash = trash_fingerprint.full_hash;
                        item.identity_status = "verified".to_string();
                        item.status = "moved".to_string();
                        item.operation_phase = "completed".to_string();
                        item.message = Some(
                            "Recovered an interrupted Safe Trash journal after restart."
                                .to_string(),
                        );
                    } else {
                        item.status = "pending".to_string();
                        item.operation_phase = "manual_review".to_string();
                        item.identity_status = "unverifiable".to_string();
                        item.message = Some(
                            "Interrupted Safe Trash destination identity is unverifiable; manual review is required."
                                .to_string(),
                        );
                    }
                }
                (
                    JournalPathState::Missing,
                    JournalPathState::Missing,
                    JournalPathState::Matches,
                ) => {
                    item.status = "manual_review".to_string();
                    item.operation_phase = "source_claimed".to_string();
                    item.identity_status = "pending_recovery".to_string();
                    item.message = Some(
                        "manual_review_required: Safe Trash source claim was recovered without a committed target; manual review is required."
                            .to_string(),
                    );
                }
                (
                    JournalPathState::Missing,
                    JournalPathState::Matches,
                    JournalPathState::Matches,
                ) => {
                    item.status = "manual_review".to_string();
                    item.operation_phase = "source_cleanup_pending".to_string();
                    item.identity_status = "pending_recovery".to_string();
                    item.message = Some(
                        "target_committed_source_cleanup_pending: Safe Trash target and source claim both exist; source cleanup requires manual review."
                            .to_string(),
                    );
                }
                (
                    JournalPathState::Matches,
                    JournalPathState::Missing,
                    JournalPathState::Missing,
                ) => {
                    item.status = "failed".to_string();
                    item.operation_phase = "rolled_back".to_string();
                    item.identity_status = "rolled_back".to_string();
                    item.message = Some(
                        "Safe Trash move was interrupted before the filesystem change; the source is intact."
                            .to_string(),
                    );
                }
                _ => {
                    let preserve_pending_recovery = item.status == "manual_review"
                        && item.identity_status == "pending_recovery";
                    item.status = "manual_review".to_string();
                    item.operation_phase = match item.operation_phase.as_str() {
                        "target_committed" => "target_committed",
                        "source_cleanup_pending" => "source_cleanup_pending",
                        _ => "manual_review",
                    }
                    .to_string();
                    item.identity_status = if preserve_pending_recovery {
                        "pending_recovery"
                    } else {
                        "mismatch"
                    }
                    .to_string();
                    item.message = Some(
                        "manual_review_required: interrupted Safe Trash move found a missing, inaccessible, or mismatched identity/path; manual review is required."
                            .to_string(),
                    );
                }
            }
        }
        db.update_cleanup_trash_item_status(item)
            .map_err(|error| error.to_string())?;
    }
    if !items.is_empty() {
        db.recompute_cleanup_batch_statuses()
            .map_err(|error| error.to_string())?;
    }
    Ok(items.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JournalPathState {
    Missing,
    Matches,
    Mismatch,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CleanupRestoreReconciliationDecision {
    status: &'static str,
    operation_phase: &'static str,
    identity_status: &'static str,
    message: &'static str,
    clear_claim: bool,
    restored: bool,
}

fn classify_cleanup_restore_reconciliation(
    original_state: JournalPathState,
    trash_state: JournalPathState,
    claim_state: JournalPathState,
    current_phase: &str,
    _current_status: &str,
    _current_identity_status: &str,
) -> CleanupRestoreReconciliationDecision {
    if matches!(
        claim_state,
        JournalPathState::Mismatch | JournalPathState::Unreadable
    ) {
        let message = if claim_state == JournalPathState::Unreadable {
            "claim_identity_unreadable: Safe Trash restore Claim identity could not be read; do not auto retry."
        } else {
            "claim_identity_mismatch: Safe Trash restore Claim identity does not match the journal; do not auto retry."
        };
        return CleanupRestoreReconciliationDecision {
            status: "manual_review",
            operation_phase: cleanup_restore_recovery_phase(
                original_state,
                trash_state,
                current_phase,
            ),
            identity_status: "restore_pending_recovery",
            message,
            clear_claim: false,
            restored: false,
        };
    }

    if matches!(
        original_state,
        JournalPathState::Mismatch | JournalPathState::Unreadable
    ) || matches!(
        trash_state,
        JournalPathState::Mismatch | JournalPathState::Unreadable
    ) {
        let unreadable = original_state == JournalPathState::Unreadable
            || trash_state == JournalPathState::Unreadable;
        return CleanupRestoreReconciliationDecision {
            status: "manual_review",
            operation_phase: cleanup_restore_recovery_phase(
                original_state,
                trash_state,
                current_phase,
            ),
            identity_status: "restore_pending_recovery",
            message: if unreadable {
                "target_committed_identity_unreadable: Safe Trash restore target or source identity could not be read; do not auto retry."
            } else {
                "target_committed_identity_mismatch: Safe Trash restore target or source identity does not match the journal; do not auto retry."
            },
            clear_claim: false,
            restored: false,
        };
    }

    match (original_state, trash_state, claim_state) {
        (JournalPathState::Matches, JournalPathState::Missing, JournalPathState::Missing) => {
            CleanupRestoreReconciliationDecision {
                status: "restored",
                operation_phase: "completed",
                identity_status: "verified",
                message: "Recovered an interrupted Safe Trash restore after restart.",
                clear_claim: true,
                restored: true,
            }
        }
        (JournalPathState::Missing, JournalPathState::Matches, JournalPathState::Missing) => {
            CleanupRestoreReconciliationDecision {
                status: "moved",
                operation_phase: "rolled_back",
                identity_status: "verified",
                message: "Safe Trash restore was interrupted before filesystem commit; it remains available and will not be auto-retried.",
                clear_claim: true,
                restored: false,
            }
        }
        (JournalPathState::Missing, JournalPathState::Missing, JournalPathState::Matches) => {
            CleanupRestoreReconciliationDecision {
                status: "manual_review",
                operation_phase: "source_claimed",
                identity_status: "restore_pending_recovery",
                message: "restore_pending_reconciliation: Safe Trash restore source Claim was recovered without a committed target; do not auto retry.",
                clear_claim: false,
                restored: false,
            }
        }
        (JournalPathState::Missing, JournalPathState::Matches, JournalPathState::Matches) => {
            CleanupRestoreReconciliationDecision {
                status: "manual_review",
                operation_phase: "source_cleanup_pending",
                identity_status: "restore_pending_recovery",
                message: "target_committed_source_cleanup_pending: Safe Trash restore target and source Claim both exist; source cleanup requires manual review.",
                clear_claim: false,
                restored: false,
            }
        }
        (JournalPathState::Matches, JournalPathState::Missing, JournalPathState::Matches) => {
            CleanupRestoreReconciliationDecision {
                status: "manual_review",
                operation_phase: "source_cleanup_pending",
                identity_status: "restore_pending_recovery",
                message: "target_committed_source_cleanup_pending: Safe Trash restore target and source Claim both exist; source cleanup requires manual review.",
                clear_claim: false,
                restored: false,
            }
        }
        (JournalPathState::Missing, JournalPathState::Missing, JournalPathState::Missing) => {
            CleanupRestoreReconciliationDecision {
                status: "manual_review",
                operation_phase: "target_committed",
                identity_status: "restore_pending_recovery",
                message: "target_committed_durability_unknown: Safe Trash restore has neither a matching source, destination, nor Claim; do not auto retry.",
                clear_claim: false,
                restored: false,
            }
        }
        _ => CleanupRestoreReconciliationDecision {
            status: "manual_review",
            operation_phase: cleanup_restore_recovery_phase(
                original_state,
                trash_state,
                current_phase,
            ),
            identity_status: "restore_pending_recovery",
            message: "restore_pending_reconciliation: interrupted Safe Trash restore has an ambiguous path state; do not auto retry.",
            clear_claim: false,
            restored: false,
        },
    }
}

fn cleanup_restore_recovery_phase(
    original_state: JournalPathState,
    trash_state: JournalPathState,
    current_phase: &str,
) -> &'static str {
    if matches!(
        current_phase,
        "prepared"
            | "source_claimed"
            | "copying"
            | "target_committed"
            | "source_cleanup_pending"
            | "manual_review"
    ) {
        return match current_phase {
            "prepared" => "prepared",
            "source_claimed" => "source_claimed",
            "copying" => "copying",
            "target_committed" => "target_committed",
            "source_cleanup_pending" => "source_cleanup_pending",
            _ => "manual_review",
        };
    }

    match (original_state, trash_state) {
        (JournalPathState::Matches, JournalPathState::Missing) => "target_committed",
        (JournalPathState::Missing, JournalPathState::Matches) => "source_claimed",
        _ => "manual_review",
    }
}

fn cleanup_journal_path_state(
    path: &Path,
    identity_matches: impl FnOnce(&Path) -> Result<bool, ()>,
) -> JournalPathState {
    match fs::symlink_metadata(path) {
        Ok(_) => match identity_matches(path) {
            Ok(true) => JournalPathState::Matches,
            Ok(false) => JournalPathState::Mismatch,
            Err(()) => JournalPathState::Unreadable,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => JournalPathState::Missing,
        Err(_) => JournalPathState::Unreadable,
    }
}

pub fn is_forbidden_storage_path_for_test(path: &Path) -> bool {
    is_forbidden_storage_path(path, &[])
}

pub fn is_cleanup_execution_forbidden(path: &Path, app_data_dir: Option<&Path>) -> bool {
    if path.as_os_str().is_empty() {
        return true;
    }
    let lower = normalize_for_compare(path);
    if lower.contains('\0') || lower.contains('*') || lower.contains('?') {
        return true;
    }
    if path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return true;
    }
    if path.parent().is_none() || path.file_name().is_none() || is_drive_root(path) {
        return true;
    }
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return true;
    }
    #[cfg(target_os = "macos")]
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
        && !crate::platform::macos::file_semantics::content_read_eligibility(path).is_eligible()
    {
        return true;
    }
    #[cfg(target_os = "macos")]
    if crate::platform::macos::package::is_package(path) {
        return true;
    }

    if is_current_user_temp_path(path) {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(_) => return true,
        };
        let stats = collect_temp_tree_stats(path);
        let candidate = classify_system_temp_candidate(
            path,
            stats.size,
            temp_entry_facts(path, metadata.is_dir(), stats),
        );
        if !candidate.trash_allowed {
            return true;
        }
    }

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    app_data_dir
        .map(|dir| is_same_or_child(&lower, &normalize_for_compare(dir)))
        .unwrap_or(false)
        || crate::identity_migration::is_legacy_app_data_path(path)
        || is_forbidden_storage_path(path, &[])
        || is_program_files_path_text(&lower)
        || is_appdata_core_path_text(&lower)
        || is_browser_profile_path(&lower)
        || is_chat_database_path(&lower, &extension)
        || is_database_extension(&extension)
}

pub fn cleanup_preview_items_for_candidates(
    ids: Vec<String>,
    candidates: &[StorageCandidate],
) -> Result<Vec<CleanupPreviewItem>, String> {
    let requested: HashSet<&str> = ids.iter().map(String::as_str).collect();
    Ok(candidates
        .iter()
        .filter(|candidate| requested.contains(candidate.id.as_str()))
        .filter(|candidate| {
            candidate.tier == CleanupTier::Safe
                && candidate.trash_allowed
                && candidate.suggested_action == CleanupActionKind::MoveToTrash
                && !generated_output_requires_review(Path::new(&candidate.path))
                && !is_forbidden_storage_path(Path::new(&candidate.path), &[])
        })
        .map(cleanup_preview_item)
        .collect())
}

pub fn preview_cleanup_operations_for_candidates(
    ids: Vec<String>,
    candidates: &[StorageCandidate],
    app_data_dir: Option<&Path>,
) -> Result<OperationPreviewScopeResult, String> {
    let requested: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let previews = candidates
        .iter()
        .filter(|candidate| requested.contains(candidate.id.as_str()))
        .filter(|candidate| cleanup_candidate_can_enter_operation_preview(candidate, app_data_dir))
        .map(cleanup_operation_preview)
        .collect::<Vec<_>>();

    Ok(OperationPreviewScopeResult {
        total: previews.len() as i64,
        limit: previews.len() as u32,
        offset: 0,
        truncated: false,
        has_more: false,
        previews,
    })
}

fn cleanup_candidate_can_enter_operation_preview(
    candidate: &StorageCandidate,
    app_data_dir: Option<&Path>,
) -> bool {
    let path = Path::new(&candidate.path);
    candidate.tier == CleanupTier::Safe
        && candidate.trash_allowed
        && candidate.suggested_action == CleanupActionKind::MoveToTrash
        && !generated_output_requires_review(path)
        && path.exists()
        && !is_cleanup_execution_forbidden(path, app_data_dir)
}

fn cleanup_operation_preview(candidate: &StorageCandidate) -> OperationPreviewDto {
    #[cfg(target_os = "macos")]
    let (target_path, is_executable, blocking_reason) =
        ("Fylura Safe Trash".to_string(), Some(true), None);
    #[cfg(not(target_os = "macos"))]
    let (target_path, is_executable, blocking_reason) = (
        "Recycle Bin".to_string(),
        Some(false),
        Some("system_trash_source_binding_unsupported".to_string()),
    );
    OperationPreviewDto {
        id: format!("cleanup-trash-{}", candidate.id),
        file_id: candidate.id.clone(),
        operation_type: "move_to_trash".to_string(),
        source_path: candidate.path.clone(),
        target_path,
        old_name: candidate.name.clone(),
        new_name: candidate.name.clone(),
        status: "pending".to_string(),
        risk_level: "Normal".to_string(),
        confidence: 1.0,
        requires_confirmation: true,
        suggested_action: "DeleteCandidate".to_string(),
        is_duplicate: false,
        reason: candidate.reason.clone(),
        selected_by_default: Some(true),
        is_executable,
        blocking_reason,
        editable_new_name: Some(false),
        target_parent_exists: Some(true),
        will_create_parent: Some(false),
        strategy: Some("safe_trash_recoverable".to_string()),
        conflict_policy: Some("safe_trash_recoverable".to_string()),
        will_copy: Some(false),
        will_move: Some(true),
        will_download: Some(false),
        materialization_requirement: Some("none".to_string()),
        materialization_requirement_v2: Some("none".to_string()),
        operation_fingerprint: None,
        cross_volume_copy_required: Some(false),
        metadata_degradation_possible: Some(false),
        source_retirement_capability: None,
        source_retirement_eligible: None,
        source_retirement_probe_required: None,
        provider_coordination: Some(false),
        source_identity_fingerprint: None,
        provider_identity_fingerprint: None,
        will_replace: Some(false),
        will_trash: Some(true),
    }
}

#[allow(dead_code)]
fn run_storage_cleanup_scan_job<R: Runtime>(
    roots: Vec<PathBuf>,
    excluded_paths: Vec<PathBuf>,
    app: AppHandle<R>,
    state: StorageCleanupState,
    job_id: String,
    cancel_flag: Arc<AtomicBool>,
) {
    let app_for_progress = app.clone();
    let state_for_progress = state.clone();
    let result = analyze_storage_roots_with_progress(
        roots,
        excluded_paths,
        Some(Arc::clone(&cancel_flag)),
        job_id.clone(),
        move |progress| {
            state_for_progress.update_job_progress(progress.clone())?;
            app_for_progress
                .emit(STORAGE_CLEANUP_PROGRESS_EVENT, progress)
                .map_err(|error| error.to_string())
        },
    );
    finish_storage_cleanup_scan_job(result, app, state, job_id, cancel_flag);
}

fn run_storage_cleanup_scan_job_without_events(
    roots: Vec<PathBuf>,
    excluded_paths: Vec<PathBuf>,
    state: StorageCleanupState,
    job_id: String,
    cancel_flag: Arc<AtomicBool>,
) {
    let state_for_progress = state.clone();
    let result = analyze_storage_roots_with_progress(
        roots,
        excluded_paths,
        Some(Arc::clone(&cancel_flag)),
        job_id.clone(),
        move |progress| state_for_progress.update_job_progress(progress),
    );
    match result {
        Ok(analysis) if cancel_flag.load(Ordering::Relaxed) => {
            let _ = state.mark_job_cancelled(&job_id);
            let _ = analysis;
        }
        Ok(analysis) => {
            let _ = state.complete_job(&job_id, analysis);
        }
        Err(error) => {
            let _ = state.fail_job(&job_id, error);
        }
    }
}

#[allow(dead_code)]
fn finish_storage_cleanup_scan_job<R: Runtime>(
    result: Result<StorageAnalysis, String>,
    app: AppHandle<R>,
    state: StorageCleanupState,
    job_id: String,
    cancel_flag: Arc<AtomicBool>,
) {
    match result {
        Ok(analysis) if cancel_flag.load(Ordering::Relaxed) => {
            let _ = state.mark_job_cancelled(&job_id);
            let _ = app.emit(
                STORAGE_CLEANUP_CANCELLED_EVENT,
                StorageCleanupJobMessage {
                    job_id,
                    message: "Storage cleanup scan was cancelled.".to_string(),
                },
            );
            let _ = analysis;
        }
        Ok(analysis) => match state.complete_job(&job_id, analysis) {
            Ok(analysis) => {
                let completed = StorageCleanupCompleted { job_id, analysis };
                let _ = app.emit(STORAGE_CLEANUP_COMPLETED_EVENT, completed);
            }
            Err(error) => {
                let _ = app.emit(
                    STORAGE_CLEANUP_FAILED_EVENT,
                    StorageCleanupJobMessage {
                        job_id,
                        message: error,
                    },
                );
            }
        },
        Err(error) => {
            let _ = state.fail_job(&job_id, error.clone());
            let _ = app.emit(
                STORAGE_CLEANUP_FAILED_EVENT,
                StorageCleanupJobMessage {
                    job_id,
                    message: error,
                },
            );
        }
    }
}

fn analyze_storage_roots(
    roots: Vec<PathBuf>,
    excluded_paths: Vec<PathBuf>,
) -> Result<StorageAnalysis, String> {
    analyze_storage_roots_with_progress(roots, excluded_paths, None, String::new(), |_| Ok(()))
}

pub(crate) fn analyze_storage_roots_with_progress<F>(
    roots: Vec<PathBuf>,
    excluded_paths: Vec<PathBuf>,
    cancel_flag: Option<Arc<AtomicBool>>,
    job_id: String,
    mut on_progress: F,
) -> Result<StorageAnalysis, String>
where
    F: FnMut(StorageCleanupProgress) -> Result<(), String>,
{
    let root_contexts = build_cleanup_root_contexts(&roots);
    let scan_roots = roots
        .iter()
        .map(|root| normalize_for_compare(root))
        .collect::<HashSet<_>>();
    let mut context = ScanContext {
        scan_roots,
        root_contexts,
        excluded_paths,
        candidates: Vec::new(),
        denied_paths: Vec::new(),
        warnings: Vec::new(),
        progress: StorageCleanupProgress {
            job_id,
            scanned_entries: 0,
            current_path: None,
            total_size: 0,
        },
        cancel_flag,
        cancelled: false,
        last_emit: Instant::now(),
    };
    let mut total_size = 0_u64;

    for root in roots {
        if context.is_cancelled() {
            break;
        }
        if root.as_os_str().is_empty() {
            continue;
        }
        if is_drive_root(&root) {
            context.warnings.push(format!(
                "{} may take longer because it is a whole disk root.",
                normalize_path(&root)
            ));
        }
        if is_forbidden_storage_path(&root, &context.excluded_paths) {
            context.denied_paths.push(normalize_path(&root));
            continue;
        }
        if !root.exists() {
            continue;
        }
        total_size =
            total_size.saturating_add(scan_path_stats(&root, &mut context, &mut on_progress).size);
    }

    context.emit_progress(&mut on_progress, true)?;
    context.candidates.sort_by(|left, right| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left.path.cmp(&right.path))
    });
    dedupe_candidates(&mut context.candidates);
    context.candidates.sort_by(|left, right| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left.path.cmp(&right.path))
    });

    let reclaimable_estimate = context
        .candidates
        .iter()
        .filter(|candidate| candidate.tier == CleanupTier::Safe && candidate.trash_allowed)
        .map(|candidate| candidate.size)
        .sum();
    let review_estimate = context
        .candidates
        .iter()
        .filter(|candidate| candidate.tier == CleanupTier::Review)
        .map(|candidate| candidate.size)
        .sum();

    let candidate_total = context.candidates.len();
    Ok(StorageAnalysis {
        total_size,
        reclaimable_estimate,
        review_estimate,
        candidates: context.candidates,
        denied_paths: context.denied_paths,
        warnings: context.warnings,
        candidate_total,
        candidate_offset: 0,
        candidate_limit: candidate_total,
        has_more: false,
    })
}

fn storage_analysis_page(
    analysis: &StorageAnalysis,
    offset: usize,
    limit: usize,
) -> StorageAnalysis {
    let total = analysis.candidates.len();
    let offset = offset.min(total);
    let end = offset.saturating_add(limit).min(total);
    StorageAnalysis {
        total_size: analysis.total_size,
        reclaimable_estimate: analysis.reclaimable_estimate,
        review_estimate: analysis.review_estimate,
        candidates: analysis.candidates[offset..end].to_vec(),
        denied_paths: analysis.denied_paths.clone(),
        warnings: analysis.warnings.clone(),
        candidate_total: total,
        candidate_offset: offset,
        candidate_limit: limit,
        has_more: end < total,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupRootKind {
    UserSelected,
    SystemTemp,
    UserCache,
    Downloads,
    DevelopmentCache,
}

#[derive(Debug, Clone)]
struct CleanupRootContext {
    requested_root: PathBuf,
    canonical_root: PathBuf,
    kind: CleanupRootKind,
}

impl CleanupRootContext {
    fn matches(&self, path: &str) -> bool {
        let normalized_path = normalize_text_for_compare(path);
        [&self.requested_root, &self.canonical_root]
            .into_iter()
            .any(|root| is_same_or_child(&normalized_path, &normalize_for_compare(root)))
    }
}

fn cleanup_root_kind_for(
    root: &Path,
    current_temp: &Path,
    user_cache: Option<&Path>,
    downloads: Option<&Path>,
) -> CleanupRootKind {
    let normalized = normalize_for_compare(root);
    let temp = normalize_for_compare(current_temp);
    if is_same_or_child(&normalized, &temp) {
        return CleanupRootKind::SystemTemp;
    }
    if downloads.is_some_and(|path| is_same_or_child(&normalized, &normalize_for_compare(path))) {
        return CleanupRootKind::Downloads;
    }
    let lower_name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if is_package_cache_path(&normalized, &lower_name) {
        return CleanupRootKind::DevelopmentCache;
    }
    if user_cache.is_some_and(|path| is_same_or_child(&normalized, &normalize_for_compare(path))) {
        return CleanupRootKind::UserCache;
    }
    CleanupRootKind::UserSelected
}

fn build_cleanup_root_contexts(roots: &[PathBuf]) -> Vec<CleanupRootContext> {
    let current_temp = env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| env::temp_dir());
    let user_cache = dirs::cache_dir().and_then(|path| path.canonicalize().ok());
    let downloads = dirs::download_dir().and_then(|path| path.canonicalize().ok());
    roots
        .iter()
        .map(|root| {
            let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
            let kind = cleanup_root_kind_for(
                &canonical_root,
                &current_temp,
                user_cache.as_deref(),
                downloads.as_deref(),
            );
            CleanupRootContext {
                requested_root: root.clone(),
                canonical_root,
                kind,
            }
        })
        .collect()
}

struct ScanContext {
    scan_roots: HashSet<String>,
    root_contexts: Vec<CleanupRootContext>,
    excluded_paths: Vec<PathBuf>,
    candidates: Vec<StorageCandidate>,
    denied_paths: Vec<String>,
    warnings: Vec<String>,
    progress: StorageCleanupProgress,
    cancel_flag: Option<Arc<AtomicBool>>,
    cancelled: bool,
    last_emit: Instant,
}

impl ScanContext {
    fn root_kind_for(&self, path: &Path) -> CleanupRootKind {
        let normalized = normalize_for_compare(path);
        self.root_contexts
            .iter()
            .filter(|context| context.matches(&normalized))
            .max_by_key(|context| context.canonical_root.components().count())
            .map(|context| context.kind)
            .unwrap_or(CleanupRootKind::UserSelected)
    }

    fn is_cancelled(&mut self) -> bool {
        let cancelled = self
            .cancel_flag
            .as_ref()
            .map(|flag| flag.load(Ordering::Relaxed))
            .unwrap_or(false);
        if cancelled {
            self.cancelled = true;
        }
        cancelled
    }

    fn record_progress<F>(
        &mut self,
        path: &Path,
        size: u64,
        on_progress: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(StorageCleanupProgress) -> Result<(), String>,
    {
        self.progress.scanned_entries = self.progress.scanned_entries.saturating_add(1);
        self.progress.current_path = Some(normalize_path(path));
        self.progress.total_size = self.progress.total_size.saturating_add(size);
        self.emit_progress(on_progress, false)
    }

    fn emit_progress<F>(&mut self, on_progress: &mut F, force: bool) -> Result<(), String>
    where
        F: FnMut(StorageCleanupProgress) -> Result<(), String>,
    {
        let now = Instant::now();
        if force
            || self
                .progress
                .scanned_entries
                .is_multiple_of(STORAGE_CLEANUP_EMIT_ENTRY_INTERVAL)
            || now.duration_since(self.last_emit) >= STORAGE_CLEANUP_EMIT_INTERVAL
        {
            self.last_emit = now;
            on_progress(self.progress.clone())?;
        }
        Ok(())
    }
}

pub(crate) fn validate_cleanup_roots(roots: Vec<String>) -> Result<Vec<PathBuf>, String> {
    if roots.is_empty() {
        return Err("Choose a disk or folder before scanning storage cleanup.".to_string());
    }

    let mut validated = roots
        .into_iter()
        .map(|root| root.trim().to_string())
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .map(validate_cleanup_root)
        .collect::<Result<Vec<_>, _>>()?;

    dedupe_paths(&mut validated);
    if validated.is_empty() {
        return Err("Choose a disk or folder before scanning storage cleanup.".to_string());
    }
    Ok(validated)
}

fn validate_cleanup_root(root: PathBuf) -> Result<PathBuf, String> {
    let extended_namespace = if current_platform() == PathPlatform::Windows {
        let root_text = root.as_os_str().to_string_lossy();
        match validate_windows_path_text(&root_text) {
            Ok(namespace) => namespace != WindowsPathNamespace::Ordinary,
            Err(reason) => {
                return Err(format!(
                    "Cleanup scope contains unsupported Windows path prefix: {} ({reason})",
                    normalize_path(&root)
                ));
            }
        }
    } else {
        false
    };
    let root_text = root.as_os_str().to_string_lossy();
    if root_text.contains('\0')
        || root_text.contains('*')
        || (root_text.contains('?') && !extended_namespace)
    {
        return Err(format!(
            "Cleanup scope contains unsupported path characters: {}",
            normalize_path(&root)
        ));
    }
    if root
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(format!(
            "Cleanup scope cannot contain parent-directory traversal: {}",
            normalize_path(&root)
        ));
    }
    if !root.is_absolute() {
        return Err(format!(
            "Cleanup scope must be an absolute path: {}",
            normalize_path(&root)
        ));
    }
    let metadata = fs::symlink_metadata(&root).map_err(|error| {
        format!(
            "Cleanup scope cannot be inspected: {} ({error})",
            normalize_path(&root)
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Cleanup scope cannot be a symlink: {}",
            normalize_path(&root)
        ));
    }
    let canonical = root.canonicalize().map_err(|error| {
        format!(
            "Cleanup scope cannot be canonicalized: {} ({error})",
            normalize_path(&root)
        )
    })?;
    if is_forbidden_storage_path(&canonical, &[]) {
        return Err(format!(
            "Cleanup scope is protected and cannot be scanned: {}",
            normalize_path(&canonical)
        ));
    }
    Ok(canonical)
}

#[derive(Debug, Clone, Copy)]
struct ScanPathStats {
    size: u64,
    latest_modified: Option<SystemTime>,
    ownership: TempOwnership,
    has_special_entry: bool,
}

impl Default for ScanPathStats {
    fn default() -> Self {
        Self {
            size: 0,
            latest_modified: None,
            ownership: TempOwnership::Unknown,
            has_special_entry: true,
        }
    }
}

impl ScanPathStats {
    fn include(&mut self, child: ScanPathStats) {
        self.size = self.size.saturating_add(child.size);
        self.latest_modified = match (self.latest_modified, child.latest_modified) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (None, value) | (value, None) => value,
        };
        self.ownership = self.ownership.merge(child.ownership);
        self.has_special_entry |= child.has_special_entry;
    }
}

fn scan_path_stats<F>(path: &Path, context: &mut ScanContext, on_progress: &mut F) -> ScanPathStats
where
    F: FnMut(StorageCleanupProgress) -> Result<(), String>,
{
    if context.is_cancelled() {
        return ScanPathStats::default();
    }
    if is_forbidden_storage_path(path, &context.excluded_paths) {
        context.denied_paths.push(normalize_path(path));
        return ScanPathStats::default();
    }

    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => {
            context.denied_paths.push(normalize_path(path));
            return ScanPathStats::default();
        }
    };

    if metadata.file_type().is_symlink() {
        context.denied_paths.push(normalize_path(path));
        return ScanPathStats::default();
    }

    #[cfg(target_os = "macos")]
    if crate::platform::macos::package::is_package(path) {
        let semantics = crate::platform::macos::file_semantics::inspect(path);
        if !semantics.content_availability.is_local() {
            context.denied_paths.push(normalize_path(path));
            let _ = context.record_progress(path, 0, on_progress);
            return ScanPathStats::default();
        }
        let size = semantics.allocated_size.unwrap_or(metadata.len());
        let stats = ScanPathStats {
            size,
            latest_modified: metadata.modified().ok(),
            ownership: temp_ownership(&metadata),
            has_special_entry: true,
        };
        let _ = context.record_progress(path, size, on_progress);
        if size > 0 && !context.scan_roots.contains(&normalize_for_compare(path)) {
            context.candidates.push(macos_package_candidate(path, size));
        }
        return stats;
    }

    if metadata.is_file() {
        #[cfg(target_os = "macos")]
        let native_semantics = crate::platform::macos::file_semantics::inspect(path);
        #[cfg(target_os = "macos")]
        if !native_semantics.content_availability.is_local() {
            context.denied_paths.push(normalize_path(path));
            return ScanPathStats::default();
        }
        #[cfg(target_os = "macos")]
        let size = native_semantics.allocated_size.unwrap_or(metadata.len());
        #[cfg(not(target_os = "macos"))]
        let size = metadata.len();
        let stats = ScanPathStats {
            size,
            latest_modified: metadata.modified().ok(),
            ownership: temp_ownership(&metadata),
            has_special_entry: !matches!(temp_entry_kind(&metadata), TempEntryKind::RegularFile),
        };
        let _ = context.record_progress(path, size, on_progress);
        maybe_record_candidate(path, false, stats, context);
        return stats;
    }

    if !metadata.is_dir() {
        return ScanPathStats::default();
    }

    let mut stats = ScanPathStats {
        size: 0,
        latest_modified: metadata.modified().ok(),
        ownership: temp_ownership(&metadata),
        has_special_entry: false,
    };
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => {
            context.denied_paths.push(normalize_path(path));
            return ScanPathStats::default();
        }
    };

    for entry in entries.flatten() {
        if context.is_cancelled() {
            break;
        }
        stats.include(scan_path_stats(&entry.path(), context, on_progress));
    }

    let _ = context.record_progress(path, 0, on_progress);
    maybe_record_candidate(path, true, stats, context);
    stats
}

fn maybe_record_candidate(
    path: &Path,
    is_dir: bool,
    stats: ScanPathStats,
    context: &mut ScanContext,
) {
    let size = stats.size;
    if size == 0 || context.scan_roots.contains(&normalize_for_compare(path)) {
        return;
    }

    let root_kind = context.root_kind_for(path);
    let candidate = if root_kind == CleanupRootKind::SystemTemp {
        classify_system_temp_candidate(path, size, temp_entry_facts(path, is_dir, stats))
    } else {
        classify_candidate_without_temp_text(path, size)
    };
    let is_large = if is_dir {
        size >= LARGE_DIR_THRESHOLD
    } else {
        size >= LARGE_FILE_THRESHOLD
    };
    let known_candidate = candidate.tier != CleanupTier::Review || candidate.trash_allowed;
    let review_candidate = candidate.tier == CleanupTier::Review && is_large;

    if known_candidate || review_candidate || is_generated_dir_name(path) {
        context.candidates.push(candidate);
    }
}

#[cfg(target_os = "macos")]
fn macos_package_candidate(path: &Path, size: u64) -> StorageCandidate {
    let normalized = normalize_path(path);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(normalized.as_str())
        .to_string();
    StorageCandidate {
        id: candidate_id(&normalized),
        path: normalized,
        name,
        size,
        tier: CleanupTier::Caution,
        category: "macos_package".to_string(),
        reason: "macos_package_logical_entity".to_string(),
        suggested_action: CleanupActionKind::Reveal,
        risk_note: Some("macos_package_cleanup_disabled".to_string()),
        trash_allowed: false,
        selected_by_default: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TempOwnership {
    CurrentUser,
    OtherUser,
    Unknown,
}

impl TempOwnership {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::OtherUser, _) | (_, Self::OtherUser) => Self::OtherUser,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            _ => Self::CurrentUser,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
enum TempEntryKind {
    RegularFile,
    Directory,
    Symlink,
    Socket,
    Pipe,
    Device,
}

#[derive(Debug, Clone, Copy)]
struct TempEntryFacts {
    age: Option<Duration>,
    ownership: TempOwnership,
    kind: TempEntryKind,
    database_like: bool,
}

fn classify_system_temp_candidate(
    path: &Path,
    size: u64,
    facts: TempEntryFacts,
) -> StorageCandidate {
    let normalized = normalize_path(path);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(normalized.as_str())
        .to_string();
    let (tier, reason, suggested_action, risk_note) = if facts.database_like {
        (
            CleanupTier::Caution,
            "Database-like temporary content may still contain active application state."
                .to_string(),
            CleanupActionKind::Reveal,
            Some("Use the owning application's cleanup controls.".to_string()),
        )
    } else if !matches!(
        facts.kind,
        TempEntryKind::RegularFile | TempEntryKind::Directory
    ) {
        (
            CleanupTier::Caution,
            "Special filesystem entries are not eligible for automated cleanup.".to_string(),
            CleanupActionKind::None,
            Some("Sockets, pipes, devices, and symlinks must not be moved to trash.".to_string()),
        )
    } else if facts.ownership == TempOwnership::OtherUser {
        (
            CleanupTier::Caution,
            "Temporary content is owned by another user.".to_string(),
            CleanupActionKind::None,
            Some("Fylura will not offer cleanup for another user's files.".to_string()),
        )
    } else if facts.ownership == TempOwnership::Unknown {
        (
            CleanupTier::Review,
            "Temporary content ownership could not be verified.".to_string(),
            CleanupActionKind::Reveal,
            Some("Ownership must be verified before cleanup.".to_string()),
        )
    } else if facts
        .age
        .is_some_and(|age| age >= Duration::from_secs(7 * 24 * 60 * 60))
    {
        (
            CleanupTier::Safe,
            "Current-user temporary content has not changed for at least 7 days.".to_string(),
            CleanupActionKind::MoveToTrash,
            None,
        )
    } else {
        (
            CleanupTier::Review,
            "Recent temporary content may still be in use by an application.".to_string(),
            CleanupActionKind::Reveal,
            Some("Only temporary content older than 7 days is selected by default.".to_string()),
        )
    };
    let trash_allowed = tier == CleanupTier::Safe;
    StorageCandidate {
        id: candidate_id(&normalized),
        path: normalized,
        name,
        size,
        tier,
        category: "Temporary files".to_string(),
        reason,
        suggested_action,
        risk_note,
        trash_allowed,
        selected_by_default: trash_allowed,
    }
}

fn temp_entry_facts(path: &Path, is_dir: bool, stats: ScanPathStats) -> TempEntryFacts {
    let metadata = fs::symlink_metadata(path).ok();
    let mut kind = metadata.as_ref().map(temp_entry_kind).unwrap_or(if is_dir {
        TempEntryKind::Directory
    } else {
        TempEntryKind::Device
    });
    if stats.has_special_entry {
        kind = TempEntryKind::Device;
    }
    let modified = stats
        .latest_modified
        .or_else(|| metadata.as_ref().and_then(|value| value.modified().ok()));
    let age = modified.and_then(|time| SystemTime::now().duration_since(time).ok());
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    TempEntryFacts {
        age,
        ownership: stats.ownership,
        kind,
        database_like: is_database_extension(&extension),
    }
}

fn collect_temp_tree_stats(path: &Path) -> ScanPathStats {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return ScanPathStats::default(),
    };
    let kind = temp_entry_kind(&metadata);
    let mut stats = ScanPathStats {
        size: if metadata.is_file() {
            metadata.len()
        } else {
            0
        },
        latest_modified: metadata.modified().ok(),
        ownership: temp_ownership(&metadata),
        has_special_entry: !matches!(kind, TempEntryKind::RegularFile | TempEntryKind::Directory),
    };
    if metadata.is_dir() {
        #[cfg(target_os = "macos")]
        if crate::platform::macos::package::is_package(path) {
            return stats;
        }
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(_) => return ScanPathStats::default(),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => return ScanPathStats::default(),
            };
            stats.include(collect_temp_tree_stats(&entry.path()));
        }
    }
    stats
}

fn temp_entry_kind(metadata: &fs::Metadata) -> TempEntryKind {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return TempEntryKind::Symlink;
    }
    if file_type.is_file() {
        return TempEntryKind::RegularFile;
    }
    if file_type.is_dir() {
        return TempEntryKind::Directory;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if file_type.is_socket() {
            return TempEntryKind::Socket;
        }
        if file_type.is_fifo() {
            return TempEntryKind::Pipe;
        }
        if file_type.is_block_device() || file_type.is_char_device() {
            return TempEntryKind::Device;
        }
    }
    TempEntryKind::Device
}

#[cfg(unix)]
fn temp_ownership(metadata: &fs::Metadata) -> TempOwnership {
    use std::os::unix::fs::MetadataExt;
    let current_uid = unsafe { libc::geteuid() };
    if metadata.uid() == current_uid {
        TempOwnership::CurrentUser
    } else {
        TempOwnership::OtherUser
    }
}

#[cfg(not(unix))]
fn temp_ownership(_metadata: &fs::Metadata) -> TempOwnership {
    TempOwnership::CurrentUser
}

fn classify_candidate(path: &Path, size: u64) -> StorageCandidate {
    classify_candidate_with_temp_text(path, size, true)
}

fn classify_candidate_without_temp_text(path: &Path, size: u64) -> StorageCandidate {
    classify_candidate_with_temp_text(path, size, false)
}

fn classify_candidate_with_temp_text(
    path: &Path,
    size: u64,
    allow_temp_text: bool,
) -> StorageCandidate {
    let normalized = normalize_path(path);
    let lower = normalized.to_ascii_lowercase();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(normalized.as_str())
        .to_string();
    let lower_name = name.to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let (tier, category, reason, suggested_action, risk_note) = if is_system_path_text(&lower)
        || lower.contains("/programdata")
    {
        (
            CleanupTier::Caution,
            "System".to_string(),
            "System-managed location. Manual cleanup can break Windows or shared app state."
                .to_string(),
            CleanupActionKind::None,
            Some("No cleanup action is offered for system paths.".to_string()),
        )
    } else if is_program_files_path_text(&lower) {
        (
                CleanupTier::Caution,
                "Application".to_string(),
                "Installed application body. Use the app uninstaller or Windows Apps settings instead of deleting files directly.".to_string(),
                CleanupActionKind::UninstallAdvice,
                Some("Manual deletion can leave services, registry entries, and shared components behind.".to_string()),
            )
    } else if is_virtual_machine_image(&extension) {
        (
            CleanupTier::Caution,
            "Virtual machine image".to_string(),
            "Virtual machine images can contain full working systems or user data.".to_string(),
            CleanupActionKind::Reveal,
            Some("Confirm in the virtualization app before moving or deleting.".to_string()),
        )
    } else if is_chat_database_path(&lower, &extension) || is_database_extension(&extension) {
        (
                CleanupTier::Caution,
                "Application database".to_string(),
                "Database-like application data should not be moved while the owning app may be using it.".to_string(),
                CleanupActionKind::Reveal,
                Some("Use the app's own cleanup/export tools before touching database files.".to_string()),
            )
    } else if is_browser_profile_path(&lower) {
        (
            CleanupTier::Caution,
            "Browser profile".to_string(),
            "Browser profiles can contain sessions, passwords, cookies, and extension state."
                .to_string(),
            CleanupActionKind::AppInternalCleanup,
            Some("Use browser settings for cache, profile, or account cleanup.".to_string()),
        )
    } else if lower_name == "node_modules" {
        (
            CleanupTier::Safe,
            "Regenerable dependency folder".to_string(),
            "node_modules dependencies can usually be recreated by the package manager.".to_string(),
            CleanupActionKind::MoveToTrash,
            Some(
                "Review project context first: dependency folders can contain linked packages or local patches."
                    .to_string(),
            ),
        )
    } else if allow_temp_text && is_temp_path(&lower) {
        (
            CleanupTier::Safe,
            "Temporary files".to_string(),
            "Temporary directory content is normally regenerated by applications.".to_string(),
            CleanupActionKind::MoveToTrash,
            None,
        )
    } else if is_package_cache_path(&lower, &lower_name) {
        (
            CleanupTier::Safe,
            "Developer cache".to_string(),
            "Package-manager cache is explicitly regenerable.".to_string(),
            CleanupActionKind::MoveToTrash,
            None,
        )
    } else if is_generated_dir_name(path) {
        (
            CleanupTier::Review,
            "Unverified development output".to_string(),
            "A directory name alone does not prove its contents can be regenerated.".to_string(),
            CleanupActionKind::Reveal,
            Some(
                "Open the location to review its contents; automated cleanup is unavailable."
                    .to_string(),
            ),
        )
    } else if is_installer_residue(&lower, &extension) {
        (
            CleanupTier::Safe,
            "Installer residue".to_string(),
            "Installer packages can usually be downloaded again if needed.".to_string(),
            CleanupActionKind::MoveToTrash,
            None,
        )
    } else if is_appdata_path(&lower) {
        (
            CleanupTier::Caution,
            "Application data".to_string(),
            "AppData can contain account state, local databases, and app configuration."
                .to_string(),
            CleanupActionKind::Reveal,
            Some("Prefer app-provided cleanup controls for this location.".to_string()),
        )
    } else if is_downloads_path(&lower)
        || is_chat_file_path(&lower)
        || is_review_extension(&extension)
        || size >= LARGE_DIR_THRESHOLD
        || size >= LARGE_FILE_THRESHOLD
    {
        (
            CleanupTier::Review,
            review_category(&lower, &extension),
            "User-owned or unknown large content needs human review before cleanup.".to_string(),
            CleanupActionKind::Reveal,
            Some(
                "Open the location and select explicit items only after reviewing them."
                    .to_string(),
            ),
        )
    } else {
        (
            CleanupTier::Review,
            "Other".to_string(),
            "Unknown storage item.".to_string(),
            CleanupActionKind::Reveal,
            Some("No cleanup action is selected by default.".to_string()),
        )
    };

    let trash_allowed = tier == CleanupTier::Safe
        && suggested_action == CleanupActionKind::MoveToTrash
        && !is_forbidden_storage_path(path, &[]);
    let selected_by_default = trash_allowed
        && ((allow_temp_text && is_temp_path(&lower))
            || is_package_cache_path(&lower, &lower_name)
            || lower_name == "node_modules");

    StorageCandidate {
        id: candidate_id(&normalized),
        path: normalized,
        name,
        size,
        tier,
        category,
        reason,
        suggested_action,
        risk_note,
        trash_allowed,
        selected_by_default,
    }
}

fn cleanup_preview_item(candidate: &StorageCandidate) -> CleanupPreviewItem {
    CleanupPreviewItem {
        id: format!("cleanup-preview-{}", candidate.id),
        candidate_id: candidate.id.clone(),
        path: candidate.path.clone(),
        name: candidate.name.clone(),
        size: candidate.size,
        tier: candidate.tier.clone(),
        category: candidate.category.clone(),
        reason: candidate.reason.clone(),
        operation_type: "move_to_trash_preview".to_string(),
        target_path: "Recycle Bin".to_string(),
        status: "pending".to_string(),
        requires_confirmation: true,
        is_executable: false,
        blocking_reason: Some(
            "Preview only: Fylura has not enabled direct recycle-bin execution for storage cleanup."
                .to_string(),
        ),
    }
}

impl Database {
    pub fn save_cleanup_trash_batch(&self, batch: &CleanupTrashBatch) -> Result<(), DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO cleanup_trash_batches (id, created_at, root, total_items, total_size, status)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(id) DO UPDATE SET
                created_at = excluded.created_at,
                root = excluded.root,
                total_items = excluded.total_items,
                total_size = excluded.total_size,
                status = excluded.status
            "#,
            params![
                batch.id,
                batch.created_at,
                batch.root,
                i64::try_from(batch.total_items).unwrap_or(i64::MAX),
                i64::try_from(batch.total_size).unwrap_or(i64::MAX),
                batch.status
            ],
        )?;
        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO cleanup_trash_items (
                    id, batch_id, original_path, trash_path, name, size, moved_at, restored_at,
                    status, message, source_modified_ns, source_platform_volume_id,
                    source_platform_file_id, source_quick_hash, source_full_hash,
                    trash_modified_ns, trash_platform_volume_id, trash_platform_file_id,
                    trash_quick_hash, trash_full_hash, identity_status,
                    source_claim_path, operation_phase, claim_created_at,
                    claim_platform_file_id, claim_full_hash
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)
                ON CONFLICT(id) DO UPDATE SET
                    batch_id = excluded.batch_id,
                    original_path = excluded.original_path,
                    trash_path = excluded.trash_path,
                    name = excluded.name,
                    size = excluded.size,
                    moved_at = excluded.moved_at,
                    restored_at = excluded.restored_at,
                    status = excluded.status,
                    message = excluded.message,
                    source_modified_ns = excluded.source_modified_ns,
                    source_platform_volume_id = excluded.source_platform_volume_id,
                    source_platform_file_id = excluded.source_platform_file_id,
                    source_quick_hash = excluded.source_quick_hash,
                    source_full_hash = excluded.source_full_hash,
                    trash_modified_ns = excluded.trash_modified_ns,
                    trash_platform_volume_id = excluded.trash_platform_volume_id,
                    trash_platform_file_id = excluded.trash_platform_file_id,
                    trash_quick_hash = excluded.trash_quick_hash,
                    trash_full_hash = excluded.trash_full_hash,
                    identity_status = excluded.identity_status,
                    source_claim_path = excluded.source_claim_path,
                    operation_phase = excluded.operation_phase,
                    claim_created_at = excluded.claim_created_at,
                    claim_platform_file_id = excluded.claim_platform_file_id,
                    claim_full_hash = excluded.claim_full_hash
                "#,
            )?;
            for item in &batch.items {
                stmt.execute(params![
                    item.id,
                    item.batch_id,
                    item.original_path,
                    item.trash_path,
                    item.name,
                    i64::try_from(item.size).unwrap_or(i64::MAX),
                    item.moved_at,
                    item.restored_at,
                    item.status,
                    item.message,
                    item.source_modified_ns,
                    item.source_platform_volume_id,
                    item.source_platform_file_id,
                    item.source_quick_hash,
                    item.source_full_hash,
                    item.trash_modified_ns,
                    item.trash_platform_volume_id,
                    item.trash_platform_file_id,
                    item.trash_quick_hash,
                    item.trash_full_hash,
                    item.identity_status,
                    item.source_claim_path,
                    item.operation_phase,
                    item.claim_created_at,
                    item.claim_platform_file_id,
                    item.claim_full_hash
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_cleanup_trash_batches(&self) -> Result<Vec<CleanupTrashBatch>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT b.id, b.created_at, b.root, b.total_items, b.total_size, b.status,
                   i.id, i.batch_id, i.original_path, i.trash_path, i.name, i.size,
                   i.moved_at, i.restored_at, i.status, i.message,
                   i.source_modified_ns, i.source_platform_volume_id, i.source_platform_file_id,
                   i.source_quick_hash, i.source_full_hash, i.trash_modified_ns,
                   i.trash_platform_volume_id, i.trash_platform_file_id,
                   i.trash_quick_hash, i.trash_full_hash, i.identity_status,
                   i.source_claim_path, i.operation_phase, i.claim_created_at,
                   i.claim_platform_file_id, i.claim_full_hash
            FROM cleanup_trash_batches AS b
            LEFT JOIN cleanup_trash_items AS i ON i.batch_id = b.id
            ORDER BY CAST(b.created_at AS INTEGER) DESC, i.name COLLATE NOCASE ASC
            "#,
        )?;
        let mut rows = stmt.query([])?;
        let mut batches = Vec::new();
        let mut batch_indexes = HashMap::<String, usize>::new();
        while let Some(row) = rows.next()? {
            let id = row.get::<_, String>(0)?;
            let index = if let Some(index) = batch_indexes.get(&id) {
                *index
            } else {
                let index = batches.len();
                batches.push(CleanupTrashBatch {
                    id: id.clone(),
                    created_at: row.get(1)?,
                    root: row.get(2)?,
                    total_items: usize::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                    total_size: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                    status: row.get(5)?,
                    items: Vec::new(),
                });
                batch_indexes.insert(id, index);
                index
            };
            let item_id = row.get::<_, Option<String>>(6)?;
            if let Some(item_id) = item_id {
                batches[index].items.push(CleanupTrashItem {
                    id: item_id,
                    batch_id: row.get(7)?,
                    original_path: row.get(8)?,
                    trash_path: row.get(9)?,
                    name: row.get(10)?,
                    size: u64::try_from(row.get::<_, i64>(11)?).unwrap_or(0),
                    moved_at: row.get(12)?,
                    restored_at: row.get(13)?,
                    status: row.get(14)?,
                    message: row.get(15)?,
                    source_modified_ns: row.get(16)?,
                    source_platform_volume_id: row.get(17)?,
                    source_platform_file_id: row.get(18)?,
                    source_quick_hash: row.get(19)?,
                    source_full_hash: row.get(20)?,
                    trash_modified_ns: row.get(21)?,
                    trash_platform_volume_id: row.get(22)?,
                    trash_platform_file_id: row.get(23)?,
                    trash_quick_hash: row.get(24)?,
                    trash_full_hash: row.get(25)?,
                    identity_status: row.get(26)?,
                    source_claim_path: row.get(27)?,
                    operation_phase: row.get(28)?,
                    claim_created_at: row.get(29)?,
                    claim_platform_file_id: row.get(30)?,
                    claim_full_hash: row.get(31)?,
                });
            }
        }
        Ok(batches)
    }

    pub fn cleanup_trash_items_for_batch(
        &self,
        batch_id: &str,
    ) -> Result<Vec<CleanupTrashItem>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, original_path, trash_path, name, size, moved_at, restored_at,
                   status, message, source_modified_ns, source_platform_volume_id,
                   source_platform_file_id, source_quick_hash, source_full_hash,
                   trash_modified_ns, trash_platform_volume_id, trash_platform_file_id,
                   trash_quick_hash, trash_full_hash, identity_status,
                   source_claim_path, operation_phase, claim_created_at,
                   claim_platform_file_id, claim_full_hash
            FROM cleanup_trash_items
            WHERE batch_id = ?1
            ORDER BY name COLLATE NOCASE ASC
            "#,
        )?;
        let rows = stmt.query_map(params![batch_id], cleanup_trash_item_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }

    pub fn cleanup_trash_item(&self, id: &str) -> Result<Option<CleanupTrashItem>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            r#"
            SELECT id, batch_id, original_path, trash_path, name, size, moved_at, restored_at,
                   status, message, source_modified_ns, source_platform_volume_id,
                   source_platform_file_id, source_quick_hash, source_full_hash,
                   trash_modified_ns, trash_platform_volume_id, trash_platform_file_id,
                   trash_quick_hash, trash_full_hash, identity_status,
                   source_claim_path, operation_phase, claim_created_at,
                   claim_platform_file_id, claim_full_hash
            FROM cleanup_trash_items
            WHERE id = ?1
            "#,
            params![id],
            cleanup_trash_item_from_row,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub fn pending_cleanup_trash_items(&self) -> Result<Vec<CleanupTrashItem>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, original_path, trash_path, name, size, moved_at, restored_at,
                   status, message, source_modified_ns, source_platform_volume_id,
                   source_platform_file_id, source_quick_hash, source_full_hash,
                   trash_modified_ns, trash_platform_volume_id, trash_platform_file_id,
                   trash_quick_hash, trash_full_hash, identity_status,
                   source_claim_path, operation_phase, claim_created_at,
                   claim_platform_file_id, claim_full_hash
            FROM cleanup_trash_items
            WHERE status = 'pending'
               OR (
                    status = 'manual_review'
                    AND identity_status IN ('pending_recovery', 'restore_pending', 'restore_pending_recovery')
                    AND operation_phase IN (
                        'prepared', 'source_claimed', 'copying',
                        'target_committed', 'source_cleanup_pending', 'manual_review'
                    )
               )
            ORDER BY moved_at ASC
            "#,
        )?;
        let rows = stmt.query_map([], cleanup_trash_item_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }

    pub fn recompute_cleanup_batch_statuses(&self) -> Result<(), DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let batch_ids = {
            let mut stmt =
                tx.prepare("SELECT DISTINCT batch_id FROM cleanup_trash_items ORDER BY batch_id")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for batch_id in batch_ids {
            update_cleanup_batch_status_tx(&tx, &batch_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Persist every Safe Trash restore outcome together with its batch status.
    /// The item and batch writes intentionally share one transaction so an
    /// injected batch failure cannot leave a terminal item without a matching
    /// batch state.
    pub fn finalize_cleanup_restore_outcome(&self, item: &CleanupTrashItem) -> Result<(), DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        update_cleanup_trash_item_status_tx(&tx, item)?;
        update_cleanup_batch_status_tx(&tx, &item.batch_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn finalize_successful_cleanup_restore(
        &self,
        item: &CleanupTrashItem,
    ) -> Result<(), DbError> {
        let mut finalized = item.clone();
        finalized.restored_at = Some(
            item.restored_at
                .clone()
                .unwrap_or_else(|| current_timestamp_ms().to_string()),
        );
        finalized.status = "restored".to_string();
        finalized.message = Some("Restored from Fylura Safe Trash.".to_string());
        finalized.identity_status = "verified".to_string();
        finalized.source_claim_path = None;
        finalized.operation_phase = "completed".to_string();
        finalized.claim_created_at = None;
        finalized.claim_platform_file_id = None;
        finalized.claim_full_hash = None;
        self.finalize_cleanup_restore_outcome(&finalized)
    }

    pub fn update_cleanup_trash_item_status(&self, item: &CleanupTrashItem) -> Result<(), DbError> {
        self.finalize_cleanup_restore_outcome(item)
    }
}

fn update_cleanup_trash_item_status_tx(
    tx: &Transaction<'_>,
    item: &CleanupTrashItem,
) -> Result<(), DbError> {
    let updated = tx.execute(
        r#"
        UPDATE cleanup_trash_items
        SET restored_at = ?2,
            status = ?3,
            message = ?4,
            source_modified_ns = ?5,
            source_platform_volume_id = ?6,
            source_platform_file_id = ?7,
            source_quick_hash = ?8,
            source_full_hash = ?9,
            trash_modified_ns = ?10,
            trash_platform_volume_id = ?11,
            trash_platform_file_id = ?12,
            trash_quick_hash = ?13,
            trash_full_hash = ?14,
            identity_status = ?15,
            source_claim_path = ?16,
            operation_phase = ?17,
            claim_created_at = ?18,
            claim_platform_file_id = ?19,
            claim_full_hash = ?20
        WHERE id = ?1
        "#,
        params![
            item.id,
            item.restored_at,
            item.status,
            item.message,
            item.source_modified_ns,
            item.source_platform_volume_id,
            item.source_platform_file_id,
            item.source_quick_hash,
            item.source_full_hash,
            item.trash_modified_ns,
            item.trash_platform_volume_id,
            item.trash_platform_file_id,
            item.trash_quick_hash,
            item.trash_full_hash,
            item.identity_status,
            item.source_claim_path,
            item.operation_phase,
            item.claim_created_at,
            item.claim_platform_file_id,
            item.claim_full_hash
        ],
    )?;
    if updated != 1 {
        return Err(DbError::Validation(format!(
            "Safe Trash item disappeared during finalization: {}",
            item.id
        )));
    }
    Ok(())
}

fn update_cleanup_batch_status_tx(tx: &Transaction<'_>, batch_id: &str) -> Result<(), DbError> {
    let updated = tx.execute(
        r#"
        UPDATE cleanup_trash_batches AS batch
        SET status = CASE
            WHEN EXISTS (
                SELECT 1 FROM cleanup_trash_items AS item
                WHERE item.batch_id = batch.id AND item.status IN ('pending', 'manual_review')
            ) THEN 'pending'
            WHEN EXISTS (
                SELECT 1 FROM cleanup_trash_items AS item
                WHERE item.batch_id = batch.id AND item.status = 'failed'
            ) THEN 'partial_failed'
            ELSE 'success'
        END
        WHERE batch.id = ?1
          AND EXISTS (
              SELECT 1 FROM cleanup_trash_items AS item WHERE item.batch_id = batch.id
          )
        "#,
        params![batch_id],
    )?;
    if updated != 1 {
        return Err(DbError::Validation(format!(
            "Safe Trash restore batch disappeared during finalization: {batch_id}"
        )));
    }
    Ok(())
}

fn cleanup_trash_item_from_row(row: &Row<'_>) -> rusqlite::Result<CleanupTrashItem> {
    let size: i64 = row.get(5)?;
    Ok(CleanupTrashItem {
        id: row.get(0)?,
        batch_id: row.get(1)?,
        original_path: row.get(2)?,
        trash_path: row.get(3)?,
        name: row.get(4)?,
        size: u64::try_from(size).unwrap_or(0),
        moved_at: row.get(6)?,
        restored_at: row.get(7)?,
        status: row.get(8)?,
        message: row.get(9)?,
        source_modified_ns: row.get(10)?,
        source_platform_volume_id: row.get(11)?,
        source_platform_file_id: row.get(12)?,
        source_quick_hash: row.get(13)?,
        source_full_hash: row.get(14)?,
        trash_modified_ns: row.get(15)?,
        trash_platform_volume_id: row.get(16)?,
        trash_platform_file_id: row.get(17)?,
        trash_quick_hash: row.get(18)?,
        trash_full_hash: row.get(19)?,
        identity_status: row.get(20)?,
        source_claim_path: row.get(21)?,
        operation_phase: row.get(22)?,
        claim_created_at: row.get(23)?,
        claim_platform_file_id: row.get(24)?,
        claim_full_hash: row.get(25)?,
    })
}

fn candidate_item_id(_candidate: &StorageCandidate, _index: usize) -> String {
    new_job_id("cleanup-item")
}

fn safe_trash_item_path(
    source: &Path,
    batch_id: &str,
    item_id: &str,
    name: &str,
    app_data_dir: Option<&Path>,
) -> PathBuf {
    let root = preferred_safe_trash_root(source)
        .or_else(|| app_data_dir.map(|dir| dir.join("safe-trash")))
        .unwrap_or_else(|| env::temp_dir().join("Fylura").join("safe-trash"));
    root.join("items").join(batch_id).join(item_id).join(name)
}

fn preferred_safe_trash_root(source: &Path) -> Option<PathBuf> {
    source
        .parent()
        .map(|parent| parent.join(".zen-canvas-trash"))
}

struct SafeTrashExpectedIdentity {
    size: u64,
    modified_ns: Option<String>,
    platform_volume_id: Option<String>,
    platform_file_id: Option<String>,
    quick_hash: Option<String>,
    full_hash: Option<String>,
}

fn move_path_to_safe_trash(
    source: &Path,
    trash_path: &Path,
    expected: SafeTrashExpectedIdentity,
    planned_claim_path: Option<&Path>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<(), SafeTrashMutationError> {
    move_path_with_copy_fallback(
        source,
        trash_path,
        &expected,
        planned_claim_path,
        phase_observer,
        actual_path_observer,
    )
}

fn move_path_to_restore_location(
    trash_path: &Path,
    original: &Path,
    expected: SafeTrashExpectedIdentity,
    planned_claim_path: Option<&Path>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
) -> Result<(), SafeTrashMutationError> {
    move_path_with_copy_fallback(
        trash_path,
        original,
        &expected,
        planned_claim_path,
        phase_observer,
        None,
    )
}

fn move_path_with_copy_fallback(
    source: &Path,
    target: &Path,
    expected_identity: &SafeTrashExpectedIdentity,
    planned_claim_path: Option<&Path>,
    phase_observer: Option<&mut crate::fs_safety::PhaseObserver<'_>>,
    actual_path_observer: Option<&mut crate::fs_safety::ActualPathObserver<'_>>,
) -> Result<(), SafeTrashMutationError> {
    crate::fs_safety::platform_support::ensure_supported_file_mutation()
        .map_err(|error| error.to_string())?;
    let expected = crate::fs_safety::ExpectedFileIdentity {
        size: expected_identity.size,
        modified_ns: expected_identity
            .modified_ns
            .as_deref()
            .and_then(|value| value.parse::<i128>().ok()),
        platform_volume_id: expected_identity.platform_volume_id.clone(),
        platform_file_id: expected_identity.platform_file_id.clone(),
        sample_hash: expected_identity.quick_hash.clone(),
        full_hash: expected_identity.full_hash.clone(),
    };
    if let Some(parent) = target.parent() {
        #[cfg(target_os = "macos")]
        {
            let existing_parent = if parent.exists() {
                parent
            } else {
                source.parent().ok_or_else(|| {
                    SafeTrashMutationError::Validation(
                        crate::platform::macos::mutation::MAC_TARGET_PARENT_CHANGED.to_string(),
                    )
                })?
            };
            crate::platform::macos::mutation::ensure_path_eligible(source, existing_parent)
                .map_err(|code| SafeTrashMutationError::Validation(code.to_string()))?;
        }
        crate::fs_safety::create_directory_chain_no_links(parent).map_err(|error| {
            SafeTrashMutationError::Validation(format!("target parent rejected: {error}"))
        })?;
        #[cfg(target_os = "macos")]
        crate::platform::macos::mutation::ensure_path_eligible(source, parent)
            .map_err(|code| SafeTrashMutationError::Validation(code.to_string()))?;
    }
    crate::fs_safety::atomic_move::atomic_move_noreplace_with_claim_path_and_observer_for_operation_with_actual_paths(
        source,
        target,
        Some(&expected),
        planned_claim_path,
        None,
        phase_observer,
        crate::fs_safety::atomic_move::AtomicMoveOperation::Trash,
        actual_path_observer,
    )
    .map(|_| ())
    .map_err(SafeTrashMutationError::Atomic)
}

fn safe_trash_operation_fingerprint(
    path: &Path,
    full_hash: Option<&str>,
    quick_hash: Option<&str>,
) -> Result<crate::file_ops::FileIdentityFingerprint, String> {
    if full_hash.is_some() || quick_hash.is_some() || !cfg!(target_os = "macos") {
        crate::file_ops::file_identity_fingerprint(path)
    } else {
        crate::file_ops::file_namespace_fingerprint(path)
    }
}

fn cleanup_physical_identity_matches(
    expected_volume: Option<&str>,
    expected_file: Option<&str>,
    actual: &crate::file_ops::FileIdentityFingerprint,
) -> bool {
    if cfg!(target_os = "macos") {
        return matches!(
            (
                expected_volume,
                expected_file,
                actual.platform_volume_id.as_deref(),
                actual.platform_file_id.as_deref(),
            ),
            (Some(expected_volume), Some(expected_file), Some(actual_volume), Some(actual_file))
                if expected_volume == actual_volume && expected_file == actual_file
        );
    }

    expected_volume.is_none_or(|expected| actual.platform_volume_id.as_deref() == Some(expected))
        && expected_file.is_none_or(|expected| actual.platform_file_id.as_deref() == Some(expected))
}

fn cleanup_has_explicit_macos_identity_provenance(item: &CleanupTrashItem) -> bool {
    // Schema 35 makes the source volume the persisted proof that the
    // physical IDs came from an explicit macOS capture. Trash/Claim IDs
    // alone are historical evidence and cannot establish that provenance.
    !cfg!(target_os = "macos") || item.source_platform_volume_id.is_some()
}

fn cleanup_claim_source_volume_id(item: &CleanupTrashItem) -> Option<&str> {
    if matches!(
        item.identity_status.as_str(),
        "restore_pending" | "restore_pending_recovery"
    ) {
        item.trash_platform_volume_id.as_deref()
    } else {
        item.source_platform_volume_id.as_deref()
    }
}

fn safe_trash_identity_matches(item: &CleanupTrashItem, path: &Path) -> bool {
    if item.identity_status != "verified" {
        return false;
    }
    if !cleanup_has_explicit_macos_identity_provenance(item) {
        return false;
    }
    if !cfg!(target_os = "macos") && item.trash_full_hash.is_none() {
        return false;
    }
    let Ok(actual) = safe_trash_operation_fingerprint(
        path,
        item.trash_full_hash.as_deref(),
        item.trash_quick_hash.as_deref(),
    ) else {
        return false;
    };
    if actual.size != item.size
        || item
            .trash_full_hash
            .as_deref()
            .is_some_and(|expected| actual.full_hash.as_deref() != Some(expected))
    {
        return false;
    }
    if let Some(expected_quick_hash) = item.trash_quick_hash.as_deref() {
        if actual.quick_hash.as_deref() != Some(expected_quick_hash) {
            return false;
        }
    }
    cleanup_physical_identity_matches(
        item.trash_platform_volume_id.as_deref(),
        item.trash_platform_file_id.as_deref(),
        &actual,
    )
}

fn safe_trash_restore_source_identity_check(
    item: &CleanupTrashItem,
    path: &Path,
) -> Result<bool, crate::recovery::RecoveryFailure> {
    if item.identity_status != "verified" {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::ManualReviewRequired,
            "Safe Trash item has no verified identity fingerprint",
        ));
    }
    if !cleanup_has_explicit_macos_identity_provenance(item) {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::RestoreSourceIdentityUnreadable,
            "Safe Trash item lacks explicit macOS source identity provenance",
        ));
    }
    if !cfg!(target_os = "macos") && item.trash_full_hash.is_none() {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::ClaimIdentityUnreadable,
            "Safe Trash item full hash is missing",
        ));
    }
    let actual = safe_trash_operation_fingerprint(
        path,
        item.trash_full_hash.as_deref(),
        item.trash_quick_hash.as_deref(),
    )
    .map_err(|error| {
        crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::ClaimIdentityUnreadable,
            format!("Safe Trash item identity could not be read: {error}"),
        )
    })?;
    let matches = actual.size == item.size
        && item
            .trash_quick_hash
            .as_deref()
            .is_none_or(|expected| actual.quick_hash.as_deref() == Some(expected))
        && item
            .trash_full_hash
            .as_deref()
            .is_none_or(|expected| actual.full_hash.as_deref() == Some(expected))
        && cleanup_physical_identity_matches(
            item.trash_platform_volume_id.as_deref(),
            item.trash_platform_file_id.as_deref(),
            &actual,
        );
    if !matches {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::ClaimIdentityMismatch,
            "Safe Trash item identity changed; automatic restore is blocked for manual review",
        ));
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::ClaimIdentityUnreadable,
            format!("Safe Trash item type could not be read: {error}"),
        )
    })?;
    Ok(metadata.is_dir())
}

fn safe_trash_restore_identity_check(
    item: &CleanupTrashItem,
    path: &Path,
    expected_is_dir: bool,
) -> Result<(), crate::recovery::RecoveryFailure> {
    if !cfg!(target_os = "macos") && item.trash_full_hash.is_none() {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
            "Safe Trash restore identity is incomplete",
        ));
    }
    let actual = safe_trash_operation_fingerprint(
        path,
        item.trash_full_hash.as_deref(),
        item.trash_quick_hash.as_deref(),
    )
    .map_err(|error| {
        crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
            format!("restored target identity could not be read: {error}"),
        )
    })?;
    let actual_is_dir = fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir())
        .map_err(|error| {
            crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                format!("restored target type could not be read: {error}"),
            )
        })?;
    safe_trash_restore_target_identity_matches(
        item,
        &actual,
        actual_is_dir,
        expected_is_dir,
        item.trash_full_hash.as_deref().unwrap_or_default(),
    )
}

fn safe_trash_restore_target_identity_matches(
    item: &CleanupTrashItem,
    actual: &crate::file_ops::FileIdentityFingerprint,
    actual_is_dir: bool,
    expected_is_dir: bool,
    expected_hash: &str,
) -> Result<(), crate::recovery::RecoveryFailure> {
    if actual.size != item.size || actual_is_dir != expected_is_dir {
        return Err(crate::recovery::RecoveryFailure::new(
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch,
            "restored target content or object type does not match the Safe Trash journal",
        ));
    }
    if !expected_hash.is_empty() {
        let Some(actual_full_hash) = actual.full_hash.as_deref() else {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                "restored target full hash could not be read",
            ));
        };
        if actual_full_hash != expected_hash {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch,
                "restored target full hash does not match the Safe Trash journal",
            ));
        }
    }
    if let Some(expected_quick_hash) = item.trash_quick_hash.as_deref() {
        let Some(actual_quick_hash) = actual.quick_hash.as_deref() else {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                "restored target quick hash could not be read",
            ));
        };
        if actual_quick_hash != expected_quick_hash {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch,
                "restored target quick hash does not match the Safe Trash journal",
            ));
        }
    }

    let volume_relation = match (
        item.trash_platform_volume_id.as_deref(),
        actual.platform_volume_id.as_deref(),
    ) {
        (Some(expected), Some(actual)) if expected == actual => Some(true),
        (Some(_), Some(_)) => Some(false),
        _ => None,
    };
    if volume_relation == Some(true) {
        if item.trash_platform_file_id.is_none() || actual.platform_file_id.is_none() {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable,
                "restored target file identity is unavailable on a proven same-volume restore",
            ));
        }
        if !cleanup_physical_identity_matches(
            item.trash_platform_volume_id.as_deref(),
            item.trash_platform_file_id.as_deref(),
            actual,
        ) {
            return Err(crate::recovery::RecoveryFailure::new(
                crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch,
                "restored target physical identity does not match the same-volume Safe Trash journal",
            ));
        }
    }
    Ok(())
}

fn pending_safe_trash_source_identity_matches(
    item: &CleanupTrashItem,
    path: &Path,
) -> Result<bool, ()> {
    if !cleanup_has_explicit_macos_identity_provenance(item) {
        return Err(());
    }
    if !cfg!(target_os = "macos") && item.source_full_hash.is_none() {
        return Err(());
    }
    let actual = safe_trash_operation_fingerprint(
        path,
        item.source_full_hash.as_deref(),
        item.source_quick_hash.as_deref(),
    )
    .map_err(|_| ())?;
    if actual.size != item.size
        || item
            .source_full_hash
            .as_deref()
            .is_some_and(|expected| actual.full_hash.as_deref() != Some(expected))
    {
        return Ok(false);
    }
    if let Some(expected_quick_hash) = item.source_quick_hash.as_deref() {
        if actual.quick_hash.as_deref() != Some(expected_quick_hash) {
            return Ok(false);
        }
    }
    if !cleanup_physical_identity_matches(
        item.source_platform_volume_id.as_deref(),
        item.source_platform_file_id.as_deref(),
        &actual,
    ) {
        return Ok(false);
    }
    Ok(true)
}

fn pending_safe_trash_target_identity_matches(
    item: &CleanupTrashItem,
    path: &Path,
) -> Result<bool, ()> {
    if !cleanup_has_explicit_macos_identity_provenance(item) {
        return Err(());
    }
    let expected_hash = item
        .trash_full_hash
        .as_deref()
        .or(item.source_full_hash.as_deref());
    if !cfg!(target_os = "macos") && expected_hash.is_none() {
        return Err(());
    }
    let actual = safe_trash_operation_fingerprint(
        path,
        expected_hash,
        item.trash_quick_hash
            .as_deref()
            .or(item.source_quick_hash.as_deref()),
    )
    .map_err(|_| ())?;
    if actual.size != item.size
        || expected_hash.is_some_and(|expected| actual.full_hash.as_deref() != Some(expected))
    {
        return Ok(false);
    }
    if let Some(expected_quick_hash) = item.trash_quick_hash.as_deref() {
        if actual.quick_hash.as_deref() != Some(expected_quick_hash) {
            return Ok(false);
        }
    }
    if !cleanup_physical_identity_matches(
        item.trash_platform_volume_id.as_deref(),
        item.trash_platform_file_id.as_deref(),
        &actual,
    ) {
        return Ok(false);
    }
    Ok(true)
}

fn pending_safe_trash_claim_identity_matches(
    item: &CleanupTrashItem,
    path: &Path,
) -> Result<bool, ()> {
    if !cleanup_has_explicit_macos_identity_provenance(item) {
        return Err(());
    }
    let restore_claim = matches!(
        item.identity_status.as_str(),
        "restore_pending" | "restore_pending_recovery"
    );
    let expected_hash = item.claim_full_hash.as_deref().or(if restore_claim {
        item.trash_full_hash.as_deref()
    } else {
        item.source_full_hash.as_deref()
    });
    if !cfg!(target_os = "macos") && expected_hash.is_none() {
        return Err(());
    }
    let expected_quick_hash = if restore_claim {
        item.trash_quick_hash
            .as_deref()
            .or(item.source_quick_hash.as_deref())
    } else {
        item.source_quick_hash.as_deref()
    };
    let actual = safe_trash_operation_fingerprint(path, expected_hash, expected_quick_hash)
        .map_err(|_| ())?;
    if actual.size != item.size
        || expected_hash.is_some_and(|expected| actual.full_hash.as_deref() != Some(expected))
    {
        return Ok(false);
    }
    if let Some(expected_quick_hash) = expected_quick_hash {
        if actual.quick_hash.as_deref() != Some(expected_quick_hash) {
            return Ok(false);
        }
    }
    Ok(cleanup_physical_identity_matches(
        cleanup_claim_source_volume_id(item),
        item.claim_platform_file_id.as_deref(),
        &actual,
    ))
}

fn default_scan_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    push_if_some(&mut roots, dirs::download_dir());
    push_if_some(&mut roots, dirs::desktop_dir());
    push_if_some(&mut roots, dirs::document_dir());
    add_standard_user_roots(&mut roots);
    roots.push(env::temp_dir());
    add_home_dev_cache_roots(&mut roots);
    dedupe_paths(&mut roots);
    roots
}

fn add_standard_user_roots(roots: &mut Vec<PathBuf>) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    for relative in ["Downloads", "Desktop", "Documents"] {
        roots.push(home.join(relative));
    }
}

fn add_home_dev_cache_roots(roots: &mut Vec<PathBuf>) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    for relative in [
        ".npm",
        ".pnpm-store",
        ".cargo/registry/cache",
        ".cargo/git/checkouts",
        ".gradle/caches",
        ".m2/repository",
        "AppData/Local/npm-cache",
        "AppData/Local/pnpm-store",
        "AppData/Local/Cargo",
        "AppData/Local/Temp",
    ] {
        roots.push(home.join(relative));
    }
}

fn push_if_some(roots: &mut Vec<PathBuf>, path: Option<PathBuf>) {
    if let Some(path) = path {
        roots.push(path);
    }
}

fn is_forbidden_storage_path(path: &Path, excluded_paths: &[PathBuf]) -> bool {
    let lower = normalize_for_compare(path);
    excluded_paths
        .iter()
        .any(|excluded| is_same_or_child(&lower, &normalize_for_compare(excluded)))
        || (!is_current_user_temp_path(path) && is_system_path_text(&lower))
        || is_zen_canvas_safe_trash_path_text(&lower)
        || lower.contains("/programdata")
        || lower.contains("/startlan/zen canvas")
        || lower.contains("/startlan/fylura")
        || lower.ends_with("/zen-canvas.sqlite3")
        || lower.ends_with("/zen-canvas.sqlite")
        || lower.ends_with("/zen-canvas.db")
        || lower.ends_with("/fylura.sqlite3")
}

fn is_current_user_temp_path(path: &Path) -> bool {
    let temp = match env::temp_dir().canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };
    let candidate = match path.canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };
    is_same_or_child(
        &normalize_for_compare(&candidate),
        &normalize_for_compare(&temp),
    )
}

pub fn is_zen_canvas_safe_trash_path(path: &Path) -> bool {
    is_zen_canvas_safe_trash_path_text(&normalize_for_compare(path))
}

fn is_zen_canvas_safe_trash_path_text(lower: &str) -> bool {
    lower.ends_with("/.zen-canvas-trash")
        || lower.contains("/.zen-canvas-trash/")
        || lower.ends_with("/safe-trash")
        || lower.contains("/safe-trash/items/")
}

fn is_system_path_text(lower: &str) -> bool {
    is_system_path_text_for_os(lower, env::consts::OS)
}

fn is_system_path_text_for_os(value: &str, os: &str) -> bool {
    let normalized = value.replace('\\', "/");
    let comparable = if os == "windows" || os == "macos" {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    };
    let unix_system_prefix = if os == "macos" {
        comparable.starts_with("/system/")
    } else {
        comparable.starts_with("/System/")
    };
    comparable.starts_with("c:/windows")
        || comparable.contains("/windows/system32")
        || comparable.contains("/windows/winsxs")
        || comparable.contains("/system volume information")
        || comparable.contains("/$recycle.bin")
        || matches!(
            comparable.as_str(),
            "/" | "/system"
                | "/library"
                | "/applications"
                | "/usr"
                | "/etc"
                | "/var"
                | "/bin"
                | "/sbin"
                | "/private"
        )
        || unix_system_prefix
        || comparable.starts_with("/usr/")
        || comparable.starts_with("/etc/")
        || comparable.starts_with("/var/")
        || comparable.starts_with("/bin/")
        || comparable.starts_with("/sbin/")
        || comparable.starts_with("/library/")
        || comparable.starts_with("/applications/")
        || comparable.starts_with("/private/")
}

fn is_program_files_path_text(lower: &str) -> bool {
    lower.starts_with("c:/program files/")
        || lower.starts_with("c:/program files (x86)/")
        || lower.contains("/program files/")
        || lower.contains("/program files (x86)/")
}

fn is_appdata_path(lower: &str) -> bool {
    lower.contains("/appdata/local/") || lower.contains("/appdata/roaming/")
}

fn is_appdata_core_path_text(lower: &str) -> bool {
    lower.ends_with("/appdata")
        || lower.ends_with("/appdata/local")
        || lower.ends_with("/appdata/roaming")
        || lower.ends_with("/appdata/locallow")
}

fn is_temp_path(lower: &str) -> bool {
    lower.contains("/appdata/local/temp/") || lower.ends_with("/appdata/local/temp")
}

fn is_drive_root(path: &Path) -> bool {
    let normalized = normalize_path(path);
    let trimmed = normalized.trim_end_matches('/');
    let bytes = trimmed.as_bytes();
    bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn is_generated_dir_name(path: &Path) -> bool {
    let lower_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        lower_name.as_str(),
        "node_modules" | "target" | "dist" | "build" | ".next" | ".nuxt" | ".turbo"
    )
}

// A narrow upgrade guard for the output basenames previously classified Safe.
// Keep the existing dependency/cache policies and independently verified system
// temp policy; neither a saved Finding nor an AI/review decision proves safety.
fn generated_output_requires_review(path: &Path) -> bool {
    if !is_generated_dir_name(path)
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("node_modules"))
    {
        return false;
    }
    if is_current_user_temp_path(path) {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return true;
        };
        let stats = collect_temp_tree_stats(path);
        return !classify_system_temp_candidate(
            path,
            stats.size,
            temp_entry_facts(path, metadata.is_dir(), stats),
        )
        .trash_allowed;
    }
    !classify_candidate_without_temp_text(path, 0).trash_allowed
}

fn is_package_cache_path(lower: &str, lower_name: &str) -> bool {
    matches!(
        lower_name,
        ".npm" | ".pnpm-store" | "npm-cache" | "pnpm-store"
    ) || lower.ends_with("/.cargo/registry/cache")
        || lower.contains("/.cargo/registry/cache/")
        || lower.ends_with("/.cargo/git/checkouts")
        || lower.contains("/.cargo/git/checkouts/")
        || lower.ends_with("/.gradle/caches")
        || lower.contains("/.gradle/caches/")
        || lower.ends_with("/.m2/repository")
        || lower.contains("/.m2/repository/")
}

fn is_installer_residue(lower: &str, extension: &str) -> bool {
    is_downloads_path(lower) && matches!(extension, "msi" | "dmg" | "pkg")
}

fn is_downloads_path(lower: &str) -> bool {
    lower.contains("/downloads/")
}

fn is_chat_file_path(lower: &str) -> bool {
    (lower.contains("/wechat files/")
        || lower.contains("/tencent/files/")
        || lower.contains("/qq/"))
        && !is_chat_database_text(lower)
}

fn is_chat_database_path(lower: &str, extension: &str) -> bool {
    is_chat_database_text(lower)
        || ((lower.contains("wechat") || lower.contains("/qq/"))
            && is_database_extension(extension))
}

fn is_chat_database_text(lower: &str) -> bool {
    (lower.contains("wechat") || lower.contains("/qq/") || lower.contains("tencent"))
        && (lower.ends_with(".db") || lower.ends_with(".sqlite") || lower.ends_with(".sqlite3"))
}

fn is_browser_profile_path(lower: &str) -> bool {
    (lower.contains("/google/chrome/user data")
        || lower.contains("/microsoft/edge/user data")
        || lower.contains("/mozilla/firefox/profiles"))
        && !lower.contains("/cache/")
}

fn is_database_extension(extension: &str) -> bool {
    matches!(extension, "db" | "sqlite" | "sqlite3" | "mdb" | "accdb")
}

fn is_virtual_machine_image(extension: &str) -> bool {
    matches!(
        extension,
        "vmdk" | "vhd" | "vhdx" | "qcow2" | "ova" | "avhdx"
    )
}

fn is_review_extension(extension: &str) -> bool {
    matches!(
        extension,
        "zip"
            | "rar"
            | "7z"
            | "tar"
            | "gz"
            | "mp4"
            | "mov"
            | "mkv"
            | "avi"
            | "pdf"
            | "doc"
            | "docx"
            | "ppt"
            | "pptx"
            | "xls"
            | "xlsx"
    )
}

fn review_category(lower: &str, extension: &str) -> String {
    if is_downloads_path(lower) {
        "Downloads".to_string()
    } else if is_chat_file_path(lower) {
        "Chat files".to_string()
    } else if matches!(extension, "zip" | "rar" | "7z" | "tar" | "gz") {
        "Archive".to_string()
    } else if matches!(extension, "mp4" | "mov" | "mkv" | "avi") {
        "Video".to_string()
    } else if matches!(
        extension,
        "pdf" | "doc" | "docx" | "ppt" | "pptx" | "xls" | "xlsx"
    ) {
        "Documents".to_string()
    } else {
        "Large unknown directory".to_string()
    }
}

fn dedupe_candidates(candidates: &mut Vec<StorageCandidate>) {
    candidates.sort_by(|left, right| {
        let left_depth = Path::new(&left.path).components().count();
        let right_depth = Path::new(&right.path).components().count();
        cleanup_candidate_specificity(right)
            .cmp(&cleanup_candidate_specificity(left))
            .then_with(|| left_depth.cmp(&right_depth))
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut seen = HashSet::new();
    let mut retained_safe_parents = Vec::<String>::new();
    candidates.retain(|candidate| {
        let normalized = normalize_text_for_compare(&candidate.path);
        if !seen.insert(normalized.clone()) {
            return false;
        }
        if candidate.trash_allowed {
            if retained_safe_parents.iter().any(|retained| {
                normalized != *retained
                    && (is_same_or_child(&normalized, retained)
                        || is_same_or_child(retained, &normalized))
            }) {
                return false;
            }
            retained_safe_parents.push(normalized);
        }
        true
    });
}

fn cleanup_candidate_specificity(candidate: &StorageCandidate) -> u8 {
    match candidate.category.as_str() {
        "Regenerable dependency folder" | "Developer cache" => 3,
        "Regenerable development output" => 2,
        "Temporary files" => 1,
        _ => 0,
    }
}

fn dedupe_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(normalize_for_compare(path)));
}

fn is_same_or_child(path: &str, parent: &str) -> bool {
    path == parent || path.starts_with(&format!("{parent}/"))
}

fn candidate_id(path: &str) -> String {
    let digest = blake3::hash(normalize_text_for_compare(path).as_bytes())
        .to_hex()
        .to_string();
    format!("storage-{}", &digest[..32])
}

fn candidate_id_for_job(job_id: &str, path: &str) -> String {
    let digest = blake3::hash(format!("{job_id}\0{}", normalize_text_for_compare(path)).as_bytes())
        .to_hex()
        .to_string();
    format!("storage-{}", &digest[..32])
}

fn current_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod temp_safety_tests {
    use super::*;

    fn safe_trash_restore_test_item() -> CleanupTrashItem {
        CleanupTrashItem {
            id: "safe-trash-test-item".to_string(),
            batch_id: "safe-trash-test-batch".to_string(),
            original_path: "C:/restore/original.txt".to_string(),
            trash_path: "D:/restore/trash/original.txt".to_string(),
            name: "original.txt".to_string(),
            size: 7,
            moved_at: "1900000000000".to_string(),
            restored_at: None,
            status: "moved".to_string(),
            message: None,
            source_modified_ns: None,
            source_platform_volume_id: Some("source-volume".to_string()),
            source_platform_file_id: Some("source-file".to_string()),
            source_quick_hash: Some("source-quick".to_string()),
            source_full_hash: Some("source-full".to_string()),
            trash_modified_ns: None,
            trash_platform_volume_id: Some("trash-volume".to_string()),
            trash_platform_file_id: Some("trash-file".to_string()),
            trash_quick_hash: Some("trash-quick".to_string()),
            trash_full_hash: Some("trash-full".to_string()),
            identity_status: "verified".to_string(),
            source_claim_path: None,
            operation_phase: "completed".to_string(),
            claim_created_at: None,
            claim_platform_file_id: None,
            claim_full_hash: None,
        }
    }

    #[test]
    fn safe_trash_restore_identity_allows_cross_volume_content_identity() {
        let item = safe_trash_restore_test_item();
        let same_volume = crate::file_ops::FileIdentityFingerprint {
            size: 7,
            modified_ns: None,
            platform_volume_id: Some("trash-volume".to_string()),
            platform_file_id: Some("trash-file".to_string()),
            quick_hash: Some("trash-quick".to_string()),
            full_hash: Some("trash-full".to_string()),
        };
        assert!(safe_trash_restore_target_identity_matches(
            &item,
            &same_volume,
            false,
            false,
            "trash-full"
        )
        .is_ok());

        let cross_volume = crate::file_ops::FileIdentityFingerprint {
            platform_volume_id: Some("original-volume".to_string()),
            platform_file_id: Some("new-file-on-original-volume".to_string()),
            ..same_volume.clone()
        };
        assert!(safe_trash_restore_target_identity_matches(
            &item,
            &cross_volume,
            false,
            false,
            "trash-full"
        )
        .is_ok());

        let unknown_volume = crate::file_ops::FileIdentityFingerprint {
            platform_volume_id: None,
            platform_file_id: None,
            ..same_volume.clone()
        };
        assert!(safe_trash_restore_target_identity_matches(
            &item,
            &unknown_volume,
            false,
            false,
            "trash-full"
        )
        .is_ok());

        let hash_mismatch = crate::file_ops::FileIdentityFingerprint {
            full_hash: Some("wrong-full".to_string()),
            ..cross_volume.clone()
        };
        let mismatch = safe_trash_restore_target_identity_matches(
            &item,
            &hash_mismatch,
            false,
            false,
            "trash-full",
        )
        .expect_err("cross-volume hash mismatch must fail closed");
        assert_eq!(
            mismatch.code,
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch
        );

        let unreadable = crate::file_ops::FileIdentityFingerprint {
            full_hash: None,
            ..cross_volume.clone()
        };
        let unreadable = safe_trash_restore_target_identity_matches(
            &item,
            &unreadable,
            false,
            false,
            "trash-full",
        )
        .expect_err("unreadable cross-volume identity must fail closed");
        assert_eq!(
            unreadable.code,
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityUnreadable
        );

        let same_volume_mismatch = crate::file_ops::FileIdentityFingerprint {
            platform_file_id: Some("different-file-on-same-volume".to_string()),
            ..same_volume.clone()
        };
        let mismatch = safe_trash_restore_target_identity_matches(
            &item,
            &same_volume_mismatch,
            false,
            false,
            "trash-full",
        )
        .expect_err("same-volume file-id mismatch must fail closed");
        assert_eq!(
            mismatch.code,
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch
        );

        let object_type_mismatch = safe_trash_restore_target_identity_matches(
            &item,
            &cross_volume,
            false,
            true,
            "trash-full",
        )
        .expect_err("restored object type mismatch must fail closed");
        assert_eq!(
            object_type_mismatch.code,
            crate::recovery::RecoveryErrorCode::TargetCommittedIdentityMismatch
        );
    }

    #[test]
    fn pending_safe_trash_matchers_keep_source_target_and_claim_identities_separate() {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-pending-safe-trash-matcher-{}-{}",
            std::process::id(),
            current_timestamp_ms()
        ));
        fs::write(&path, "pending-safe-trash-content").expect("write matcher fixture");
        let fingerprint = crate::file_ops::file_identity_fingerprint(&path)
            .expect("capture matcher fixture identity");
        let mut item = safe_trash_restore_test_item();
        item.size = fingerprint.size;
        item.source_platform_volume_id = fingerprint.platform_volume_id.clone();
        item.source_platform_file_id = fingerprint.platform_file_id.clone();
        item.source_quick_hash = fingerprint.quick_hash.clone();
        item.source_full_hash = fingerprint.full_hash.clone();
        item.trash_platform_file_id = fingerprint.platform_file_id.clone();
        item.trash_platform_volume_id = fingerprint.platform_volume_id.clone();
        item.trash_quick_hash = fingerprint.quick_hash.clone();
        item.trash_full_hash = fingerprint.full_hash.clone();
        item.claim_platform_file_id = fingerprint.platform_file_id.clone();
        item.claim_full_hash = fingerprint.full_hash.clone();

        assert!(pending_safe_trash_source_identity_matches(&item, &path)
            .expect("source identity is readable"));
        assert!(pending_safe_trash_target_identity_matches(&item, &path)
            .expect("target identity is readable"));
        assert!(pending_safe_trash_claim_identity_matches(&item, &path)
            .expect("claim identity is readable"));

        item.source_platform_file_id = Some("source-only-mismatch".to_string());
        assert!(!pending_safe_trash_source_identity_matches(&item, &path)
            .expect("source mismatch remains readable"));
        assert!(pending_safe_trash_target_identity_matches(&item, &path)
            .expect("target uses trash identity only"));
        item.source_platform_file_id = fingerprint.platform_file_id.clone();
        item.source_platform_volume_id = Some("source-only-mismatch".to_string());
        assert!(!pending_safe_trash_source_identity_matches(&item, &path)
            .expect("source volume mismatch remains readable"));
        item.source_platform_volume_id = fingerprint.platform_volume_id.clone();
        item.trash_platform_volume_id = Some("trash-volume-mismatch".to_string());
        assert!(!pending_safe_trash_target_identity_matches(&item, &path)
            .expect("trash volume mismatch remains readable"));
        item.trash_platform_volume_id = fingerprint.platform_volume_id.clone();
        item.trash_platform_file_id = Some("trash-file-mismatch".to_string());
        assert!(!pending_safe_trash_target_identity_matches(&item, &path)
            .expect("trash file mismatch remains readable"));
        item.trash_platform_file_id = fingerprint.platform_file_id.clone();
        if cfg!(target_os = "macos") {
            item.source_platform_volume_id = None;
            assert!(
                pending_safe_trash_source_identity_matches(&item, &path).is_err(),
                "macOS source volume absence remains fail closed"
            );
            item.source_platform_volume_id = fingerprint.platform_volume_id.clone();
        }

        item.claim_platform_file_id = Some("claim-mismatch".to_string());
        assert!(!pending_safe_trash_claim_identity_matches(&item, &path)
            .expect("claim mismatch remains readable"));
        item.claim_platform_file_id = None;
        if cfg!(target_os = "macos") {
            assert!(!pending_safe_trash_claim_identity_matches(&item, &path)
                .expect("missing macOS claim identity must fail closed"));
        } else {
            assert!(pending_safe_trash_claim_identity_matches(&item, &path)
                .expect("missing non-macOS optional claim identity remains usable"));
        }
        item.source_platform_file_id = fingerprint.platform_file_id.clone();
        item.claim_platform_file_id = fingerprint.platform_file_id.clone();
        assert!(pending_safe_trash_claim_identity_matches(&item, &path)
            .expect("claim identity remains independently bound"));
        item.source_platform_volume_id = Some("claim-volume-mismatch".to_string());
        assert!(!pending_safe_trash_claim_identity_matches(&item, &path)
            .expect("claim source volume mismatch remains readable"));
        item.source_platform_volume_id = fingerprint.platform_volume_id.clone();
        item.identity_status = "restore_pending".to_string();
        item.source_platform_volume_id = Some("restore-source-volume-mismatch".to_string());
        assert!(pending_safe_trash_claim_identity_matches(&item, &path)
            .expect("restore Claim uses the verified Trash volume"));
        item.trash_platform_volume_id = Some("restore-trash-volume-mismatch".to_string());
        assert!(!pending_safe_trash_claim_identity_matches(&item, &path)
            .expect("restore Claim rejects a mismatched Trash volume"));
        item.trash_platform_volume_id = fingerprint.platform_volume_id.clone();
        item.source_platform_volume_id = fingerprint.platform_volume_id.clone();
        item.identity_status = "verified".to_string();

        item.trash_full_hash = None;
        item.trash_quick_hash = None;
        item.trash_platform_file_id = None;
        item.trash_platform_volume_id = None;
        if cfg!(target_os = "macos") {
            assert!(!pending_safe_trash_target_identity_matches(&item, &path)
                .expect("missing macOS target physical identity must fail closed"));
        } else {
            assert!(pending_safe_trash_target_identity_matches(&item, &path)
                .expect("missing non-macOS optional target physical identity remains usable"));
        }

        fs::remove_file(&path).expect("remove matcher fixture");
    }

    #[test]
    fn cleanup_identity_components_require_explicit_physical_match() {
        let actual = crate::file_ops::FileIdentityFingerprint {
            size: 0,
            modified_ns: None,
            platform_volume_id: Some("dev-7".to_string()),
            platform_file_id: Some("ino-9".to_string()),
            quick_hash: None,
            full_hash: None,
        };
        assert!(cleanup_physical_identity_matches(
            Some("dev-7"),
            Some("ino-9"),
            &actual
        ));
        assert!(!cleanup_physical_identity_matches(
            Some("other-volume"),
            Some("ino-9"),
            &actual
        ));
        if cfg!(target_os = "macos") {
            assert!(!cleanup_physical_identity_matches(None, None, &actual));
            assert!(!cleanup_physical_identity_matches(
                None,
                Some("ino-9"),
                &actual
            ));
        } else {
            assert!(cleanup_physical_identity_matches(None, None, &actual));
            assert!(cleanup_physical_identity_matches(
                None,
                Some("ino-9"),
                &actual
            ));
        }

        let without_optional_ids = crate::file_ops::FileIdentityFingerprint {
            platform_volume_id: None,
            platform_file_id: None,
            ..actual
        };
        if cfg!(target_os = "macos") {
            assert!(!cleanup_physical_identity_matches(
                None,
                None,
                &without_optional_ids
            ));
        } else {
            assert!(cleanup_physical_identity_matches(
                None,
                None,
                &without_optional_ids
            ));
        }
    }

    #[test]
    fn cleanup_restore_reconciliation_covers_three_paths_and_preserves_claim_boundaries() {
        let missing = JournalPathState::Missing;
        let matches = JournalPathState::Matches;

        for phase in ["completed", "target_committed", "source_cleanup_pending"] {
            let restored = classify_cleanup_restore_reconciliation(
                matches,
                missing,
                missing,
                phase,
                "pending",
                "restore_pending",
            );
            assert_eq!(restored.status, "restored");
            assert_eq!(restored.operation_phase, "completed");
            assert_eq!(restored.identity_status, "verified");
            assert!(restored.clear_claim);
            assert!(restored.restored);
        }

        let rolled_back = classify_cleanup_restore_reconciliation(
            missing,
            matches,
            missing,
            "prepared",
            "pending",
            "restore_pending",
        );
        assert_eq!(rolled_back.status, "moved");
        assert_eq!(rolled_back.operation_phase, "rolled_back");
        assert_eq!(rolled_back.identity_status, "verified");
        assert!(rolled_back.clear_claim);
        assert!(!rolled_back.restored);

        for (original, trash, claim, phase) in [
            (missing, missing, matches, "source_claimed"),
            (missing, matches, matches, "source_cleanup_pending"),
            (missing, missing, missing, "manual_review"),
        ] {
            let review = classify_cleanup_restore_reconciliation(
                original,
                trash,
                claim,
                phase,
                "pending",
                "restore_pending_recovery",
            );
            assert_eq!(review.status, "manual_review");
            assert_eq!(review.identity_status, "restore_pending_recovery");
            assert!(!review.clear_claim);
            assert!(!review.restored);
            assert!(
                review.message.contains("do not auto retry")
                    || review.message.contains("manual review")
            );
        }
    }

    #[test]
    fn cleanup_restore_reconciliation_treats_claim_identity_read_errors_as_unreadable_review() {
        let review = classify_cleanup_restore_reconciliation(
            JournalPathState::Missing,
            JournalPathState::Matches,
            JournalPathState::Unreadable,
            "prepared",
            "pending",
            "restore_pending",
        );

        assert_eq!(review.status, "manual_review");
        assert_eq!(review.operation_phase, "prepared");
        assert_eq!(review.identity_status, "restore_pending_recovery");
        assert!(review.message.starts_with("claim_identity_unreadable:"));
        assert!(!review.clear_claim);
    }

    #[test]
    fn cleanup_root_context_matches_requested_and_canonical_aliases() {
        let context = CleanupRootContext {
            requested_root: PathBuf::from("/var/folders/user/T/job"),
            canonical_root: PathBuf::from("/private/var/folders/user/T/job"),
            kind: CleanupRootKind::SystemTemp,
        };

        assert!(context.matches("/var/folders/user/T/job/old.tmp"));
        assert!(context.matches("/private/var/folders/user/T/job/old.tmp"));
        assert!(!context.matches("/private/var/folders/other/T/job/old.tmp"));
    }

    #[test]
    #[cfg(windows)]
    fn cleanup_root_context_matches_windows_short_path_alias() {
        let context = CleanupRootContext {
            requested_root: PathBuf::from("C:/Users/RUNNER~1/AppData/Local/Temp/job"),
            canonical_root: PathBuf::from("C:/Users/runneradmin/AppData/Local/Temp/job"),
            kind: CleanupRootKind::SystemTemp,
        };

        assert!(context.matches("c:/users/runner~1/appdata/local/temp/job/old.tmp"));
        assert!(context.matches("c:/users/runneradmin/appdata/local/temp/job/old.tmp"));
        assert!(!context.matches("c:/users/other/appdata/local/temp/job/old.tmp"));
    }

    #[test]
    fn current_user_old_temp_file_is_safe_and_selected() {
        let candidate = classify_system_temp_candidate(
            Path::new("/tmp/zen-canvas-old.tmp"),
            128,
            TempEntryFacts {
                age: Some(Duration::from_secs(8 * 24 * 60 * 60)),
                ownership: TempOwnership::CurrentUser,
                kind: TempEntryKind::RegularFile,
                database_like: false,
            },
        );

        assert_eq!(candidate.tier, CleanupTier::Safe);
        assert!(candidate.trash_allowed);
        assert!(candidate.selected_by_default);
    }

    #[test]
    fn current_user_recent_temp_file_is_review_only() {
        let candidate = classify_system_temp_candidate(
            Path::new("/tmp/zen-canvas-new.tmp"),
            128,
            TempEntryFacts {
                age: Some(Duration::from_secs(12 * 60 * 60)),
                ownership: TempOwnership::CurrentUser,
                kind: TempEntryKind::RegularFile,
                database_like: false,
            },
        );

        assert_eq!(candidate.tier, CleanupTier::Review);
        assert!(!candidate.trash_allowed);
        assert!(!candidate.selected_by_default);
    }

    #[test]
    fn temp_database_is_caution_even_when_old() {
        let candidate = classify_system_temp_candidate(
            Path::new("/tmp/cache.sqlite"),
            128,
            TempEntryFacts {
                age: Some(Duration::from_secs(30 * 24 * 60 * 60)),
                ownership: TempOwnership::CurrentUser,
                kind: TempEntryKind::RegularFile,
                database_like: true,
            },
        );

        assert_eq!(candidate.tier, CleanupTier::Caution);
        assert!(!candidate.trash_allowed);
    }

    #[test]
    fn unverifiable_or_foreign_temp_ownership_is_not_trashable() {
        for ownership in [TempOwnership::Unknown, TempOwnership::OtherUser] {
            let candidate = classify_system_temp_candidate(
                Path::new("/tmp/unowned.tmp"),
                128,
                TempEntryFacts {
                    age: Some(Duration::from_secs(30 * 24 * 60 * 60)),
                    ownership,
                    kind: TempEntryKind::RegularFile,
                    database_like: false,
                },
            );

            assert_ne!(candidate.tier, CleanupTier::Safe);
            assert!(!candidate.trash_allowed);
            assert!(!candidate.selected_by_default);
        }
    }

    #[test]
    fn special_temp_file_types_are_not_trashable() {
        for kind in [
            TempEntryKind::Symlink,
            TempEntryKind::Socket,
            TempEntryKind::Pipe,
            TempEntryKind::Device,
        ] {
            let candidate = classify_system_temp_candidate(
                Path::new("/tmp/special.tmp"),
                128,
                TempEntryFacts {
                    age: Some(Duration::from_secs(30 * 24 * 60 * 60)),
                    ownership: TempOwnership::CurrentUser,
                    kind,
                    database_like: false,
                },
            );

            assert_eq!(candidate.tier, CleanupTier::Caution);
            assert!(!candidate.trash_allowed);
        }
    }

    #[test]
    fn only_current_real_temp_root_gets_system_temp_context() {
        let current_temp = Path::new("/private/var/folders/current/T");

        assert_eq!(
            cleanup_root_kind_for(
                Path::new("/private/var/folders/current/T/scan"),
                current_temp,
                None,
                None,
            ),
            CleanupRootKind::SystemTemp
        );
        assert_eq!(
            cleanup_root_kind_for(
                Path::new("/private/var/folders/another/T"),
                current_temp,
                None,
                None,
            ),
            CleanupRootKind::UserSelected
        );
        assert_eq!(
            cleanup_root_kind_for(Path::new("/private/var/folders"), current_temp, None, None),
            CleanupRootKind::UserSelected
        );
    }

    #[test]
    fn tmp_uses_system_temp_context_when_it_is_the_real_temp_root() {
        assert_eq!(
            cleanup_root_kind_for(Path::new("/tmp/session"), Path::new("/tmp"), None, None),
            CleanupRootKind::SystemTemp
        );
    }

    #[test]
    fn macos_var_folders_outside_current_temp_remain_protected() {
        for path in [
            "/var/folders",
            "/var/folders/another-user/T",
            "/private/var/folders",
            "/private/var/folders/another-user/T",
            "/private/var/db",
        ] {
            assert!(
                is_system_path_text_for_os(path, "macos"),
                "expected macOS system path to remain protected: {path}"
            );
        }
    }

    #[test]
    fn macos_system_path_case_variants_remain_protected() {
        for path in ["/sYsTeM", "/LIBRARY/Application Support", "/aPpLiCaTiOnS"] {
            assert!(is_system_path_text_for_os(path, "macos"));
        }
    }

    #[test]
    fn temp_directory_age_uses_newest_child_mtime() {
        let now = SystemTime::now();
        let mut directory = ScanPathStats {
            size: 0,
            latest_modified: Some(now - Duration::from_secs(10 * 24 * 60 * 60)),
            ownership: TempOwnership::CurrentUser,
            has_special_entry: false,
        };
        directory.include(ScanPathStats {
            size: 128,
            latest_modified: Some(now - Duration::from_secs(2 * 60 * 60)),
            ownership: TempOwnership::CurrentUser,
            has_special_entry: false,
        });
        let age = directory
            .latest_modified
            .and_then(|modified| now.duration_since(modified).ok());
        let candidate = classify_system_temp_candidate(
            Path::new("/tmp/directory"),
            directory.size,
            TempEntryFacts {
                age,
                ownership: TempOwnership::CurrentUser,
                kind: TempEntryKind::Directory,
                database_like: false,
            },
        );

        assert_eq!(candidate.tier, CleanupTier::Review);
        assert!(!candidate.selected_by_default);
    }
}

#[cfg(test)]
mod analysis_cleanup_contract_tests {
    use super::*;
    use crate::db::{
        AnalysisFindingDto, AnalysisScopeRequest, FindingDraft, FindingEvidenceDraft,
        StartAnalysisRunRequest,
    };
    use serde_json::json;

    fn durable_cleanup_finding(
        db: &Database,
        path: &Path,
        tier: &str,
        executable: bool,
    ) -> AnalysisFindingDto {
        let run = db
            .start_analysis_run(
                &StartAnalysisRunRequest {
                    scope: AnalysisScopeRequest {
                        kind: "approved_cleanup_paths".to_string(),
                        root_ids: Vec::new(),
                        paths: vec![path
                            .parent()
                            .expect("cleanup fixture parent")
                            .to_string_lossy()
                            .into_owned()],
                    },
                    detector_ids: vec!["cleanup_heuristics_v1".to_string()],
                    request_key: Some(format!("cleanup-selection-{}", new_job_id("test"))),
                },
                &[("cleanup_heuristics_v1".to_string(), 1)],
            )
            .expect("start durable cleanup fixture")
            .run;
        let _running = db
            .claim_analysis_run(&run.id)
            .expect("claim durable cleanup fixture")
            .expect("durable cleanup fixture is queued");
        let detector = db
            .list_analysis_run_detectors(&run.id)
            .expect("list cleanup fixture detector")
            .into_iter()
            .next()
            .expect("cleanup fixture detector");
        let running_detector = db
            .set_analysis_detector_status(
                &run.id,
                &detector.detector_id,
                detector.revision,
                "running",
                1,
                0,
                0,
                0,
                None,
                None,
            )
            .expect("start cleanup fixture detector");
        let metadata = fs::symlink_metadata(path).expect("read cleanup fixture metadata");
        let identity =
            crate::analysis::candidate_identity(&path.to_string_lossy(), 10, Some(&metadata));
        db.stage_analysis_findings(
            &run.id,
            &run.scope_hash,
            &[FindingDraft {
                id: "cleanup-selection-finding".to_string(),
                finding_key: "cleanup-selection-key".to_string(),
                detector_id: "cleanup_heuristics_v1".to_string(),
                detector_version: 1,
                tier: tier.to_string(),
                category: "test_cleanup".to_string(),
                action_kind: "safe_trash_candidate".to_string(),
                title: "Cleanup fixture".to_string(),
                reason: "Cleanup fixture".to_string(),
                risk_note: None,
                confidence: if tier == "safe" {
                    "exact".to_string()
                } else {
                    "estimated".to_string()
                },
                size_bytes: 10,
                exact_reclaimable_bytes: executable.then_some(10),
                potential_reclaimable_bytes: 10,
                requires_confirmation: true,
                executable,
                primary_subject_kind: "approved_path".to_string(),
                primary_subject_id: path.to_string_lossy().into_owned(),
                path_snapshot: Some(path.to_string_lossy().into_owned()),
                identity_snapshot: identity,
                evidence_summary: json!({"trashAllowed": executable}),
                evidence: vec![FindingEvidenceDraft {
                    evidence_kind: "test_identity".to_string(),
                    subject_kind: "approved_path".to_string(),
                    subject_id: Some(path.to_string_lossy().into_owned()),
                    path_snapshot: Some(path.to_string_lossy().into_owned()),
                    value: json!({"test": true}),
                }],
            }],
        )
        .expect("stage cleanup fixture finding");
        let completed_detector = db
            .set_analysis_detector_status(
                &run.id,
                &running_detector.detector_id,
                running_detector.revision,
                "completed",
                1,
                1,
                if executable { 10 } else { 0 },
                10,
                None,
                None,
            )
            .expect("complete cleanup fixture detector");
        db.publish_analysis_run(&run.id)
            .expect("publish cleanup fixture run");
        let finding = db
            .get_analysis_finding("cleanup-selection-finding")
            .expect("load cleanup fixture finding")
            .expect("cleanup fixture finding exists");
        assert_eq!(completed_detector.status, "completed");
        finding
    }

    #[test]
    fn output_name_guard_preserves_independent_temp_and_package_cache_safety() {
        let root = std::env::temp_dir().join(new_job_id("verified-temp-output-name"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("build");
        fs::write(&path, b"old temporary content").unwrap();
        let old = SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();
        assert!(!generated_output_requires_review(&path));
        assert!(!is_cleanup_execution_forbidden(&path, None));
        fs::remove_dir_all(root).unwrap();

        for path in [
            "C:/Users/zen/.cargo/registry/cache/build",
            "C:/Users/zen/.gradle/caches/dist",
            "C:/Users/zen/.m2/repository/target",
        ] {
            let candidate = classify_candidate_without_temp_text(Path::new(path), 100);
            assert_eq!(candidate.tier, CleanupTier::Safe, "{path}");
            assert!(candidate.trash_allowed);
            assert!(candidate.selected_by_default);
            assert!(!generated_output_requires_review(Path::new(path)));
        }
    }

    #[test]
    fn legacy_safe_output_findings_and_confirmed_review_cannot_authorize_cleanup() {
        for name in [
            "build", "dist", "target", ".next", ".nuxt", ".turbo", "BuIlD",
        ] {
            let root = std::env::current_dir()
                .unwrap()
                .join(".tmp-tests")
                .join(new_job_id("legacy-output-finding"));
            let path = root.join("Library").join(name);
            fs::create_dir_all(&path).expect("create ordinary user output-named directory");
            let manual_file = path.join("build-name-test.txt");
            fs::write(&manual_file, b"manual user content").expect("write user fixture");
            let db = Database::open(root.join("analysis.sqlite3")).expect("open fixture db");
            let mut finding = durable_cleanup_finding(&db, &path, "safe", true);
            assert!(crate::analysis::finding_identity_matches(&db, &finding));
            assert_eq!(finding.confidence, "exact");
            let selection = CleanupFindingSelection {
                finding_id: finding.id.clone(),
                expected_revision: finding.revision,
                review_confirmation: None,
            };
            let candidates =
                resolve_analysis_candidates_for_cleanup(&db, &finding.run_id, &[selection], false)
                    .expect("legacy findings remain inspectable");
            let mut candidate = candidates[0].clone();
            assert_eq!(candidate.tier, CleanupTier::Review);
            assert_eq!(candidate.suggested_action, CleanupActionKind::Reveal);
            assert!(!candidate.trash_allowed);
            assert!(!candidate.selected_by_default);
            assert!(authorize_cleanup_candidate(&finding, &mut candidate, None).is_err());

            // Even a caller holding the old Safe projection cannot bypass the
            // independent authorization/preview/execution check.
            candidate.tier = CleanupTier::Safe;
            candidate.suggested_action = CleanupActionKind::MoveToTrash;
            candidate.trash_allowed = true;
            assert!(authorize_cleanup_candidate(&finding, &mut candidate, None).is_err());
            assert!(cleanup_preview_items_for_candidates(
                vec![candidate.id.clone()],
                std::slice::from_ref(&candidate),
            )
            .unwrap()
            .is_empty());
            assert!(preview_cleanup_operations_for_candidates(
                vec![candidate.id.clone()],
                std::slice::from_ref(&candidate),
                None,
            )
            .unwrap()
            .previews
            .is_empty());
            assert!(!cleanup_candidate_can_enter_operation_preview(
                &candidate, None
            ));
            assert!(
                !is_cleanup_execution_forbidden(&path, None),
                "ordinary user paths retain the separate explicit file-operation policy"
            );

            finding.tier = "review".to_string();
            finding.decision = Some("acknowledged".to_string());
            finding.decision_revision = Some(3);
            assert!(authorize_cleanup_candidate(
                &finding,
                &mut candidate,
                Some(&ReviewFindingConfirmation {
                    decision_revision: 3
                }),
            )
            .is_err());
            assert_eq!(fs::read(&manual_file).unwrap(), b"manual user content");
            drop(db);
            fs::remove_dir_all(root).expect("remove only task fixture");
        }
    }

    #[test]
    fn cleanup_selection_requires_finding_revision_and_review_decision_cas() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-analysis-cleanup-selection-{}",
            new_job_id("test")
        ));
        fs::create_dir_all(&root).expect("create cleanup selection root");
        let path = root.join("candidate.txt");
        fs::write(&path, b"1234567890").expect("write cleanup selection candidate");
        let db_path = root.join("database.sqlite3");
        let db = Database::open(&db_path).expect("open cleanup selection database");
        let finding = durable_cleanup_finding(&db, &path, "review", false);

        let stale = CleanupFindingSelection {
            finding_id: finding.id.clone(),
            expected_revision: finding.revision.saturating_sub(1),
            review_confirmation: None,
        };
        assert!(
            resolve_analysis_candidates_for_cleanup(&db, &finding.run_id, &[stale], true,).is_err()
        );

        let unconfirmed = CleanupFindingSelection {
            finding_id: finding.id.clone(),
            expected_revision: finding.revision,
            review_confirmation: None,
        };
        assert!(resolve_analysis_candidates_for_cleanup(
            &db,
            &finding.run_id,
            &[unconfirmed],
            true,
        )
        .is_err());

        let decision = db
            .set_analysis_finding_decision(
                &finding.finding_key,
                "acknowledged",
                None,
                Some("explicit review confirmation"),
                0,
            )
            .expect("acknowledge review finding");
        let confirmed = CleanupFindingSelection {
            finding_id: finding.id.clone(),
            expected_revision: finding.revision,
            review_confirmation: Some(ReviewFindingConfirmation {
                decision_revision: decision.revision,
            }),
        };
        let missing_assessment =
            resolve_analysis_candidates_for_cleanup(&db, &finding.run_id, &[confirmed], true)
                .expect_err("execution requires current AI assessment before review authorization");
        assert!(missing_assessment.contains("requires a current AI assessment"));

        let current_finding = db
            .get_analysis_finding(&finding.id)
            .expect("reload acknowledged finding")
            .expect("acknowledged finding exists");
        let mut candidate =
            storage_candidate_from_analysis_finding(&current_finding).expect("project candidate");
        authorize_cleanup_candidate(
            &current_finding,
            &mut candidate,
            Some(&ReviewFindingConfirmation {
                decision_revision: decision.revision,
            }),
        )
        .expect("review decision CAS remains independently valid");
        assert_eq!(candidate.tier, CleanupTier::Safe);
        assert_eq!(candidate.suggested_action, CleanupActionKind::MoveToTrash);

        drop(db);
        fs::remove_dir_all(root).expect("remove cleanup selection root");
    }
}
