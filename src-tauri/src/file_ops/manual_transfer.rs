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
const EXTERNAL_PATH_LIMIT: usize = 256;
const EXTERNAL_TREE_ENTRY_LIMIT: usize = 5000;
const EXTERNAL_TREE_DEPTH_LIMIT: usize = 64;
#[derive(Default)]
pub(crate) struct TransferState {
    sources: HashMap<String, Source>,
    previews: HashMap<String, TransferPlan>,
}
#[derive(Clone)]
struct Source {
    ticket: TransferTicket,
    file_id: String,
    identity: FileIdentityFingerprint,
    relative_path: PathBuf,
    tree_guard: Option<TreeGuard>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct TreeGuard {
    root: PathBuf,
    fingerprint: String,
}
struct TransferPlan {
    ticket_ids: Vec<String>,
    operations: Vec<OperationPreviewRequest>,
    trees: Vec<TreeGuard>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferTicket {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub relative_path: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRequest {
    #[serde(default)]
    pub refs: Vec<EntryRef>,
    pub selection: Option<LibrarySelectionV1>,
    #[serde(default)]
    pub release_ids: Vec<String>,
    #[serde(default)]
    pub external_paths: Vec<String>,
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransferAction {
    Copy,
    Move,
    Rename,
    Trash,
    CreateDirectory,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferPreviewRequest {
    pub ticket_ids: Vec<String>,
    pub action: TransferAction,
    pub destination_directory: String,
    #[serde(default)]
    pub destination_entry: Option<crate::file_workspace::BrowseEntryRef>,
    #[serde(default)]
    pub new_name: Option<String>,
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
        if request.refs.len() > LIMIT
            || request.release_ids.len() > LIMIT
            || request.external_paths.len() > EXTERNAL_PATH_LIMIT
        {
            return Err("transfer_selection_limit_1000".into());
        }
        if !request.external_paths.is_empty()
            && (!request.refs.is_empty() || request.selection.is_some())
        {
            return Err("transfer_selection_invalid".into());
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
            let name = utf8_file_name(&path)?;
            let relative_path = PathBuf::from(&name);
            let ticket = TransferTicket {
                id: new_job_id("transfer-source"),
                name,
                path: normalize_path(&path),
                is_directory: path.is_dir(),
                relative_path: utf8_relative_path(&relative_path)?,
            };
            captured.push(Source {
                ticket,
                file_id,
                identity,
                relative_path,
                tree_guard: None,
            });
        }
        if !request.external_paths.is_empty() {
            captured.extend(capture_external_sources(request.external_paths)?);
        }
        let mut state = self
            .inner
            .transfers
            .lock()
            .map_err(|_| "transfer_state_unavailable")?;
        let release_ids = request.release_ids.into_iter().collect::<HashSet<_>>();
        for id in &release_ids {
            state.sources.remove(id);
        }
        if !release_ids.is_empty() {
            state
                .previews
                .retain(|_, plan| plan.ticket_ids.iter().all(|id| !release_ids.contains(id)));
        }
        // Ticket IDs are ownership handles. A second capture of the same
        // physical source must receive a different ID so releasing a short-
        // lived drag cannot invalidate a basket, clipboard, or dialog ticket.
        let mut unique = HashSet::new();
        captured.retain(|source| {
            unique.insert((
                source.ticket.path.clone(),
                format!("{:?}", source.identity),
                source.ticket.relative_path.clone(),
                source
                    .tree_guard
                    .as_ref()
                    .map(|guard| guard.fingerprint.clone()),
            ))
        });
        let additions = captured.len();
        if state.sources.len() + additions > LIMIT {
            return Err("transfer_selection_limit_1000".into());
        }
        let mut tickets = Vec::new();
        for source in captured {
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
        if request.action == TransferAction::CreateDirectory {
            if !request.ticket_ids.is_empty()
                || request.new_name.is_none()
                || request.destination_entry.is_some()
                || !request.keep_both.is_empty()
                || !request.skip.is_empty()
            {
                return Err("transfer_create_directory_request_invalid".into());
            }
            let directory = self.resolve_transfer_destination(&request)?;
            let name = request
                .new_name
                .as_deref()
                .ok_or("transfer_create_directory_request_invalid")?;
            let operation = create_directory_operation(&directory, name)?;
            let target = Path::new(&operation.target_path);
            if path_entry_exists(target)? {
                return Ok(TransferPreview {
                    id: new_job_id("transfer-preview"),
                    destination_directory: normalize_path(&directory),
                    rows: vec![TransferRow {
                        ticket_id: "create-directory".into(),
                        preview: None,
                        error: Some("transfer_target_exists".into()),
                        skipped: false,
                    }],
                });
            }
            let id = new_job_id("transfer-preview");
            let mut state = self
                .inner
                .transfers
                .lock()
                .map_err(|_| "transfer_state_unavailable")?;
            if state.previews.len() >= 4 {
                state.previews.clear();
            }
            state.previews.insert(
                id.clone(),
                TransferPlan {
                    ticket_ids: Vec::new(),
                    operations: vec![operation.clone()],
                    trees: Vec::new(),
                },
            );
            return Ok(TransferPreview {
                id,
                destination_directory: normalize_path(&directory),
                rows: vec![TransferRow {
                    ticket_id: "create-directory".into(),
                    preview: Some(preview_dto(&operation)),
                    error: None,
                    skipped: false,
                }],
            });
        }
        if request.action == TransferAction::Rename
            && (request.ticket_ids.len() != 1
                || request.new_name.is_none()
                || request.destination_entry.is_some()
                || !request.keep_both.is_empty()
                || !request.skip.is_empty())
        {
            return Err("transfer_rename_selection_invalid".into());
        }
        if request.action == TransferAction::Trash
            && (request.new_name.is_some()
                || request.destination_entry.is_some()
                || !request.keep_both.is_empty()
                || !request.skip.is_empty())
        {
            return Err("transfer_trash_request_invalid".into());
        }
        if !matches!(
            request.action,
            TransferAction::Rename | TransferAction::Trash
        ) && request.new_name.is_some()
        {
            return Err("transfer_new_name_unexpected".into());
        }
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
        let directory = self.resolve_transfer_destination(&request)?;
        ensure_general_file_operation_allowed(&directory)?;
        let mut trees = Vec::new();
        let mut checked_trees = HashMap::<PathBuf, Result<(), String>>::new();
        for source in sources.iter().filter_map(|(_, source)| source.as_ref()) {
            if let Some(guard) = &source.tree_guard {
                if !checked_trees.contains_key(&guard.root) {
                    checked_trees.insert(
                        guard.root.clone(),
                        validate_tree_guard(guard)
                            .map_err(|_| "transfer_source_tree_changed".into()),
                    );
                    trees.push(guard.clone());
                }
            }
        }
        let mut operations = Vec::new();
        let mut rows = Vec::new();
        let mut reserved = HashSet::new();
        for (id, source) in sources {
            let skipped = request.skip.contains(&id);
            let source = match source {
                Some(source) => source,
                None => {
                    rows.push(TransferRow {
                        ticket_id: id,
                        preview: None,
                        error: Some("transfer_reference_expired".into()),
                        skipped: false,
                    });
                    continue;
                }
            };
            let validation = (|| {
                if source
                    .tree_guard
                    .as_ref()
                    .is_some_and(|guard| checked_trees.get(&guard.root).is_some_and(Result::is_err))
                {
                    return Err("transfer_source_tree_changed".into());
                }
                if file_namespace_fingerprint(Path::new(&source.ticket.path))? != source.identity {
                    return Err("transfer_source_changed".into());
                }
                if source
                    .tree_guard
                    .as_ref()
                    .is_some_and(|guard| directory.starts_with(&guard.root))
                {
                    return Err("transfer_directory_into_itself".into());
                }
                Ok(())
            })();
            if let Err(error) = validation {
                rows.push(TransferRow {
                    ticket_id: id,
                    preview: None,
                    error: Some(error),
                    skipped: false,
                });
                continue;
            }
            // Skip is an explicit per-item conflict decision. It has no file
            // system side effect, so validate ticket identity/tree first, then
            // avoid target-name checks that would turn Skip back into a conflict.
            if skipped {
                rows.push(TransferRow {
                    ticket_id: id,
                    preview: None,
                    error: None,
                    skipped: true,
                });
                continue;
            }
            let result = (|| {
                let mut name = if request.action == TransferAction::Rename {
                    request
                        .new_name
                        .clone()
                        .ok_or("transfer_rename_selection_invalid")?
                } else {
                    source.ticket.name.clone()
                };
                if request.action == TransferAction::Rename && name == source.ticket.name {
                    return Err("transfer_rename_name_unchanged".into());
                }
                let source_path = Path::new(&source.ticket.path);
                let target_for_name = |name: &str| -> Result<PathBuf, String> {
                    if request.action == TransferAction::Rename {
                        validate_safe_file_name(name)?;
                        return Ok(source_path
                            .parent()
                            .ok_or("transfer_source_invalid")?
                            .join(name));
                    }
                    let parent = source
                        .relative_path
                        .parent()
                        .unwrap_or_else(|| Path::new(""));
                    let relative = parent.join(name);
                    validate_relative_target(&relative)?;
                    Ok(directory.join(relative))
                };
                let conflict = |name: &str, reserved: &HashSet<String>| -> Result<bool, String> {
                    let target = target_for_name(name)?;
                    if reserved.contains(&transfer_target_key(&target)) {
                        return Ok(true);
                    }
                    match std::fs::symlink_metadata(&target) {
                        Ok(_) => Ok(true),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                        Err(error) => Err(error.to_string()),
                    }
                };
                if request.keep_both.contains(&id)
                    && matches!(request.action, TransferAction::Copy | TransferAction::Move)
                    && conflict(&name, &reserved)?
                {
                    let original = Path::new(&source.ticket.name);
                    let stem = original.file_stem().unwrap_or_default().to_string_lossy();
                    let extension = original.extension();
                    for number in 2..=10000 {
                        name = format!(
                            "{stem} ({number}){}",
                            extension
                                .map(|x| format!(".{}", x.to_string_lossy()))
                                .unwrap_or_default()
                        );
                        if !conflict(&name, &reserved)? {
                            break;
                        }
                    }
                }
                if request.action != TransferAction::Trash && conflict(&name, &reserved)? {
                    return Err("transfer_target_exists".into());
                }
                let operation_directory = if matches!(
                    request.action,
                    TransferAction::Rename | TransferAction::Trash
                ) {
                    source_path.parent().ok_or("transfer_source_invalid")?
                } else {
                    &directory
                };
                if request.action == TransferAction::Rename {
                    let source_parent = operation_directory
                        .canonicalize()
                        .map_err(|error| error.to_string())?;
                    let requested_parent = directory
                        .canonicalize()
                        .map_err(|error| error.to_string())?;
                    if source_parent != requested_parent {
                        return Err("transfer_rename_same_directory_required".into());
                    }
                }
                let target = if request.action == TransferAction::Trash {
                    source_path.to_path_buf()
                } else {
                    target_for_name(&name)?
                };
                let operation = make_operation(
                    &source.file_id,
                    source_path,
                    operation_directory,
                    &target,
                    request.action,
                )?;
                let preview = preview_dto(&operation);
                if matches!(request.action, TransferAction::Copy | TransferAction::Move) {
                    reserved.insert(transfer_target_key(&target));
                }
                operations.push(operation);
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
            state.previews.insert(
                id.clone(),
                TransferPlan {
                    ticket_ids: request.ticket_ids,
                    operations,
                    trees,
                },
            );
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
        let plan = self
            .inner
            .transfers
            .lock()
            .map_err(|_| "transfer_state_unavailable")?
            .previews
            .remove(&request.preview_id)
            .ok_or("operation_preview_stale")?;
        for guard in &plan.trees {
            validate_tree_guard(guard)?;
        }
        for operation in &plan.operations {
            revalidate(operation)?;
        }
        Ok(ExecuteMovesRequest {
            operations: plan.operations,
        })
    }

    fn resolve_transfer_destination(
        &self,
        request: &TransferPreviewRequest,
    ) -> Result<PathBuf, String> {
        let directory = match &request.destination_entry {
            Some(entry) => {
                if !request.destination_directory.trim().is_empty() {
                    return Err("transfer_destination_invalid".into());
                }
                let resolved = self
                    .inner
                    .browse
                    .resolve_current_entry(entry)
                    .map_err(|error| error.to_string())?;
                if resolved.kind != crate::file_workspace::browse::BrowseEntryKind::Directory {
                    return Err("transfer_destination_invalid".into());
                }
                crate::fs_safety::capture_source_object_identity(&resolved.path)
                    .map_err(|error| error.to_string())?;
                resolved.path
            }
            None => {
                if request.destination_directory.trim().is_empty() {
                    return Err("transfer_destination_invalid".into());
                }
                PathBuf::from(&request.destination_directory)
            }
        };
        if !directory.is_absolute() {
            return Err("transfer_destination_invalid".into());
        }
        VerifiedDirectory::open_existing(&directory).map_err(|error| error.to_string())?;
        let directory = directory
            .canonicalize()
            .map_err(|error| error.to_string())?;
        ensure_general_file_operation_allowed(&directory)?;
        Ok(directory)
    }
}

fn action_name(action: TransferAction) -> &'static str {
    match action {
        TransferAction::Copy => "copy",
        TransferAction::Move => "move",
        TransferAction::Rename => "rename",
        TransferAction::Trash => "move_to_trash",
        TransferAction::CreateDirectory => "create_directory",
    }
}

fn path_entry_exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn validate_transfer_source_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(FileOpError::RelativePath.to_string());
    }
    if path.to_string_lossy().contains('\0')
        || path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(FileOpError::UnsafePathTraversal.to_string());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            FileOpError::SourceMissing.to_string()
        } else {
            FileOpError::Io(error).to_string()
        }
    })?;
    if metadata.is_dir() {
        let directory =
            VerifiedDirectory::open_existing(path).map_err(|error| error.to_string())?;
        ensure_general_file_operation_allowed(directory.path())?;
        Ok(directory.path().to_path_buf())
    } else {
        validate_source_path(path)
    }
}

fn snapshot(
    file_id: &str,
    source: &Path,
    destination_root: &Path,
    target: &Path,
    action: TransferAction,
) -> Result<(String, DirectoryIdentity), String> {
    validate_safe_file_name(
        target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("transfer_target_invalid")?,
    )?;
    let source = if matches!(action, TransferAction::Rename | TransferAction::Trash) {
        validate_namespace_entry_path(source)?
    } else {
        validate_transfer_source_path(source)?
    };
    let source_is_directory = source.is_dir();
    if cfg!(windows) && source_is_directory && action == TransferAction::Move {
        return Err("transfer_windows_directory_move_unsupported".into());
    }
    ensure_not_safe_trash_operation(&source)?;
    let tree_fingerprint =
        if source_is_directory && matches!(action, TransferAction::Copy | TransferAction::Move) {
            Some(directory_tree_snapshot(&source)?)
        } else {
            None
        };
    let destination = VerifiedDirectory::open_existing(destination_root)
        .map_err(|e| format!("transfer_destination_identity:{e}"))?;
    VerifiedDirectory::open_existing(source.parent().ok_or("transfer_source_invalid")?)
        .map_err(|e| format!("transfer_source_parent_identity:{e}"))?;
    ensure_general_file_operation_allowed(destination_root)?;
    ensure_not_safe_trash_operation(destination_root)?;
    let identity = file_namespace_fingerprint(&source)?;
    let canonical_root = destination_root
        .canonicalize()
        .map_err(|error| format!("transfer_destination_identity:{error}"))?;
    if matches!(action, TransferAction::Copy | TransferAction::Move)
        && source_is_directory
        && canonical_root.starts_with(&source)
    {
        return Err("transfer_directory_into_itself".into());
    }
    let nearest_identity = if matches!(action, TransferAction::Rename | TransferAction::Trash) {
        let canonical_source_parent = source
            .parent()
            .ok_or("transfer_source_invalid")?
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if canonical_source_parent != canonical_root {
            return Err("transfer_rename_same_directory_required".into());
        }
        if action == TransferAction::Rename && !target.starts_with(destination_root) {
            return Err("transfer_target_outside_destination".into());
        }
        destination.identity().clone()
    } else {
        if !target.starts_with(destination_root) {
            return Err("transfer_target_outside_destination".into());
        }
        let target_parent = target.parent().ok_or("transfer_target_invalid")?;
        let (_nearest_parent, nearest_identity) =
            verify_existing_target_route(destination_root, &canonical_root, target_parent)
                .map_err(|error| format!("transfer_target_parent_identity:{error}"))?;
        nearest_identity
    };
    if action == TransferAction::Move && source.parent() == Some(canonical_root.as_path()) {
        return Err("transfer_move_same_directory".into());
    }
    let target_identity = if action == TransferAction::Trash {
        "safe-trash-target"
    } else {
        "filesystem-target"
    };
    let target_path_identity = if action == TransferAction::Trash {
        target_identity.to_string()
    } else {
        normalize_path(target)
    };
    let digest = blake3::hash(
        format!(
            "transfer:v4:{file_id}:{}:{}:{action:?}:{identity:?}:{tree_fingerprint:?}:{}:{:?}",
            normalize_path(&source),
            target_path_identity,
            normalize_path(&canonical_root),
            destination.identity(),
        )
        .as_bytes(),
    );
    let destination_path = normalize_path(&canonical_root);
    Ok((
        format!(
            "op-transfer-v4-{}-{}",
            digest.to_hex(),
            encode_path_for_transfer_id(&destination_path)
        ),
        nearest_identity,
    ))
}
fn make_operation(
    file_id: &str,
    source: &Path,
    directory: &Path,
    target: &Path,
    action: TransferAction,
) -> Result<OperationPreviewRequest, String> {
    let (id, _) = snapshot(file_id, source, directory, target, action)?;
    let target_path = if action == TransferAction::Trash {
        safe_trash_operation_target(source, &id)
    } else {
        normalize_path(target)
    };
    Ok(OperationPreviewRequest {
        id,
        file_id: file_id.into(),
        operation_type: action_name(action).into(),
        source_path: normalize_path(source),
        target_path,
        old_name: source
            .file_name()
            .ok_or("transfer_source_invalid")?
            .to_string_lossy()
            .into_owned(),
        new_name: target
            .file_name()
            .ok_or("transfer_target_invalid")?
            .to_string_lossy()
            .into_owned(),
        is_executable: Some(true),
    })
}
pub(crate) fn revalidate(operation: &OperationPreviewRequest) -> Result<DirectoryIdentity, String> {
    if operation.operation_type == "create_directory" {
        let current =
            create_directory_operation(Path::new(&operation.source_path), &operation.new_name)?;
        if current.id != operation.id
            || current.target_path != operation.target_path
            || path_entry_exists(Path::new(&current.target_path))?
            || operation.file_id != super::RECOVERY_ACTION_FILE_ID
        {
            return Err("operation_preview_stale".into());
        }
        return Ok(
            VerifiedDirectory::open_existing(Path::new(&operation.source_path))
                .map_err(|error| error.to_string())?
                .identity()
                .clone(),
        );
    }
    let action = match operation.operation_type.as_str() {
        "copy" => TransferAction::Copy,
        "move" => TransferAction::Move,
        "rename" => TransferAction::Rename,
        "move_to_trash" => TransferAction::Trash,
        _ => return Err("operation_preview_stale".into()),
    };
    let destination_root = decode_transfer_root(&operation.id)?;
    let source = Path::new(&operation.source_path);
    if action == TransferAction::Trash
        && operation.target_path != safe_trash_operation_target(source, &operation.id)
    {
        return Err("operation_preview_stale".into());
    }
    let snapshot_target = if action == TransferAction::Trash {
        source.to_path_buf()
    } else {
        PathBuf::from(&operation.target_path)
    };
    let (id, identity) = snapshot(
        &operation.file_id,
        source,
        &destination_root,
        &snapshot_target,
        action,
    )?;
    if id != operation.id {
        return Err("operation_preview_stale".into());
    }
    Ok(identity)
}

fn encode_path_for_transfer_id(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(path.len() * 2);
    for byte in path.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_transfer_root(id: &str) -> Result<PathBuf, String> {
    let payload = id
        .strip_prefix("op-transfer-v4-")
        .ok_or("operation_preview_stale")?;
    let (digest, encoded_path) = payload.split_once('-').ok_or("operation_preview_stale")?;
    if digest.len() != 64 || encoded_path.is_empty() || encoded_path.len() % 2 != 0 {
        return Err("operation_preview_stale".into());
    }
    let mut bytes = Vec::with_capacity(encoded_path.len() / 2);
    for pair in encoded_path.as_bytes().chunks_exact(2) {
        let high = (pair[0] as char)
            .to_digit(16)
            .ok_or("operation_preview_stale")?;
        let low = (pair[1] as char)
            .to_digit(16)
            .ok_or("operation_preview_stale")?;
        bytes.push(((high << 4) | low) as u8);
    }
    let path = String::from_utf8(bytes).map_err(|_| "operation_preview_stale")?;
    let root = PathBuf::from(path);
    if !root.is_absolute()
        || root
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err("operation_preview_stale".into());
    }
    Ok(root)
}
pub(crate) fn execute(
    operation: &OperationPreviewRequest,
    context: OperationExecutionContext<'_>,
) -> Result<FileOperationResult, FileMutationError> {
    // Directory copy requires a content-bound claim. Capture it before
    // revalidating the preview, then let preview revalidation bind the source
    // back to the accepted tree. The subsequent source claim detects any
    // content change after that revalidation without publishing a replacement.
    let directory_copy_identity = if operation.operation_type == "copy"
        && std::fs::symlink_metadata(&operation.source_path)
            .is_ok_and(|metadata| metadata.file_type().is_dir())
    {
        Some(
            crate::fs_safety::capture_identity(
                Path::new(&operation.source_path),
                context.cancel_flag,
            )
            .map_err(|error| FileMutationError::Validation(error.to_string()))?,
        )
    } else {
        None
    };
    revalidate(operation)?;
    if context
        .cancel_flag
        .is_some_and(|flag| flag.load(Ordering::Acquire))
    {
        return Err(FileMutationError::Atomic(
            crate::fs_safety::AtomicMoveError::Cancelled,
        ));
    }
    if operation.operation_type == "create_directory" {
        return create_directory_with_safety(
            operation.source_path.clone(),
            operation.new_name.clone(),
            context.expected_identity,
            context.phase_observer,
            context.actual_path_observer,
        );
    }
    if operation.operation_type == "rename" {
        return rename_file_with_identity(
            operation.source_path.clone(),
            operation.new_name.clone(),
            context.expected_identity,
            context.cancel_flag,
            context.planned_claim_path,
            context.phase_observer,
            context.actual_path_observer,
        );
    }
    if operation.operation_type == "move_to_trash" {
        return move_to_trash_with_safety(
            operation.source_path.clone(),
            context.app_data_dir,
            context.expected_identity,
            context.planned_claim_path,
            &operation.id,
            context.phase_observer,
            context.actual_path_observer,
        );
    }
    let source = validate_transfer_source_path(Path::new(&operation.source_path))?;
    let target_path = Path::new(&operation.target_path);
    let target_parent_path = target_path
        .parent()
        .ok_or_else(|| FileMutationError::Validation("transfer_target_invalid".into()))?;
    crate::fs_safety::create_directory_chain_no_links(target_parent_path)
        .map_err(|error| FileMutationError::Validation(error.to_string()))?;
    let current_parent_identity = VerifiedDirectory::open_existing(target_parent_path)
        .map_err(|error| FileMutationError::Validation(error.to_string()))?
        .identity()
        .clone();
    let target = validate_target_path(target_path)?;
    if operation.operation_type == "copy" {
        let expected_identity = directory_copy_identity
            .as_ref()
            .or(context.expected_identity);
        crate::fs_safety::atomic_move::atomic_copy_with_verified_destination(
            &source,
            &target,
            "copy",
            expected_identity,
            context.planned_claim_path,
            context.cancel_flag,
            context.phase_observer,
            context.actual_path_observer,
            Some(&current_parent_identity),
        )
        .map_err(FileMutationError::Atomic)?;
    } else {
        crate::fs_safety::atomic_move::atomic_move_with_verified_destination_for_transfer(
            &source,
            &target,
            context.expected_identity,
            context.planned_claim_path,
            context.cancel_flag,
            context.phase_observer,
            crate::fs_safety::atomic_move::AtomicMoveOperation::Move,
            context.actual_path_observer,
            Some(&current_parent_identity),
        )
        .map_err(FileMutationError::Atomic)?;
    }
    Ok(FileOperationResult {
        operation: operation.operation_type.clone(),
        source_path: normalize_path(&source),
        target_path: normalize_path(&target),
    })
}

fn verify_existing_target_route(
    destination_root: &Path,
    canonical_root: &Path,
    target_parent: &Path,
) -> Result<(PathBuf, DirectoryIdentity), String> {
    if !target_parent.starts_with(destination_root) {
        return Err("transfer_target_outside_destination".into());
    }
    let relative = target_parent
        .strip_prefix(destination_root)
        .map_err(|_| "transfer_target_outside_destination")?;
    let mut current = canonical_root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("transfer_target_invalid".into());
        };
        let child = current.join(name);
        match std::fs::symlink_metadata(&child) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err("transfer_target_parent_invalid".into());
                }
                crate::fs_safety::capture_source_object_identity(&child)
                    .map_err(|error| error.to_string())?;
                current = child.canonicalize().map_err(|error| error.to_string())?;
                if !current.starts_with(canonical_root) {
                    return Err("transfer_target_route_changed".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    let verified = VerifiedDirectory::open_existing(&current).map_err(|error| error.to_string())?;
    Ok((verified.path().to_path_buf(), verified.identity().clone()))
}

fn create_directory_operation(
    directory: &Path,
    name: &str,
) -> Result<OperationPreviewRequest, String> {
    validate_safe_file_name(name)?;
    let parent = VerifiedDirectory::open_existing(directory).map_err(|error| error.to_string())?;
    ensure_general_file_operation_allowed(parent.path())?;
    ensure_not_safe_trash_operation(parent.path())?;
    let target = parent.path().join(name);
    ensure_not_safe_trash_operation(&target)?;
    let digest = blake3::hash(
        format!(
            "create-directory:v1:{}:{:?}:{name}",
            normalize_path(parent.path()),
            parent.identity()
        )
        .as_bytes(),
    );
    Ok(OperationPreviewRequest {
        id: format!("op-transfer-{}", digest.to_hex()),
        file_id: super::RECOVERY_ACTION_FILE_ID.into(),
        operation_type: "create_directory".into(),
        source_path: normalize_path(parent.path()),
        target_path: normalize_path(&target),
        old_name: name.into(),
        new_name: name.into(),
        is_executable: Some(true),
    })
}

fn safe_trash_operation_target(source: &Path, operation_id: &str) -> String {
    let operation_key = blake3::hash(format!("{}\0{}", source.display(), operation_id).as_bytes())
        .to_hex()
        .to_string();
    normalize_path(
        &source
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .join(".zen-canvas-trash")
            .join("operations")
            .join(&operation_key[..24])
            .join(source.file_name().unwrap_or_default()),
    )
}

fn capture_external_sources(paths: Vec<String>) -> Result<Vec<Source>, String> {
    let mut captured = Vec::new();
    let mut seen = HashSet::new();
    let mut operation_count = 0usize;
    for candidate in paths {
        if candidate.trim().is_empty() {
            return Err("transfer_source_invalid".into());
        }
        let candidate = PathBuf::from(candidate);
        if !candidate.is_absolute()
            || candidate.to_string_lossy().contains('\0')
            || candidate
                .components()
                .any(|component| component == std::path::Component::ParentDir)
        {
            return Err("transfer_source_invalid".into());
        }
        crate::fs_safety::capture_source_object_identity(&candidate)
            .map_err(|error| error.to_string())?;
        let path = candidate
            .canonicalize()
            .map_err(|error| error.to_string())?;
        ensure_general_file_operation_allowed(&path)?;
        if path.is_file() {
            let name = utf8_file_name(&path)?;
            let identity = file_namespace_fingerprint(&path)?;
            let relative_path = PathBuf::from(&name);
            let dedupe_key = (normalize_path(&path), utf8_relative_path(&relative_path)?);
            if !seen.insert(dedupe_key) {
                continue;
            }
            captured.push(make_captured_source(
                path,
                relative_path,
                identity,
                None,
                false,
            )?);
            operation_count += 1;
        } else if path.is_dir() {
            let root_name = utf8_file_name(&path)?;
            let fingerprint = directory_tree_snapshot(&path)?;
            let guard = TreeGuard {
                root: path.clone(),
                fingerprint,
            };
            let relative_path = PathBuf::from(&root_name);
            let dedupe_key = (normalize_path(&path), utf8_relative_path(&relative_path)?);
            if !seen.insert(dedupe_key) {
                continue;
            }
            let identity = file_namespace_fingerprint(&path)?;
            captured.push(make_captured_source(
                path,
                relative_path,
                identity,
                Some(guard),
                true,
            )?);
            operation_count += 1;
        } else {
            return Err("transfer_source_invalid".into());
        }
        if operation_count > LIMIT {
            return Err("transfer_selection_limit_1000".into());
        }
    }
    Ok(captured)
}

fn make_captured_source(
    path: PathBuf,
    relative_path: PathBuf,
    identity: FileIdentityFingerprint,
    tree_guard: Option<TreeGuard>,
    is_directory: bool,
) -> Result<Source, String> {
    if identity.platform_file_id.is_none() || identity.platform_volume_id.is_none() {
        return Err("transfer_identity_unavailable".into());
    }
    validate_relative_target(&relative_path)?;
    let name = utf8_file_name(&path)?;
    let relative_path_display = utf8_relative_path(&relative_path)?;
    let ticket = TransferTicket {
        id: new_job_id("transfer-source"),
        name,
        path: normalize_path(&path),
        is_directory,
        relative_path: relative_path_display,
    };
    Ok(Source {
        ticket,
        file_id: super::RECOVERY_ACTION_FILE_ID.into(),
        identity,
        relative_path,
        tree_guard,
    })
}

fn directory_tree_snapshot(root: &Path) -> Result<String, String> {
    let root_identity = crate::fs_safety::capture_source_object_identity(root)
        .map_err(|error| error.to_string())?;
    if !root.is_dir() {
        return Err("transfer_source_invalid".into());
    }
    let root_namespace = file_namespace_fingerprint(root)?;
    let mut records = vec![format!(".:directory:{root_identity:?}:{root_namespace:?}")];
    let mut entry_count = 0usize;
    let mut file_count = 0usize;
    collect_directory_tree(
        root,
        root,
        0,
        &mut entry_count,
        &mut records,
        &mut file_count,
    )?;
    records.sort();
    let mut hasher = blake3::Hasher::new();
    for record in records {
        hasher.update(record.as_bytes());
        hasher.update(&[0]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn collect_directory_tree(
    root: &Path,
    directory: &Path,
    depth: usize,
    entry_count: &mut usize,
    records: &mut Vec<String>,
    file_count: &mut usize,
) -> Result<(), String> {
    if depth >= EXTERNAL_TREE_DEPTH_LIMIT {
        return Err("transfer_directory_depth_limit".into());
    }
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        *entry_count += 1;
        if *entry_count > EXTERNAL_TREE_ENTRY_LIMIT {
            return Err("transfer_directory_entry_limit".into());
        }
        let path = entry.path();
        let physical_identity = crate::fs_safety::capture_source_object_identity(&path)
            .map_err(|error| error.to_string())?;
        let identity = file_namespace_fingerprint(&path)?;
        ensure_general_file_operation_allowed(&path)?;
        let relative_path = path
            .strip_prefix(root)
            .map_err(|_| "transfer_source_invalid")?
            .to_path_buf();
        let relative_display = utf8_relative_path(&relative_path)?;
        if path.is_dir() {
            records.push(format!(
                "{relative_display}:directory:{physical_identity:?}:{identity:?}"
            ));
            collect_directory_tree(root, &path, depth + 1, entry_count, records, file_count)?;
        } else if path.is_file() {
            validate_source_path(&path)?;
            records.push(format!(
                "{relative_display}:file:{physical_identity:?}:{identity:?}"
            ));
            *file_count += 1;
            if *file_count > LIMIT {
                return Err("transfer_selection_limit_1000".into());
            }
        } else {
            return Err("transfer_source_invalid".into());
        }
    }
    Ok(())
}

fn validate_tree_guard(guard: &TreeGuard) -> Result<(), String> {
    let fingerprint = directory_tree_snapshot(&guard.root)?;
    if fingerprint == guard.fingerprint {
        Ok(())
    } else {
        Err("transfer_source_tree_changed".into())
    }
}

fn validate_relative_target(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("transfer_target_invalid".into());
    }
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("transfer_target_invalid".into());
        };
        let name = name.to_str().ok_or("transfer_source_name_unsupported")?;
        validate_safe_file_name(name)?;
    }
    Ok(())
}

fn utf8_file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| "transfer_source_name_unsupported".into())
}

fn utf8_relative_path(path: &Path) -> Result<String, String> {
    validate_relative_target(path)?;
    path.components()
        .map(|component| match component {
            std::path::Component::Normal(name) => name
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "transfer_source_name_unsupported".to_string()),
            _ => Err("transfer_target_invalid".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

fn transfer_target_key(path: &Path) -> String {
    let normalized = normalize_path(path);
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
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
