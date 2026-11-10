use crate::{
    db::{
        current_unix_seconds, BuiltGroup, BuiltMember, Database, DbError, DedupeCandidate,
        DedupeCheckpoint, DedupeGroupDto, DedupeGroupMemberDto, DedupeGroupPageDto, DedupeRunDto,
        DedupeScopeRequest, FingerprintCas, FingerprintRow, PublishOutcome, StartDedupeRunRequest,
        PREHASH_MIN_SIZE, PREHASH_SAMPLE_BYTES,
    },
    fs_safety::{capture_physical_identity, PhysicalFileIdentity, PhysicalIdentityError},
    ids::new_job_id,
    scheduler::{AcquireError, CancellationToken, ResourceHints, WorkRequest, WorkScheduler},
    window_auth::require_main_window,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::sync_channel,
        Arc, Mutex,
    },
    thread,
    time::Instant,
};
use tauri::{AppHandle, Emitter, Runtime, State, WebviewWindow};
use thiserror::Error;

pub const DEDUPE_PROGRESS_EVENT: &str = "dedupe-progress";
pub const DEDUPE_COMPLETE_EVENT: &str = "dedupe-complete";

#[derive(Clone, Default)]
pub struct DedupeJobManager(Arc<Mutex<DedupeJobs>>);

#[derive(Default)]
struct DedupeJobs {
    jobs: HashMap<String, DedupeJob>,
    scan_to_dedupe: HashMap<String, String>,
}

struct DedupeJob {
    cancel_flag: Arc<AtomicBool>,
    parent_scan_job_id: Option<String>,
}

impl DedupeJobManager {
    fn register(
        &self,
        job_id: String,
        parent_scan_job_id: Option<String>,
    ) -> Result<Arc<AtomicBool>, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "Dedupe job manager is unavailable.".to_string())?;
        if state.jobs.contains_key(&job_id) {
            return Err(format!("Dedupe job already exists: {job_id}."));
        }
        let cancel_flag = Arc::new(AtomicBool::new(false));
        if let Some(scan_id) = parent_scan_job_id.as_ref() {
            state.scan_to_dedupe.insert(scan_id.clone(), job_id.clone());
        }
        state.jobs.insert(
            job_id,
            DedupeJob {
                cancel_flag: Arc::clone(&cancel_flag),
                parent_scan_job_id,
            },
        );
        Ok(cancel_flag)
    }

    pub fn cancel(&self, job_id: &str) -> bool {
        let job_id = job_id.trim();
        let canceled = self
            .0
            .lock()
            .ok()
            .and_then(|state| {
                state.jobs.get(job_id).map(|job| {
                    job.cancel_flag.store(true, Ordering::Release);
                    true
                })
            })
            .unwrap_or(false);
        if canceled {
            WorkScheduler::global().cancel_session(job_id);
        }
        canceled
    }

    pub fn cancel_for_scan(&self, scan_job_id: &str) -> bool {
        let job_id = self.0.lock().ok().and_then(|state| {
            state
                .scan_to_dedupe
                .get(scan_job_id.trim())
                .filter(|job_id| state.jobs.contains_key(*job_id))
                .cloned()
        });
        let Some(job_id) = job_id else {
            return false;
        };
        if let Ok(state) = self.0.lock() {
            if let Some(job) = state.jobs.get(&job_id) {
                job.cancel_flag.store(true, Ordering::Release);
            }
        }
        WorkScheduler::global().cancel_session(&job_id);
        true
    }

    pub fn cancel_all(&self) -> usize {
        let job_ids = {
            let Ok(state) = self.0.lock() else {
                return 0;
            };
            for job in state.jobs.values() {
                job.cancel_flag.store(true, Ordering::Release);
            }
            state.jobs.keys().cloned().collect::<Vec<_>>()
        };
        let canceled = job_ids.len();
        for job_id in job_ids {
            WorkScheduler::global().cancel_session(&job_id);
        }
        canceled
    }

    fn finish(&self, job_id: &str) {
        if let Ok(mut state) = self.0.lock() {
            if let Some(job) = state.jobs.remove(job_id) {
                if let Some(scan_id) = job.parent_scan_job_id {
                    if state
                        .scan_to_dedupe
                        .get(&scan_id)
                        .is_some_and(|mapped_job_id| mapped_job_id == job_id)
                    {
                        state.scan_to_dedupe.remove(&scan_id);
                    }
                }
            }
        }
    }

    fn contains(&self, job_id: &str) -> bool {
        self.0
            .lock()
            .map(|state| state.jobs.contains_key(job_id))
            .unwrap_or(false)
    }
}

#[derive(Debug, Error)]
pub enum DedupeError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("event emit failed: {0}")]
    Event(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupeProgressPayload {
    pub dedupe_job_id: String,
    pub parent_scan_job_id: Option<String>,
    pub processed: u64,
    pub total: u64,
    pub status: String,
    pub phase: String,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub revision: i64,
    pub warning_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupeCompletePayload {
    pub dedupe_job_id: String,
    pub parent_scan_job_id: Option<String>,
    pub status: String,
    pub candidate_files: u64,
    pub hashed_files: u64,
    pub duplicate_files: i64,
    pub skipped_files: u64,
    pub error_files: u64,
    pub duration_ms: u128,
    pub success: bool,
    pub error: Option<String>,
    pub phase: String,
    pub revision: i64,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub warning_count: u64,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DedupeSummary {
    pub candidate_files: u64,
    pub hashed_files: u64,
    pub duplicate_files: i64,
    pub skipped_files: u64,
    pub error_files: u64,
    pub duration_ms: u128,
    pub status: String,
    pub run_revision: i64,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub warning_count: u64,
    pub error_code: Option<String>,
}

impl DedupeSummary {
    fn complete_payload(
        &self,
        job_id: &str,
        parent_scan_job_id: Option<&str>,
        status: &str,
    ) -> DedupeCompletePayload {
        DedupeCompletePayload {
            dedupe_job_id: job_id.to_string(),
            parent_scan_job_id: parent_scan_job_id.map(str::to_string),
            status: status.to_string(),
            candidate_files: self.candidate_files,
            hashed_files: self.hashed_files,
            duplicate_files: self.duplicate_files,
            skipped_files: self.skipped_files,
            error_files: self.error_files,
            duration_ms: self.duration_ms,
            success: matches!(status, "completed" | "completed_with_warnings"),
            error: self.error_code.clone(),
            phase: "completed".to_string(),
            revision: self.run_revision,
            processed_bytes: self.processed_bytes,
            total_bytes: self.total_bytes,
            warning_count: self.warning_count,
            error_code: self.error_code.clone(),
        }
    }
}

pub trait ContentHasher {
    fn hash_file(&mut self, path: &Path) -> Result<String, DedupeError>;

    fn hash_file_with_bytes(&mut self, path: &Path) -> Result<(String, u64), DedupeError> {
        let hash = self.hash_file(path)?;
        let bytes = std::fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok((hash, bytes))
    }
}

pub struct Blake3ContentHasher;

impl ContentHasher for Blake3ContentHasher {
    fn hash_file(&mut self, path: &Path) -> Result<String, DedupeError> {
        hash_file_blake3(path)
    }

    fn hash_file_with_bytes(&mut self, path: &Path) -> Result<(String, u64), DedupeError> {
        hash_file_blake3_with_bytes(path)
    }
}

pub trait DedupeEventEmitter {
    fn emit_progress(&self, payload: &DedupeProgressPayload) -> Result<(), DedupeError>;
    fn emit_complete(&self, payload: &DedupeCompletePayload) -> Result<(), DedupeError>;
    fn emit_run_updated(&self, _run: &DedupeRunDto) -> Result<(), DedupeError> {
        Ok(())
    }
}

pub struct NoopDedupeEventEmitter;

impl DedupeEventEmitter for NoopDedupeEventEmitter {
    fn emit_progress(&self, _payload: &DedupeProgressPayload) -> Result<(), DedupeError> {
        Ok(())
    }

    fn emit_complete(&self, _payload: &DedupeCompletePayload) -> Result<(), DedupeError> {
        Ok(())
    }
}

pub struct TauriDedupeEventEmitter<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriDedupeEventEmitter<R> {
    fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> DedupeEventEmitter for TauriDedupeEventEmitter<R> {
    fn emit_progress(&self, payload: &DedupeProgressPayload) -> Result<(), DedupeError> {
        self.app
            .emit(DEDUPE_PROGRESS_EVENT, payload.clone())
            .map_err(|error| DedupeError::Event(error.to_string()))
    }

    fn emit_complete(&self, payload: &DedupeCompletePayload) -> Result<(), DedupeError> {
        self.app
            .emit(DEDUPE_COMPLETE_EVENT, payload.clone())
            .map_err(|error| DedupeError::Event(error.to_string()))
    }

    fn emit_run_updated(&self, run: &DedupeRunDto) -> Result<(), DedupeError> {
        self.app
            .emit("dedupe-run-updated", run.clone())
            .map_err(|error| DedupeError::Event(error.to_string()))
    }
}

pub fn run_duplicate_detection(
    db: &Database,
    emitter: &impl DedupeEventEmitter,
) -> Result<DedupeSummary, DedupeError> {
    let mut hasher = Blake3ContentHasher;
    run_duplicate_detection_with_hasher(db, emitter, &mut hasher)
}

pub fn run_duplicate_detection_with_hasher(
    db: &Database,
    emitter: &impl DedupeEventEmitter,
    hasher: &mut impl ContentHasher,
) -> Result<DedupeSummary, DedupeError> {
    let request = StartDedupeRunRequest {
        scope: DedupeScopeRequest {
            kind: "allManagedFileLibrary".to_string(),
            root_ids: Vec::new(),
        },
        request_key: Some(format!("legacy-api:{}", new_job_id("request"))),
        parent_scan_session_id: None,
    };
    match db.start_dedupe_run(&request) {
        Ok(admission) => {
            if !admission.created {
                return Ok(summary_from_run(&admission.run));
            }
            let cancel_flag = Arc::new(AtomicBool::new(false));
            run_durable_dedupe_with_custom_hasher(db, emitter, &admission.run, &cancel_flag, hasher)
        }
        Err(error) => Err(DedupeError::Db(error)),
    }
}

#[derive(Debug, Clone)]
struct DurableCandidate {
    candidate: DedupeCandidate,
    identity: PhysicalFileIdentity,
    fingerprint: FingerprintRow,
}

#[derive(Debug, Clone)]
struct HashSubject {
    file_ids: Vec<String>,
    representative_id: String,
    path: PathBuf,
    size: i64,
    prehash: String,
    full_hash: Option<String>,
    members: Vec<DurableCandidate>,
}

#[derive(Debug)]
struct HashTask {
    subject_index: usize,
    path: PathBuf,
    expected_identity: PhysicalFileIdentity,
}

#[derive(Debug)]
struct HashResult {
    subject_index: usize,
    result: Result<String, DedupeError>,
    bytes_read: u64,
}

fn run_durable_dedupe_with_custom_hasher(
    db: &Database,
    emitter: &impl DedupeEventEmitter,
    requested_run: &DedupeRunDto,
    cancel_flag: &Arc<AtomicBool>,
    hasher: &mut impl ContentHasher,
) -> Result<DedupeSummary, DedupeError> {
    run_durable_dedupe_inner(
        db,
        emitter,
        requested_run,
        cancel_flag,
        Some(hasher as &mut dyn ContentHasher),
    )
}

fn run_durable_dedupe(
    db: &Database,
    emitter: &impl DedupeEventEmitter,
    run_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<DedupeSummary, DedupeError> {
    let Some(run) = db.claim_dedupe_run(run_id)? else {
        return Ok(summary_from_run(&db.get_dedupe_run(run_id)?));
    };
    run_durable_dedupe_inner(db, emitter, &run, cancel_flag, None)
}

fn run_durable_dedupe_inner(
    db: &Database,
    emitter: &impl DedupeEventEmitter,
    requested_run: &DedupeRunDto,
    cancel_flag: &Arc<AtomicBool>,
    custom_hasher: Option<&mut dyn ContentHasher>,
) -> Result<DedupeSummary, DedupeError> {
    let started_at = Instant::now();
    let mut run = if requested_run.status == "queued" {
        db.claim_dedupe_run(&requested_run.id)?.ok_or_else(|| {
            DedupeError::Db(DbError::Validation(
                "Dedupe run claim was lost.".to_string(),
            ))
        })?
    } else {
        requested_run.clone()
    };
    let parent_scan_job_id = run.parent_scan_session_id.clone();
    let mut checkpoint = DedupeCheckpoint {
        phase: "collecting".to_string(),
        ..DedupeCheckpoint::default()
    };
    emit_checkpoint(db, emitter, &mut run, &checkpoint)?;

    let candidates = db.collect_dedupe_candidates(&run.scope)?;
    checkpoint.candidate_files = i64::try_from(candidates.len()).unwrap_or(i64::MAX);
    checkpoint.candidate_bytes = candidates
        .iter()
        .map(|candidate| candidate.size.max(0))
        .fold(0_i64, i64::saturating_add);
    // Metadata and physical-identity reads are not content-byte IO.  The
    // byte budget is populated only after the prehash/full-hash work is
    // known, so a scan cannot report a fake 100% content completion while it
    // is still hashing.
    checkpoint.total_bytes = 0;
    emit_checkpoint(db, emitter, &mut run, &checkpoint)?;

    let mut by_subject = HashMap::<String, Vec<DurableCandidate>>::new();
    let mut identity_processed = 0_i64;
    for candidate in candidates {
        if should_cancel(db, &run.id, cancel_flag.as_ref())? {
            return finish_cancelled(db, emitter, &run, checkpoint, started_at);
        }
        #[cfg(target_os = "macos")]
        let content_read_reason = {
            let eligibility = crate::platform::macos::file_semantics::content_read_eligibility(
                Path::new(&candidate.path),
            );
            (!eligibility.is_eligible()).then_some(eligibility.reason())
        };
        #[cfg(not(target_os = "macos"))]
        let content_read_reason: Option<&'static str> = None;
        if let Some(content_read_reason) = content_read_reason {
            checkpoint.identity_unknown_files += 1;
            checkpoint.warning_count += 1;
            checkpoint.error_count += 1;
            record_dedupe_error(
                db,
                &mut run,
                Some(&candidate.file_id),
                &candidate.path,
                "capturing_identity",
                content_read_reason,
                "Content bytes are not eligible or the source is not a regular file; duplicate detection deferred.",
            )?;
            checkpoint.processed_files = checkpoint.processed_files.saturating_add(1);
            if checkpoint.processed_files % 64 == 0 {
                emit_checkpoint(db, emitter, &mut run, &checkpoint)?;
            }
            continue;
        }
        match capture_physical_identity(Path::new(&candidate.path)) {
            Ok(identity) => {
                let mut fingerprint = db.upsert_physical_identity(&candidate, &identity)?;
                if fingerprint.full_hash.is_none() {
                    if let Some(cached) =
                        db.find_cached_fingerprint_by_physical(&identity, &candidate.file_id)?
                    {
                        let _ =
                            db.copy_cached_fingerprint_for_rename(&candidate, &identity, &cached)?;
                        fingerprint = db.get_fingerprint(&candidate.file_id)?.ok_or_else(|| {
                            DbError::Validation("Fingerprint cache copy lost its row.".to_string())
                        })?;
                    }
                }
                let subject_key = identity
                    .physical_key
                    .clone()
                    .unwrap_or_else(|| format!("path:{}", candidate.file_id));
                by_subject
                    .entry(subject_key.clone())
                    .or_default()
                    .push(DurableCandidate {
                        candidate,
                        identity,
                        fingerprint,
                    });
                identity_processed += 1;
            }
            Err(error) => {
                checkpoint.identity_unknown_files += 1;
                checkpoint.warning_count += 1;
                checkpoint.error_count += 1;
                let (identity_status, fingerprint_status) = match &error {
                    PhysicalIdentityError::Missing => ("missing", "missing"),
                    PhysicalIdentityError::UnsupportedLink
                    | PhysicalIdentityError::UnsupportedType => ("unsupported", "unsupported"),
                    PhysicalIdentityError::Io(_) => ("error", "error"),
                };
                db.record_fingerprint_error(
                    &candidate,
                    identity_status,
                    fingerprint_status,
                    physical_identity_error_code(&error),
                    &error.to_string(),
                )?;
                record_dedupe_error(
                    db,
                    &mut run,
                    Some(&candidate.file_id),
                    &candidate.path,
                    "capturing_identity",
                    physical_identity_error_code(&error),
                    &error.to_string(),
                )?;
            }
        }
        checkpoint.processed_files = checkpoint.processed_files.saturating_add(1);
        if checkpoint.processed_files % 64 == 0 {
            emit_checkpoint(db, emitter, &mut run, &checkpoint)?;
        }
    }
    checkpoint.candidate_physical_objects = i64::try_from(by_subject.len()).unwrap_or(i64::MAX);
    checkpoint.identity_verified_files = identity_processed;
    checkpoint.hardlink_aliases = by_subject
        .values()
        .filter(|members| members.len() > 1 && members[0].identity.physical_key.is_some())
        .map(|members| i64::try_from(members.len() - 1).unwrap_or(i64::MAX))
        .fold(0_i64, i64::saturating_add);
    checkpoint.phase = "capturing_identity".to_string();
    emit_checkpoint(db, emitter, &mut run, &checkpoint)?;

    let mut subjects = Vec::<HashSubject>::with_capacity(by_subject.len());
    let mut prehash_io_bytes = 0_u64;
    for (_subject_key, mut members) in by_subject {
        members.sort_by(|left, right| {
            left.candidate
                .path
                .to_ascii_lowercase()
                .cmp(&right.candidate.path.to_ascii_lowercase())
                .then_with(|| left.candidate.file_id.cmp(&right.candidate.file_id))
        });
        let representative = members
            .first()
            .ok_or_else(|| DbError::Validation("Dedupe subject unexpectedly empty.".to_string()))?;
        let mut subject = HashSubject {
            file_ids: members
                .iter()
                .map(|member| member.candidate.file_id.clone())
                .collect(),
            representative_id: representative.candidate.file_id.clone(),
            path: PathBuf::from(&representative.candidate.path),
            size: representative.candidate.size,
            prehash: String::new(),
            full_hash: members
                .iter()
                .find_map(|member| member.fingerprint.full_hash.clone()),
            members,
        };
        if let Some(prehash) = subject.members.iter().find_map(|member| {
            (member.fingerprint.prehash_algorithm == "blake3-head-tail"
                && member.fingerprint.prehash_version == 1)
                .then(|| member.fingerprint.prehash.clone())
                .flatten()
        }) {
            subject.prehash = prehash;
        } else if subject.size >= PREHASH_MIN_SIZE {
            match hash_file_prehash_with_identity(
                &subject.path,
                subject.size,
                &subject.members[0].identity,
            ) {
                Ok((prehash, bytes_read)) => {
                    subject.prehash = prehash.clone();
                    prehash_io_bytes = prehash_io_bytes.saturating_add(bytes_read);
                    checkpoint.processed_bytes = checkpoint
                        .processed_bytes
                        .saturating_add(i64::try_from(bytes_read).unwrap_or(i64::MAX));
                    let entries = subject
                        .members
                        .iter()
                        .map(|member| FingerprintCas {
                            file_id: member.candidate.file_id.clone(),
                            path_snapshot: member.candidate.path.clone(),
                            size: member.candidate.size,
                            indexed_mtime: member.candidate.mtime,
                            modified_ns: member.identity.modified_ns,
                            physical_key: member.identity.physical_key.clone(),
                            expected_revision: member.fingerprint.revision,
                        })
                        .collect::<Vec<_>>();
                    let updated = db.save_prehash(&entries, &prehash)?;
                    if updated != entries.len() {
                        checkpoint.warning_count += 1;
                        checkpoint.error_count += 1;
                        record_dedupe_error(
                            db,
                            &mut run,
                            Some(&subject.representative_id),
                            &subject.path.to_string_lossy(),
                            "prehashing",
                            "fingerprint_cas_miss",
                            "Fingerprint prehash persistence lost its file or revision CAS.",
                        )?;
                        continue;
                    }
                    checkpoint.prehashed_files +=
                        i64::try_from(subject.file_ids.len()).unwrap_or(i64::MAX);
                }
                Err(error) => {
                    checkpoint.warning_count += 1;
                    checkpoint.error_count += 1;
                    record_dedupe_error(
                        db,
                        &mut run,
                        Some(&subject.representative_id),
                        &subject.path.to_string_lossy(),
                        "prehashing",
                        "prehash_read_failed",
                        &error.to_string(),
                    )?;
                    continue;
                }
            }
        }
        subjects.push(subject);
    }
    checkpoint.phase = "prehashing".to_string();
    emit_checkpoint(db, emitter, &mut run, &checkpoint)?;

    let mut prehash_buckets = HashMap::<(i64, String), Vec<usize>>::new();
    for (index, subject) in subjects.iter().enumerate() {
        let bucket_key = if subject.size < PREHASH_MIN_SIZE {
            // Small files intentionally skip the sample stage and proceed to
            // one full read.  They must never be pruned by an empty or
            // synthetic prehash bucket.
            "__small_file_full_hash__".to_string()
        } else {
            subject.prehash.clone()
        };
        prehash_buckets
            .entry((subject.size, bucket_key))
            .or_default()
            .push(index);
    }
    checkpoint.prehash_pruned_files = prehash_buckets
        .values()
        .filter(|indexes| {
            indexes.len() == 1
                && subjects[indexes[0]].size >= PREHASH_MIN_SIZE
                && subjects[indexes[0]].full_hash.is_none()
        })
        .map(|indexes| subjects[indexes[0]].file_ids.len() as i64)
        .fold(0_i64, i64::saturating_add);
    let mut hash_indexes = Vec::new();
    for indexes in prehash_buckets.values() {
        if indexes.len() > 1 || subjects[indexes[0]].size < PREHASH_MIN_SIZE {
            hash_indexes.extend(
                indexes
                    .iter()
                    .copied()
                    .filter(|index| subjects[*index].full_hash.is_none()),
            );
        }
    }
    hash_indexes.sort_unstable();
    hash_indexes.dedup();
    let full_hash_io_budget = hash_indexes.iter().fold(0_u64, |total, index| {
        total.saturating_add(subjects[*index].size.max(0).try_into().unwrap_or(u64::MAX))
    });
    checkpoint.total_bytes =
        i64::try_from(prehash_io_bytes.saturating_add(full_hash_io_budget)).unwrap_or(i64::MAX);

    checkpoint.phase = "full_hashing".to_string();
    emit_checkpoint(db, emitter, &mut run, &checkpoint)?;
    let mut hash_results = Vec::new();
    if let Some(hasher) = custom_hasher {
        for index in hash_indexes.iter().copied() {
            if should_cancel(db, &run.id, cancel_flag.as_ref())? {
                break;
            }
            let subject = &subjects[index];
            let before = capture_physical_identity(&subject.path);
            let result = match before {
                Ok(identity) if identity == subject.members[0].identity => {
                    hasher.hash_file_with_bytes(&subject.path)
                }
                Ok(_) => Err(DedupeError::Db(DbError::Validation(
                    "file_changed_before_hash".to_string(),
                ))),
                Err(error) => Err(DedupeError::Io {
                    path: subject.path.to_string_lossy().into_owned(),
                    source: std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string()),
                }),
            };
            let result = match result {
                Ok((hash, bytes_read)) => match capture_physical_identity(&subject.path) {
                    Ok(after) if after == subject.members[0].identity => Ok((hash, bytes_read)),
                    Ok(_) => Err(DedupeError::Db(DbError::Validation(
                        "file_changed_during_hash".to_string(),
                    ))),
                    Err(error) => Err(DedupeError::Io {
                        path: subject.path.to_string_lossy().into_owned(),
                        source: std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            error.to_string(),
                        ),
                    }),
                },
                Err(error) => Err(error),
            };
            let (result, bytes_read) = match result {
                Ok((hash, bytes_read)) => (Ok(hash), bytes_read),
                Err(error) => (Err(error), 0),
            };
            hash_results.push(HashResult {
                subject_index: index,
                result,
                bytes_read,
            });
        }
    } else {
        let tasks = hash_indexes
            .iter()
            .map(|index| HashTask {
                subject_index: *index,
                path: subjects[*index].path.clone(),
                expected_identity: subjects[*index].members[0].identity.clone(),
            })
            .collect::<Vec<_>>();
        let (results, invalid_worker_config) =
            bounded_hash_subjects(tasks, Arc::clone(cancel_flag), &run.id)?;
        if invalid_worker_config {
            checkpoint.warning_count += 1;
            record_dedupe_warning(
                db,
                &mut run,
                "full_hashing",
                "invalid_dedupe_worker_count",
                "ZEN_CANVAS_DEDUPE_HASH_WORKERS was invalid; the bounded default was used.",
            )?;
        }
        hash_results = results;
    }
    for hash_result in hash_results {
        checkpoint.processed_bytes = checkpoint
            .processed_bytes
            .saturating_add(i64::try_from(hash_result.bytes_read).unwrap_or(i64::MAX));
        let subject = &mut subjects[hash_result.subject_index];
        match hash_result.result {
            Ok(hash) => {
                let mut entries = Vec::with_capacity(subject.members.len());
                for member in &subject.members {
                    let current =
                        db.get_fingerprint(&member.candidate.file_id)?
                            .ok_or_else(|| {
                                DbError::Validation(
                                    "Fingerprint row disappeared before full-hash CAS.".to_string(),
                                )
                            })?;
                    entries.push((
                        FingerprintCas {
                            file_id: member.candidate.file_id.clone(),
                            path_snapshot: member.candidate.path.clone(),
                            size: member.candidate.size,
                            indexed_mtime: member.candidate.mtime,
                            modified_ns: member.identity.modified_ns,
                            physical_key: member.identity.physical_key.clone(),
                            expected_revision: current.revision,
                        },
                        hash.clone(),
                    ));
                }
                let updated = db.save_full_hash(&entries)?;
                if updated == entries.len() {
                    subject.full_hash = Some(hash.clone());
                    checkpoint.full_hashed_files +=
                        i64::try_from(entries.len()).unwrap_or(i64::MAX);
                } else {
                    checkpoint.warning_count += 1;
                    checkpoint.error_count += 1;
                    record_dedupe_error(
                        db,
                        &mut run,
                        Some(&subject.representative_id),
                        &subject.path.to_string_lossy(),
                        "full_hashing",
                        "fingerprint_cas_miss",
                        "Fingerprint full-hash persistence lost its file or revision CAS.",
                    )?;
                    subject.full_hash = None;
                }
            }
            Err(error) => {
                checkpoint.warning_count += 1;
                checkpoint.error_count += 1;
                record_dedupe_error(
                    db,
                    &mut run,
                    Some(&subject.representative_id),
                    &subject.path.to_string_lossy(),
                    "full_hashing",
                    hash_error_code(&error),
                    &error.to_string(),
                )?;
            }
        }
    }
    emit_checkpoint(db, emitter, &mut run, &checkpoint)?;
    if should_cancel(db, &run.id, cancel_flag.as_ref())? {
        return finish_cancelled(db, emitter, &run, checkpoint, started_at);
    }

    checkpoint.phase = "building_groups".to_string();
    let groups = build_duplicate_groups(&subjects);
    checkpoint.duplicate_groups = i64::try_from(groups.len()).unwrap_or(i64::MAX);
    checkpoint.duplicate_members = groups
        .iter()
        .map(|group| i64::try_from(group.members.len()).unwrap_or(i64::MAX))
        .fold(0_i64, i64::saturating_add);
    checkpoint.exact_reclaimable_bytes = groups
        .iter()
        .filter_map(|group| group.exact_reclaimable_bytes)
        .fold(0_i64, i64::saturating_add);
    checkpoint.potential_reclaimable_bytes = groups
        .iter()
        .map(|group| group.potential_reclaimable_bytes)
        .fold(0_i64, i64::saturating_add);
    emit_checkpoint(db, emitter, &mut run, &checkpoint)?;
    if should_cancel(db, &run.id, cancel_flag.as_ref())? {
        return finish_cancelled(db, emitter, &run, checkpoint, started_at);
    }

    checkpoint.phase = "finalizing".to_string();
    emit_checkpoint(db, emitter, &mut run, &checkpoint)?;
    let publish_outcome = db.publish_dedupe_groups(&run.id, &groups, &checkpoint)?;
    let final_run = db.get_dedupe_run(&run.id)?;
    emitter.emit_run_updated(&final_run)?;
    let status = match publish_outcome {
        PublishOutcome::Completed => "completed",
        PublishOutcome::CompletedWithWarnings => "completed_with_warnings",
        PublishOutcome::Cancelled => "cancelled",
    };
    let summary = summary_from_run(&final_run);
    emitter.emit_complete(&summary.complete_payload(
        &run.id,
        parent_scan_job_id.as_deref(),
        status,
    ))?;
    Ok(DedupeSummary {
        duration_ms: started_at.elapsed().as_millis(),
        ..summary
    })
}

fn record_dedupe_error(
    db: &Database,
    run: &mut DedupeRunDto,
    file_id: Option<&str>,
    path: &str,
    phase: &str,
    error_code: &str,
    error_message: &str,
) -> Result<(), DedupeError> {
    *run = db.record_dedupe_error(
        &run.id,
        run.revision,
        file_id,
        path,
        phase,
        error_code,
        error_message,
    )?;
    Ok(())
}

fn record_dedupe_warning(
    db: &Database,
    run: &mut DedupeRunDto,
    phase: &str,
    error_code: &str,
    error_message: &str,
) -> Result<(), DedupeError> {
    *run = db.record_dedupe_warning(&run.id, run.revision, phase, error_code, error_message)?;
    Ok(())
}

fn emit_checkpoint(
    db: &Database,
    emitter: &impl DedupeEventEmitter,
    run: &mut DedupeRunDto,
    checkpoint: &DedupeCheckpoint,
) -> Result<(), DedupeError> {
    let updated = db.checkpoint_dedupe_run(&run.id, run.revision, checkpoint)?;
    *run = updated.clone();
    emitter.emit_progress(&DedupeProgressPayload {
        dedupe_job_id: updated.id.clone(),
        parent_scan_job_id: updated.parent_scan_session_id.clone(),
        processed: checkpoint.processed_files.max(0) as u64,
        total: checkpoint.candidate_files.max(0) as u64,
        status: "running".to_string(),
        phase: checkpoint.phase.clone(),
        processed_bytes: checkpoint.processed_bytes.max(0) as u64,
        total_bytes: checkpoint.total_bytes.max(0) as u64,
        revision: updated.revision,
        warning_count: checkpoint.warning_count.max(0) as u64,
        error_count: checkpoint.error_count.max(0) as u64,
    })?;
    emitter.emit_run_updated(&updated)?;
    Ok(())
}

fn should_cancel(db: &Database, run_id: &str, flag: &AtomicBool) -> Result<bool, DedupeError> {
    Ok(flag.load(Ordering::Acquire) || db.is_dedupe_cancel_requested(run_id)?)
}

fn finish_cancelled(
    db: &Database,
    emitter: &impl DedupeEventEmitter,
    run: &DedupeRunDto,
    checkpoint: DedupeCheckpoint,
    started_at: Instant,
) -> Result<DedupeSummary, DedupeError> {
    let latest = db.get_dedupe_run(&run.id)?;
    let final_run = db.mark_dedupe_terminal(
        &run.id,
        latest.revision,
        "cancelled",
        Some("cancelled"),
        Some("Duplicate detection was cancelled before publication."),
        &checkpoint,
    )?;
    emitter.emit_run_updated(&final_run)?;
    let mut summary = summary_from_run(&final_run);
    summary.duration_ms = started_at.elapsed().as_millis();
    emitter.emit_complete(&summary.complete_payload(
        &run.id,
        run.parent_scan_session_id.as_deref(),
        "cancelled",
    ))?;
    Ok(summary)
}

fn summary_from_run(run: &DedupeRunDto) -> DedupeSummary {
    DedupeSummary {
        candidate_files: run.candidate_files.max(0) as u64,
        hashed_files: run.full_hashed_files.max(0) as u64,
        duplicate_files: run.duplicate_members,
        skipped_files: (run.candidate_files - run.full_hashed_files).max(0) as u64,
        error_files: run.error_count.max(0) as u64,
        duration_ms: 0,
        status: run.status.clone(),
        run_revision: run.revision,
        processed_bytes: run.processed_bytes.max(0) as u64,
        total_bytes: run.total_bytes.max(0) as u64,
        warning_count: run.warning_count.max(0) as u64,
        error_code: run.error_code.clone(),
    }
}

fn physical_identity_error_code(error: &PhysicalIdentityError) -> &'static str {
    match error {
        PhysicalIdentityError::Missing => "source_missing",
        PhysicalIdentityError::UnsupportedLink => "unsupported_link",
        PhysicalIdentityError::UnsupportedType => "unsupported_type",
        PhysicalIdentityError::Io(_) => "identity_io_failed",
    }
}

fn hash_error_code(error: &DedupeError) -> &'static str {
    match error {
        DedupeError::Io { .. } => "hash_io_failed",
        DedupeError::Db(DbError::Validation(message)) if message == "file_changed_during_hash" => {
            "file_changed_during_hash"
        }
        DedupeError::Db(DbError::Validation(message)) if message == "file_changed_before_hash" => {
            "file_changed_before_hash"
        }
        DedupeError::Db(DbError::Validation(message))
            if message == "file_changed_during_prehash" =>
        {
            "file_changed_during_prehash"
        }
        DedupeError::Db(DbError::Validation(message))
            if message == "file_changed_before_prehash" =>
        {
            "file_changed_before_prehash"
        }
        _ => "hash_failed",
    }
}

#[allow(dead_code)]
fn hash_file_prehash(path: &Path, expected_size: i64) -> Result<String, DedupeError> {
    hash_file_prehash_bytes(path, expected_size).map(|(hash, _)| hash)
}

fn hash_file_prehash_with_identity(
    path: &Path,
    expected_size: i64,
    expected_identity: &PhysicalFileIdentity,
) -> Result<(String, u64), DedupeError> {
    match capture_physical_identity(path) {
        Ok(identity) if identity == *expected_identity => {}
        Ok(_) => {
            return Err(DedupeError::Db(DbError::Validation(
                "file_changed_before_prehash".to_string(),
            )))
        }
        Err(error) => {
            return Err(DedupeError::Io {
                path: path.to_string_lossy().into_owned(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string()),
            })
        }
    }
    let result = hash_file_prehash_bytes(path, expected_size)?;
    match capture_physical_identity(path) {
        Ok(identity) if identity == *expected_identity => Ok(result),
        Ok(_) => Err(DedupeError::Db(DbError::Validation(
            "file_changed_during_prehash".to_string(),
        ))),
        Err(error) => Err(DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string()),
        }),
    }
}

fn hash_file_prehash_bytes(path: &Path, expected_size: i64) -> Result<(String, u64), DedupeError> {
    let mut file = open_content_file(path)?;
    #[cfg(windows)]
    crate::platform::windows::cloud_metadata::ensure_local_content(&file).map_err(|source| {
        DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        }
    })?;
    let file_size = u64::try_from(expected_size.max(0)).unwrap_or(0);
    let sample = if (0..PREHASH_MIN_SIZE).contains(&expected_size) {
        u64::try_from(expected_size).unwrap_or(0)
    } else {
        u64::try_from(PREHASH_SAMPLE_BYTES).unwrap_or(4096)
    };
    let head_len = file_size.min(sample);
    let tail_start = file_size.saturating_sub(sample);
    let mut head = vec![0_u8; head_len as usize];
    file.read_exact(&mut head)
        .map_err(|source| DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        })?;
    let mut tail = Vec::new();
    if tail_start > head_len {
        file.seek(SeekFrom::Start(tail_start))
            .map_err(|source| DedupeError::Io {
                path: path.to_string_lossy().into_owned(),
                source,
            })?;
        tail.resize((file_size - tail_start) as usize, 0);
        file.read_exact(&mut tail)
            .map_err(|source| DedupeError::Io {
                path: path.to_string_lossy().into_owned(),
                source,
            })?;
    }
    #[cfg(windows)]
    crate::platform::windows::cloud_metadata::ensure_local_content(&file).map_err(|source| {
        DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        }
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"zen-canvas-prehash:v1\0");
    hasher.update(&expected_size.to_le_bytes());
    hasher.update(&head);
    hasher.update(&tail);
    Ok((
        hasher.finalize().to_hex().to_string(),
        head.len().saturating_add(tail.len()) as u64,
    ))
}

fn bounded_hash_subjects(
    tasks: Vec<HashTask>,
    cancel_flag: Arc<AtomicBool>,
    run_id: &str,
) -> Result<(Vec<HashResult>, bool), DedupeError> {
    if tasks.is_empty() {
        return Ok((Vec::new(), false));
    }
    let scheduler = WorkScheduler::global();
    let detected = scheduler.config().capacities.cpu as usize;
    let (workers, invalid_override) = dedupe_worker_count(
        detected,
        std::env::var("ZEN_CANVAS_DEDUPE_HASH_WORKERS")
            .ok()
            .as_deref(),
    );
    let leases = match acquire_dedupe_hash_leases(&scheduler, run_id, &cancel_flag, workers) {
        Ok(leases) => leases,
        Err(AcquireError::Cancelled) if cancel_flag.load(Ordering::Acquire) => {
            return Ok((Vec::new(), invalid_override));
        }
        Err(error) => return Err(DedupeError::Db(DbError::Validation(error.to_string()))),
    };
    if cancel_flag.load(Ordering::Acquire) {
        return Ok((Vec::new(), invalid_override));
    }
    let results = bounded_hash_subjects_with_workers(tasks, cancel_flag, leases.len())?;
    drop(leases);
    Ok((results, invalid_override))
}

fn acquire_dedupe_hash_leases(
    scheduler: &WorkScheduler,
    run_id: &str,
    cancel_flag: &Arc<AtomicBool>,
    workers: usize,
) -> Result<Vec<crate::scheduler::ResourceLease>, AcquireError> {
    let cancellation = CancellationToken::from_flag(Arc::clone(cancel_flag));
    let first_request = dedupe_hash_resource_request(run_id, 0, cancellation.clone());
    let first_lease = scheduler.acquire_with_backpressure(first_request)?;
    let mut leases = Vec::with_capacity(workers.max(1));
    leases.push(first_lease);
    for index in 1..workers.max(1) {
        if cancel_flag.load(Ordering::Acquire) {
            break;
        }
        match scheduler.try_acquire(dedupe_hash_resource_request(
            run_id,
            index,
            cancellation.clone(),
        )) {
            Ok(lease) => leases.push(lease),
            Err(
                AcquireError::WouldBlock | AcquireError::QueueFull | AcquireError::PolicyDenied,
            ) => break,
            Err(AcquireError::Cancelled) if cancel_flag.load(Ordering::Acquire) => break,
            Err(error) => return Err(error),
        }
    }
    Ok(leases)
}

fn dedupe_hash_resource_request(
    run_id: &str,
    worker_index: usize,
    cancellation: CancellationToken,
) -> WorkRequest {
    WorkRequest::new(
        format!("dedupe-{run_id}-hash-{worker_index}"),
        crate::file_workspace::WorkClass::Background,
        ResourceHints {
            cpu: 1,
            io: 1,
            open_handles: 1,
            ..ResourceHints::empty()
        },
    )
    .with_session_id(run_id.to_string())
    .with_cancellation(cancellation)
}

fn bounded_hash_subjects_with_workers(
    tasks: Vec<HashTask>,
    cancel_flag: Arc<AtomicBool>,
    workers: usize,
) -> Result<Vec<HashResult>, DedupeError> {
    bounded_hash_subjects_with_workers_and_qos(tasks, cancel_flag, workers, || {
        let _ = crate::resource_governor::apply_thread_qos(
            crate::file_workspace::WorkClass::Background,
        );
    })
}

fn bounded_hash_subjects_with_workers_and_qos<F>(
    tasks: Vec<HashTask>,
    cancel_flag: Arc<AtomicBool>,
    workers: usize,
    apply_worker_qos: F,
) -> Result<Vec<HashResult>, DedupeError>
where
    F: Fn() + Send + Sync + 'static,
{
    let workers = workers.max(1);
    if tasks.is_empty() {
        return Ok(Vec::new());
    }
    let (task_tx, task_rx) = sync_channel::<Option<HashTask>>(workers.saturating_mul(2).max(1));
    let (result_tx, result_rx) = std::sync::mpsc::channel::<HashResult>();
    let shared_rx = Arc::new(Mutex::new(task_rx));
    let apply_worker_qos = Arc::new(apply_worker_qos);
    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let shared_rx = Arc::clone(&shared_rx);
        let result_tx = result_tx.clone();
        let cancel_flag = Arc::clone(&cancel_flag);
        let apply_worker_qos = Arc::clone(&apply_worker_qos);
        handles.push(thread::spawn(move || {
            apply_worker_qos();
            loop {
                let task = {
                    let Ok(receiver) = shared_rx.lock() else {
                        return;
                    };
                    receiver.recv()
                };
                let Ok(Some(task)) = task else { return };
                // The flag is owned by the run and outlives the bounded worker set.
                let cancelled = cancel_flag.load(Ordering::Acquire);
                let (result, bytes_read) = if cancelled {
                    (
                        Err(DedupeError::Db(DbError::Validation(
                            "cancelled".to_string(),
                        ))),
                        0,
                    )
                } else {
                    let result = hash_subject_with_identity(
                        &task.path,
                        &task.expected_identity,
                        cancel_flag.as_ref(),
                    );
                    let (result, bytes_read) = match result {
                        Ok((hash, bytes_read)) => (Ok(hash), bytes_read),
                        Err(error) => (Err(error), 0),
                    };
                    (result, bytes_read)
                };
                if result_tx
                    .send(HashResult {
                        subject_index: task.subject_index,
                        result,
                        bytes_read,
                    })
                    .is_err()
                {
                    return;
                }
            }
        }));
    }
    drop(result_tx);
    for task in tasks {
        if cancel_flag.load(Ordering::Acquire) {
            break;
        }
        task_tx.send(Some(task)).map_err(|_| {
            DedupeError::Db(DbError::Validation("hash worker queue closed".to_string()))
        })?;
    }
    for _ in 0..workers {
        let _ = task_tx.send(None);
    }
    drop(task_tx);
    let mut results = Vec::new();
    while let Ok(result) = result_rx.recv() {
        results.push(result);
    }
    for handle in handles {
        let _ = handle.join();
    }
    Ok(results)
}

fn hash_subject_with_identity(
    path: &Path,
    expected_identity: &PhysicalFileIdentity,
    cancel_flag: &AtomicBool,
) -> Result<(String, u64), DedupeError> {
    let before = capture_physical_identity(path);
    let result = match before {
        Ok(identity) if identity == *expected_identity => {
            hash_file_blake3_cancellable_with_bytes(path, cancel_flag)
        }
        Ok(_) => Err(DedupeError::Db(DbError::Validation(
            "file_changed_before_hash".to_string(),
        ))),
        Err(error) => Err(DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string()),
        }),
    }?;
    match capture_physical_identity(path) {
        Ok(identity) if identity == *expected_identity => Ok(result),
        Ok(_) => Err(DedupeError::Db(DbError::Validation(
            "file_changed_during_hash".to_string(),
        ))),
        Err(error) => Err(DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string()),
        }),
    }
}

fn dedupe_worker_count(detected: usize, configured: Option<&str>) -> (usize, bool) {
    let available = detected.max(1);
    let default_workers = available.min(4);
    match configured {
        None => (default_workers, false),
        Some(value) => match value.parse::<usize>() {
            Ok(requested) if (1..=8).contains(&requested) => (requested.min(available), false),
            _ => (default_workers, true),
        },
    }
}

fn hash_file_blake3_cancellable_with_bytes(
    path: &Path,
    cancel_flag: &AtomicBool,
) -> Result<(String, u64), DedupeError> {
    let mut file = open_content_file(path)?;
    #[cfg(windows)]
    crate::platform::windows::cloud_metadata::ensure_local_content(&file).map_err(|source| {
        DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        }
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 1024 * 1024];
    let mut bytes_read = 0_u64;
    loop {
        if cancel_flag.load(Ordering::Acquire) {
            return Err(DedupeError::Db(DbError::Validation(
                "cancelled".to_string(),
            )));
        }
        let read = file.read(&mut buffer).map_err(|source| DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes_read = bytes_read.saturating_add(read as u64);
    }
    #[cfg(windows)]
    crate::platform::windows::cloud_metadata::ensure_local_content(&file).map_err(|source| {
        DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        }
    })?;
    Ok((hasher.finalize().to_hex().to_string(), bytes_read))
}

fn build_duplicate_groups(subjects: &[HashSubject]) -> Vec<BuiltGroup> {
    let mut buckets = HashMap::<(i64, String), Vec<&HashSubject>>::new();
    for subject in subjects {
        if let Some(full_hash) = subject.full_hash.as_ref() {
            buckets
                .entry((subject.size, full_hash.clone()))
                .or_default()
                .push(subject);
        }
    }
    let mut groups = Vec::new();
    for ((size, full_hash), subjects) in buckets {
        if subjects.len() < 2 {
            continue;
        }
        let mut members = subjects
            .iter()
            .flat_map(|subject| subject.members.iter())
            .cloned()
            .collect::<Vec<_>>();
        members.sort_by(|left, right| {
            left.candidate
                .path
                .to_ascii_lowercase()
                .cmp(&right.candidate.path.to_ascii_lowercase())
                .then_with(|| left.candidate.file_id.cmp(&right.candidate.file_id))
        });
        let mut seen_physical = HashMap::<String, usize>::new();
        let mut built_members = Vec::with_capacity(members.len());
        let mut unknown_physical = 0_i64;
        for member in members {
            let is_alias = if let Some(key) = member.identity.physical_key.as_ref() {
                let count = seen_physical.entry(key.clone()).or_default();
                let alias = *count > 0;
                *count += 1;
                alias
            } else {
                unknown_physical += 1;
                false
            };
            let physical_key = member.identity.physical_key.clone();
            built_members.push(BuiltMember {
                file_id: member.candidate.file_id,
                path_snapshot: member.candidate.path,
                physical_key,
                identity_status: if member.identity.physical_key.is_some() {
                    "verified".to_string()
                } else {
                    "path_only".to_string()
                },
                is_hardlink_alias: is_alias,
                size: member.candidate.size,
                modified_ns: member.identity.modified_ns,
                verified_at: current_unix_seconds(),
            });
        }
        let physical_copy_count =
            i64::try_from(seen_physical.len() + usize::try_from(unknown_physical).unwrap_or(0))
                .unwrap_or(i64::MAX);
        let member_count = i64::try_from(built_members.len()).unwrap_or(i64::MAX);
        let potential = size.saturating_mul(physical_copy_count.saturating_sub(1).max(0));
        let all_verified = unknown_physical == 0;
        let exact =
            all_verified.then(|| size.saturating_mul(physical_copy_count.saturating_sub(1).max(0)));
        let id_input = format!("v1:{size}:blake3:1:{full_hash}");
        let id = format!(
            "dedupe-group-{}",
            blake3::hash(id_input.as_bytes()).to_hex()
        );
        groups.push(BuiltGroup {
            id,
            size_each: size,
            full_hash,
            members: built_members,
            physical_copy_count,
            hardlink_alias_count: member_count.saturating_sub(physical_copy_count).max(0),
            exact_reclaimable_bytes: exact,
            potential_reclaimable_bytes: potential,
            reclaimable_confidence: if all_verified {
                "exact".to_string()
            } else {
                "estimated".to_string()
            },
        });
    }
    groups.sort_by(|left, right| {
        right
            .potential_reclaimable_bytes
            .cmp(&left.potential_reclaimable_bytes)
            .then_with(|| left.id.cmp(&right.id))
    });
    groups
}

pub fn spawn_duplicate_detection<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    jobs: DedupeJobManager,
    parent_scan_job_id: Option<String>,
) -> Result<String, String> {
    let admission = if let Some(scan_session_id) = parent_scan_job_id.as_deref() {
        db.start_dedupe_run_for_scan_session(scan_session_id)
    } else {
        db.start_dedupe_run(&StartDedupeRunRequest {
            scope: DedupeScopeRequest {
                kind: "allManagedFileLibrary".to_string(),
                root_ids: Vec::new(),
            },
            request_key: Some(format!("manual:{}", new_job_id("dedupe-request"))),
            parent_scan_session_id: None,
        })
    }
    .map_err(|error| error.to_string())?;
    let job_id = admission.run.id.clone();
    if !admission.created && jobs.contains(&job_id) {
        return Ok(job_id);
    }
    spawn_admitted_dedupe_run(app, db, jobs, job_id.clone(), parent_scan_job_id)?;
    Ok(job_id)
}

fn spawn_admitted_dedupe_run<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    jobs: DedupeJobManager,
    job_id: String,
    parent_scan_job_id: Option<String>,
) -> Result<(), String> {
    let cancel_flag = jobs.register(job_id.clone(), parent_scan_job_id.clone())?;
    let task_job_id = job_id.clone();
    let task_parent_scan_job_id = parent_scan_job_id.clone();
    let follow_up_app = app.clone();
    let follow_up_jobs = jobs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        struct RunningGuard {
            jobs: DedupeJobManager,
            job_id: String,
        }
        impl Drop for RunningGuard {
            fn drop(&mut self) {
                self.jobs.finish(&self.job_id);
            }
        }
        let _guard = RunningGuard {
            jobs,
            job_id: task_job_id.clone(),
        };
        let emitter = TauriDedupeEventEmitter::new(app);
        match run_durable_dedupe(&db, &emitter, &task_job_id, &cancel_flag) {
            Ok(_) => {
                if let Ok(final_run) = db.get_dedupe_run(&task_job_id) {
                    if final_run.rerun_required {
                        if let Some(session_id) = final_run.parent_scan_session_id.as_deref() {
                            schedule_scan_dedupe_follow_up(
                                follow_up_app.clone(),
                                db.clone(),
                                follow_up_jobs.clone(),
                                session_id,
                            );
                        }
                    }
                }
            }
            Err(error) => {
                let checkpoint = DedupeCheckpoint {
                    phase: "completed".to_string(),
                    ..DedupeCheckpoint::default()
                };
                let final_run = db
                    .get_dedupe_run(&task_job_id)
                    .and_then(|current| {
                        db.mark_dedupe_terminal(
                            &task_job_id,
                            current.revision,
                            "failed",
                            Some(hash_error_code(&error)),
                            Some(&error.to_string()),
                            &checkpoint,
                        )
                    })
                    .or_else(|_| db.get_dedupe_run(&task_job_id));
                if let Ok(final_run) = final_run {
                    if let Err(emit_error) = emitter.emit_run_updated(&final_run) {
                        eprintln!("Dedupe run update event failed: {emit_error}");
                    }
                    let summary = summary_from_run(&final_run);
                    let payload = summary.complete_payload(
                        &task_job_id,
                        task_parent_scan_job_id.as_deref(),
                        "failed",
                    );
                    if let Err(emit_error) = emitter.emit_complete(&payload) {
                        eprintln!("Dedupe complete event failed: {emit_error}");
                    }
                } else {
                    eprintln!("Dedupe run failed before durable terminal state: {error}");
                }
            }
        }
    });
    Ok(())
}

fn schedule_scan_dedupe_follow_up<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    jobs: DedupeJobManager,
    session_id: &str,
) {
    let Ok(Some(dispatching)) = db.claim_dedupe_dispatch(session_id) else {
        return;
    };
    match spawn_duplicate_detection(app, db.clone(), jobs, Some(session_id.to_string())) {
        Ok(job_id) => {
            if let Err(error) =
                db.record_dedupe_dispatch(session_id, dispatching.revision, Some(&job_id), None)
            {
                eprintln!("Dedupe follow-up dispatch record failed: {error}");
            }
        }
        Err(error) => {
            if let Err(record_error) =
                db.record_dedupe_dispatch(session_id, dispatching.revision, None, Some(&error))
            {
                eprintln!("Dedupe follow-up dispatch failure record failed: {record_error}");
            }
        }
    }
}

#[tauri::command]
pub fn start_dedupe_run<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    jobs: State<'_, DedupeJobManager>,
    request: StartDedupeRunRequest,
) -> Result<DedupeRunDto, String> {
    require_main_window(&window)?;
    let admission = db
        .start_dedupe_run(&request)
        .map_err(|error| error.to_string())?;
    let run = admission.run.clone();
    if admission.created {
        spawn_admitted_dedupe_run(
            app,
            db.inner().clone(),
            jobs.inner().clone(),
            run.id.clone(),
            run.parent_scan_session_id.clone(),
        )?;
    }
    Ok(run)
}

#[tauri::command]
pub fn retry_dedupe_run<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    jobs: State<'_, DedupeJobManager>,
    run_id: String,
) -> Result<DedupeRunDto, String> {
    require_main_window(&window)?;
    let admission = db
        .retry_dedupe_run(run_id.trim())
        .map_err(|error| error.to_string())?;
    let run = admission.run.clone();
    if admission.created {
        spawn_admitted_dedupe_run(
            app,
            db.inner().clone(),
            jobs.inner().clone(),
            run.id.clone(),
            run.parent_scan_session_id.clone(),
        )?;
    }
    Ok(run)
}

#[tauri::command]
pub fn cancel_dedupe_run<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    jobs: State<'_, DedupeJobManager>,
    run_id: String,
) -> Result<DedupeRunDto, String> {
    require_main_window(&window)?;
    let _ = jobs.cancel(run_id.trim());
    db.request_dedupe_cancellation(run_id.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_dedupe_run(db: State<'_, Database>, run_id: String) -> Result<DedupeRunDto, String> {
    db.get_dedupe_run(run_id.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_dedupe_runs(
    db: State<'_, Database>,
    limit: Option<usize>,
) -> Result<Vec<DedupeRunDto>, String> {
    db.list_dedupe_runs(limit.unwrap_or(20))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_active_dedupe_run(db: State<'_, Database>) -> Result<Option<DedupeRunDto>, String> {
    db.get_active_dedupe_run()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_duplicate_groups(
    db: State<'_, Database>,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<DedupeGroupPageDto, String> {
    db.list_duplicate_groups(cursor.as_deref(), limit.unwrap_or(50))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_duplicate_group(
    db: State<'_, Database>,
    group_id: String,
) -> Result<Option<DedupeGroupDto>, String> {
    db.get_duplicate_group(group_id.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_duplicate_group_members(
    db: State<'_, Database>,
    group_id: String,
) -> Result<Vec<DedupeGroupMemberDto>, String> {
    db.list_duplicate_group_members(group_id.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_file_duplicate_membership(
    db: State<'_, Database>,
    file_id: String,
) -> Result<Vec<DedupeGroupDto>, String> {
    db.get_file_duplicate_membership(file_id.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn cancel_dedupe<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    jobs: tauri::State<'_, DedupeJobManager>,
    job_id: String,
) -> Result<(), String> {
    require_main_window(&window)?;
    let _ = jobs.cancel(&job_id);
    db.request_dedupe_cancellation(job_id.trim())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn hash_file_blake3(path: &Path) -> Result<String, DedupeError> {
    hash_file_blake3_with_bytes(path).map(|(hash, _)| hash)
}

fn hash_file_blake3_with_bytes(path: &Path) -> Result<(String, u64), DedupeError> {
    let mut file = open_content_file(path)?;
    #[cfg(windows)]
    crate::platform::windows::cloud_metadata::ensure_local_content(&file).map_err(|source| {
        DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        }
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher
        .update_reader(&mut file)
        .map_err(|source| DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        })?;
    #[cfg(windows)]
    crate::platform::windows::cloud_metadata::ensure_local_content(&file).map_err(|source| {
        DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        }
    })?;
    let bytes_read = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);

    Ok((hasher.finalize().to_hex().to_string(), bytes_read))
}

fn open_content_file(path: &Path) -> Result<File, DedupeError> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::file_semantics::open_content_read(path)
            .map_err(|reason| DedupeError::Db(DbError::Validation(reason.to_string())))
    }

    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(windows)]
        let file = crate::platform::windows::cloud_metadata::open_local_content(path);
        #[cfg(not(windows))]
        let file = File::open(path);
        file.map_err(|source| DedupeError::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        })
    }
}

#[cfg(test)]
mod job_manager_tests {
    use super::*;
    use crate::scheduler::{PermissiveResourcePolicy, ResourceCapacities, SchedulerConfig};
    use std::{fs, io::Write, path::PathBuf};

    #[test]
    fn scan_jobs_map_to_independent_dedupe_jobs() {
        let manager = DedupeJobManager::default();
        let a = manager
            .register("dedupe-a".to_string(), Some("scan-a".to_string()))
            .expect("register a");
        let b = manager
            .register("dedupe-b".to_string(), Some("scan-b".to_string()))
            .expect("register b");

        assert!(manager.cancel_for_scan("scan-a"));
        assert!(a.load(Ordering::Acquire));
        assert!(!b.load(Ordering::Acquire));
    }

    #[test]
    fn cancelling_one_dedupe_job_does_not_affect_another() {
        let manager = DedupeJobManager::default();
        let a = manager
            .register("dedupe-a".to_string(), None)
            .expect("register a");
        let b = manager
            .register("dedupe-b".to_string(), None)
            .expect("register b");

        assert!(manager.cancel("dedupe-a"));
        assert!(a.load(Ordering::Acquire));
        assert!(!b.load(Ordering::Acquire));
    }

    #[test]
    fn duplicate_job_ids_are_rejected_and_finished_jobs_are_removed() {
        let manager = DedupeJobManager::default();
        manager
            .register("dedupe-a".to_string(), Some("scan-a".to_string()))
            .expect("register job");
        assert!(manager.register("dedupe-a".to_string(), None).is_err());

        manager.finish("dedupe-a");

        assert!(!manager.cancel("dedupe-a"));
        assert!(!manager.cancel_for_scan("scan-a"));
        assert!(manager.register("dedupe-a".to_string(), None).is_ok());
    }

    #[test]
    fn worker_count_uses_bounded_default_and_rejects_invalid_overrides() {
        assert_eq!(dedupe_worker_count(16, None), (4, false));
        assert_eq!(dedupe_worker_count(2, None), (2, false));
        assert_eq!(dedupe_worker_count(8, Some("1")), (1, false));
        assert_eq!(dedupe_worker_count(8, Some("8")), (8, false));
        assert_eq!(dedupe_worker_count(8, Some("0")), (4, true));
        assert_eq!(dedupe_worker_count(8, Some("nine")), (4, true));
        assert_eq!(dedupe_worker_count(2, Some("8")), (2, false));
    }

    #[test]
    fn dedupe_hash_workers_cannot_exceed_shared_scheduler_capacity() {
        let scheduler = WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        );
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let leases =
            acquire_dedupe_hash_leases(&scheduler, "dedupe-capacity-test", &cancel_flag, 4)
                .expect("acquire bounded hash leases");
        assert_eq!(leases.len(), 1);
        assert_eq!(scheduler.snapshot().running_background, 1);
        drop(leases);
        assert_eq!(scheduler.snapshot().running, 0);
        assert_eq!(scheduler.snapshot().granted, ResourceHints::empty());
    }

    #[test]
    fn hash_worker_qos_seam_runs_once_on_each_worker_thread() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let expected_identity = capture_physical_identity(&path).expect("manifest identity");
        let coordinator_thread = thread::current().id();
        let worker_count = 3;
        let qos_calls = Arc::new(Mutex::new(Vec::with_capacity(worker_count)));
        let qos_calls_for_workers = Arc::clone(&qos_calls);

        let results = bounded_hash_subjects_with_workers_and_qos(
            vec![HashTask {
                subject_index: 0,
                path,
                expected_identity,
            }],
            Arc::new(AtomicBool::new(false)),
            worker_count,
            move || {
                qos_calls_for_workers
                    .lock()
                    .expect("QoS call list lock")
                    .push(thread::current().id());
            },
        )
        .expect("hash worker result");

        assert_eq!(results.len(), 1);
        assert!(results[0].result.is_ok());
        let worker_threads = qos_calls.lock().expect("QoS call list lock").clone();
        assert_eq!(worker_threads.len(), worker_count);
        assert!(worker_threads
            .iter()
            .all(|worker| worker != &coordinator_thread));
        for (index, worker) in worker_threads.iter().enumerate() {
            assert!(worker_threads[..index]
                .iter()
                .all(|previous| previous != worker));
        }
    }

    #[test]
    fn cancellation_releases_dedupe_while_waiting_for_shared_admission() {
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        let holder = scheduler
            .try_acquire(WorkRequest::new(
                "dedupe-cancel-holder",
                crate::file_workspace::WorkClass::Foreground,
                ResourceHints::cpu_io(1, 1),
            ))
            .expect("hold shared scheduler capacity");
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag_for_waiter = Arc::clone(&cancel_flag);
        let scheduler_for_waiter = Arc::clone(&scheduler);
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            tx.send(acquire_dedupe_hash_leases(
                &scheduler_for_waiter,
                "dedupe-cancel-run",
                &cancel_flag_for_waiter,
                4,
            ))
            .expect("send dedupe admission result");
        });
        for _ in 0..10_000 {
            if scheduler.snapshot().queued == 1 {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(scheduler.snapshot().queued, 1);
        cancel_flag.store(true, Ordering::Release);
        assert_eq!(scheduler.cancel_session("dedupe-cancel-run"), 1);
        assert!(matches!(
            rx.recv_timeout(std::time::Duration::from_secs(1))
                .expect("cancellation wakes dedupe admission"),
            Err(AcquireError::Cancelled)
        ));
        waiter.join().expect("dedupe waiter joins");
        drop(holder);
        assert_eq!(scheduler.snapshot().queued, 0);
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn one_worker_and_multi_worker_hash_results_are_identical() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-worker-parity-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create fixture directory");
        let first = root.join("first.bin");
        let second = root.join("second.bin");
        fs::write(&first, vec![1_u8; 2 * 1024 * 1024]).expect("first fixture");
        fs::write(&second, vec![2_u8; 2 * 1024 * 1024]).expect("second fixture");
        let first_identity = capture_physical_identity(&first).expect("first identity");
        let second_identity = capture_physical_identity(&second).expect("second identity");
        let tasks = || {
            vec![
                HashTask {
                    subject_index: 0,
                    path: first.clone(),
                    expected_identity: first_identity.clone(),
                },
                HashTask {
                    subject_index: 1,
                    path: second.clone(),
                    expected_identity: second_identity.clone(),
                },
            ]
        };
        let mut one_worker =
            bounded_hash_subjects_with_workers(tasks(), Arc::new(AtomicBool::new(false)), 1)
                .expect("one worker");
        let mut many_workers =
            bounded_hash_subjects_with_workers(tasks(), Arc::new(AtomicBool::new(false)), 4)
                .expect("multiple workers");
        let normalize = |results: &mut Vec<HashResult>| {
            results.sort_by_key(|result| result.subject_index);
            results
                .iter()
                .map(|result| {
                    (
                        result.subject_index,
                        result.result.as_ref().expect("hash result").clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(normalize(&mut one_worker), normalize(&mut many_workers));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "Task 02 hash IO benchmark; invoked with a reduced fixture by npm run test:performance"]
    fn performance_task02_hash_io_1000x16mib_1_worker_and_default_workers() {
        let file_count = std::env::var("ZC_TASK02_IO_FILES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(16)
            .max(2);
        let bytes_each = std::env::var("ZC_TASK02_IO_BYTES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1024 * 1024)
            .max(1);
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-io-benchmark-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create IO benchmark directory");
        let chunk = vec![0_u8; 1024 * 1024];
        let mut paths = Vec::with_capacity(file_count);
        for index in 0..file_count {
            let path = root.join(format!("file-{index:04}.bin"));
            let mut file = File::create(&path).expect("create IO benchmark file");
            let fill = u8::try_from(index % 251).expect("fixture pattern fits");
            let mut remaining = bytes_each;
            while remaining > 0 {
                let write_len = remaining.min(chunk.len());
                if fill == 0 {
                    file.write_all(&chunk[..write_len])
                        .expect("write IO benchmark chunk");
                } else {
                    file.write_all(&vec![fill; write_len])
                        .expect("write IO benchmark chunk");
                }
                remaining -= write_len;
            }
            paths.push(path);
        }

        let identity_started = Instant::now();
        let identities = paths
            .iter()
            .map(|path| capture_physical_identity(path).expect("capture benchmark identity"))
            .collect::<Vec<_>>();
        let identity_elapsed = identity_started.elapsed();

        let prehash_started = Instant::now();
        for path in &paths {
            hash_file_prehash(path, i64::try_from(bytes_each).expect("fixture size fits"))
                .expect("prehash benchmark file");
        }
        let prehash_elapsed = prehash_started.elapsed();
        let prehash_bytes_each = if bytes_each < PREHASH_MIN_SIZE as usize {
            bytes_each
        } else {
            PREHASH_SAMPLE_BYTES.saturating_mul(2)
        };
        let prehash_bytes = prehash_bytes_each.saturating_mul(file_count);
        let full_hash_bytes = bytes_each.saturating_mul(file_count);
        let tasks = || {
            paths
                .iter()
                .zip(identities.iter())
                .enumerate()
                .map(|(subject_index, (path, expected_identity))| HashTask {
                    subject_index,
                    path: path.clone(),
                    expected_identity: expected_identity.clone(),
                })
                .collect::<Vec<_>>()
        };

        let one_worker_started = Instant::now();
        let one_worker =
            bounded_hash_subjects_with_workers(tasks(), Arc::new(AtomicBool::new(false)), 1)
                .expect("one-worker full hash benchmark");
        let one_worker_elapsed = one_worker_started.elapsed();
        assert_eq!(one_worker.len(), file_count);
        assert!(one_worker.iter().all(|result| result.result.is_ok()));

        let detected = thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(2);
        let (default_workers, _) = dedupe_worker_count(detected, None);
        let default_worker_started = Instant::now();
        let default_worker = bounded_hash_subjects_with_workers(
            tasks(),
            Arc::new(AtomicBool::new(false)),
            default_workers,
        )
        .expect("default-worker full hash benchmark");
        let default_worker_elapsed = default_worker_started.elapsed();
        assert_eq!(default_worker.len(), file_count);
        assert!(default_worker.iter().all(|result| result.result.is_ok()));

        println!(
            "Task 02 hash IO benchmark: files={file_count}, bytes_each={bytes_each}, identity_io_ms={:.3}, prehash_bytes={prehash_bytes}, prehash_ms={:.3}, full_hash_bytes={full_hash_bytes}, one_worker_ms={:.3}, default_workers={default_workers}, default_worker_ms={:.3}",
            identity_elapsed.as_secs_f64() * 1000.0,
            prehash_elapsed.as_secs_f64() * 1000.0,
            one_worker_elapsed.as_secs_f64() * 1000.0,
            default_worker_elapsed.as_secs_f64() * 1000.0,
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_worker_rejects_a_file_changed_before_hashing() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-worker-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create fixture directory");
        let path = root.join("changed-before-hash.bin");
        fs::write(&path, b"original").expect("write original");
        let expected_identity =
            capture_physical_identity(&path).expect("capture original identity");
        fs::write(&path, b"changed-before-hash").expect("change fixture");

        let (results, _) = bounded_hash_subjects(
            vec![HashTask {
                subject_index: 0,
                path: PathBuf::from(&path),
                expected_identity,
            }],
            Arc::new(AtomicBool::new(false)),
            "dedupe-worker-test-run",
        )
        .expect("bounded worker result");
        let error = results
            .into_iter()
            .next()
            .expect("one worker result")
            .result
            .expect_err("changed file must not be hashed");
        assert!(error.to_string().contains("file_changed_before_hash"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finishing_an_old_scan_job_does_not_remove_a_new_follow_up_owner() {
        let manager = DedupeJobManager::default();
        manager
            .register("dedupe-old".to_string(), Some("scan-follow-up".to_string()))
            .expect("register old job");
        let new_flag = manager
            .register("dedupe-new".to_string(), Some("scan-follow-up".to_string()))
            .expect("register follow-up job");

        manager.finish("dedupe-old");
        assert!(manager.cancel_for_scan("scan-follow-up"));
        assert!(new_flag.load(Ordering::Acquire));
    }
}
