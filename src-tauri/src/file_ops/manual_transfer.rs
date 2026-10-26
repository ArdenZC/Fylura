//! Manual Library/Browse intent adapter. The operation journal/executor remains
//! the only mutation authority. Tickets are bounded Main-lifetime references,
//! not paths, managed imports, a clipboard filesystem or a second work queue.
use super::*;
use crate::db::{LibrarySelectionV1, OperationPreviewDto};
use crate::file_workspace::integration::FileWorkspaceRuntime;
use crate::file_workspace::{BrowseEntryRef, EntryRef};
use crate::fs_safety::{DirectoryIdentity, VerifiedDirectory};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const LIMIT: usize = 1000;
#[derive(Default)]
pub(crate) struct TransferState {
    sources: HashMap<String, Source>,
    previews: HashMap<String, Vec<OperationPreviewRequest>>,
}
#[derive(Clone)]
struct Source {
    ticket: TransferTicket,
    file_id: String,
    identity: FileIdentityFingerprint,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferTicket {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_directory: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRequest {
    #[serde(default)]
    pub refs: Vec<EntryRef>,
    pub selection: Option<LibrarySelectionV1>,
    #[serde(default)]
    pub release_ids: Vec<String>,
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransferAction {
    Copy,
    Move,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferPreviewRequest {
    pub ticket_ids: Vec<String>,
    pub action: TransferAction,
    pub destination_directory: String,
    #[serde(default)]
    pub keep_both: Vec<String>,
    #[serde(default)]
    pub skip: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferPreview {
    pub id: String,
    pub destination_directory: String,
    pub rows: Vec<TransferRow>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRow {
    pub ticket_id: String,
    pub preview: Option<OperationPreviewDto>,
    pub error: Option<String>,
    pub skipped: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferExecuteRequest {
    pub preview_id: String,
    pub confirmed: bool,
}

impl FileWorkspaceRuntime {
    pub(crate) fn capture_transfer(
        &self,
        mut request: CaptureRequest,
    ) -> Result<Vec<TransferTicket>, String> {
        self.ensure_live()?;
        if request.refs.len() > LIMIT || request.release_ids.len() > LIMIT {
            return Err("transfer_selection_limit_1000".into());
        }
        if let Some(selection) = request.selection {
            if !request.refs.is_empty() {
                return Err("transfer_selection_invalid".into());
            }
            request.refs = self
                .inner
                .database
                .manual_transfer_selection_ids(&selection)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|file_id| EntryRef::Managed { file_id })
                .collect();
        }
        let mut captured = Vec::new();
        // Resolve before taking the registry lock. No lock covers filesystem IO.
        for reference in request.refs {
            let (path, file_id) = match reference {
                EntryRef::Managed { file_id } => {
                    self.inner
                        .database
                        .verify_indexed_file_identity(&file_id)
                        .map_err(|e| e.to_string())?;
                    let detail = self
                        .inner
                        .database
                        .get_file_library_detail(&file_id)
                        .map_err(|e| e.to_string())?;
                    (PathBuf::from(detail.path), file_id)
                }
                EntryRef::Ephemeral {
                    browse_session_id,
                    entry_id,
                } => {
                    let reference = BrowseEntryRef::Ephemeral {
                        browse_session_id,
                        entry_id,
                    };
                    self.inner
                        .browse
                        .resolve_entry_generation(&reference)
                        .map_err(|e| e.to_string())?;
                    let resolved = self
                        .inner
                        .browse
                        .resolve_entry(&reference)
                        .map_err(|e| e.to_string())?;
                    (resolved.path, super::RECOVERY_ACTION_FILE_ID.into())
                }
            };
            let path = if cfg!(windows) && path.is_dir() {
                if !path.is_absolute() {
                    return Err("transfer_source_invalid".into());
                }
                crate::fs_safety::capture_source_object_identity(&path)
                    .map_err(|e| e.to_string())?;
                ensure_general_file_operation_allowed(&path)?;
                path.canonicalize().map_err(|e| e.to_string())?
            } else {
                validate_source_path(&path)?
            };
            VerifiedDirectory::open_existing(path.parent().ok_or("transfer_source_invalid")?)
                .map_err(|e| e.to_string())?;
            let identity = file_namespace_fingerprint(&path)?;
            if identity.platform_file_id.is_none() || identity.platform_volume_id.is_none() {
                return Err("transfer_identity_unavailable".into());
            }
            let ticket = TransferTicket {
                id: new_job_id("transfer-source"),
                name: path
                    .file_name()
                    .ok_or("transfer_source_invalid")?
                    .to_string_lossy()
                    .into_owned(),
                path: normalize_path(&path),
                is_directory: path.is_dir(),
            };
            captured.push(Source {
                ticket,
                file_id,
                identity,
            });
        }
        let mut state = self
            .inner
            .transfers
            .lock()
            .map_err(|_| "transfer_state_unavailable")?;
        for id in request.release_ids {
            state.sources.remove(&id);
        }
        let mut tickets = Vec::new();
        for source in captured {
            if let Some(old) = state.sources.values().find(|old| {
                old.ticket.path == source.ticket.path && old.identity == source.identity
            }) {
                tickets.push(old.ticket.clone());
                continue;
            }
            if state.sources.len() >= LIMIT {
                return Err("transfer_selection_limit_1000".into());
            }
            tickets.push(source.ticket.clone());
            state.sources.insert(source.ticket.id.clone(), source);
        }
        Ok(tickets)
    }

    pub(crate) fn preview_transfer(
        &self,
        request: TransferPreviewRequest,
    ) -> Result<TransferPreview, String> {
        self.ensure_live()?;
        if request.ticket_ids.is_empty() || request.ticket_ids.len() > LIMIT {
            return Err("transfer_selection_limit_1000".into());
        }
        let sources = {
            let state = self
                .inner
                .transfers
                .lock()
                .map_err(|_| "transfer_state_unavailable")?;
            request
                .ticket_ids
                .iter()
                .map(|id| (id.clone(), state.sources.get(id).cloned()))
                .collect::<Vec<_>>()
        };
        let directory = PathBuf::from(&request.destination_directory);
        let destination =
            VerifiedDirectory::open_existing(&directory).map_err(|e| e.to_string())?;
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        ensure_general_file_operation_allowed(&directory)?;
        let mut operations = Vec::new();
        let mut rows = Vec::new();
        let mut reserved = HashSet::new();
        for (id, source) in sources {
            let skipped = request.skip.contains(&id);
            let result = (|| {
                let source = source.ok_or("transfer_reference_expired")?;
                if file_namespace_fingerprint(Path::new(&source.ticket.path))? != source.identity {
                    return Err("transfer_source_changed".into());
                }
                let mut name = source.ticket.name.clone();
                let conflict = |name: &str, reserved: &HashSet<String>| -> bool {
                    directory.join(name).symlink_metadata().is_ok()
                        || reserved.contains(&name.to_lowercase())
                };
                if request.keep_both.contains(&id) {
                    let original = Path::new(&source.ticket.name);
                    let stem = if source.ticket.is_directory {
                        source.ticket.name.clone()
                    } else {
                        original
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    };
                    let extension = if source.ticket.is_directory {
                        None
                    } else {
                        original.extension()
                    };
                    for number in 2..=10000 {
                        name = format!(
                            "{stem} ({number}){}",
                            extension
                                .map(|x| format!(".{}", x.to_string_lossy()))
                                .unwrap_or_default()
                        );
                        if !conflict(&name, &reserved) {
                            break;
                        }
                    }
                }
                if conflict(&name, &reserved) {
                    return Err("transfer_target_exists".into());
                }
                let operation = make_operation(
                    &source.file_id,
                    Path::new(&source.ticket.path),
                    &directory,
                    &name,
                    request.action,
                )?;
                reserved.insert(name.to_lowercase());
                let preview = preview_dto(&operation);
                if !skipped {
                    operations.push(operation);
                }
                Ok(preview)
            })();
            rows.push(match result {
                Ok(preview) => TransferRow {
                    ticket_id: id,
                    preview: Some(preview),
                    error: None,
                    skipped,
                },
                Err(error) => TransferRow {
                    ticket_id: id,
                    preview: None,
                    error: Some(error),
                    skipped,
                },
            });
        }
        let id = new_job_id("transfer-preview");
        let mut state = self
            .inner
            .transfers
            .lock()
            .map_err(|_| "transfer_state_unavailable")?;
        // Superseded previews have no consent authority; at most four remain.
        if state.previews.len() >= 4 {
            state.previews.clear();
        }
        if rows.iter().all(|r| r.skipped || r.error.is_none()) && !operations.is_empty() {
            state.previews.insert(id.clone(), operations);
        }
        Ok(TransferPreview {
            id,
            destination_directory: normalize_path(&directory),
            rows,
        })
    }
    pub(crate) fn take_transfer(
        &self,
        request: TransferExecuteRequest,
    ) -> Result<ExecuteMovesRequest, String> {
        self.ensure_live()?;
        if !request.confirmed {
            return Err("transfer_confirmation_required".into());
        }
        let operations = self
            .inner
            .transfers
            .lock()
            .map_err(|_| "transfer_state_unavailable")?
            .previews
            .remove(&request.preview_id)
            .ok_or("operation_preview_stale")?;
        for operation in &operations {
            revalidate(operation)?;
        }
        Ok(ExecuteMovesRequest { operations })
    }
}

fn action_name(action: TransferAction) -> &'static str {
    match action {
        TransferAction::Copy => "copy",
        TransferAction::Move => "move",
    }
}
fn snapshot(
    file_id: &str,
    source: &Path,
    directory: &Path,
    name: &str,
    action: TransferAction,
) -> Result<(String, DirectoryIdentity), String> {
    validate_safe_file_name(name)?;
    if cfg!(windows) && source.is_dir() {
        return Err("transfer_windows_directory_not_supported".into());
    }
    let source = validate_source_path(source)?;
    let destination = VerifiedDirectory::open_existing(directory).map_err(|e| e.to_string())?;
    VerifiedDirectory::open_existing(source.parent().ok_or("transfer_source_invalid")?)
        .map_err(|e| e.to_string())?;
    ensure_general_file_operation_allowed(directory)?;
    let identity = file_namespace_fingerprint(&source)?;
    let canonical_directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if source.is_dir() && canonical_directory.starts_with(&source) {
        return Err("transfer_directory_into_itself".into());
    }
    if action == TransferAction::Move {
        if identity.platform_volume_id.as_deref() != Some(destination.identity().volume_id.as_str())
        {
            return Err("transfer_cross_volume_move_use_copy".into());
        }
        if source.parent() == Some(canonical_directory.as_path()) {
            return Err("transfer_move_same_directory".into());
        }
        #[cfg(target_os = "macos")]
        if !matches!(
            crate::platform::macos::strategy::select(&source, &directory.join(name)),
            crate::platform::macos::strategy::MacMutationStrategy::LocalApfs
        ) {
            return Err("manual_move_local_same_volume_required".into());
        }
    }
    let digest = blake3::hash(
        format!(
            "transfer:v1:{file_id}:{}:{}:{name}:{action:?}:{identity:?}:{:?}",
            normalize_path(&source),
            normalize_path(&canonical_directory),
            destination.identity()
        )
        .as_bytes(),
    );
    Ok((
        format!("op-transfer-{}", digest.to_hex()),
        destination.identity().clone(),
    ))
}
fn make_operation(
    file_id: &str,
    source: &Path,
    directory: &Path,
    name: &str,
    action: TransferAction,
) -> Result<OperationPreviewRequest, String> {
    let (id, _) = snapshot(file_id, source, directory, name, action)?;
    Ok(OperationPreviewRequest {
        id,
        file_id: file_id.into(),
        operation_type: action_name(action).into(),
        source_path: normalize_path(source),
        target_path: normalize_path(&directory.join(name)),
        old_name: source
            .file_name()
            .ok_or("transfer_source_invalid")?
            .to_string_lossy()
            .into_owned(),
        new_name: name.into(),
        is_executable: Some(true),
    })
}
pub(crate) fn revalidate(operation: &OperationPreviewRequest) -> Result<DirectoryIdentity, String> {
    let action = match operation.operation_type.as_str() {
        "copy" => TransferAction::Copy,
        "move" => TransferAction::Move,
        _ => return Err("operation_preview_stale".into()),
    };
    let (id, identity) = snapshot(
        &operation.file_id,
        Path::new(&operation.source_path),
        Path::new(&operation.target_path)
            .parent()
            .ok_or("operation_preview_stale")?,
        &operation.new_name,
        action,
    )?;
    if id != operation.id {
        return Err("operation_preview_stale".into());
    }
    Ok(identity)
}
pub(crate) fn execute(
    operation: &OperationPreviewRequest,
    context: OperationExecutionContext<'_>,
) -> Result<FileOperationResult, FileMutationError> {
    let parent_identity = revalidate(operation)?;
    let source = validate_source_path(Path::new(&operation.source_path))?;
    let target = validate_target_path(Path::new(&operation.target_path))?;
    if operation.operation_type == "copy" {
        crate::fs_safety::atomic_move::atomic_copy_with_verified_destination(
            &source,
            &target,
            "copy",
            context.expected_identity,
            context.planned_claim_path,
            context.cancel_flag,
            context.phase_observer,
            context.actual_path_observer,
            Some(&parent_identity),
        )
        .map_err(FileMutationError::Atomic)?;
    } else {
        crate::fs_safety::atomic_move::atomic_move_with_verified_destination(
            &source,
            &target,
            context.expected_identity,
            context.planned_claim_path,
            context.cancel_flag,
            context.phase_observer,
            crate::fs_safety::atomic_move::AtomicMoveOperation::Move,
            context.actual_path_observer,
            Some(&parent_identity),
        )
        .map_err(FileMutationError::Atomic)?;
    }
    Ok(FileOperationResult {
        operation: operation.operation_type.clone(),
        source_path: normalize_path(&source),
        target_path: normalize_path(&target),
    })
}
fn preview_dto(operation: &OperationPreviewRequest) -> OperationPreviewDto {
    OperationPreviewDto {
        id: operation.id.clone(),
        file_id: operation.file_id.clone(),
        operation_type: operation.operation_type.clone(),
        source_path: operation.source_path.clone(),
        target_path: operation.target_path.clone(),
        old_name: operation.old_name.clone(),
        new_name: operation.new_name.clone(),
        status: "pending".into(),
        risk_level: "Normal".into(),
        confidence: 1.0,
        requires_confirmation: true,
        suggested_action: operation.operation_type.clone(),
        reason: "Manual transfer; no overwrite. Revalidated before execution.".into(),
        selected_by_default: Some(true),
        is_executable: Some(true),
        editable_new_name: Some(false),
        operation_fingerprint: Some(operation.id.clone()),
        ..Default::default()
    }
}

pub(crate) async fn execute_registered<R: Runtime>(
    app: AppHandle<R>,
    runtime: FileWorkspaceRuntime,
    cancel: &OperationCancellationToken,
    request: TransferExecuteRequest,
) -> Result<ExecuteMovesResult, String> {
    let guard = cancel.begin()?;
    let cancel_flag = Arc::clone(&cancel.cancel);
    let app_data_dir = app.path().app_data_dir().ok();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let request = runtime.take_transfer(request)?;
        let paths = request
            .operations
            .iter()
            .flat_map(|op| [op.source_path.clone(), op.target_path.clone()])
            .collect::<Vec<_>>();
        let db = &runtime.inner.database;
        let emitter = TauriOperationProgressEmitter::new(app.clone());
        reconciliation::with_reconciliation_wake(&app, db, &paths, || {
            execute_moves_with_persistence_with_progress_and_app_data(
                db,
                request,
                cancel_flag,
                &emitter,
                app_data_dir,
                None,
            )
        })
    })
    .await
    .map_err(|e| format!("transfer_execution_failed:{e}"))?
}
