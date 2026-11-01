use crate::path_identity::normalize_path;
use crate::{
    db::Database,
    file_naming::{normalize_proposed_file_name, ExtensionChangePolicy},
    ids::new_job_id,
    window_auth::require_main_window,
};
#[cfg(all(test, windows))]
use std::io::Read;
#[cfg(not(target_os = "macos"))]
use std::process::Command as ProcessCommand;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{command, AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};

mod authority;
mod execution;
mod identity;
mod journal;
pub(crate) mod manual_move;
#[cfg(all(test, any(target_os = "windows", target_os = "macos")))]
mod manual_move_tests;
#[cfg(all(test, any(target_os = "windows", target_os = "macos")))]
mod manual_rename_tests;
pub(crate) mod manual_transfer;
#[cfg(all(test, any(windows, target_os = "macos")))]
mod manual_transfer_tests;
mod preview;
mod progress;
mod reconciliation;
#[cfg(all(
    test,
    feature = "performance-test-tauri",
    any(target_os = "windows", target_os = "macos")
))]
mod reconciliation_tests;
mod recovery;
mod restore;
#[cfg(all(test, any(target_os = "windows", target_os = "macos")))]
mod restore_finalization_tests;
mod reveal;
mod types;
mod validation;
pub(crate) use authority::resolve_execute_selections;
pub(crate) use execution::*;
pub(crate) use identity::{
    file_identity_fingerprint, file_namespace_fingerprint, file_operation_fingerprint,
    FileIdentityFingerprint,
};
pub(crate) use journal::*;
pub(crate) use preview::*;
use progress::OperationProgressBuffer;
pub use recovery::reconcile_pending_operation_journal;
pub(crate) use recovery::*;
#[cfg(any(test, feature = "native-qa", target_os = "macos"))]
pub use restore::restore_moves_with_persistence;
pub(crate) use restore::*;
pub(crate) use reveal::*;
pub use types::*;
pub(crate) use validation::*;

pub const OPERATION_PROGRESS_EVENT: &str = "operation-progress";
const OPERATION_PROGRESS_BATCH_SIZE: u64 = 10;
const OPERATION_PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(200);

#[cfg(any(test, feature = "native-qa"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationTestFaultPoint {
    AfterCompletedPhaseBeforeFinalLogPersist,
    AfterRestoreJournalPreparedBeforeClaim,
    AfterRestoreSourceClaimedBeforeTargetCommit,
    AfterRestoreTargetCommittedBeforeFinalPersist,
    AfterRestoreCompletedPhaseBeforeFinalTransaction,
}

#[cfg(all(test, target_os = "macos"))]
mod r_fl_01_macos_tests {
    use super::*;
    use crate::db::{Database, InsertFileRequest};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn macos_test_root(label: &str, nonce: u128) -> PathBuf {
        std::env::current_dir()
            .expect("test cwd")
            .join(".tmp-tests")
            .join(format!(
                "zen-canvas-r-fl-01-macos-{label}-{}-{nonce}",
                std::process::id()
            ))
    }

    #[test]
    fn r_fl_01_t11_macos_stale_permanent_delete_keeps_source_and_journal_empty() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = macos_test_root("stale", nonce);
        fs::create_dir_all(&root).expect("fixture root");
        let source = root.join("permanent-delete.txt");
        fs::write(&source, "original").expect("source");
        let db_path = root.join("fixture.sqlite3");
        let db = Database::open(&db_path).expect("database");
        let file_id = source.to_string_lossy().into_owned();
        let metadata = fs::metadata(&source).expect("metadata");
        let mtime = metadata
            .modified()
            .expect("mtime")
            .duration_since(UNIX_EPOCH)
            .expect("unix mtime")
            .as_secs() as i64;
        db.insert_file(InsertFileRequest {
            id: file_id.clone(),
            path: file_id.clone(),
            name: "permanent-delete.txt".to_string(),
            extension: "txt".to_string(),
            size: metadata.len() as i64,
            mtime,
            ctime: mtime,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert file");
        let preview = db
            .get_permanent_delete_operation_preview(&file_id)
            .expect("macOS permanent delete preview");
        fs::write(&source, "changed-source").expect("change source");
        let fingerprint = preview
            .operation_fingerprint
            .clone()
            .expect("preview fingerprint");
        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![OperationSelection {
                    id: preview.id,
                    file_id,
                    operation_fingerprint: fingerprint.clone(),
                    expected_revision: fingerprint,
                    new_name: None,
                    destination_directory: None,
                }],
            },
        )
        .expect_err("stale permanent delete must be rejected");
        assert_eq!(error, "operation_preview_stale");
        assert!(source.exists());
        assert_eq!(db.get_operation_logs(Some(10)).expect("logs").len(), 0);

        drop(db);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn r_fl_01_t16_macos_permanent_delete_uses_authoritative_preview_and_journal() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = macos_test_root("positive", nonce);
        fs::create_dir_all(&root).expect("fixture root");
        let source = root.join("permanent-delete.txt");
        fs::write(&source, "permanent delete fixture").expect("source");
        let db_path = root.join("fixture.sqlite3");
        let db = Database::open(&db_path).expect("database");
        let file_id = source.to_string_lossy().into_owned();
        let metadata = fs::metadata(&source).expect("metadata");
        let mtime = metadata
            .modified()
            .expect("mtime")
            .duration_since(UNIX_EPOCH)
            .expect("unix mtime")
            .as_secs() as i64;
        db.insert_file(InsertFileRequest {
            id: file_id.clone(),
            path: file_id.clone(),
            name: "permanent-delete.txt".to_string(),
            extension: "txt".to_string(),
            size: metadata.len() as i64,
            mtime,
            ctime: mtime,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert file");

        let preview = db
            .get_permanent_delete_operation_preview(&file_id)
            .expect("macOS permanent delete preview");
        let repeated_preview = db
            .get_permanent_delete_operation_preview(&file_id)
            .expect("repeated macOS permanent delete preview");
        assert_eq!(preview.id, repeated_preview.id);
        assert!(preview.id.starts_with("op-permanent-delete-"));
        assert_eq!(preview.operation_type, "permanent_delete");
        assert_eq!(preview.risk_level, "Sensitive");
        assert!(preview.requires_confirmation);
        assert_eq!(
            preview.conflict_policy.as_deref(),
            Some("permanent_delete_quarantine")
        );
        assert_eq!(preview.is_executable, Some(true));
        let fingerprint = preview
            .operation_fingerprint
            .clone()
            .expect("preview fingerprint");
        assert!(!fingerprint.is_empty());
        let resolved = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![OperationSelection {
                    id: preview.id,
                    file_id,
                    operation_fingerprint: fingerprint.clone(),
                    expected_revision: fingerprint,
                    new_name: None,
                    destination_directory: None,
                }],
            },
        )
        .expect("resolve permanent delete");
        assert_eq!(resolved.operations[0].operation_type, "permanent_delete");
        let executed = execute_moves_with_persistence(&db, resolved).expect("execute");
        assert_eq!(executed.logs.len(), 1);
        assert_eq!(executed.logs[0].operation_type, "permanent_delete");
        let log = &executed.logs[0];
        assert_eq!(
            log.status,
            "success",
            "T16 failed: status={:?}, error_message={:?}, operation_phase={:?}, source_claim_path={:?}, path_before={:?}, path_after={:?}",
            log.status,
            log.error_message,
            log.operation_phase,
            log.source_claim_path,
            log.path_before,
            log.path_after
        );
        assert_eq!(log.operation_phase, "completed");
        assert!(!source.exists());
        assert_eq!(db.get_operation_logs(Some(10)).expect("logs").len(), 1);

        drop(db);
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(any(test, feature = "native-qa"))]
thread_local! {
    static OPERATION_TEST_FAULT: std::cell::Cell<Option<OperationTestFaultPoint>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(any(test, feature = "native-qa"))]
pub fn set_operation_test_fault(point: Option<OperationTestFaultPoint>) {
    OPERATION_TEST_FAULT.with(|fault| fault.set(point));
}

#[cfg(any(test, feature = "native-qa"))]
fn take_operation_test_fault(point: OperationTestFaultPoint) -> bool {
    OPERATION_TEST_FAULT.with(|fault| {
        if fault.get() == Some(point) {
            fault.set(None);
            true
        } else {
            false
        }
    })
}

pub trait OperationProgressEmitter {
    fn emit_progress(&self, payload: OperationProgressPayload);
}

struct NoopOperationProgressEmitter;

impl OperationProgressEmitter for NoopOperationProgressEmitter {
    fn emit_progress(&self, _payload: OperationProgressPayload) {}
}

struct TauriOperationProgressEmitter<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriOperationProgressEmitter<R> {
    fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> OperationProgressEmitter for TauriOperationProgressEmitter<R> {
    fn emit_progress(&self, payload: OperationProgressPayload) {
        if let Err(error) = self.app.emit(OPERATION_PROGRESS_EVENT, payload) {
            eprintln!("Operation progress event failed: {error}");
        }
    }
}

pub fn move_file(source_path: String, target_path: String) -> Result<FileOperationResult, String> {
    crate::fs_safety::platform_support::ensure_supported_file_mutation()
        .map_err(|error| error.to_string())?;
    let source = validate_source_path(&PathBuf::from(source_path))?;
    let target = validate_target_path(&PathBuf::from(target_path))?;

    ensure_general_file_operation_allowed(&source)?;
    ensure_general_file_operation_allowed(&target)?;
    move_file_no_overwrite(&source, &target).map_err(|error| error.to_string())?;

    Ok(FileOperationResult {
        operation: "move".to_string(),
        source_path: normalize_path(&source),
        target_path: normalize_path(&target),
    })
}

#[command]
pub async fn execute_moves<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    cancel: State<'_, OperationCancellationToken>,
    request: ExecuteMovesByIdRequest,
) -> Result<ExecuteMovesResult, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    let request = resolve_execute_selections(&db, request)?;
    let paths = request
        .operations
        .iter()
        .flat_map(|operation| [operation.source_path.clone(), operation.target_path.clone()])
        .collect::<Vec<_>>();
    let app_data_dir = app.path().app_data_dir().ok();
    let guard = cancel.begin()?;
    let cancel_flag = Arc::clone(&cancel.cancel);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let emitter = TauriOperationProgressEmitter::new(app.clone());
        reconciliation::with_reconciliation_wake(&app, &db, &paths, || {
            execute_moves_with_persistence_with_progress_and_app_data(
                &db,
                request,
                cancel_flag,
                &emitter,
                app_data_dir,
                None,
            )
        })
    })
    .await
    .map_err(|error| format!("operation task failed: {error}"))?
}

#[tauri::command]
pub async fn materialize_provider_preview<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    cancel: State<'_, OperationCancellationToken>,
    request: MaterializeProviderRequest,
) -> Result<MaterializeProviderResult, String> {
    require_main_window(&window)?;
    let preview = db
        .get_operation_previews_by_file_ids(std::slice::from_ref(&request.file_id))
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|preview| preview.id == request.preview_id)
        .ok_or_else(|| "The materialization preview is stale or unavailable.".to_string())?;
    let current_fingerprint = preview
        .operation_fingerprint
        .clone()
        .ok_or_else(|| "The materialization preview has no operation fingerprint.".to_string())?;
    if current_fingerprint != request.operation_fingerprint {
        return Err(
            "The materialization preview changed; refresh it before downloading.".to_string(),
        );
    }
    if request.expected_revision != request.operation_fingerprint {
        return Err(
            "The materialization preview revision is stale; refresh it before downloading."
                .to_string(),
        );
    }
    let materialization_requirement = preview
        .materialization_requirement_v2
        .as_deref()
        .or(preview.materialization_requirement.as_deref());
    if !matches!(
        materialization_requirement,
        Some("explicit_download_required" | "required" | "provider_managed")
    ) {
        return Err("This preview does not require explicit materialization.".to_string());
    }
    if !matches!(
        preview.operation_type.as_str(),
        "copy" | "duplicate" | "replace"
    ) {
        return Err("This operation does not require explicit materialization.".to_string());
    }
    #[cfg(target_os = "macos")]
    let source = std::path::PathBuf::from(&preview.source_path);
    let preview_provider_identity = preview.provider_identity_fingerprint.clone();
    let request_preview_id = request.preview_id.clone();
    let request_file_id = request.file_id.clone();
    let db = db.inner().clone();
    let guard = cancel.begin()?;
    #[cfg(target_os = "macos")]
    let cancel_flag = std::sync::Arc::clone(&cancel.cancel);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let emitter = TauriOperationProgressEmitter::new(app);
        emitter.emit_progress(OperationProgressPayload {
            kind: "materialize".to_string(),
            batch_id: preview.id.clone(),
            processed: 0,
            total: 1,
            current_path: preview.source_path.clone(),
        });
        #[cfg(target_os = "macos")]
        let result = crate::platform::macos::strategy::materialize_explicit(
            &source,
            Some(cancel_flag.as_ref()),
            |processed, total| {
                emitter.emit_progress(OperationProgressPayload {
                    kind: "materialize".to_string(),
                    batch_id: preview.id.clone(),
                    processed,
                    total,
                    current_path: preview.source_path.clone(),
                });
            },
        )
        .map_err(|error| error.to_string());
        #[cfg(not(target_os = "macos"))]
        let result: Result<(), String> = Err(
            "mac_provider_materialization_required: native macOS materialization is unavailable."
                .to_string(),
        );
        result?;
        let next = db
            .get_operation_previews_by_file_ids(std::slice::from_ref(&request_file_id))
            .ok()
            .and_then(|previews| {
                previews
                    .into_iter()
                    .find(|candidate| candidate.id == request_preview_id)
            });
        if let Some(candidate) = next.as_ref() {
            if candidate.source_path != preview.source_path
                || candidate.target_path != preview.target_path
                || candidate.operation_type != preview.operation_type
                || candidate.conflict_policy != preview.conflict_policy
                || candidate.provider_identity_fingerprint != preview_provider_identity
            {
                return Err("mac_provider_url_changed".to_string());
            }
        }
        let next_operation_fingerprint = next
            .and_then(|candidate| candidate.operation_fingerprint)
            .ok_or_else(|| "mac_provider_url_changed".to_string())?;
        Ok(MaterializeProviderResult {
            preview_id: request_preview_id,
            file_id: request_file_id,
            materialization: "boundary_readable".to_string(),
            next_operation_fingerprint: Some(next_operation_fingerprint),
        })
    })
    .await
    .map_err(|error| format!("materialization task failed: {error}"))?
}

/// Executes an already canonicalized backend-owned operation set without
/// resolving a second preview/target after the caller's approval fingerprint.
pub(crate) async fn execute_canonical_operations<R: Runtime>(
    app: AppHandle<R>,
    db: Database,
    cancel: OperationCancellationToken,
    request: ExecuteMovesRequest,
    batch_id: String,
) -> Result<ExecuteMovesResult, String> {
    if request.operations.is_empty() {
        return Err("At least one canonical operation is required.".to_string());
    }
    let app_data_dir = app.path().app_data_dir().ok();
    let guard = cancel.begin()?;
    let cancel_flag = Arc::clone(&cancel.cancel);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let emitter = TauriOperationProgressEmitter::new(app);
        execute_moves_with_persistence_with_progress_and_app_data(
            &db,
            request,
            cancel_flag,
            &emitter,
            app_data_dir,
            Some(batch_id),
        )
    })
    .await
    .map_err(|error| format!("operation task failed: {error}"))?
}

#[command]
pub fn cancel_operations<R: Runtime>(
    window: WebviewWindow<R>,
    cancel: State<'_, OperationCancellationToken>,
) -> Result<(), String> {
    require_main_window(&window)?;
    cancel.cancel.store(true, Ordering::Relaxed);
    Ok(())
}

pub fn execute_moves_with_persistence(
    db: &Database,
    request: ExecuteMovesRequest,
) -> Result<ExecuteMovesResult, String> {
    execute_moves_with_persistence_with_progress(
        db,
        request,
        Arc::new(AtomicBool::new(false)),
        &NoopOperationProgressEmitter,
    )
}

fn execute_moves_with_persistence_with_progress(
    db: &Database,
    request: ExecuteMovesRequest,
    cancel_flag: Arc<AtomicBool>,
    emitter: &impl OperationProgressEmitter,
) -> Result<ExecuteMovesResult, String> {
    execute_moves_with_persistence_with_progress_and_app_data(
        db,
        request,
        cancel_flag,
        emitter,
        None,
        None,
    )
}

pub fn execute_moves_core(request: ExecuteMovesRequest) -> ExecuteMovesResult {
    execute_moves_core_with_progress(
        request,
        Arc::new(AtomicBool::new(false)),
        &NoopOperationProgressEmitter,
    )
}

pub fn execute_moves_core_with_progress(
    request: ExecuteMovesRequest,
    cancel_flag: Arc<AtomicBool>,
    emitter: &impl OperationProgressEmitter,
) -> ExecuteMovesResult {
    execute_moves_core_with_progress_and_app_data(request, cancel_flag, emitter, None)
}

#[command]
pub fn reveal_in_folder(path: String) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty.".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::finder::reveal_path(Path::new(trimmed))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let command = build_reveal_command(Path::new(trimmed))?;
        ProcessCommand::new(command.program)
            .args(&command.args)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Failed to reveal path in file manager: {error}"))
    }
}

#[command]
pub async fn request_macos_thumbnail<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    thumbnails: State<'_, crate::platform::macos::quick_look::MacThumbnailService>,
    file_id: String,
    size: u32,
    request_id: String,
) -> Result<String, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    let thumbnails = thumbnails.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = db
            .resolve_file_library_path(&file_id)
            .map_err(|error| error.to_string())?;
        let job = thumbnails.request(Path::new(&path), size, &request_id)?;
        job.join()
            .map(|thumbnail| thumbnail.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| format!("macos_quick_look_task_failed: {error}"))?
}

#[command]
pub fn cancel_macos_thumbnail<R: Runtime>(
    window: WebviewWindow<R>,
    thumbnails: State<'_, crate::platform::macos::quick_look::MacThumbnailService>,
    request_id: String,
) -> Result<bool, String> {
    require_main_window(&window)?;
    if request_id.is_empty() || request_id.len() > 128 {
        return Err("macos_quick_look_request_id_invalid".to_string());
    }
    Ok(thumbnails.cancel(&request_id))
}

pub fn rename_file(source_path: String, new_name: String) -> Result<FileOperationResult, String> {
    crate::fs_safety::platform_support::ensure_supported_file_mutation()
        .map_err(|error| error.to_string())?;
    rename_file_with_identity(source_path, new_name, None, None, None, None, None)
        .map_err(|error| error.to_string())
}

#[command]
pub async fn restore_moves<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    cancel: State<'_, OperationCancellationToken>,
    request: RestoreMovesByIdRequest,
) -> Result<RestoreMovesResult, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    let requested_count = request.log_ids.len();
    let logs = db
        .get_restorable_operation_logs_by_ids(&request.log_ids)
        .map_err(|error| error.to_string())?;
    if logs.len() != requested_count {
        return Err(
            "One or more operation log IDs are missing or no longer restorable.".to_string(),
        );
    }
    let request = RestoreMovesRequest { logs };
    let paths = request
        .logs
        .iter()
        .flat_map(|log| [log.path_before.clone(), log.path_after.clone()])
        .collect::<Vec<_>>();
    let guard = cancel.begin()?;
    let cancel_flag = Arc::clone(&cancel.cancel);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let emitter = TauriOperationProgressEmitter::new(app.clone());
        reconciliation::with_reconciliation_wake(&app, &db, &paths, || {
            restore_moves_with_persistence_with_progress(&db, request, cancel_flag, &emitter)
        })
    })
    .await
    .map_err(|error| format!("restore task failed: {error}"))?
}

const RECOVERY_ACTION_FILE_ID: &str = "__zen_canvas_recovery_action__";

#[command]
pub async fn resolve_operation_recovery<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    cancel: State<'_, OperationCancellationToken>,
    request: RecoveryActionRequest,
) -> Result<RecoveryActionResult, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    let original = db
        .get_operation_log_by_id(&request.log_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "recovery_operation_log_missing".to_string())?;
    let source_cleanup_recovery = original.status == "manual_review"
        && original.operation_phase == "source_cleanup_pending"
        && original.source_claim_path.is_some();
    if original.status != "manual_review"
        || (!source_cleanup_recovery && original.restore_status != "manual_review")
    {
        return Err("recovery_action_requires_manual_review".to_string());
    }
    let action = request.action.trim().to_ascii_lowercase();
    let retry_source_cleanup = source_cleanup_recovery
        && matches!(
            action.as_str(),
            "retry_cleanup" | "retry_source_retirement" | "retry-source-retirement"
        );
    let expected_claim_path = if source_cleanup_recovery {
        original.source_claim_path.clone()
    } else {
        original.restore_claim_path.clone()
    };
    let recovery_claim_exists = source_cleanup_recovery
        && expected_claim_path
            .as_deref()
            .is_some_and(|path| fs::symlink_metadata(path).is_ok());
    let recovery_source_path = if source_cleanup_recovery {
        if recovery_claim_exists {
            PathBuf::from(
                expected_claim_path
                    .as_deref()
                    .ok_or_else(|| "recovery_source_claim_missing".to_string())?,
            )
        } else {
            // A portable target-first move can commit the target before a safe
            // source claim is available. The original source is deliberately
            // preserved in that state, so recovery retries against it after
            // rechecking the journal identity instead of treating a missing
            // claim pathname as the object to delete.
            PathBuf::from(&original.path_before)
        }
    } else if original.operation_type == "replace" {
        replacement_backup_path_for_log(&original)
    } else {
        PathBuf::from(
            expected_claim_path
                .as_deref()
                .ok_or_else(|| "recovery_claim_missing".to_string())?,
        )
    };
    let recovery_identity_matches = if source_cleanup_recovery {
        if recovery_claim_exists {
            source_cleanup_claim_identity_matches(&original, &recovery_source_path)
                .map_err(|_| "recovery_claim_identity_unreadable".to_string())?
        } else {
            recovery::journal_identity_matches(&original, &recovery_source_path)
                .map_err(|_| "recovery_source_identity_unreadable".to_string())?
        }
    } else if original.operation_type == "replace" {
        replacement_backup_identity_result(&original, &recovery_source_path).is_ok()
    } else {
        restore_claim_identity_matches(&original, &recovery_source_path)
            .map_err(|_| "recovery_claim_identity_unreadable".to_string())?
    };
    if !recovery_identity_matches {
        return Err("recovery_claim_identity_mismatch".to_string());
    }

    let (operation_type, target_path) = match action.as_str() {
        "retry_cleanup" | "retry_source_retirement" | "retry-source-retirement"
            if retry_source_cleanup =>
        {
            ("permanent_delete".to_string(), None)
        }
        "keep_both" | "keep-both" | "keepboth" => {
            let target = if source_cleanup_recovery {
                recovery_action_target_path(
                    &original,
                    &recovery_source_path,
                    Some(&original.source_path),
                )?
            } else {
                recovery_action_target_path(
                    &original,
                    &recovery_source_path,
                    request.target_path.as_deref(),
                )?
            };
            (
                if source_cleanup_recovery {
                    "move"
                } else {
                    "copy"
                }
                .to_string(),
                Some(target),
            )
        }
        "move" | "move_recovery_item" | "move-recovery-item" => (
            "move".to_string(),
            Some(recovery_action_target_path(
                &original,
                &recovery_source_path,
                request.target_path.as_deref(),
            )?),
        ),
        "delete" | "delete_recovery_item" | "delete-recovery-item" => {
            ("permanent_delete".to_string(), None)
        }
        _ => return Err("recovery_action_unknown".to_string()),
    };
    let source_path = recovery_source_path;
    let source_name = source_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "recovery_claim_name_invalid".to_string())?
        .to_string();
    let target_for_request = target_path
        .as_deref()
        .unwrap_or("Permanent deletion quarantine")
        .to_string();
    let target_name = target_path
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|value| value.to_str())
        .unwrap_or(&source_name)
        .to_string();
    let operation = OperationPreviewRequest {
        id: new_job_id("recovery-action"),
        file_id: RECOVERY_ACTION_FILE_ID.to_string(),
        operation_type,
        source_path: normalize_path(&source_path),
        target_path: target_for_request,
        old_name: source_name,
        new_name: target_name,
        is_executable: Some(true),
    };

    let guard = cancel.begin()?;
    let cancel_flag = Arc::clone(&cancel.cancel);
    let app_data_dir = app.path().app_data_dir().ok();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let emitter = TauriOperationProgressEmitter::new(app);
        let action_result = execute_moves_with_persistence_with_progress_and_app_data(
            &db,
            ExecuteMovesRequest {
                operations: vec![operation],
            },
            cancel_flag,
            &emitter,
            app_data_dir,
            None,
        )?;
        let action_log = action_result
            .logs
            .into_iter()
            .next()
            .ok_or_else(|| "recovery_action_log_missing".to_string())?;
        if action_log.status != "success" {
            return Err(action_log
                .error_message
                .clone()
                .unwrap_or_else(|| "recovery_action_failed".to_string()));
        }

        let mut updated = original;
        if retry_source_cleanup
            || (source_cleanup_recovery && action_log.operation_type == "permanent_delete")
        {
            updated.status = "success".to_string();
            updated.error_message = None;
            updated.operation_phase = "completed".to_string();
            updated.source_claim_path = None;
            updated.can_undo = updated.operation_type != "permanent_delete";
            updated.can_restore = updated.can_undo;
            updated.restore_status = "not_restored".to_string();
            updated.restore_phase = "idle".to_string();
            updated.restore_error = None;
            db.finalize_source_cleanup_retry(
                &updated.id,
                expected_claim_path
                    .as_deref()
                    .ok_or_else(|| "recovery_source_claim_missing".to_string())?,
                updated.can_restore,
            )
            .map_err(|error| error.to_string())?;
            return Ok(RecoveryActionResult {
                original_log: updated,
                target_path: None,
                action_log,
            });
        }
        if source_cleanup_recovery {
            updated.status = "manual_review".to_string();
            updated.operation_phase = "manual_review".to_string();
            updated.source_claim_path = None;
            updated.can_undo = false;
            updated.can_restore = false;
            updated.restored_at = None;
            updated.restore_status = "manual_review".to_string();
            updated.restore_phase = "manual_review".to_string();
            updated.restore_error = Some(match action_log.operation_type.as_str() {
                "move" => format!(
                    "recovery_action_keep_both_completed:{}",
                    action_log.path_after
                ),
                _ => format!("recovery_action_completed:{}", action_log.path_after),
            });
            updated.restore_claim_path = Some(action_log.path_after.clone());
            updated.restore_claim_created_at = action_log.claim_created_at.clone();
            updated.restore_claim_platform_file_id = action_log.target_platform_file_id.clone();
            updated.restore_claim_platform_volume_id = action_log.target_platform_volume_id.clone();
            updated.restore_claim_full_hash = action_log.target_full_hash.clone();
            db.finalize_source_cleanup_recovery_action(
                &updated,
                expected_claim_path
                    .as_deref()
                    .ok_or_else(|| "recovery_source_claim_missing".to_string())?,
            )
            .map_err(|error| error.to_string())?;
            return Ok(RecoveryActionResult {
                original_log: updated,
                target_path: Some(action_log.path_after.clone()),
                action_log,
            });
        }
        updated.status = "manual_review".to_string();
        updated.can_undo = false;
        updated.can_restore = false;
        updated.restored_at = None;
        updated.restore_status = "manual_review".to_string();
        // This is a resolved human decision, not an active restore phase.  Do
        // not leave it in `completed`, because startup reconciliation treats
        // manual-review/completed rows as interrupted restore journals.
        updated.restore_phase = "manual_review".to_string();
        updated.restore_error = Some(match action_log.operation_type.as_str() {
            "copy" => format!(
                "recovery_action_keep_both_completed:{}",
                action_log.path_after
            ),
            "move" => format!("recovery_action_move_completed:{}", action_log.path_after),
            "permanent_delete" => "recovery_action_delete_completed".to_string(),
            _ => "recovery_action_completed".to_string(),
        });
        match action_log.operation_type.as_str() {
            "move" => {
                updated.restore_claim_path = Some(action_log.path_after.clone());
                updated.restore_claim_created_at = action_log.claim_created_at.clone();
                updated.restore_claim_platform_file_id = action_log.target_platform_file_id.clone();
                updated.restore_claim_platform_volume_id =
                    action_log.target_platform_volume_id.clone();
                updated.restore_claim_full_hash = action_log.target_full_hash.clone();
            }
            "copy"
                if updated.operation_type == "replace" || updated.restore_claim_path.is_none() =>
            {
                updated.restore_claim_path = Some(action_log.source_path.clone());
                updated.restore_claim_created_at = action_log.claim_created_at.clone();
                updated.restore_claim_platform_file_id = action_log.source_platform_file_id.clone();
                updated.restore_claim_platform_volume_id =
                    action_log.source_platform_volume_id.clone();
                updated.restore_claim_full_hash = action_log.source_full_hash.clone();
            }
            "permanent_delete" => clear_restore_claim(&mut updated),
            _ => {}
        }
        db.finalize_operation_recovery_action(&updated, expected_claim_path.as_deref())
            .map_err(|error| error.to_string())?;
        Ok(RecoveryActionResult {
            original_log: updated,
            target_path: (action_log.operation_type != "permanent_delete")
                .then(|| action_log.path_after.clone()),
            action_log,
        })
    })
    .await
    .map_err(|error| format!("recovery action task failed: {error}"))?
}

fn source_cleanup_claim_identity_matches(
    log: &OperationLogDto,
    claim_path: &Path,
) -> Result<bool, String> {
    let expected = expected_identity_from_log(log)
        .ok_or_else(|| "recovery_claim_identity_missing".to_string())?;
    let actual = if expected.full_hash.is_some() || expected.sample_hash.is_some() {
        crate::fs_safety::capture_identity(claim_path, None).map_err(|error| error.to_string())?
    } else {
        crate::fs_safety::capture_namespace_identity_only(claim_path, None)
            .map_err(|error| error.to_string())?
    };
    Ok(crate::fs_safety::recovery_identity_matches(
        &expected, &actual,
    ))
}

fn recovery_action_target_path(
    log: &OperationLogDto,
    recovery_source_path: &Path,
    requested_path: Option<&str>,
) -> Result<String, String> {
    let candidate = requested_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| default_recovery_target_path(log, recovery_source_path))?;
    let target = validate_target_path(&candidate)?;
    Ok(normalize_path(&target))
}

fn default_recovery_target_path(
    log: &OperationLogDto,
    recovery_source_path: &Path,
) -> Result<PathBuf, String> {
    let preferred = Path::new(&log.path_before);
    let parent = preferred
        .parent()
        .filter(|path| path.is_dir())
        .or_else(|| recovery_source_path.parent().filter(|path| path.is_dir()))
        .ok_or_else(|| "recovery_target_parent_missing".to_string())?;
    let original_name = preferred
        .file_name()
        .and_then(|value| value.to_str())
        .or_else(|| {
            recovery_source_path
                .file_name()
                .and_then(|value| value.to_str())
        })
        .unwrap_or("recovered-item");
    let path = Path::new(original_name);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && validate_safe_file_name(value).is_ok())
        .unwrap_or("recovered-item");
    for index in 0..10_000_u32 {
        let suffix = if index == 0 {
            " (recovered)".to_string()
        } else {
            format!(" (recovered {index})")
        };
        let candidate = parent.join(format!("{stem}{suffix}{extension}"));
        if fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    Err("recovery_target_name_exhausted".to_string())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::db::{Database, InsertFileRequest, OperationPreviewDto};
    use std::{
        fs,
        sync::{
            atomic::{AtomicBool, AtomicU64, Ordering},
            Arc,
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    static TEST_FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Environmental-failure resilience for tests that move real files
    /// (`docs/remediation/issue-file-ops-flaky.md`, F1).  Under system load,
    /// external processes (antivirus / indexer scans) briefly hold handles on
    /// freshly created fixtures, so a move fails with a Windows sharing
    /// violation (`os error 32`) or trips the post-commit identity check into
    /// `target_committed_identity_mismatch`.  Both are environment noise, not
    /// logic regressions — the safety layer responding to them is behaving
    /// correctly.  Affected tests rebuild their fixture from scratch and retry
    /// a bounded number of times; every retry is reported and the run aborts
    /// loudly once the limit is reached.  Silent retry is forbidden (裁决 D7).
    const ENVIRONMENTAL_ATTEMPT_LIMIT: usize = 3;

    fn environmental_failure_signature(logs: &[OperationLogDto]) -> Option<String> {
        logs.iter().find_map(|log| {
            [log.error_message.as_deref(), log.restore_error.as_deref()]
                .into_iter()
                .flatten()
                .find(|message| {
                    message.contains("os error 32")
                        || message.contains("target_committed_identity_mismatch")
                })
                .map(|message| format!("{}: {message}", log.id))
        })
    }

    /// Appends one line per environmental event to the file named by
    /// `ZEN_CANVAS_ENV_RETRY_LOG` (set by CI, F2) so retries leave a durable,
    /// countable trace even when the test ultimately passes.
    fn record_environmental_event(test_name: &str, event: &str) {
        eprintln!("[env-retry] {test_name}: {event}");
        if let Ok(log_path) = std::env::var("ZEN_CANVAS_ENV_RETRY_LOG") {
            if let Ok(mut file) = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
            {
                use std::io::Write as _;
                // Parallel tests append concurrently: emit the whole line in one
                // write so records never interleave mid-line.
                let line = format!("{test_name}\t{event}\n");
                let _ = file.write_all(line.as_bytes());
            }
        }
    }

    fn with_environmental_retry(test_name: &str, mut attempt: impl FnMut() -> Result<(), String>) {
        let mut signatures = Vec::new();
        for attempt_number in 1..=ENVIRONMENTAL_ATTEMPT_LIMIT {
            match attempt() {
                Ok(()) => {
                    if attempt_number > 1 {
                        record_environmental_event(
                            test_name,
                            &format!(
                                "passed on attempt {attempt_number}/{ENVIRONMENTAL_ATTEMPT_LIMIT} after: {signatures:?}"
                            ),
                        );
                    }
                    return;
                }
                Err(signature) => {
                    record_environmental_event(
                        test_name,
                        &format!(
                            "environmental failure on attempt {attempt_number}/{ENVIRONMENTAL_ATTEMPT_LIMIT}: {signature}"
                        ),
                    );
                    signatures.push(signature);
                }
            }
        }
        record_environmental_event(
            test_name,
            &format!("exhausted {ENVIRONMENTAL_ATTEMPT_LIMIT} attempts: {signatures:?}"),
        );
        panic!(
            "[env-retry] {test_name}: environmental failure persisted through \
             {ENVIRONMENTAL_ATTEMPT_LIMIT} attempts ({signatures:?}); see \
             docs/remediation/issue-file-ops-flaky.md — investigate, do not dismiss by rerunning"
        );
    }

    #[test]
    fn environmental_retry_reports_and_recovers_within_the_attempt_limit() {
        let mut attempts = 0;
        with_environmental_retry("environmental-retry-recovers", || {
            attempts += 1;
            if attempts < ENVIRONMENTAL_ATTEMPT_LIMIT {
                Err(format!("op-{attempts}: simulated (os error 32)"))
            } else {
                Ok(())
            }
        });
        assert_eq!(attempts, ENVIRONMENTAL_ATTEMPT_LIMIT);
    }

    #[test]
    #[should_panic(expected = "environmental failure persisted")]
    fn environmental_retry_fails_loudly_when_attempts_are_exhausted() {
        with_environmental_retry("environmental-retry-exhausted", || {
            Err("op-x: simulated (os error 32)".to_string())
        });
    }

    #[test]
    fn execute_selection_resolves_authoritative_paths_from_database() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("source.txt");
        let target_dir = root.join("organized");
        fs::write(&source, "hello").expect("write source");
        insert_indexed_file(&db, &source, "source.txt", "txt");
        let file_id = source.to_string_lossy().into_owned();
        let metadata = fs::metadata(&source).expect("source metadata");
        let mtime = metadata
            .modified()
            .expect("mtime")
            .duration_since(UNIX_EPOCH)
            .expect("unix mtime")
            .as_secs() as i64;
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET suggested_action = 'Move', suggested_target_path = ?2, suggested_name = 'source.txt', confidence = 0.95, size = ?3, mtime = ?4 WHERE path = ?1",
            rusqlite::params![file_id, normalize_path(&target_dir), metadata.len() as i64, mtime],
        )
        .expect("set suggestion");
        let preview = db
            .get_operation_previews_by_file_ids(std::slice::from_ref(&file_id))
            .expect("preview")
            .pop()
            .expect("operation preview");

        let request = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![OperationSelection {
                    id: preview.id,
                    file_id,
                    operation_fingerprint: preview
                        .operation_fingerprint
                        .clone()
                        .expect("fingerprint"),
                    expected_revision: preview.operation_fingerprint.clone().expect("fingerprint"),
                    new_name: None,
                    destination_directory: None,
                }],
            },
        )
        .expect("resolve selection");

        assert_eq!(
            normalize_path(Path::new(&request.operations[0].source_path)),
            normalize_path(&source)
        );
        assert_eq!(
            request.operations[0].target_path,
            normalize_path(&target_dir.join("source.txt"))
        );
    }

    #[test]
    fn execute_selection_preserves_indexed_extension_and_rejects_tampering() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("Install_Package.lnk");
        let target_dir = root.join("organized");
        fs::write(&source, b"shortcut fixture").expect("write shortcut");
        insert_indexed_file(&db, &source, "Install_Package.lnk", "lnk");
        let file_id = source.to_string_lossy().into_owned();
        let metadata = fs::metadata(&source).expect("shortcut metadata");
        let mtime = metadata
            .modified()
            .expect("shortcut mtime")
            .duration_since(UNIX_EPOCH)
            .expect("unix mtime")
            .as_secs() as i64;
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET suggested_action = 'Move', suggested_target_path = ?2, suggested_name = 'Install_Package', confidence = 0.95, size = ?3, mtime = ?4 WHERE path = ?1",
            rusqlite::params![file_id, normalize_path(&target_dir), metadata.len() as i64, mtime],
        )
        .expect("set shortcut suggestion");
        let preview = db
            .get_operation_previews_by_file_ids(std::slice::from_ref(&file_id))
            .expect("shortcut preview")
            .pop()
            .expect("shortcut operation preview");

        let normalized = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![OperationSelection {
                    id: preview.id.clone(),
                    file_id: file_id.clone(),
                    operation_fingerprint: preview
                        .operation_fingerprint
                        .clone()
                        .expect("fingerprint"),
                    expected_revision: preview.operation_fingerprint.clone().expect("fingerprint"),
                    new_name: Some("Install_Package".to_string()),
                    destination_directory: None,
                }],
            },
        )
        .expect("missing shortcut extension is normalized");
        assert_eq!(normalized.operations[0].new_name, "Install_Package.lnk");
        assert!(normalized.operations[0]
            .target_path
            .ends_with("Install_Package.lnk"));

        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![OperationSelection {
                    id: preview.id,
                    file_id,
                    operation_fingerprint: preview
                        .operation_fingerprint
                        .clone()
                        .expect("fingerprint"),
                    expected_revision: preview.operation_fingerprint.clone().expect("fingerprint"),
                    new_name: Some("Install_Package.exe".to_string()),
                    destination_directory: None,
                }],
            },
        )
        .expect_err("extension tampering must be rejected");
        assert!(error.contains("Changing a file extension is not allowed during organization."));
        assert_eq!(
            fs::read(&source).expect("read shortcut"),
            b"shortcut fixture"
        );
    }

    #[test]
    fn execute_selection_rejects_forged_preview_id() {
        let db = Database::open(test_db_path()).expect("open database");

        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![OperationSelection {
                    id: "op-forged".to_string(),
                    file_id: "file-forged".to_string(),
                    operation_fingerprint: String::new(),
                    expected_revision: String::new(),
                    new_name: None,
                    destination_directory: None,
                }],
            },
        )
        .expect_err("reject forged selection");

        assert_eq!(error, "operation_preview_stale");
    }

    #[test]
    fn r_fl_01_t1_exact_fingerprint_executes_current_preview() {
        with_environmental_retry("r-fl-01-t1-exact-fingerprint", || {
            let (db, db_path, root, source, target_dir, file_id) =
                create_authoritative_move_fixture();
            let preview = current_operation_preview(&db, &file_id);
            let result = execute_moves_with_persistence(
                &db,
                resolve_execute_selections(
                    &db,
                    ExecuteMovesByIdRequest {
                        operations: vec![selection_for_preview(&preview, None)],
                    },
                )
                .expect("resolve current preview"),
            );
            match result {
                Ok(executed) => {
                    if let Some(signature) = environmental_failure_signature(&executed.logs) {
                        cleanup_preview_fixture(db, db_path, root);
                        return Err(signature);
                    }
                    assert_eq!(executed.logs.len(), 1);
                    assert_eq!(executed.logs[0].status, "success");
                    assert!(!source.exists());
                    assert!(target_dir.join("source.txt").exists());
                    assert_eq!(operation_log_count(&db), 1);
                    cleanup_preview_fixture(db, db_path, root);
                    Ok(())
                }
                Err(error) if error.contains("os error 32") => {
                    cleanup_preview_fixture(db, db_path, root);
                    Err(error)
                }
                Err(error) => {
                    cleanup_preview_fixture(db, db_path, root);
                    panic!("exact current preview failed unexpectedly: {error}");
                }
            }
        });
    }

    #[test]
    fn r_fl_01_t2_target_change_invalidates_preview_before_journal() {
        let (db, db_path, root, source, _target_dir, file_id) = create_authoritative_move_fixture();
        let preview = current_operation_preview(&db, &file_id);
        let changed_target = root.join("changed-target");
        fs::create_dir_all(&changed_target).expect("changed target");
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET suggested_target_path = ?2 WHERE id = ?1",
            rusqlite::params![file_id, normalize_path(&changed_target)],
        )
        .expect("change target suggestion");

        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![selection_for_preview(&preview, None)],
            },
        )
        .expect_err("changed target must be stale");
        assert_eq!(error, "operation_preview_stale");
        assert!(source.exists());
        assert_eq!(operation_log_count(&db), 0);
        cleanup_preview_fixture(db, db_path, root);
    }

    #[test]
    fn r_fl_01_t3_operation_type_change_invalidates_preview_before_journal() {
        let (db, db_path, root, source, _target_dir, file_id) = create_authoritative_move_fixture();
        let preview = current_operation_preview(&db, &file_id);
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET suggested_action = 'Copy' WHERE id = ?1",
            rusqlite::params![file_id],
        )
        .expect("change operation type");

        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![selection_for_preview(&preview, None)],
            },
        )
        .expect_err("changed operation type must be stale");
        assert_eq!(error, "operation_preview_stale");
        assert!(source.exists());
        assert_eq!(operation_log_count(&db), 0);
        cleanup_preview_fixture(db, db_path, root);
    }

    #[test]
    fn r_fl_01_t4_source_change_invalidates_preview_before_identity_check() {
        let (db, db_path, root, source, _target_dir, file_id) = create_authoritative_move_fixture();
        let preview = current_operation_preview(&db, &file_id);
        let changed_source = root.join("changed-source.txt");
        fs::write(&changed_source, "changed source").expect("changed source");
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET path = ?2 WHERE id = ?1",
            rusqlite::params![file_id, changed_source.to_string_lossy().into_owned()],
        )
        .expect("change source path");

        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![selection_for_preview(&preview, None)],
            },
        )
        .expect_err("changed source must be stale");
        assert_eq!(error, "operation_preview_stale");
        assert!(source.exists());
        assert!(changed_source.exists());
        assert_eq!(operation_log_count(&db), 0);
        cleanup_preview_fixture(db, db_path, root);
    }

    #[test]
    fn r_fl_01_t5_missing_fingerprint_and_t6_mismatched_revision_fail_closed() {
        let (db, db_path, root, source, _target_dir, file_id) = create_authoritative_move_fixture();
        let preview = current_operation_preview(&db, &file_id);

        let mut missing = selection_for_preview(&preview, None);
        missing.operation_fingerprint.clear();
        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![missing],
            },
        )
        .expect_err("missing fingerprint must be stale");
        assert_eq!(error, "operation_preview_stale");

        let mut mismatched = selection_for_preview(&preview, None);
        mismatched.expected_revision = "different-revision".to_string();
        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![mismatched],
            },
        )
        .expect_err("mismatched revision must be stale");
        assert_eq!(error, "operation_preview_stale");
        assert!(source.exists());
        assert_eq!(operation_log_count(&db), 0);
        cleanup_preview_fixture(db, db_path, root);
    }

    #[test]
    fn r_fl_01_t7_batch_with_one_stale_selection_has_no_side_effects() {
        let db_path = test_db_path();
        let db = Database::open(&db_path).expect("open database");
        let root = test_dir();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source dir");
        fs::create_dir_all(&target_dir).expect("target dir");
        let mut sources = Vec::new();
        let mut file_ids = Vec::new();
        for index in 0..20 {
            let source = source_dir.join(format!("batch-{index}.txt"));
            fs::write(&source, format!("batch {index}")).expect("batch source");
            insert_indexed_file(&db, &source, &format!("batch-{index}.txt"), "txt");
            set_indexed_move_suggestion(&db, &source, &target_dir, &format!("batch-{index}.txt"));
            file_ids.push(source.to_string_lossy().into_owned());
            sources.push(source);
        }
        let previews = db
            .get_operation_previews_by_file_ids(&file_ids)
            .expect("batch previews");
        assert_eq!(previews.len(), 20);
        let selections = previews
            .iter()
            .map(|preview| selection_for_preview(preview, None))
            .collect::<Vec<_>>();
        let changed_target = root.join("changed-target");
        fs::create_dir_all(&changed_target).expect("changed target");
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET suggested_target_path = ?2 WHERE id = ?1",
            rusqlite::params![file_ids[19], normalize_path(&changed_target)],
        )
        .expect("stale one batch item");

        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: selections,
            },
        )
        .expect_err("one stale item must reject the whole batch");
        assert_eq!(error, "operation_preview_stale");
        assert!(sources.iter().all(|source| source.exists()));
        assert_eq!(
            fs::read_dir(&target_dir).expect("target entries").count(),
            0
        );
        assert_eq!(operation_log_count(&db), 0);
        cleanup_preview_fixture(db, db_path, root);
    }

    #[test]
    fn r_fl_01_t9_changed_target_rejects_valid_name_override() {
        let (db, db_path, root, source, _target_dir, file_id) = create_authoritative_move_fixture();
        let preview = current_operation_preview(&db, &file_id);
        let changed_target = root.join("changed-target");
        fs::create_dir_all(&changed_target).expect("changed target");
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET suggested_target_path = ?2 WHERE id = ?1",
            rusqlite::params![file_id, normalize_path(&changed_target)],
        )
        .expect("change target suggestion");

        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![selection_for_preview(&preview, Some("renamed"))],
            },
        )
        .expect_err("new name cannot bypass a changed canonical target");
        assert_eq!(error, "operation_preview_stale");
        assert!(source.exists());
        assert_eq!(operation_log_count(&db), 0);
        cleanup_preview_fixture(db, db_path, root);
    }

    #[test]
    fn r_fl_01_t10_windows_permanent_delete_is_unsupported_without_fallback() {
        let db_path = test_db_path();
        let db = Database::open(&db_path).expect("open database");
        let error = db
            .get_permanent_delete_operation_preview("missing-file")
            .expect_err("Windows must not expose a permanent delete preview");
        assert_eq!(error.to_string(), "permanent_delete_unsupported");
        drop(db);
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn r_fl_01_t15_risk_only_change_invalidates_independently() {
        let (db, db_path, root, source, _target_dir, file_id) = create_authoritative_move_fixture();
        let preview = current_operation_preview(&db, &file_id);
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET risk_level = 'System' WHERE id = ?1",
            rusqlite::params![file_id],
        )
        .expect("change risk only");
        let changed = current_operation_preview(&db, &file_id);
        assert_eq!(changed.operation_type, preview.operation_type);
        assert_eq!(changed.source_path, preview.source_path);
        assert_eq!(changed.target_path, preview.target_path);
        assert_ne!(changed.operation_fingerprint, preview.operation_fingerprint);
        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![selection_for_preview(&preview, None)],
            },
        )
        .expect_err("risk-only change must be stale");
        assert_eq!(error, "operation_preview_stale");
        assert!(source.exists());
        assert_eq!(operation_log_count(&db), 0);
        cleanup_preview_fixture(db, db_path, root);
    }

    #[test]
    fn r_fl_01_t15_confirmation_only_change_invalidates_independently() {
        let (db, db_path, root, source, _target_dir, file_id) = create_authoritative_move_fixture();
        let preview = current_operation_preview(&db, &file_id);
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET requires_confirmation = 1 WHERE id = ?1",
            rusqlite::params![file_id],
        )
        .expect("change confirmation only");
        let changed = current_operation_preview(&db, &file_id);
        assert_eq!(changed.operation_type, preview.operation_type);
        assert_eq!(changed.source_path, preview.source_path);
        assert_eq!(changed.target_path, preview.target_path);
        assert_ne!(changed.operation_fingerprint, preview.operation_fingerprint);
        assert!(changed.requires_confirmation);
        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![selection_for_preview(&preview, None)],
            },
        )
        .expect_err("confirmation-only change must be stale");
        assert_eq!(error, "operation_preview_stale");
        assert!(source.exists());
        assert_eq!(operation_log_count(&db), 0);
        cleanup_preview_fixture(db, db_path, root);
    }

    #[test]
    fn r_fl_01_t8_parent_creation_change_invalidates_before_journal() {
        let (db, db_path, root, source, target_dir, file_id) = create_authoritative_move_fixture();
        let preview = current_operation_preview(&db, &file_id);
        assert_eq!(preview.target_parent_exists, Some(true));
        assert_eq!(preview.will_create_parent, Some(false));

        fs::remove_dir(&target_dir).expect("remove target parent without touching source");
        let changed = current_operation_preview(&db, &file_id);
        assert_eq!(changed.file_id, preview.file_id);
        assert_eq!(changed.operation_type, preview.operation_type);
        assert_eq!(changed.source_path, preview.source_path);
        assert_eq!(changed.target_path, preview.target_path);
        assert_eq!(changed.risk_level, preview.risk_level);
        assert_eq!(changed.requires_confirmation, preview.requires_confirmation);
        assert_eq!(changed.is_executable, preview.is_executable);
        assert_eq!(
            changed.provider_identity_fingerprint,
            preview.provider_identity_fingerprint
        );
        assert_eq!(
            changed.source_identity_fingerprint,
            preview.source_identity_fingerprint
        );
        assert_eq!(changed.target_parent_exists, Some(false));
        assert_eq!(changed.will_create_parent, Some(true));
        assert_ne!(changed.operation_fingerprint, preview.operation_fingerprint);

        let error = resolve_execute_selections(
            &db,
            ExecuteMovesByIdRequest {
                operations: vec![selection_for_preview(&preview, None)],
            },
        )
        .expect_err("parent-creation change must be stale");
        assert_eq!(error, "operation_preview_stale");
        assert!(source.exists());
        assert!(!target_dir.exists());
        assert!(!target_dir.join("source.txt").exists());
        assert_eq!(operation_log_count(&db), 0);
        cleanup_preview_fixture(db, db_path, root);
    }

    #[test]
    fn restore_volume_relation_is_three_state_and_preserves_hash_fallbacks() {
        let operation = OperationPreviewRequest {
            id: "restore-volume-relation".to_string(),
            file_id: "restore-volume-file".to_string(),
            operation_type: "rename".to_string(),
            source_path: "C:/restore/before.txt".to_string(),
            target_path: "D:/restore/after.txt".to_string(),
            old_name: "before.txt".to_string(),
            new_name: "after.txt".to_string(),
            is_executable: Some(false),
        };
        let mut log = make_operation_log(
            "restore-volume-batch",
            "1900000000000",
            0,
            &operation,
            "success",
            None,
            operation.target_path.clone(),
        );
        log.source_size = Some(7);
        log.source_quick_hash = Some("quick".to_string());
        log.source_full_hash = Some("source-full".to_string());
        log.target_full_hash = Some("target-full".to_string());
        log.source_platform_file_id = Some("source-file-id".to_string());
        log.target_platform_file_id = Some("target-file-id".to_string());

        log.source_platform_volume_id = Some("volume-a".to_string());
        log.target_platform_volume_id = Some("volume-a".to_string());
        assert_eq!(
            restore_volume_relation(&log),
            RestoreVolumeRelation::SameVolume
        );
        let same = expected_restore_identity_from_log(&log).expect("same-volume identity");
        assert_eq!(same.platform_volume_id.as_deref(), Some("volume-a"));
        assert_eq!(same.platform_file_id.as_deref(), Some("target-file-id"));

        log.target_platform_volume_id = Some("volume-b".to_string());
        assert_eq!(
            restore_volume_relation(&log),
            RestoreVolumeRelation::CrossVolume
        );
        let cross = expected_restore_identity_from_log(&log).expect("cross-volume identity");
        assert_eq!(cross.platform_volume_id.as_deref(), Some("volume-b"));
        assert!(cross.platform_file_id.is_none());

        log.source_platform_volume_id = None;
        log.target_platform_volume_id = None;
        assert_eq!(
            restore_volume_relation(&log),
            RestoreVolumeRelation::Unknown
        );
        let unknown = expected_restore_identity_from_log(&log).expect("unknown-volume identity");
        assert!(unknown.platform_volume_id.is_none());
        assert!(unknown.platform_file_id.is_none());
        assert_eq!(unknown.full_hash.as_deref(), Some("target-full"));

        log.source_platform_volume_id = Some("volume-a".to_string());
        assert_eq!(
            restore_volume_relation(&log),
            RestoreVolumeRelation::Unknown
        );
        assert!(expected_restore_identity_from_log(&log)
            .expect("one-volume identity")
            .platform_file_id
            .is_none());

        log.source_platform_volume_id = None;
        log.target_platform_volume_id = Some("volume-b".to_string());
        assert_eq!(
            restore_volume_relation(&log),
            RestoreVolumeRelation::Unknown
        );
        assert!(expected_restore_identity_from_log(&log)
            .expect("target-only-volume identity")
            .platform_file_id
            .is_none());
    }

    #[test]
    fn restore_unknown_volume_uses_content_identity_and_rejects_hash_mismatch() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let after = root.join("restore-unknown-volume-after.txt");
        fs::write(&after, "unknown-volume-content").expect("write restore source");
        let identity = file_identity_fingerprint(&after).expect("capture restore source");
        let operation = OperationPreviewRequest {
            id: "restore-unknown-volume".to_string(),
            file_id: "restore-unknown-file".to_string(),
            operation_type: "rename".to_string(),
            source_path: root.join("before.txt").to_string_lossy().into_owned(),
            target_path: normalize_path(&after),
            old_name: "before.txt".to_string(),
            new_name: "restore-unknown-volume-after.txt".to_string(),
            is_executable: Some(false),
        };
        let mut log = make_operation_log(
            "restore-unknown-batch",
            "1900000000000",
            0,
            &operation,
            "success",
            None,
            operation.target_path.clone(),
        );
        log.source_size = Some(identity.size);
        log.source_quick_hash = identity.quick_hash;
        log.source_full_hash = identity.full_hash.clone();
        log.target_full_hash = identity.full_hash;
        log.source_platform_volume_id = None;
        log.target_platform_volume_id = None;
        log.source_platform_file_id = Some("old-source-id".to_string());
        log.target_platform_file_id = Some("old-target-id".to_string());

        assert!(operation_restore_identity_result(&log, &after).is_ok());
        log.target_full_hash = Some("wrong-hash".to_string());
        let error = operation_restore_identity_result(&log, &after)
            .expect_err("unknown-volume hash mismatch must fail closed");
        assert_eq!(
            error.code,
            crate::recovery::RecoveryErrorCode::RestoreSourceIdentityMismatch
        );
        drop(db);
    }

    #[test]
    fn pending_operation_journal_reconciles_a_move_completed_before_restart() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("before.txt");
        let target = root.join("after.txt");
        fs::write(&source, "hello").expect("write source");
        let request = ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "op-recovery".to_string(),
                file_id: "file-recovery".to_string(),
                operation_type: "rename".to_string(),
                source_path: normalize_path(&source),
                target_path: normalize_path(&target),
                old_name: "before.txt".to_string(),
                new_name: "after.txt".to_string(),
                is_executable: Some(true),
            }],
        };
        persist_pending_operation_journal(&db, &request, "batch-recovery", "1900000000000")
            .expect("persist pending journal");
        assert_eq!(db.get_pending_operation_logs().expect("pending").len(), 1);
        fs::rename(&source, &target).expect("simulate completed filesystem move");
        let target_file = fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .expect("open moved target");
        target_file
            .set_times(fs::FileTimes::new().set_modified(SystemTime::now()))
            .expect("change target mtime");

        let reconciled = reconcile_pending_operation_journal(&db).expect("reconcile journal");
        let logs = db.get_operation_logs(Some(10)).expect("logs");

        assert_eq!(reconciled, 1);
        assert_eq!(logs[0].status, "success");
        assert!(logs[0].can_restore);
        assert!(db
            .get_pending_operation_logs()
            .expect("pending after")
            .is_empty());
    }

    #[test]
    fn pending_operation_journal_requires_matching_target_identity() {
        for (label, replacement) in [("size", "different-size"), ("hash", "world")] {
            let db = Database::open(test_db_path()).expect("open database");
            let root = test_dir();
            let source = root.join(format!("before-{label}.txt"));
            let target = root.join(format!("after-{label}.txt"));
            fs::write(&source, "hello").expect("write source");
            let request = ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: format!("op-{label}"),
                    file_id: format!("file-{label}"),
                    operation_type: "rename".to_string(),
                    source_path: normalize_path(&source),
                    target_path: normalize_path(&target),
                    old_name: format!("before-{label}.txt"),
                    new_name: format!("after-{label}.txt"),
                    is_executable: Some(true),
                }],
            };
            persist_pending_operation_journal(&db, &request, &format!("batch-{label}"), "1")
                .expect("persist pending journal");
            fs::remove_file(&source).expect("remove source");
            fs::write(&target, replacement).expect("write unrelated target");

            reconcile_pending_operation_journal(&db).expect("reconcile journal");
            let log = db.get_operation_logs(Some(1)).expect("logs").remove(0);

            assert_eq!(log.status, "manual_review");
            assert!(!log.can_restore);
            assert!(!log.can_undo);
        }
    }

    #[test]
    fn pending_move_hash_fallback_also_requires_matching_mtime() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("before-same-content.txt");
        let target = root.join("after-same-content.txt");
        fs::write(&source, "same-content").expect("write source");
        let request = ExecuteMovesRequest {
            operations: vec![preview_operation(0, &source, &target)],
        };
        persist_pending_operation_journal(&db, &request, "batch-mtime", "1")
            .expect("persist pending journal");
        fs::remove_file(&source).expect("remove source");
        fs::write(&target, "same-content").expect("write replacement target");
        fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .expect("open replacement")
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::now() - Duration::from_secs(24 * 60 * 60)),
            )
            .expect("change replacement mtime");

        reconcile_pending_operation_journal(&db).expect("reconcile journal");
        let log = db.get_operation_logs(Some(1)).expect("logs").remove(0);

        assert_eq!(log.status, "manual_review");
        assert!(!log.can_restore);
    }

    #[test]
    fn pending_operation_journal_marks_ambiguous_path_states_for_manual_review() {
        for (label, keep_source, create_target) in [("both", true, true), ("neither", false, false)]
        {
            let db = Database::open(test_db_path()).expect("open database");
            let root = test_dir();
            let source = root.join(format!("before-{label}.txt"));
            let target = root.join(format!("after-{label}.txt"));
            fs::write(&source, "hello").expect("write source");
            let request = ExecuteMovesRequest {
                operations: vec![preview_operation(0, &source, &target)],
            };
            persist_pending_operation_journal(&db, &request, &format!("batch-{label}"), "1")
                .expect("persist pending journal");
            if !keep_source {
                fs::remove_file(&source).expect("remove source");
            }
            if create_target {
                fs::write(&target, "hello").expect("write target");
            }

            reconcile_pending_operation_journal(&db).expect("reconcile journal");
            let log = db.get_operation_logs(Some(1)).expect("logs").remove(0);
            assert_eq!(log.status, "manual_review");
            assert!(!log.can_restore);
        }
    }

    #[test]
    fn target_first_source_retirement_pending_keeps_both_paths_recoverable() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("before-portable-pending.txt");
        let target = root.join("after-portable-pending.txt");
        fs::write(&source, "hello").expect("write source");
        let request = ExecuteMovesRequest {
            operations: vec![preview_operation(0, &source, &target)],
        };
        persist_pending_operation_journal(&db, &request, "batch-portable-pending", "1")
            .expect("persist pending journal");
        let mut pending = db
            .get_pending_operation_logs()
            .expect("pending logs")
            .remove(0);
        pending.status = "pending".to_string();
        pending.operation_phase = "source_cleanup_pending".to_string();
        pending.error_message = Some("mac_source_retirement_pending".to_string());
        db.update_operation_phase(&pending)
            .expect("persist source cleanup phase");
        fs::hard_link(&source, &target).expect("create verified target");

        reconcile_pending_operation_journal(&db).expect("reconcile journal");
        let log = db.get_operation_logs(Some(1)).expect("logs").remove(0);

        assert_eq!(log.status, "manual_review");
        assert_eq!(log.operation_phase, "source_cleanup_pending");
        assert!(log
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("portable_source_retirement_pending")));
        assert_eq!(fs::read(&source).expect("source remains"), b"hello");
        assert_eq!(fs::read(&target).expect("target remains"), b"hello");
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn pending_operation_journal_marks_target_and_claim_as_source_cleanup_pending() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("before-claim-pending.txt");
        let target = root.join("after-claim-pending.txt");
        fs::write(&source, "hello").expect("write source");
        let request = ExecuteMovesRequest {
            operations: vec![preview_operation(0, &source, &target)],
        };
        persist_pending_operation_journal(&db, &request, "batch-claim-pending", "1")
            .expect("persist pending journal");
        let pending = db
            .get_pending_operation_logs()
            .expect("pending logs")
            .remove(0);
        let claim = PathBuf::from(pending.source_claim_path.expect("claim path"));

        fs::hard_link(&source, &target).expect("create target hard link");
        fs::hard_link(&source, &claim).expect("create claim hard link");
        fs::remove_file(&source).expect("remove original source");

        reconcile_pending_operation_journal(&db).expect("reconcile journal");
        let log = db.get_operation_logs(Some(1)).expect("logs").remove(0);

        assert_eq!(log.status, "manual_review");
        assert_eq!(log.operation_phase, "source_cleanup_pending");
        assert!(!log.can_restore);
        assert!(!log.can_undo);
    }

    #[test]
    fn pending_restore_journal_reconciles_a_restore_completed_before_restart() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("before.txt");
        let target = root.join("after.txt");
        fs::write(&source, "hello").expect("write source");
        let executed = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: "op-restore-recovery".to_string(),
                    file_id: "file-restore-recovery".to_string(),
                    operation_type: "rename".to_string(),
                    source_path: normalize_path(&source),
                    target_path: normalize_path(&target),
                    old_name: "before.txt".to_string(),
                    new_name: "after.txt".to_string(),
                    is_executable: Some(true),
                }],
            },
        )
        .expect("execute move");
        let log_id = executed.logs[0].id.clone();
        db.mark_operation_restores_pending(std::slice::from_ref(&log_id))
            .expect("mark restore pending");
        fs::rename(&target, &source).expect("simulate completed restore");

        let reconciled = reconcile_pending_operation_journal(&db).expect("reconcile journal");
        let logs = db.get_operation_logs(Some(10)).expect("logs");
        let restored = logs
            .iter()
            .find(|log| log.id == log_id)
            .expect("restored log");

        assert_eq!(reconciled, 1);
        assert_eq!(restored.restore_status, "restored");
        assert!(!restored.can_restore);
        assert!(db
            .get_pending_restore_logs()
            .expect("pending after")
            .is_empty());
    }

    #[test]
    fn execute_moves_core_moves_files_and_returns_success_log() {
        with_environmental_retry(
            "execute_moves_core_moves_files_and_returns_success_log",
            || {
                let root = test_dir();
                let source_dir = root.join("source");
                let target_dir = root.join("target");
                fs::create_dir_all(&source_dir).expect("source dir");
                fs::create_dir_all(&target_dir).expect("target dir");

                let source = source_dir.join("sample.txt");
                let target = target_dir.join("sample.txt");
                fs::write(&source, "hello").expect("write source");

                let result = execute_moves_core(ExecuteMovesRequest {
                    operations: vec![OperationPreviewRequest {
                        id: "op-1".to_string(),
                        file_id: "file-1".to_string(),
                        operation_type: "move".to_string(),
                        source_path: source.to_string_lossy().into_owned(),
                        target_path: target.to_string_lossy().into_owned(),
                        old_name: "sample.txt".to_string(),
                        new_name: "sample.txt".to_string(),
                        is_executable: Some(true),
                    }],
                });
                if let Some(signature) = environmental_failure_signature(&result.logs) {
                    let _ = fs::remove_dir_all(&root);
                    return Err(signature);
                }

                assert!(!source.exists());
                assert!(target.exists());
                assert_eq!(result.logs.len(), 1);
                assert_eq!(result.logs[0].status, "success");
                assert_eq!(result.logs[0].operation_type, "move");
                Ok(())
            },
        );
    }

    #[test]
    fn execute_moves_result_does_not_serialize_unused_updated_files_contract() {
        let result = execute_moves_core(ExecuteMovesRequest {
            operations: Vec::new(),
        });
        let json = serde_json::to_value(&result).expect("serialize result");

        assert!(json.get("logs").is_some());
        assert!(json.get("batch_id").is_some());
        assert!(json.get("updatedFiles").is_none());
    }

    #[test]
    fn execute_moves_core_creates_safe_missing_target_parent() {
        with_environmental_retry(
            "execute_moves_core_creates_safe_missing_target_parent",
            || {
                let root = test_dir();
                let source_dir = root.join("source");
                let target_dir = root.join("ZenCanvas").join("20_Areas").join("Projects");
                fs::create_dir_all(&source_dir).expect("source dir");

                let source = source_dir.join("sample.txt");
                let target = target_dir.join("sample.txt");
                fs::write(&source, "hello").expect("write source");

                let result = execute_moves_core(ExecuteMovesRequest {
                    operations: vec![OperationPreviewRequest {
                        id: "op-create-parent".to_string(),
                        file_id: "file-create-parent".to_string(),
                        operation_type: "move".to_string(),
                        source_path: source.to_string_lossy().into_owned(),
                        target_path: target.to_string_lossy().into_owned(),
                        old_name: "sample.txt".to_string(),
                        new_name: "sample.txt".to_string(),
                        is_executable: Some(true),
                    }],
                });
                if let Some(signature) = environmental_failure_signature(&result.logs) {
                    let _ = fs::remove_dir_all(&root);
                    return Err(signature);
                }

                assert!(!source.exists());
                assert!(target.exists());
                assert_eq!(fs::read_to_string(&target).expect("read target"), "hello");
                assert_eq!(result.logs[0].status, "success");
                assert_eq!(
                    result.logs[0].source_path,
                    source.to_string_lossy().into_owned()
                );
                assert!(result.logs[0]
                    .target_path
                    .replace('\\', "/")
                    .ends_with("ZenCanvas/20_Areas/Projects/sample.txt"));
                Ok(())
            },
        );
    }

    #[test]
    fn execute_moves_core_refuses_to_overwrite_existing_target() {
        let root = test_dir();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source dir");
        fs::create_dir_all(&target_dir).expect("target dir");

        let source = source_dir.join("sample.txt");
        let target = target_dir.join("sample.txt");
        fs::write(&source, "hello").expect("write source");
        fs::write(&target, "existing").expect("write existing target");

        let result = execute_moves_core(ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "op-no-overwrite".to_string(),
                file_id: "file-no-overwrite".to_string(),
                operation_type: "move".to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: target.to_string_lossy().into_owned(),
                old_name: "sample.txt".to_string(),
                new_name: "sample.txt".to_string(),
                is_executable: Some(true),
            }],
        });

        assert!(source.exists());
        assert_eq!(
            fs::read_to_string(&target).expect("read target"),
            "existing"
        );
        assert_eq!(result.logs[0].status, "failed");
        assert!(result.logs[0]
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("Target file already exists"));
    }

    #[test]
    fn copy_fallback_writes_through_temp_file_then_removes_source() {
        let root = test_dir();
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, "fallback content").expect("write source");

        copy_then_delete_via_temp_with_cancel(&source, &target, None).expect("copy fallback");

        assert!(!source.exists());
        assert_eq!(
            fs::read_to_string(&target).expect("read target"),
            "fallback content"
        );
        let temp_entries = fs::read_dir(&root)
            .expect("read root")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".zencanvas-tmp-")
            })
            .count();
        assert_eq!(temp_entries, 0);
    }

    #[test]
    fn copy_stream_stops_after_chunk_when_cancelled() {
        let cancel_flag = AtomicBool::new(false);
        let content = b"abcdefghijkl";
        let mut reader = CancelAfterFirstRead::new(&content[..], &cancel_flag);
        let mut writer = Vec::new();

        let error = copy_stream_to_temp(&mut reader, &mut writer, Some(&cancel_flag), 4)
            .expect_err("copy should stop after cancellation");

        assert_eq!(
            error,
            crate::fs_safety::AtomicMoveError::Cancelled.to_string()
        );
        assert!(writer.len() < content.len());
        assert_eq!(writer, b"abcd");
    }

    #[test]
    fn copy_fallback_cancel_keeps_source_and_removes_temp() {
        let root = test_dir();
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, "fallback content").expect("write source");
        let cancel_flag = Arc::new(AtomicBool::new(true));

        let error = copy_then_delete_via_temp_with_cancel(&source, &target, Some(&cancel_flag))
            .expect_err("copy fallback should stop when canceled");

        assert_eq!(
            error,
            crate::fs_safety::AtomicMoveError::Cancelled.to_string()
        );
        assert_eq!(
            fs::read_to_string(&source).expect("source remains readable"),
            "fallback content"
        );
        assert!(!target.exists());
        let temp_entries = fs::read_dir(&root)
            .expect("read root")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".zencanvas-tmp-")
            })
            .count();
        assert_eq!(temp_entries, 0);
    }

    #[test]
    fn validate_safe_file_name_rejects_traversal_separators_and_nul() {
        for name in [
            "..",
            "../escape.txt",
            "..\\escape.txt",
            "safe..looking.txt",
            "nested/name.txt",
            "nested\\name.txt",
            "nul\0name.txt",
        ] {
            assert!(
                validate_safe_file_name(name).is_err(),
                "expected unsafe file name to be rejected: {name:?}"
            );
        }
    }

    #[test]
    fn validate_target_path_rejects_traversal_and_nul() {
        let root = test_dir();
        fs::create_dir_all(&root).expect("root dir");

        for target in [
            root.join("..").join("escape.txt"),
            root.join("safe..looking.txt"),
            root.join("nul\0name.txt"),
        ] {
            assert!(
                validate_target_path_with_parent_policy(&target, true).is_err(),
                "expected unsafe target path to be rejected: {target:?}"
            );
        }
    }

    #[test]
    fn validate_safe_file_name_rejects_baseline_dot_forms() {
        for name in ["report..draft.txt", "trailing."] {
            assert!(
                validate_safe_file_name(name).is_err(),
                "baseline filename safety rule should reject: {name}"
            );
        }
        assert!(validate_safe_file_name("normal.file.txt").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn validate_safe_file_name_keeps_windows_reserved_and_invalid_character_rules() {
        for name in ["CON", "report:stream.txt", "wildcard*.txt"] {
            assert!(
                validate_safe_file_name(name).is_err(),
                "Windows filename should remain invalid: {name}"
            );
        }
    }

    #[test]
    fn validate_target_path_rejects_protected_parent() {
        let Some(protected_root) = general_file_operation_protected_roots()
            .into_iter()
            .find(|root| root.exists())
        else {
            return;
        };
        let target = protected_root.join("zencanvas-should-not-write.txt");

        let error = validate_target_path_with_parent_policy(&target, false)
            .expect_err("protected parent should be rejected");

        assert!(error.contains("protected system location"));
    }

    #[test]
    fn general_file_operation_rejects_symlink_source() {
        let root = test_dir();
        let target = root.join("target.txt");
        let link = root.join("source-link.txt");
        fs::write(&target, "target").expect("write target");
        if create_file_symlink_for_test(&target, &link).is_err() {
            return;
        }

        let error = validate_source_path(&link).expect_err("symlink source must be rejected");

        assert!(error.contains("protected system location"));
    }

    #[test]
    fn general_file_operation_rejects_parent_segments_in_source() {
        let root = test_dir();
        let target = root.join("target.txt");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::write(&target, "target").expect("write target");
        let aliased = nested.join("..").join("target.txt");

        let error = validate_source_path(&aliased).expect_err("parent segment must be rejected");

        assert!(error.contains("unsafe parent traversal"));
    }

    #[test]
    fn general_move_rejects_private_var_log() {
        let error = ensure_general_file_operation_allowed_for_os(
            Path::new("/private/var/log/example.log"),
            "macos",
        )
        .expect_err("macOS private paths must be protected");

        assert!(error.contains("protected system location"));
    }

    #[test]
    fn general_operations_preserve_macos_path_case() {
        assert_eq!(
            normalize_for_compare_for_os(Path::new("/pRiVaTe/var/log/example.log"), "macos"),
            "/pRiVaTe/var/log/example.log"
        );
        let result = ensure_general_file_operation_allowed_for_os(
            Path::new("/pRiVaTe/var/log/example.log"),
            "macos",
        );

        assert!(result.is_ok(), "macOS path identity must preserve case");
    }
    #[test]
    fn nearest_existing_target_ancestor_resolves_symlinks_before_creation() {
        let root = test_dir();
        let real_parent = root.join("real-parent");
        let link = root.join("linked-parent");
        fs::create_dir_all(&real_parent).expect("real parent");
        if create_directory_symlink_for_test(&real_parent, &link).is_err() {
            return;
        }

        let resolved = canonicalize_nearest_existing_ancestor(&link.join("missing/child"))
            .expect("resolve existing symlink ancestor");

        assert_eq!(
            resolved,
            real_parent.canonicalize().expect("canonical real parent")
        );
        assert!(!real_parent.join("missing").exists());
    }

    #[test]
    fn general_rename_rejects_applications() {
        let error = ensure_general_file_operation_allowed_for_os(
            Path::new("/Applications/Example.app/file"),
            "macos",
        )
        .expect_err("macOS Applications paths must be protected");

        assert!(error.contains("protected system location"));
    }

    #[test]
    fn general_restore_rejects_protected_destination() {
        let error = ensure_general_file_operation_allowed_for_os(
            Path::new("/Applications/Restored.app/file"),
            "macos",
        )
        .expect_err("restore destinations in Applications must be protected");

        assert!(error.contains("protected system location"));
    }

    #[test]
    fn cleanup_exception_does_not_affect_general_move() {
        let error = ensure_general_file_operation_allowed_for_os(
            Path::new("/private/tmp/zen-canvas/example.tmp"),
            "macos",
        )
        .expect_err("cleanup temp exceptions must not relax general moves");
        assert!(error.contains("protected system location"));
    }

    #[test]
    fn general_operations_allow_only_the_current_macos_temp_root() {
        let current_temp = Path::new("/private/var/folders/current/T");

        ensure_general_file_operation_allowed_for_os_with_temp(
            Path::new("/private/var/folders/current/T/zen-canvas/source.txt"),
            "macos",
            Some(current_temp),
        )
        .expect("the current macOS temp subtree should be usable");

        let error = ensure_general_file_operation_allowed_for_os_with_temp(
            Path::new("/private/var/folders/another/T/zen-canvas/source.txt"),
            "macos",
            Some(current_temp),
        )
        .expect_err("another macOS temp subtree must remain protected");

        assert!(error.contains("protected system location"));
    }

    #[test]
    fn symlink_parent_cannot_escape_protected_root() {
        let Some(protected_root) = general_file_operation_protected_roots()
            .into_iter()
            .find(|root| root.is_dir())
        else {
            return;
        };
        let root = test_dir();
        let link = root.join("protected-link");
        if create_directory_symlink_for_test(&protected_root, &link).is_err() {
            return;
        }

        let error = validate_target_path(&link.join("escape.txt"))
            .expect_err("symlink parent must not escape into a protected root");

        assert!(error.contains("protected system location"));
    }

    #[test]
    fn execute_moves_core_marks_remaining_operations_skipped_when_cancelled() {
        with_environmental_retry(
            "execute_moves_core_marks_remaining_operations_skipped_when_cancelled",
            || {
                let root = test_dir();
                let source_dir = root.join("source");
                let target_dir = root.join("target");
                fs::create_dir_all(&source_dir).expect("source dir");
                fs::create_dir_all(&target_dir).expect("target dir");
                let operations = (0..11)
                    .map(|index| {
                        let source = source_dir.join(format!("sample-{index}.txt"));
                        let target = target_dir.join(format!("sample-{index}.txt"));
                        fs::write(&source, "hello").expect("write source");
                        preview_operation(index, &source, &target)
                    })
                    .collect::<Vec<_>>();
                let cancelled_source = PathBuf::from(&operations[10].source_path);
                let cancelled_target = PathBuf::from(&operations[10].target_path);
                let cancel_flag = Arc::new(AtomicBool::new(false));
                let progress =
                    RecordingOperationProgressEmitter::cancel_after(10, Arc::clone(&cancel_flag));

                let result = execute_moves_core_with_progress(
                    ExecuteMovesRequest { operations },
                    Arc::clone(&cancel_flag),
                    &progress,
                );
                if let Some(signature) = environmental_failure_signature(&result.logs) {
                    let _ = fs::remove_dir_all(&root);
                    return Err(signature);
                }

                run_cancelled_moves_assertions(
                    result,
                    &cancelled_source,
                    &cancelled_target,
                    &progress,
                );
                Ok(())
            },
        );
    }

    fn run_cancelled_moves_assertions(
        result: ExecuteMovesResult,
        cancelled_source: &Path,
        cancelled_target: &Path,
        progress: &RecordingOperationProgressEmitter,
    ) {
        assert_eq!(
            result
                .logs
                .iter()
                .filter(|log| log.status == "success")
                .count(),
            10
        );
        assert_eq!(
            result
                .logs
                .iter()
                .filter(|log| log.status == "skipped")
                .count(),
            1
        );
        assert!(cancelled_source.exists());
        assert!(!cancelled_target.exists());
        assert!(result.logs[10].error_message.is_none());
        assert_eq!(
            progress.events().last().map(|event| event.processed),
            Some(11)
        );
        assert_eq!(progress.events().last().map(|event| event.total), Some(11));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn execute_moves_core_safe_delete_uses_journaled_trash_and_restores() {
        let root = test_dir();
        let source = root.join("trash-me.txt");
        let original = b"temporary";
        let original_hash = blake3::hash(original);
        fs::write(&source, original).expect("write source");
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .expect("open temporary source");
        file.set_times(
            fs::FileTimes::new()
                .set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60)),
        )
        .expect("age temporary source");

        let result = execute_moves_core(ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "trash-preview".to_string(),
                file_id: source.to_string_lossy().into_owned(),
                operation_type: "move_to_trash".to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: "Recycle Bin".to_string(),
                old_name: "trash-me.txt".to_string(),
                new_name: "trash-me.txt".to_string(),
                is_executable: Some(true),
            }],
        });
        assert_eq!(result.logs[0].status, "success");
        assert!(result.logs[0].error_message.is_none());
        assert_eq!(result.logs[0].operation_type, "move_to_trash");
        let trashed = PathBuf::from(&result.logs[0].path_after);
        assert!(trashed.to_string_lossy().contains(".zen-canvas-trash"));
        assert!(!source.exists());
        assert_eq!(
            blake3::hash(&fs::read(&trashed).expect("read Safe Trash copy")),
            original_hash
        );
        assert!(result.logs[0].can_restore);
        assert_eq!(result.logs[0].restore_status, "not_restored");

        let restored = restore_moves_core(RestoreMovesRequest {
            logs: result.logs.clone(),
        });
        assert_eq!(restored.restored, 1);
        assert_eq!(restored.failed, 0);
        assert_eq!(restored.logs[0].restore_status, "restored");
        assert!(source.exists());
        assert_eq!(
            blake3::hash(&fs::read(&source).expect("read restored source")),
            original_hash
        );
        assert!(!trashed.exists());
        drop(file);
    }

    #[test]
    fn cleanup_rejects_windows_system_directory_without_filesystem_access() {
        let error = ensure_general_file_operation_allowed_for_os(
            Path::new("C:/Windows/System32"),
            "windows",
        )
        .expect_err("Windows system directories must be rejected lexically");

        assert!(error.contains("protected system location"));
    }

    #[test]
    fn execute_moves_core_does_not_trash_when_operation_is_blocked() {
        let root = test_dir();
        let source = root.join("blocked.txt");
        fs::write(&source, "keep").expect("write source");

        let result = execute_moves_core(ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "trash-blocked".to_string(),
                file_id: source.to_string_lossy().into_owned(),
                operation_type: "move_to_trash".to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: "Recycle Bin".to_string(),
                old_name: "blocked.txt".to_string(),
                new_name: "blocked.txt".to_string(),
                is_executable: Some(false),
            }],
        });

        assert_eq!(result.logs[0].status, "skipped");
        assert!(source.exists());
    }

    #[test]
    fn cleanup_execution_forbidden_rejects_empty_root_and_symlink() {
        assert!(crate::storage_analyzer::is_cleanup_execution_forbidden(
            Path::new(""),
            None
        ));
        assert!(crate::storage_analyzer::is_cleanup_execution_forbidden(
            Path::new("C:/"),
            None
        ));

        let root = test_dir();
        let target = root.join("target.txt");
        let link = root.join("link.txt");
        fs::write(&target, "target").expect("write target");
        if create_file_symlink_for_test(&target, &link).is_ok() {
            assert!(crate::storage_analyzer::is_cleanup_execution_forbidden(
                &link, None
            ));
        }
    }

    #[test]
    fn restore_moves_core_does_not_restore_move_to_trash_logs() {
        let root = test_dir();
        let source = root.join("already-trashed.txt");
        let log = OperationLogDto {
            id: "trash-log".to_string(),
            batch_id: "trash-batch".to_string(),
            operation_type: "move_to_trash".to_string(),
            source_path: source.to_string_lossy().into_owned(),
            target_path: "Recycle Bin".to_string(),
            old_name: "already-trashed.txt".to_string(),
            new_name: "already-trashed.txt".to_string(),
            status: "success".to_string(),
            error_message: None,
            created_at: "1".to_string(),
            can_undo: false,
            path_before: source.to_string_lossy().into_owned(),
            path_after: "Recycle Bin".to_string(),
            name_before: "already-trashed.txt".to_string(),
            name_after: "already-trashed.txt".to_string(),
            can_restore: false,
            restored_at: None,
            restore_status: "unavailable".to_string(),
            restore_error: Some("Restore from system trash".to_string()),
            source_size: None,
            source_modified_ns: None,
            source_platform_file_id: None,
            source_platform_volume_id: None,
            source_quick_hash: None,
            source_full_hash: None,
            target_platform_file_id: None,
            target_platform_volume_id: None,
            target_full_hash: None,
            source_claim_path: None,
            operation_phase: "completed".to_string(),
            claim_created_at: None,
            claim_platform_file_id: None,
            claim_platform_volume_id: None,
            claim_full_hash: None,
            restore_claim_path: None,
            restore_phase: "idle".to_string(),
            restore_claim_created_at: None,
            restore_claim_platform_file_id: None,
            restore_claim_platform_volume_id: None,
            restore_claim_full_hash: None,
        };

        let restored = restore_moves_core(RestoreMovesRequest { logs: vec![log] });

        assert_eq!(restored.restored, 0);
        assert_eq!(restored.failed, 0);
        assert_eq!(restored.logs[0].restore_status, "unavailable");
        assert!(restored.logs[0]
            .restore_error
            .as_deref()
            .unwrap_or("")
            .contains("system trash"));
    }

    #[test]
    fn execute_preview_operation_marks_move_cancellation_as_skipped() {
        let root = test_dir();
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, "hello").expect("write source");
        let cancel_flag = AtomicBool::new(true);
        let operation = preview_operation(1, &source, &target);

        let log =
            execute_preview_operation("batch-cancel", "123", 0, &operation, Some(&cancel_flag));

        assert_eq!(log.status, "skipped");
        assert!(log.error_message.is_none());
        assert!(!log.can_undo);
        assert!(!log.can_restore);
        assert!(source.exists());
        assert!(!target.exists());
    }

    #[test]
    fn restore_moves_core_restores_successful_move_log() {
        let root = test_dir();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source dir");
        fs::create_dir_all(&target_dir).expect("target dir");

        let source = source_dir.join("sample.txt");
        let target = target_dir.join("sample.txt");
        fs::write(&source, "hello").expect("write source");

        let executed = execute_moves_core(ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "op-1".to_string(),
                file_id: "file-1".to_string(),
                operation_type: "move".to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: target.to_string_lossy().into_owned(),
                old_name: "sample.txt".to_string(),
                new_name: "sample.txt".to_string(),
                is_executable: Some(true),
            }],
        });

        let restored = restore_moves_core(RestoreMovesRequest {
            logs: executed.logs.clone(),
        });

        assert!(source.exists());
        assert!(!target.exists());
        assert_eq!(restored.restored, 1);
        assert_eq!(restored.failed, 0);
        assert_eq!(restored.logs.len(), 1);
        assert_eq!(restored.logs[0].restore_status, "restored");
        assert!(!restored.logs[0].can_restore);
        assert!(restored.logs[0].restored_at.is_some());
    }

    #[test]
    fn windows_shortcut_move_and_restore_preserve_name_bytes_and_hash() {
        let root = test_dir();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source dir");
        fs::create_dir_all(&target_dir).expect("target dir");

        let source = source_dir.join("Install_Package.lnk");
        let target = target_dir.join("Install_Package.lnk");
        let bytes = b"Windows shortcut fixture bytes\0\x01\x02".to_vec();
        let hash = blake3::hash(&bytes);
        fs::write(&source, &bytes).expect("write shortcut fixture");
        let executed = execute_moves_core(ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "op-shortcut".to_string(),
                file_id: "file-shortcut".to_string(),
                operation_type: "move".to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: target.to_string_lossy().into_owned(),
                old_name: "Install_Package.lnk".to_string(),
                new_name: "Install_Package.lnk".to_string(),
                is_executable: Some(true),
            }],
        });

        assert_eq!(executed.logs[0].status, "success");
        assert!(!source.exists());
        assert!(target.to_string_lossy().ends_with(".lnk"));
        assert_eq!(fs::read(&target).expect("read moved shortcut"), bytes);
        assert_eq!(
            blake3::hash(&fs::read(&target).expect("hash moved shortcut")),
            hash
        );

        let restored = restore_moves_core(RestoreMovesRequest {
            logs: executed.logs,
        });
        assert_eq!(restored.restored, 1);
        assert!(source.exists());
        assert!(!target.exists());
        assert_eq!(fs::read(&source).expect("read restored shortcut"), bytes);
        assert_eq!(
            blake3::hash(&fs::read(&source).expect("hash restored shortcut")),
            hash
        );
    }

    #[test]
    fn restore_blocks_a_replaced_operation_target() {
        let root = test_dir();
        let source = root.join("before.txt");
        let target = root.join("after.txt");
        fs::write(&source, "hello").expect("write source");
        let executed = execute_moves_core(ExecuteMovesRequest {
            operations: vec![preview_operation(0, &source, &target)],
        });
        fs::remove_file(&target).expect("remove moved target");
        fs::write(&target, "world").expect("write same-size replacement");

        let restored = restore_moves_core(RestoreMovesRequest {
            logs: executed.logs,
        });

        assert!(!source.exists());
        assert_eq!(
            fs::read_to_string(&target).expect("read replacement"),
            "world"
        );
        assert_eq!(restored.restored, 0);
        assert_eq!(restored.failed, 1);
        assert_eq!(restored.logs[0].restore_status, "manual_review");
        assert!(restored.logs[0]
            .restore_error
            .as_deref()
            .unwrap_or_default()
            .starts_with("restore_source_identity_mismatch:"));
    }

    #[test]
    fn restore_blocks_legacy_operation_logs_without_identity() {
        with_environmental_retry(
            "restore_blocks_legacy_operation_logs_without_identity",
            || {
                let root = test_dir();
                let source = root.join("legacy-before.txt");
                let target = root.join("legacy-after.txt");
                fs::write(&source, "legacy").expect("write source");
                let mut executed = execute_moves_core(ExecuteMovesRequest {
                    operations: vec![preview_operation(0, &source, &target)],
                });
                if let Some(signature) = environmental_failure_signature(&executed.logs) {
                    let _ = fs::remove_dir_all(&root);
                    return Err(signature);
                }
                let log = &mut executed.logs[0];
                log.source_size = None;
                log.source_modified_ns = None;
                log.source_platform_file_id = None;
                log.source_quick_hash = None;
                log.target_platform_file_id = None;

                let restored = restore_moves_core(RestoreMovesRequest {
                    logs: executed.logs,
                });

                assert!(!source.exists());
                assert!(target.exists());
                assert_eq!(restored.logs[0].restore_status, "manual_review");
                Ok(())
            },
        );
    }

    #[test]
    fn restore_moves_core_marks_remaining_logs_canceled_when_cancelled() {
        with_environmental_retry(
            "restore_moves_core_marks_remaining_logs_canceled_when_cancelled",
            || {
                let root = test_dir();
                let source_dir = root.join("source");
                let target_dir = root.join("target");
                fs::create_dir_all(&source_dir).expect("source dir");
                fs::create_dir_all(&target_dir).expect("target dir");
                let operations = (0..11)
                    .map(|index| {
                        let source = source_dir.join(format!("restore-{index}.txt"));
                        let target = target_dir.join(format!("restore-{index}.txt"));
                        fs::write(&source, "hello").expect("write source");
                        preview_operation(index, &source, &target)
                    })
                    .collect::<Vec<_>>();
                let executed = execute_moves_core(ExecuteMovesRequest { operations });
                if let Some(signature) = environmental_failure_signature(&executed.logs) {
                    let _ = fs::remove_dir_all(&root);
                    return Err(signature);
                }
                let canceled_log = executed.logs[10].clone();
                let cancel_flag = Arc::new(AtomicBool::new(false));
                let progress =
                    RecordingOperationProgressEmitter::cancel_after(10, Arc::clone(&cancel_flag));

                let restored = restore_moves_core_with_progress(
                    RestoreMovesRequest {
                        logs: executed.logs.clone(),
                    },
                    Arc::clone(&cancel_flag),
                    &progress,
                );
                if let Some(signature) = environmental_failure_signature(&restored.logs) {
                    let _ = fs::remove_dir_all(&root);
                    return Err(signature);
                }

                assert_eq!(restored.restored, 10);
                assert_eq!(restored.failed, 0);
                assert_eq!(restored.logs[10].restore_status, "canceled");
                assert!(restored.logs[10].restore_error.is_none());
                assert!(!PathBuf::from(&canceled_log.path_before).exists());
                assert!(PathBuf::from(&canceled_log.path_after).exists());
                assert_eq!(
                    progress.events().last().map(|event| event.processed),
                    Some(11)
                );
                assert_eq!(progress.events().last().map(|event| event.total), Some(11));
                Ok(())
            },
        );
    }

    #[test]
    fn restore_cancellation_distinguishes_preclaim_and_claimed_states() {
        let root = test_dir();
        let source = root.join("cancel-before-claim.txt");
        let target = root.join("cancel-before-claim-moved.txt");
        fs::write(&source, "hello").expect("write source");
        let executed = execute_moves_core(ExecuteMovesRequest {
            operations: vec![preview_operation(0, &source, &target)],
        });

        let mut before_claim = executed.logs[0].clone();
        before_claim.restore_status = "pending".to_string();
        before_claim.restore_phase = "prepared".to_string();
        before_claim.restore_claim_path = Some(
            root.join(".zen-canvas-claim-before-cancel")
                .to_string_lossy()
                .into_owned(),
        );
        let canceled_before_claim = mark_restore_canceled(&before_claim);
        assert_eq!(canceled_before_claim.status, "success");
        assert_eq!(canceled_before_claim.restore_status, "canceled");
        assert_eq!(canceled_before_claim.restore_phase, "rolled_back");
        assert!(canceled_before_claim.can_restore);
        assert!(canceled_before_claim.restore_claim_path.is_none());
        assert!(canceled_before_claim.restore_error.is_none());

        let mut after_claim = before_claim;
        after_claim.restore_phase = "source_claimed".to_string();
        after_claim.restore_claim_path = Some(
            root.join(".zen-canvas-claim-after-cancel")
                .to_string_lossy()
                .into_owned(),
        );
        let canceled_after_claim = mark_restore_canceled(&after_claim);
        assert_eq!(canceled_after_claim.status, "manual_review");
        assert_eq!(canceled_after_claim.restore_status, "manual_review");
        assert_eq!(canceled_after_claim.restore_phase, "source_claimed");
        assert!(!canceled_after_claim.can_restore);
        assert_eq!(
            canceled_after_claim.restore_claim_path,
            after_claim.restore_claim_path
        );
        assert!(canceled_after_claim
            .restore_error
            .as_deref()
            .is_some_and(|error| error.contains("restore_pending_reconciliation")));
    }

    #[test]
    fn restore_moves_refuses_to_overwrite_original_path() {
        let root = test_dir();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source dir");
        fs::create_dir_all(&target_dir).expect("target dir");

        let source = source_dir.join("sample.txt");
        let target = target_dir.join("sample.txt");
        fs::write(&source, "hello").expect("write source");

        let executed = execute_moves_core(ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "op-1".to_string(),
                file_id: "file-1".to_string(),
                operation_type: "move".to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: target.to_string_lossy().into_owned(),
                old_name: "sample.txt".to_string(),
                new_name: "sample.txt".to_string(),
                is_executable: Some(true),
            }],
        });

        fs::write(&source, "new file").expect("write conflicting source");
        let restored = restore_moves_core(RestoreMovesRequest {
            logs: executed.logs.clone(),
        });

        assert_eq!(
            fs::read_to_string(&source).expect("read conflict"),
            "new file"
        );
        assert!(target.exists());
        assert_eq!(restored.restored, 0);
        assert_eq!(restored.failed, 1);
        assert_eq!(restored.logs[0].restore_status, "failed");
        assert!(restored.logs[0]
            .restore_error
            .as_deref()
            .unwrap_or_default()
            .contains("Target file already exists"));
    }
    #[test]
    fn restore_moves_restores_successful_rename_log() {
        let root = test_dir();
        fs::create_dir_all(&root).expect("root dir");

        let source = root.join("old-name.txt");
        let renamed = root.join("new-name.txt");
        fs::write(&source, "hello").expect("write source");

        let executed = execute_moves_core(ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "op-1".to_string(),
                file_id: "file-1".to_string(),
                operation_type: "rename".to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: renamed.to_string_lossy().into_owned(),
                old_name: "old-name.txt".to_string(),
                new_name: "new-name.txt".to_string(),
                is_executable: Some(true),
            }],
        });

        assert!(!source.exists());
        assert!(renamed.exists());

        let restored = restore_moves_core(RestoreMovesRequest {
            logs: executed.logs.clone(),
        });

        assert!(source.exists());
        assert!(!renamed.exists());
        assert_eq!(restored.restored, 1);
        assert_eq!(restored.logs[0].restore_status, "restored");
    }

    #[test]
    fn execute_moves_updates_file_record_after_rename() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("old-name.txt");
        let renamed = root.join("new-name.txt");
        fs::write(&source, "hello").expect("write source");
        insert_indexed_file(&db, &source, "old-name.txt", "txt");

        let result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: "op-rename".to_string(),
                    file_id: source.to_string_lossy().into_owned(),
                    operation_type: "rename".to_string(),
                    source_path: source.to_string_lossy().into_owned(),
                    target_path: renamed.to_string_lossy().into_owned(),
                    old_name: "old-name.txt".to_string(),
                    new_name: "new-name.txt".to_string(),
                    is_executable: Some(true),
                }],
            },
        )
        .expect("execute moves with persistence");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(result.logs[0].status, "success");
        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].name, "new-name.txt");
        assert_eq!(page.files[0].path, canonical_test_path(&renamed));
        assert_eq!(page.files[0].id, canonical_test_path(&renamed));
        assert_eq!(page.files[0].extension, "txt");
        assert_eq!(page.files[0].suggested_action, "Keep");
        assert!(!page.files[0].requires_confirmation);
    }

    #[cfg(windows)]
    fn replace_source_after_journal_hook(
        point: crate::fs_safety::source_claim::ClaimTestPoint,
        source: &Path,
        _claim: &Path,
    ) {
        if point == crate::fs_safety::source_claim::ClaimTestPoint::AfterJournalPreparedBeforeClaim
        {
            fs::write(source, b"replacement after journal").expect("replacement source");
        }
    }

    #[cfg(windows)]
    #[test]
    fn pending_journal_source_replacement_is_manual_review_and_never_moves_replacement() {
        let _serial = crate::fs_safety::source_claim::lock_claim_test_hooks();
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, b"original").expect("source");
        crate::fs_safety::source_claim::set_claim_test_hook(Some(
            replace_source_after_journal_hook,
        ));
        let result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![preview_operation(0, &source, &target)],
            },
        )
        .expect("journaled execution");
        crate::fs_safety::source_claim::set_claim_test_hook(None);

        assert_eq!(result.logs[0].status, "failed");
        assert_eq!(result.logs[0].operation_phase, "rolled_back");
        assert!(!target.exists());
        assert_eq!(
            fs::read(&source).expect("replacement source"),
            b"replacement after journal"
        );
        let logs = db.get_operation_logs(Some(10)).expect("operation logs");
        assert_eq!(logs[0].operation_phase, "rolled_back");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_claimed_phase_persistence_failure_rolls_back_before_target_commit() {
        with_environmental_retry(
            "source_claimed_phase_persistence_failure_rolls_back_before_target_commit",
            || {
                let db_path = test_db_path();
                let db = Database::open(&db_path).expect("open database");
                let conn = rusqlite::Connection::open(&db_path).expect("open trigger connection");
                conn.execute_batch(
                    r#"
                    CREATE TRIGGER reject_source_claimed_phase
                    BEFORE UPDATE OF operation_phase ON operation_logs
                    WHEN NEW.operation_phase = 'source_claimed'
                    BEGIN
                        SELECT RAISE(ABORT, 'injected source_claimed persistence failure');
                    END;
                    "#,
                )
                .expect("install phase failure trigger");
                drop(conn);
                let root = test_dir();
                let source = root.join("source.txt");
                let target = root.join("target.txt");
                fs::write(&source, b"phase gated source").expect("source");

                let result = execute_moves_with_persistence(
                    &db,
                    ExecuteMovesRequest {
                        operations: vec![preview_operation(0, &source, &target)],
                    },
                );
                // Environmental modes mirror the target_committed sibling: a sharing
                // violation before the trigger fires yields Ok with a failed log, or
                // surfaces through the batch error string.
                let environmental = match &result {
                    Ok(ok_result) => environmental_failure_signature(&ok_result.logs),
                    Err(message) if message.contains("os error 32") => {
                        Some(format!("batch error: {message}"))
                    }
                    Err(_) => None,
                };
                if let Some(signature) = environmental {
                    let _ = fs::remove_dir_all(&root);
                    let _ = fs::remove_file(&db_path);
                    return Err(signature);
                }

                assert!(result.is_err());
                assert_eq!(
                    fs::read(&source).expect("rolled back source"),
                    b"phase gated source"
                );
                assert!(!target.exists());
                assert!(!fs::read_dir(&root)
                    .expect("fixture entries")
                    .filter_map(Result::ok)
                    .any(|entry| entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".zen-canvas-claim-")));
                assert_eq!(
                    db.get_pending_operation_logs().expect("pending logs").len(),
                    1
                );
                assert_eq!(
                    reconcile_pending_operation_journal(&db).expect("reconcile journal"),
                    1
                );
                let reconciled = db.get_operation_logs(Some(1)).expect("reconciled logs");
                assert_eq!(reconciled[0].status, "failed");
                assert_eq!(reconciled[0].operation_phase, "rolled_back");
                let _ = fs::remove_dir_all(&root);
                let _ = fs::remove_file(&db_path);
                Ok(())
            },
        );
    }

    #[test]
    fn target_committed_phase_persistence_failure_records_manual_review_without_rollback() {
        with_environmental_retry(
            "target_committed_phase_persistence_failure_records_manual_review_without_rollback",
            || {
                let db_path = test_db_path();
                let db = Database::open(&db_path).expect("open database");
                let conn = rusqlite::Connection::open(&db_path).expect("open trigger connection");
                conn.execute_batch(
                    r#"
                    CREATE TRIGGER reject_target_committed_phase
                    BEFORE UPDATE OF operation_phase ON operation_logs
                    WHEN NEW.operation_phase = 'target_committed'
                    BEGIN
                        SELECT RAISE(ABORT, 'injected target_committed persistence failure');
                    END;
                    "#,
                )
                .expect("install phase failure trigger");
                drop(conn);
                let root = test_dir();
                let source = root.join("source.txt");
                let target = root.join("target.txt");
                fs::write(&source, b"committed source").expect("source");

                let result = execute_moves_with_persistence(
                    &db,
                    ExecuteMovesRequest {
                        operations: vec![preview_operation(0, &source, &target)],
                    },
                );
                // Environmental modes: the move fails before reaching target_committed
                // (sharing violation / identity mismatch) so the injected trigger never
                // fires and the batch returns Ok with a failed log — or the sharing
                // violation surfaces through the batch error string itself.
                let environmental = match &result {
                    Ok(ok_result) => environmental_failure_signature(&ok_result.logs),
                    Err(message) if message.contains("os error 32") => {
                        Some(format!("batch error: {message}"))
                    }
                    Err(_) => None,
                };
                if let Some(signature) = environmental {
                    let _ = fs::remove_dir_all(&root);
                    let _ = fs::remove_file(&db_path);
                    return Err(signature);
                }

                assert!(result.is_err());
                assert!(!source.exists());
                assert_eq!(
                    fs::read(&target).expect("committed target"),
                    b"committed source"
                );
                let pending = db.get_pending_operation_logs().expect("pending logs");
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].operation_phase, "source_claimed");
                assert_eq!(
                    reconcile_pending_operation_journal(&db).expect("reconcile"),
                    1
                );
                let persisted = db.get_operation_logs(Some(1)).expect("operation logs");
                if let Some(signature) = environmental_failure_signature(&persisted) {
                    let _ = fs::remove_dir_all(&root);
                    let _ = fs::remove_file(&db_path);
                    return Err(signature);
                }
                assert_eq!(persisted[0].status, "success");
                assert_eq!(persisted[0].operation_phase, "completed");
                let _ = fs::remove_dir_all(&root);
                let _ = fs::remove_file(&db_path);
                Ok(())
            },
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn completed_phase_waits_for_final_log_persistence_and_reconciles_after_injected_failure() {
        let db_path = test_db_path();
        let db = Database::open(&db_path).expect("open database");
        let root = test_dir();
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::write(&source, b"final log boundary").expect("source");

        set_operation_test_fault(Some(
            OperationTestFaultPoint::AfterCompletedPhaseBeforeFinalLogPersist,
        ));
        let result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![preview_operation(0, &source, &target)],
            },
        );

        assert!(result.is_err());
        assert!(!source.exists());
        assert_eq!(
            fs::read(&target).expect("committed target"),
            b"final log boundary"
        );

        let pending = db
            .get_pending_operation_logs()
            .expect("pending operation logs");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, "pending");
        assert_eq!(pending[0].operation_phase, "completed");

        assert_eq!(
            reconcile_pending_operation_journal(&db).expect("reconcile"),
            1
        );
        let reconciled = db.get_operation_logs(Some(1)).expect("reconciled logs");
        assert_eq!(reconciled[0].status, "success");
        assert_eq!(reconciled[0].operation_phase, "completed");
        assert!(db
            .get_pending_operation_logs()
            .expect("pending after reconcile")
            .is_empty());

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn execute_moves_updates_fts_after_rename() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("old-name.txt");
        let renamed = root.join("new-report.txt");
        fs::write(&source, "hello").expect("write source");
        insert_indexed_file(&db, &source, "old-name.txt", "txt");

        execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: "op-rename".to_string(),
                    file_id: source.to_string_lossy().into_owned(),
                    operation_type: "rename".to_string(),
                    source_path: source.to_string_lossy().into_owned(),
                    target_path: renamed.to_string_lossy().into_owned(),
                    old_name: "old-name.txt".to_string(),
                    new_name: "new-report.txt".to_string(),
                    is_executable: Some(true),
                }],
            },
        )
        .expect("execute moves with persistence");

        let new_results = db.search_files("new-report", Some(10)).expect("search new");
        let old_results = db.search_files("old-name", Some(10)).expect("search old");
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].name, "new-report.txt");
        assert_eq!(new_results[0].path, canonical_test_path(&renamed));
        assert!(old_results
            .iter()
            .all(|result| result.path != normalize_path(&source)));
    }

    #[test]
    fn execute_moves_updates_file_record_after_move() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source dir");
        fs::create_dir_all(&target_dir).expect("target dir");
        let source = source_dir.join("a.txt");
        let target = target_dir.join("a.txt");
        fs::write(&source, "hello").expect("write source");
        insert_indexed_file(&db, &source, "a.txt", "txt");

        execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: "op-move".to_string(),
                    file_id: source.to_string_lossy().into_owned(),
                    operation_type: "move".to_string(),
                    source_path: source.to_string_lossy().into_owned(),
                    target_path: target.to_string_lossy().into_owned(),
                    old_name: "a.txt".to_string(),
                    new_name: "a.txt".to_string(),
                    is_executable: Some(true),
                }],
            },
        )
        .expect("execute moves with persistence");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].path, canonical_test_path(&target));
        assert_eq!(page.files[0].id, canonical_test_path(&target));
    }

    #[test]
    fn execute_moves_does_not_fail_when_file_record_missing() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source dir");
        fs::create_dir_all(&target_dir).expect("target dir");
        let source = source_dir.join("missing-record.txt");
        let target = target_dir.join("missing-record.txt");
        fs::write(&source, "hello").expect("write source");

        let result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: "op-missing-record".to_string(),
                    file_id: source.to_string_lossy().into_owned(),
                    operation_type: "move".to_string(),
                    source_path: source.to_string_lossy().into_owned(),
                    target_path: target.to_string_lossy().into_owned(),
                    old_name: "missing-record.txt".to_string(),
                    new_name: "missing-record.txt".to_string(),
                    is_executable: Some(true),
                }],
            },
        )
        .expect("execute moves with persistence");
        let logs = db.get_operation_logs(Some(10)).expect("operation logs");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(result.logs[0].status, "success");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].id, result.logs[0].id);
        assert_eq!(page.total, 0);
        assert!(target.exists());
    }

    #[test]
    fn restore_moves_updates_file_record_after_move_restore() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source dir");
        fs::create_dir_all(&target_dir).expect("target dir");
        let source = source_dir.join("a.txt");
        let target = target_dir.join("a.txt");
        fs::write(&source, "hello").expect("write source");
        insert_indexed_file(&db, &source, "a.txt", "txt");

        let executed = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: "op-move".to_string(),
                    file_id: source.to_string_lossy().into_owned(),
                    operation_type: "move".to_string(),
                    source_path: source.to_string_lossy().into_owned(),
                    target_path: target.to_string_lossy().into_owned(),
                    old_name: "a.txt".to_string(),
                    new_name: "a.txt".to_string(),
                    is_executable: Some(true),
                }],
            },
        )
        .expect("execute moves with persistence");

        restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: executed.logs.clone(),
            },
        )
        .expect("restore moves with persistence");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].path, normalize_path(&source));
        assert_eq!(page.files[0].id, normalize_path(&source));
        assert_eq!(page.files[0].name, "a.txt");
        assert_eq!(page.files[0].extension, "txt");
        assert_eq!(page.files[0].suggested_action, "Keep");
        assert!(!page.files[0].requires_confirmation);
    }

    #[test]
    fn restore_moves_updates_file_record_after_rename_restore() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("old-name.txt");
        let renamed = root.join("new-name.txt");
        fs::write(&source, "hello").expect("write source");
        insert_indexed_file(&db, &source, "old-name.txt", "txt");

        let executed = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: "op-rename".to_string(),
                    file_id: source.to_string_lossy().into_owned(),
                    operation_type: "rename".to_string(),
                    source_path: source.to_string_lossy().into_owned(),
                    target_path: renamed.to_string_lossy().into_owned(),
                    old_name: "old-name.txt".to_string(),
                    new_name: "new-name.txt".to_string(),
                    is_executable: Some(true),
                }],
            },
        )
        .expect("execute moves with persistence");

        let after_execute = db.get_paged_files(Some(10), Some(0), None).expect("page");
        assert_eq!(after_execute.files[0].name, "new-name.txt");

        restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: executed.logs.clone(),
            },
        )
        .expect("restore moves with persistence");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].name, "old-name.txt");
        assert_eq!(page.files[0].path, normalize_path(&source));
        assert_eq!(page.files[0].id, normalize_path(&source));
        assert_eq!(page.files[0].extension, "txt");
    }

    #[test]
    fn restore_moves_updates_fts_after_restore() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source = root.join("old-report.txt");
        let renamed = root.join("new-report.txt");
        fs::write(&source, "hello").expect("write source");
        insert_indexed_file(&db, &source, "old-report.txt", "txt");

        let executed = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: "op-rename".to_string(),
                    file_id: source.to_string_lossy().into_owned(),
                    operation_type: "rename".to_string(),
                    source_path: source.to_string_lossy().into_owned(),
                    target_path: renamed.to_string_lossy().into_owned(),
                    old_name: "old-report.txt".to_string(),
                    new_name: "new-report.txt".to_string(),
                    is_executable: Some(true),
                }],
            },
        )
        .expect("execute moves with persistence");
        assert_eq!(
            db.search_files("new-report", Some(10))
                .expect("search after execute")
                .len(),
            1
        );

        restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: executed.logs.clone(),
            },
        )
        .expect("restore moves with persistence");
        let old_results = db
            .search_files("old-report", Some(10))
            .expect("search old after restore");
        let new_results = db
            .search_files("new-report", Some(10))
            .expect("search new after restore");

        assert_eq!(old_results.len(), 1);
        assert_eq!(old_results[0].path, normalize_path(&source));
        assert_eq!(old_results[0].name, "old-report.txt");
        assert!(new_results
            .iter()
            .all(|result| result.path != normalize_path(&renamed)));
    }

    #[test]
    fn restore_moves_does_not_fail_when_file_record_missing() {
        let db = Database::open(test_db_path()).expect("open database");
        let root = test_dir();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        fs::create_dir_all(&source_dir).expect("source dir");
        fs::create_dir_all(&target_dir).expect("target dir");
        let source = source_dir.join("missing-record.txt");
        let target = target_dir.join("missing-record.txt");
        fs::write(&source, "hello").expect("write source");

        let executed = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![OperationPreviewRequest {
                    id: "op-missing-record".to_string(),
                    file_id: source.to_string_lossy().into_owned(),
                    operation_type: "move".to_string(),
                    source_path: source.to_string_lossy().into_owned(),
                    target_path: target.to_string_lossy().into_owned(),
                    old_name: "missing-record.txt".to_string(),
                    new_name: "missing-record.txt".to_string(),
                    is_executable: Some(true),
                }],
            },
        )
        .expect("execute moves with persistence");

        let restored = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: executed.logs.clone(),
            },
        )
        .expect("restore moves with persistence");
        let logs = db.get_operation_logs(Some(10)).expect("operation logs");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(restored.restored, 1);
        assert_eq!(restored.logs[0].restore_status, "restored");
        assert_eq!(logs[0].restore_status, "restored");
        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].path, normalize_path(&source));
        assert_eq!(page.files[0].name, "missing-record.txt");
        assert!(source.exists());
    }

    #[cfg(windows)]
    #[test]
    fn build_reveal_command_selects_file_with_windows_explorer() {
        let command = build_reveal_command(Path::new("C:/Users/example/Documents/sample.txt"))
            .expect("reveal command");

        assert_eq!(command.program, "explorer");
        assert_eq!(
            command.args,
            vec!["/select,C:\\Users\\example\\Documents\\sample.txt"]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn build_reveal_command_selects_file_with_macos_open() {
        let command = build_reveal_command(Path::new("/Users/example/Documents/sample.txt"))
            .expect("reveal command");

        assert_eq!(command.program, "/usr/bin/open");
        assert_eq!(
            command.args,
            vec!["-R", "/Users/example/Documents/sample.txt"]
        );
    }

    fn create_authoritative_move_fixture() -> (Database, PathBuf, PathBuf, PathBuf, PathBuf, String)
    {
        let db_path = test_db_path();
        let db = Database::open(&db_path).expect("open database");
        let root = test_dir();
        let source = root.join("source.txt");
        let target_dir = root.join("organized");
        fs::create_dir_all(&target_dir).expect("target dir");
        fs::write(&source, "hello").expect("source");
        insert_indexed_file(&db, &source, "source.txt", "txt");
        set_indexed_move_suggestion(&db, &source, &target_dir, "source.txt");
        let file_id = source.to_string_lossy().into_owned();
        (db, db_path, root, source, target_dir, file_id)
    }

    fn set_indexed_move_suggestion(
        db: &Database,
        source: &Path,
        target_dir: &Path,
        suggested_name: &str,
    ) {
        let metadata = fs::metadata(source).expect("source metadata");
        let mtime = metadata
            .modified()
            .expect("source mtime")
            .duration_since(UNIX_EPOCH)
            .expect("unix mtime")
            .as_secs() as i64;
        let file_id = source.to_string_lossy().into_owned();
        let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE files SET suggested_action = 'Move', suggested_target_path = ?2, suggested_name = ?3, confidence = 0.95, risk_level = 'Normal', requires_confirmation = 0, size = ?4, mtime = ?5 WHERE id = ?1",
            rusqlite::params![
                file_id,
                normalize_path(target_dir),
                suggested_name,
                metadata.len() as i64,
                mtime
            ],
        )
        .expect("set move suggestion");
    }

    fn current_operation_preview(db: &Database, file_id: &str) -> OperationPreviewDto {
        db.get_operation_previews_by_file_ids(&[file_id.to_string()])
            .expect("operation preview query")
            .into_iter()
            .next()
            .expect("current operation preview")
    }

    fn selection_for_preview(
        preview: &OperationPreviewDto,
        new_name: Option<&str>,
    ) -> OperationSelection {
        let fingerprint = preview
            .operation_fingerprint
            .clone()
            .expect("authoritative operation fingerprint");
        OperationSelection {
            id: preview.id.clone(),
            file_id: preview.file_id.clone(),
            operation_fingerprint: fingerprint.clone(),
            expected_revision: fingerprint,
            new_name: new_name.map(str::to_string),
            destination_directory: None,
        }
    }

    fn operation_log_count(db: &Database) -> usize {
        db.get_operation_logs(Some(1000))
            .expect("operation logs")
            .len()
    }

    fn cleanup_preview_fixture(db: Database, db_path: PathBuf, root: PathBuf) {
        drop(db);
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_file(db_path);
    }

    fn test_dir() -> PathBuf {
        let sequence = TEST_FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "zen-canvas-file-op-test-{}-{sequence}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("test dir");
        dir
    }

    fn test_db_path() -> PathBuf {
        let sequence = TEST_FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "zen-canvas-file-op-db-test-{}-{sequence}-{nonce}.sqlite3",
            std::process::id()
        ))
    }

    fn insert_indexed_file(db: &Database, path: &Path, name: &str, extension: &str) {
        let path = path.to_string_lossy().into_owned();
        db.insert_file(InsertFileRequest {
            id: path.clone(),
            path,
            name: name.to_string(),
            extension: extension.to_string(),
            size: 5,
            mtime: 1_900_000_000,
            ctime: 0,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert indexed file");
    }

    fn preview_operation(index: usize, source: &Path, target: &Path) -> OperationPreviewRequest {
        let name = source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("sample.txt")
            .to_string();
        OperationPreviewRequest {
            id: format!("op-{index}"),
            file_id: source.to_string_lossy().into_owned(),
            operation_type: "move".to_string(),
            source_path: source.to_string_lossy().into_owned(),
            target_path: target.to_string_lossy().into_owned(),
            old_name: name.clone(),
            new_name: name,
            is_executable: Some(true),
        }
    }

    fn create_file_symlink_for_test(target: &Path, link: &Path) -> io::Result<()> {
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, link)
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
    }

    fn create_directory_symlink_for_test(target: &Path, link: &Path) -> io::Result<()> {
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link)
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
    }

    fn canonical_test_path(path: &Path) -> String {
        normalize_path(&fs::canonicalize(path).expect("canonical test path"))
            .trim_start_matches("//?/")
            .to_string()
    }

    struct RecordingOperationProgressEmitter {
        events: std::cell::RefCell<Vec<OperationProgressPayload>>,
        cancel_after: u64,
        cancel_flag: Arc<AtomicBool>,
    }

    impl RecordingOperationProgressEmitter {
        fn cancel_after(cancel_after: u64, cancel_flag: Arc<AtomicBool>) -> Self {
            Self {
                events: std::cell::RefCell::new(Vec::new()),
                cancel_after,
                cancel_flag,
            }
        }

        fn events(&self) -> Vec<OperationProgressPayload> {
            self.events.borrow().clone()
        }
    }

    impl OperationProgressEmitter for RecordingOperationProgressEmitter {
        fn emit_progress(&self, payload: OperationProgressPayload) {
            if payload.processed >= self.cancel_after {
                self.cancel_flag.store(true, Ordering::Relaxed);
            }
            self.events.borrow_mut().push(payload);
        }
    }

    struct CancelAfterFirstRead<'a, R> {
        inner: R,
        cancel_flag: &'a AtomicBool,
        reads: usize,
    }

    impl<'a, R> CancelAfterFirstRead<'a, R> {
        fn new(inner: R, cancel_flag: &'a AtomicBool) -> Self {
            Self {
                inner,
                cancel_flag,
                reads: 0,
            }
        }
    }

    impl<R: Read> Read for CancelAfterFirstRead<'_, R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let bytes_read = self.inner.read(buf)?;
            if bytes_read > 0 && self.reads == 0 {
                self.cancel_flag.store(true, Ordering::Relaxed);
            }
            self.reads += 1;
            Ok(bytes_read)
        }
    }
}
